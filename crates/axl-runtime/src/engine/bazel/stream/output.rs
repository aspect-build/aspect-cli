//! Captured-output stream: read the Bazel child's stderr, run each record
//! through an extensible processing pipeline, and forward the surviving bytes
//! to the parent's real stderr.
//!
//! # Why this exists
//!
//! By default the Bazel child inherits the parent's stderr fd
//! (`Stdio::inherit()`), so its output reaches the terminal untouched and we
//! can't pre-process it. When a task opts into output capture, `Build::spawn`
//! hands the child a captured fd (a pipe in non-TTY contexts, a PTY master's
//! slave in interactive ones) and starts an `OutputStream` over the read end.
//!
//! # Design
//!
//! This mirrors [`super::build_event::BuildEventStream`]: a single dedicated
//! reader thread, owned by the `Build`, joined inside `Build::wait`. The
//! deliberate difference from the BES path is that we do **not** route raw
//! bytes through the unbounded [`super::broadcaster::Broadcaster`] — that
//! fan-out clones per subscriber into unbounded buffers, which is a memory
//! hazard for a high-volume byte stream. Instead the reader thread forwards
//! straight to the parent stderr with a blocking `write`, so a slow terminal
//! back-pressures the kernel pipe/PTY buffer and ultimately Bazel itself,
//! bounding memory for free. Any future best-effort side-channel to AXL (match
//! hits, dedup counts) must be bounded + drop-and-count, never the unbounded
//! broadcaster — forwarding to the terminal must never be gated on consumer
//! latency.
//!
//! # Record splitting and snappiness
//!
//! Bazel's curses progress UI rewrites in place with bare `\r` (carriage
//! return, no newline). Splitting only on `\n` would buffer the whole progress
//! region until a real newline arrived, freezing the live UI. So the loop
//! treats both `\r` and `\n` as record boundaries, and flushes the forward
//! writer once per `read()` syscall (not per newline — `LineWriter` would
//! reintroduce the freeze). The kernel already batches bytes into one read, so
//! a per-read flush is cheap and keeps output snappy.
//!
//! # Phase 1 scope
//!
//! The processing pipeline is a [`LineProcessor`] trait list, but phase 1 ships
//! only the pass-through behavior (an empty list). This is the seam where the
//! deferred line-dedup (clean up + count repeats) and pattern matchers
//! (hook-driving + hung-server detection) plug in later, additively, without
//! reshaping the loop. The `last_activity` atomic is maintained now so the
//! deferred stall/hung-detection follow-up has its substrate ready.

use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use thiserror::Error;

/// Read buffer size per `read()` syscall. One read's worth of bytes is the
/// natural flush boundary (see module docs).
const READ_BUF_SIZE: usize = 8 * 1024;

/// Hard cap on the in-flight incomplete record. A pathological producer that
/// never emits `\r`/`\n` would otherwise grow `carry` without bound; past this
/// we force-flush the partial bytes so memory stays bounded.
const MAX_CARRY_BYTES: usize = 1024 * 1024;

#[derive(Error, Debug)]
pub enum OutputStreamError {
    #[error("io error: {0}")]
    IO(#[from] std::io::Error),
}

/// A single processing stage applied to each output record before it is
/// forwarded. Returning `None` drops the record from the forwarded stream
/// (e.g. a dedup stage that collapsed a repeat); returning `Some(bytes)`
/// forwards those bytes (which may differ from the input, e.g. a `(×N)`
/// annotation appended on a count flush).
///
/// Phase 1 ships no implementors — the list is always empty and every record
/// is forwarded verbatim. The trait exists so dedup/matcher stages can be
/// added without touching the reader loop.
///
/// A `record` is the bytes between two boundaries, *excluding* the boundary
/// byte itself; the boundary (`\r` or `\n`) is forwarded separately so
/// processors operate on logical-line content, not delimiters.
pub trait LineProcessor: Send {
    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>>;
}

#[derive(Debug)]
pub struct OutputStream {
    /// Reader thread handle, in an `Option` so `join()` can `take()` it
    /// without consuming `self`.
    handle: Option<JoinHandle<Result<(), OutputStreamError>>>,

