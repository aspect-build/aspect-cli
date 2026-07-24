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
//! reader thread, owned by the `Build`, joined inside `Build::wait`. Unlike the
//! BES path it does **not** fan out through the unbounded
//! [`super::broadcaster::Broadcaster`] — that clones per subscriber into
//! unbounded buffers, a memory hazard for a high-volume byte stream. Instead
//! the reader forwards straight to the parent stderr with a blocking `write`,
//! so a slow terminal back-pressures the kernel pipe/PTY buffer and ultimately
//! Bazel itself, bounding memory for free.
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
//! # Processing pipeline
//!
//! Each record passes through an ordered [`LineProcessor`] chain before it is
//! forwarded; an empty chain forwards every record verbatim. A stage can
//! transform a record, drop it (with its boundary), or hold state across
//! records and flush it at end-of-stream via [`LineProcessor::finish`].
//! Built-in stages live in [`super::processors`]: repeated-line collapsing
//! and pattern matching for hooks and health signals.

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
/// forwarded.
///
/// A `record` is the bytes between two boundaries, *excluding* the boundary
/// byte itself (`\r` or `\n`), so processors operate on logical-line content.
/// Stages run on the reader thread — they must never block, or forwarding
/// stalls.
pub trait LineProcessor: Send {
    /// Transform one record. `Some(bytes)` forwards those bytes followed by
    /// the record's original boundary; the bytes may differ from the input
    /// and may contain embedded newlines (e.g. an annotation line prepended
    /// to the record). `None` drops the record *and* its boundary — return
    /// `Some(vec![])` instead to keep a blank line.
    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>>;

    /// Called once at end-of-stream; any returned bytes are forwarded as-is
    /// (include trailing newlines as needed). Lets a stateful stage flush
    /// held state, e.g. a pending repeat count.
    fn finish(&mut self) -> Option<Vec<u8>> {
        None
    }
}

#[derive(Debug)]
pub struct OutputStream {
    /// Reader thread handle, in an `Option` so `join()` can `take()` it
    /// without consuming `self`.
    handle: Option<JoinHandle<Result<(), OutputStreamError>>>,

    /// When the stream started, paired with `last_activity_ms` to answer
    /// [`OutputStream::idle_ms`].
    started: Instant,

    /// Millis-since-start of the last read that returned bytes. The reader
    /// bumps it on every such read; `idle_ms()` derives silence duration from
    /// it so a caller can detect a stalled invocation (child alive, no output).
    last_activity_ms: Arc<AtomicU64>,
}

impl OutputStream {
    /// Spawn the reader/forwarder thread.
    ///
    /// `reader` is the read end of the captured stderr (a pipe or a PTY master).
    /// `forward` is the parent's real stderr. `processors` is the per-record
    /// pipeline; an empty list forwards every record verbatim.
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
        let started = Instant::now();
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
                // Flush it as a record through the pipeline (not raw) so a
                // processor sees the same byte stream it would for a normal
                // record.
                if carry.len() > MAX_CARRY_BYTES {
                    flush_carry(&mut carry, &mut processors, &mut forward)?;
                }

                // Flush once per read so the `\r`-driven progress UI stays live.
                forward.flush()?;
            }

            // Drain the trailing partial record (output with no final
            // newline), then let each stage flush any held state.
            flush_carry(&mut carry, &mut processors, &mut forward)?;
            for p in processors.iter_mut() {
                if let Some(out) = p.finish() {
                    forward.write_all(&out)?;
                }
            }
            forward.flush()?;
            Ok(())
        });

        OutputStream {
            handle: Some(handle),
            started,
            last_activity_ms,
        }
    }

    /// Millis since the last read that returned bytes (since stream start if
    /// none have arrived). A caller polling this alongside child liveness can
    /// detect a hung invocation.
    pub fn idle_ms(&self) -> u64 {
        let elapsed = self.started.elapsed().as_millis() as u64;
        elapsed.saturating_sub(self.last_activity_ms.load(Ordering::Relaxed))
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
}

/// Split complete records (terminated by `\r` or `\n`) out of `carry`, run each
/// through the pipeline, and forward the survivors plus their boundary byte
/// (a dropped record drops its boundary too). Leaves the trailing incomplete
/// record in `carry`.
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
            if emit_record(record, processors, forward)? {
                forward.write_all(&[b])?;
            }
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

/// Emit the buffered incomplete record (no boundary byte) through the pipeline
/// and clear `carry`. Used at EOF and when `carry` exceeds its size cap.
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

/// Run one record through the processor chain and forward the result,
/// returning whether the record survived (`false` = some stage dropped it,
/// so its boundary byte must be suppressed too). With an empty chain the
/// record is forwarded verbatim.
fn emit_record(
    record: &[u8],
    processors: &mut [Box<dyn LineProcessor>],
    forward: &mut Box<dyn Write + Send>,
) -> Result<bool, OutputStreamError> {
    if processors.is_empty() {
        forward.write_all(record)?;
        return Ok(true);
    }
    let mut current = record.to_vec();
    for p in processors.iter_mut() {
        match p.process(&current) {
            Some(out) => current = out,
            None => return Ok(false),
        }
    }
    forward.write_all(&current)?;
    Ok(true)
}

/// Whether an error reading a captured fd should be treated as end-of-stream.
/// A PTY master read after the child closes the slave returns `EIO` on Linux;
/// macOS typically returns a 0-length read instead (handled by the `Ok(0)`
/// arm). We treat any `EIO` as EOF, accepting that a genuine mid-stream I/O
/// error would drop the tail rather than surface — the expected PTY teardown
/// path dominates.
#[cfg(unix)]
fn is_pty_eof(e: &std::io::Error) -> bool {
    e.raw_os_error() == Some(libc::EIO)
}

#[cfg(not(unix))]
fn is_pty_eof(_e: &std::io::Error) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

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

    /// Feed `input` through a fresh `OutputStream` with the given `processors`
    /// and return everything forwarded.
    fn run_with(input: &[u8], processors: Vec<Box<dyn LineProcessor>>) -> Vec<u8> {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let reader = std::io::Cursor::new(input.to_vec());
        let mut stream =
            OutputStream::spawn(Box::new(reader), Box::new(SharedSink(sink.clone())), processors);
        stream.join().unwrap();
        let out = sink.lock().unwrap().clone();
        out
    }

    fn run(input: &[u8]) -> Vec<u8> {
        run_with(input, vec![])
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

    /// A processor that drops records equal to the immediately-previous one,
    /// exercising the `None`-drops-a-record path of the pipeline.
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
        // Second "a" record dropped along with its newline boundary.
        let out = run_with(b"a\na\nb\n", vec![Box::new(ConsecutiveDedup { prev: None })]);
        assert_eq!(out, b"a\nb\n");
    }

    #[test]
    fn empty_output_keeps_blank_line() {
        // Some(vec![]) blanks the record but keeps its boundary.
        struct Blank;
        impl LineProcessor for Blank {
            fn process(&mut self, _record: &[u8]) -> Option<Vec<u8>> {
                Some(vec![])
            }
        }
        assert_eq!(run_with(b"a\nb\n", vec![Box::new(Blank)]), b"\n\n");
    }

    #[test]
    fn finish_flushes_held_state_at_eof() {
        // A stage that swallows every record and emits a summary on finish.
        struct CountOnly {
            n: usize,
        }
        impl LineProcessor for CountOnly {
            fn process(&mut self, _record: &[u8]) -> Option<Vec<u8>> {
                self.n += 1;
                None
            }
            fn finish(&mut self) -> Option<Vec<u8>> {
                Some(format!("{} lines\n", self.n).into_bytes())
            }
        }
        let out = run_with(b"a\nb\nc\n", vec![Box::new(CountOnly { n: 0 })]);
        assert_eq!(out, b"3 lines\n");
    }

    /// A processor that rewrites the record bytes (uppercases), exercising the
    /// `Some(transformed)` path.
    struct Upcase;
    impl LineProcessor for Upcase {
        fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
            Some(record.to_ascii_uppercase())
        }
    }

    #[test]
    fn processor_can_transform_records() {
        let out = run_with(b"hi\nthere\n", vec![Box::new(Upcase)]);
        assert_eq!(out, b"HI\nTHERE\n");
    }

    #[test]
    fn processor_chain_runs_in_order() {
        // Upcase then drop-consecutive: "a\nA\n" upcases to two "A" records, the
        // second of which the dedup stage (seeing the upcased bytes) collapses.
        let out = run_with(
            b"a\nA\nb\n",
            vec![Box::new(Upcase), Box::new(ConsecutiveDedup { prev: None })],
        );
        assert_eq!(out, b"A\nB\n");
    }

    #[test]
    fn carry_cap_flush_runs_through_pipeline() {
        // A >cap record with no boundary is flushed mid-stream; it must still
        // pass through the processor (here, uppercased), not be forwarded raw.
        let big = vec![b'x'; MAX_CARRY_BYTES + 4096];
        let out = run_with(&big, vec![Box::new(Upcase)]);
        assert_eq!(out, big.to_ascii_uppercase());
    }
}