    /// Millis-since-start of the last successful read. The reader bumps this on
    /// every read that returns bytes; the deferred stall watchdog will read it
    /// to detect a hung server (no output for N seconds while the child is
    /// alive). Maintained now so that follow-up has its substrate ready.
    #[allow(dead_code)]
    last_activity_ms: Arc<AtomicU64>,
}

impl OutputStream {
    /// Spawn the reader/forwarder thread.
    ///
    /// `reader` is the read end of the captured stderr (a pipe `ChildStderr`
    /// boxed as `Read + Send`, or a PTY master). `forward` is a `Write + Send`
    /// view over the parent's real stderr (see
    /// `axl_types::stream::Writable::to_boxed_write`). `processors` is the
    /// per-record pipeline (empty in phase 1).
    ///
    /// The thread runs until the reader reaches EOF — on Unix a pipe returns a
    /// 0-length read when the write end closes, while a PTY master returns
    /// `EIO` once the child closes the slave; both are treated as clean
    /// end-of-stream. The parent must drop its copy of the captured fd /
    /// PTY slave after spawning the child, or this read never terminates.
    pub fn spawn(
        mut reader: Box<dyn Read + Send>,
        mut forward: Box<dyn Write + Send>,
        mut processors: Vec<Box<dyn LineProcessor>>,
    ) -> OutputStream {
        let last_activity_ms = Arc::new(AtomicU64::new(0));
        let thread_activity = last_activity_ms.clone();

        let handle = thread::spawn(move || -> Result<(), OutputStreamError> {
            let start = Instant::now();
            let mut buf = [0u8; READ_BUF_SIZE];
            let mut carry: Vec<u8> = Vec::with_capacity(READ_BUF_SIZE);

            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) => break, // clean EOF (pipe write end closed)
                    Ok(n) => n,
                    // PTY master read after the child closes the slave returns
                    // EIO on Linux — that is end-of-stream, not an error.
                    Err(e) if is_pty_eof(&e) => break,
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => {
                        // Flush whatever we have, then surface the error.
                        let _ = flush_carry(&mut carry, &mut processors, &mut forward);
                        let _ = forward.flush();
                        return Err(OutputStreamError::IO(e));
                    }
                };

                thread_activity.store(start.elapsed().as_millis() as u64, Ordering::Relaxed);

                carry.extend_from_slice(&buf[..n]);
                process_carry(&mut carry, &mut processors, &mut forward)?;

                // Bound the incomplete-record buffer: a producer that never
                // emits a boundary byte must not grow `carry` unboundedly.
                if carry.len() > MAX_CARRY_BYTES {
                    forward.write_all(&carry)?;
                    carry.clear();
                }

                // Flush once per read so the `\r`-driven progress UI stays live.
                forward.flush()?;
            }

            // Drain the trailing partial record (output with no final newline).
            flush_carry(&mut carry, &mut processors, &mut forward)?;
            forward.flush()?;
            Ok(())
        });

        OutputStream {
            handle: Some(handle),
            last_activity_ms,
        }
    }

    /// Wait for the reader thread to finish (after the child's captured fd has
    /// reached EOF). Called from `Build::wait` after `child.wait()`.
    pub fn join(&mut self) -> Result<(), OutputStreamError> {
        if let Some(handle) = self.handle.take() {
            match handle.join() {
                Ok(result) => result,
                Err(_) => Err(OutputStreamError::IO(std::io::Error::other(
                    "output stream thread panicked",
                ))),
            }
        } else {
            Ok(())
        }
    }

    /// Millis since stream start of the last read that returned bytes (0 if no
    /// bytes have arrived yet). Substrate for the deferred stall watchdog.
    #[allow(dead_code)]
    pub fn last_activity_ms(&self) -> u64 {
        self.last_activity_ms.load(Ordering::Relaxed)
    }
}

/// Split complete records (terminated by `\r` or `\n`) out of `carry`, run each
/// through the pipeline, and forward the survivors plus their boundary byte.
/// Leaves the trailing incomplete record in `carry`.
fn process_carry(
    carry: &mut Vec<u8>,
    processors: &mut [Box<dyn LineProcessor>],
    forward: &mut Box<dyn Write + Send>,
) -> Result<(), OutputStreamError> {
    let mut start = 0;
    let mut i = 0;
    while i < carry.len() {
        let b = carry[i];
        if b == b'\n' || b == b'\r' {
            let record = &carry[start..i];
            emit_record(record, processors, forward)?;
            forward.write_all(&[b])?;
            i += 1;
            start = i;
        } else {
            i += 1;
        }
    }
    if start > 0 {
        carry.drain(..start);
    }
    Ok(())
}

/// Forward the trailing incomplete record (no boundary byte) through the
/// pipeline. Used at EOF and on the carry-cap force-flush.
fn flush_carry(
    carry: &mut Vec<u8>,
    processors: &mut [Box<dyn LineProcessor>],
    forward: &mut Box<dyn Write + Send>,
) -> Result<(), OutputStreamError> {
    if !carry.is_empty() {
        emit_record(carry, processors, forward)?;
        carry.clear();
    }
    Ok(())
}

/// Run one record through the processor chain and forward the result. With an
/// empty chain (phase 1) this forwards the record verbatim.
fn emit_record(
    record: &[u8],
    processors: &mut [Box<dyn LineProcessor>],
    forward: &mut Box<dyn Write + Send>,
) -> Result<(), OutputStreamError> {
    if processors.is_empty() {
        forward.write_all(record)?;
        return Ok(());
    }
    let mut current = record.to_vec();
    for p in processors.iter_mut() {
        match p.process(&current) {
            Some(out) => current = out,
            None => return Ok(()), // dropped by a stage (e.g. dedup collapse)
        }
    }
    forward.write_all(&current)?;
    Ok(())
}

/// Whether an error reading a captured fd means end-of-stream rather than a
/// real failure. A PTY master read after the child closes the slave returns
/// `EIO` (raw os error 5) on Linux; macOS typically returns a 0-length read
/// instead, handled by the `Ok(0)` arm.
fn is_pty_eof(e: &std::io::Error) -> bool {
    e.raw_os_error() == Some(5) // EIO
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A `Write` sink that records everything written, shared with the test
    /// thread via `Arc<Mutex<…>>`.
    #[derive(Clone)]
    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn run(input: &[u8]) -> Vec<u8> {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let reader = std::io::Cursor::new(input.to_vec());
        let mut stream =
            OutputStream::spawn(Box::new(reader), Box::new(SharedSink(sink.clone())), vec![]);
        stream.join().unwrap();
        let out = sink.lock().unwrap().clone();
        out
    }

    #[test]
    fn passthrough_newline_delimited() {
        let input = b"line one\nline two\nline three\n";
        assert_eq!(run(input), input);
    }

    #[test]
    fn passthrough_preserves_carriage_returns() {
        // The curses-style progress stream: bare \r, no newline.
        let input = b"[1 / 9] Building\r[2 / 9] Building\r[3 / 9] Building\r";
        assert_eq!(run(input), input);
    }

    #[test]
    fn flushes_trailing_partial_record() {
        // No trailing newline — must still be forwarded.
        let input = b"no trailing newline";
        assert_eq!(run(input), input);
    }

    #[test]
    fn mixed_cr_lf() {
        let input = b"progress\rprogress\rdone\n";
        assert_eq!(run(input), input);
    }

    #[test]
    fn empty_input_eof() {
        assert_eq!(run(b""), b"");
    }

    #[test]
    fn carry_cap_force_flushes_giant_line() {
        // A record larger than MAX_CARRY_BYTES with no boundary must still be
        // forwarded in full (in chunks), never dropped or grown unbounded.
        let big = vec![b'x'; MAX_CARRY_BYTES + 4096];
        assert_eq!(run(&big), big);
    }

    /// A processor that drops records equal to the previous one and annotates
    /// a count — exercises the pipeline seam the deferred dedup will use.
    struct ConsecutiveDedup {
        prev: Option<Vec<u8>>,
    }

    impl LineProcessor for ConsecutiveDedup {
        fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
            if self.prev.as_deref() == Some(record) {
                return None;
            }
            self.prev = Some(record.to_vec());
            Some(record.to_vec())
        }
    }

    #[test]
    fn processor_can_drop_records() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let reader = std::io::Cursor::new(b"a\na\nb\n".to_vec());
        let mut stream = OutputStream::spawn(
            Box::new(reader),
            Box::new(SharedSink(sink.clone())),
            vec![Box::new(ConsecutiveDedup { prev: None })],
        );
        stream.join().unwrap();
        // Second "a" record dropped; its newline boundary is still forwarded
        // (boundaries are forwarded independently of record content).
        let out = sink.lock().unwrap().clone();
        assert_eq!(out, b"a\n\nb\n");
    }
}
