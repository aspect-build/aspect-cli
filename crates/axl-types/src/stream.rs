use anyhow::anyhow;
use starlark::values::Heap;

use std::cell::RefCell;
use std::fmt::Debug;
use std::fmt::Display;
use std::io::IsTerminal;
use std::io::Read;
use std::io::Stderr;
use std::io::Stdin;
use std::io::Stdout;
use std::io::Write;
use std::process::{ChildStderr, ChildStdin, ChildStdout};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

/// Whether AXL wrote anything to this process's stdout
/// (`ctx.std.io.stdout.write`).
static STDOUT_WRITTEN: AtomicBool = AtomicBool::new(false);

/// Whether AXL has written to this process's stdout, so a caller that owes
/// it a document (`--output=json`'s error) does not add a second.
pub fn stdout_written() -> bool {
    STDOUT_WRITTEN.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether AXL wrote a stream's last line to stdout (`--output=ndjson`'s
/// `result` or `error`).
static STDOUT_ENDED: AtomicBool = AtomicBool::new(false);

/// Whether an `--output=ndjson` stream on stdout has had its last line, so
/// an error after it adds no second; before it, one is the stream's end.
pub fn stdout_ended() -> bool {
    STDOUT_ENDED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether `data` is a stream's last line: `result` or `error` typed, written
/// compact with `type` its first key, as `results_json.emit` and
/// `emit_error` write it.
fn ends_stream(data: &[u8]) -> bool {
    data.starts_with(br#"{"type":"result""#) || data.starts_with(br#"{"type":"error""#)
}

use allocative::Allocative;
use dupe::Dupe;

use starlark::StarlarkResultExt;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::UnpackValue;
use starlark::values::ValueLike;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;

use crate::stream_iter;
use starlark::values::bytes::StarlarkBytes as Bytes;

/// Asked before every blocking read of stdin: `Some(err)` ends the read with
/// that error instead of waiting for input. The runtime installs one that
/// carries the task's exit once the run is cancelled.
pub type Interrupt = Arc<dyn Fn() -> Option<std::io::Error> + Send + Sync>;

/// The process's stdin, read only once input is actually available so a
/// cancelled run is not stuck behind a prompt nobody will answer.
pub struct StdinSource {
    inner: Stdin,
    interrupt: Mutex<Option<Interrupt>>,
}

impl Debug for StdinSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StdinSource")
    }
}

impl StdinSource {
    pub fn new(inner: Stdin) -> Self {
        Self {
            inner,
            interrupt: Mutex::new(None),
        }
    }

    pub fn is_terminal(&self) -> bool {
        self.inner.is_terminal()
    }

    pub fn set_interrupt(&self, interrupt: Interrupt) {
        if let Ok(mut slot) = self.interrupt.lock() {
            *slot = Some(interrupt);
        }
    }

    /// Wait until stdin has input (or hit EOF), checking the interrupt every
    /// slice. On a platform without `poll` the read simply blocks.
    fn wait_readable(&self) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let mut pfd = libc::pollfd {
                fd: self.inner.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            loop {
                let interrupted = self
                    .interrupt
                    .lock()
                    .ok()
                    .and_then(|slot| slot.as_ref().and_then(|check| check()));
                if let Some(err) = interrupted {
                    return Err(err);
                }
                // SAFETY: `pfd` is a valid, initialized pollfd for the call's duration.
                let ready = unsafe { libc::poll(&mut pfd, 1, 100) };
                if ready > 0 {
                    return Ok(());
                }
                if ready < 0 {
                    let err = std::io::Error::last_os_error();
                    if err.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    // A descriptor `poll` cannot watch: fall back to a
                    // blocking read.
                    return Ok(());
                }
            }
        }
        #[cfg(not(unix))]
        Ok(())
    }
}

impl Read for &StdinSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.wait_readable()?;
        self.inner.lock().read(buf)
    }
}

/// Where what AXL writes to stdout and stderr is also copied (the task
/// history's output files); see [`copy_output_to`].
static COPY_OUTPUT: std::sync::OnceLock<fn(&[u8])> = std::sync::OnceLock::new();

/// Copy what `ctx.std.io.stdout` and `stderr` write to `copy` from now on.
/// The first call wins.
pub fn copy_output_to(copy: fn(&[u8])) {
    let _ = COPY_OUTPUT.set(copy);
}

fn copy_output(data: &[u8]) {
    if let Some(copy) = COPY_OUTPUT.get() {
        copy(data);
    }
}

#[derive(Debug, ProvidesStaticType, Dupe, Clone, NoSerialize, Allocative)]
pub enum Readable {
    Stdin(#[allocative(skip)] Arc<StdinSource>),
    ChildStderr(#[allocative(skip)] Arc<Mutex<RefCell<ChildStderr>>>),
    ChildStdout(#[allocative(skip)] Arc<Mutex<RefCell<ChildStdout>>>),
    File(#[allocative(skip)] Arc<Mutex<std::fs::File>>),
}

#[derive(Debug, ProvidesStaticType, Dupe, Clone, NoSerialize, Allocative)]
pub enum Writable {
    ChildStdin(#[allocative(skip)] Arc<Mutex<RefCell<Option<ChildStdin>>>>),
    /// The process's stdout, and whether it may carry ANSI color.
    Stdout(
        #[allocative(skip)] Arc<Mutex<RefCell<Option<Stdout>>>>,
        bool,
    ),
    /// The process's stderr, and whether it may carry ANSI color.
    Stderr(
        #[allocative(skip)] Arc<Mutex<RefCell<Option<Stderr>>>>,
        bool,
    ),
    File(#[allocative(skip)] Arc<Mutex<Option<std::fs::File>>>),
}

impl Display for Readable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stdin(_) => write!(f, "stream<stdin>"),
            Self::ChildStderr(_) => write!(f, "stream<child_stderr>"),
            Self::ChildStdout(_) => write!(f, "stream<child_stdout>"),
            Self::File(_) => write!(f, "stream<file>"),
        }
    }
}

impl Display for Writable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChildStdin(_) => write!(f, "stream<child_stdin>"),
            Self::Stderr(..) => write!(f, "stream<stderr>"),
            Self::Stdout(..) => write!(f, "stream<stdout>"),
            Self::File(_) => write!(f, "stream<file>"),
        }
    }
}

impl From<Stdin> for Readable {
    fn from(stdin: Stdin) -> Self {
        Self::Stdin(Arc::new(StdinSource::new(stdin)))
    }
}

impl Readable {
    /// Install the check every blocking stdin read makes first. A no-op for
    /// any other source.
    pub fn set_stdin_interrupt(&self, interrupt: Interrupt) {
        if let Readable::Stdin(stdin) = self {
            stdin.set_interrupt(interrupt);
        }
    }
}

impl From<ChildStderr> for Readable {
    fn from(stderr: ChildStderr) -> Self {
        Self::ChildStderr(Arc::new(Mutex::new(RefCell::new(stderr))))
    }
}

impl From<ChildStdout> for Readable {
    fn from(stdout: ChildStdout) -> Self {
        Self::ChildStdout(Arc::new(Mutex::new(RefCell::new(stdout))))
    }
}

impl From<ChildStdin> for Writable {
    fn from(stdin: ChildStdin) -> Self {
        Self::ChildStdin(Arc::new(Mutex::new(RefCell::new(Some(stdin)))))
    }
}

impl Writable {
    /// The process's stdout. `color` is whether it may carry ANSI color, as
    /// the runtime decides it; the stream reports it as `color`.
    pub fn stdout(stdout: Stdout, color: bool) -> Self {
        Self::Stdout(Arc::new(Mutex::new(RefCell::new(Some(stdout)))), color)
    }

    /// The process's stderr. `color` is whether it may carry ANSI color, as
    /// the runtime decides it; the stream reports it as `color`.
    pub fn stderr(stderr: Stderr, color: bool) -> Self {
        Self::Stderr(Arc::new(Mutex::new(RefCell::new(Some(stderr)))), color)
    }
}

impl From<std::fs::File> for Writable {
    fn from(file: std::fs::File) -> Self {
        Self::File(Arc::new(Mutex::new(Some(file))))
    }
}

impl<'v> UnpackValue<'v> for Writable {
    type Error = anyhow::Error;

    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        Ok(value.downcast_ref::<Writable>().map(|v| v.dupe()))
    }
}

impl<'v> UnpackValue<'v> for Readable {
    type Error = anyhow::Error;

    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        let v = value.downcast_ref_err::<Readable>().into_anyhow_result()?;
        Ok(Some(v.dupe()))
    }
}

// --- Reader wrappers for to_boxed_read ---

struct StdinReader(Arc<StdinSource>);

impl Read for StdinReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        (&*self.0).read(buf)
    }
}

unsafe impl Send for StdinReader {}

struct ArcMutexRefCellReader<T: Read>(Arc<Mutex<RefCell<T>>>);

impl<T: Read> Read for ArcMutexRefCellReader<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().borrow_mut().read(buf)
    }
}

unsafe impl<T: Read> Send for ArcMutexRefCellReader<T> {}

struct ArcMutexReader<T: Read>(Arc<Mutex<T>>);

impl<T: Read> Read for ArcMutexReader<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().read(buf)
    }
}

unsafe impl<T: Read> Send for ArcMutexReader<T> {}

impl Readable {
    /// Produce a boxed `Read + Send` from this `Readable`.
    pub fn to_boxed_read(&self) -> Box<dyn Read + Send> {
        match self {
            Readable::Stdin(arc) => Box::new(StdinReader(arc.clone())),
            Readable::ChildStdout(arc) => Box::new(ArcMutexRefCellReader(arc.clone())),
            Readable::ChildStderr(arc) => Box::new(ArcMutexRefCellReader(arc.clone())),
            Readable::File(arc) => Box::new(ArcMutexReader(arc.clone())),
        }
    }
}

#[starlark_value(type = "std.io.Readable")]
impl<'v> values::StarlarkValue<'v> for Readable {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("readable_methods", readable_methods);
        Some(RES.methods())
    }

    unsafe fn iterate(
        &self,
        _me: values::Value<'v>,
        heap: Heap<'v>,
    ) -> starlark::Result<values::Value<'v>> {
        Ok(heap.alloc_simple(stream_iter::ReadIterator::new(self.dupe())))
    }
}

starlark_simple_value!(Readable);

#[starlark_module]
fn readable_methods(registry: &mut MethodsBuilder) {
    /// Returns true if the underlying stream is connected to a terminal/tty.
    #[starlark(attribute)]
    fn is_tty<'v>(this: values::Value) -> anyhow::Result<bool> {
        let io = this.downcast_ref_err::<Readable>().into_anyhow_result()?;
        Ok(match &*io {
            Readable::Stdin(stdin) => stdin.is_terminal(),
            Readable::ChildStderr(_) => false,
            Readable::ChildStdout(_) => false,
            Readable::File(_) => false,
        })
    }

    /// Reads bytes from this source.
    ///
    /// If `size` is provided, reads up to that many bytes.
    /// If `size` is not provided, reads until EOF.
    /// Returns the bytes read.
    fn read<'v>(
        this: values::Value,
        #[starlark(require=pos, default = -1)] size: i32,
    ) -> anyhow::Result<Bytes> {
        let io = this.downcast_ref_err::<Readable>().into_anyhow_result()?;

        if size < 0 {
            // Read until EOF
            let mut buf = Vec::new();
            match &*io {
                Readable::Stdin(stdin) => {
                    (&**stdin).read_to_end(&mut buf)?;
                }
                Readable::ChildStderr(stderr) => {
                    stderr.lock().unwrap().borrow_mut().read_to_end(&mut buf)?;
                }
                Readable::ChildStdout(stdout) => {
                    stdout.lock().unwrap().borrow_mut().read_to_end(&mut buf)?;
                }
                Readable::File(file) => {
                    file.lock().unwrap().read_to_end(&mut buf)?;
                }
            };
            Ok(Bytes::new(buf.as_slice()))
        } else {
            // Read up to size bytes
            let mut buf = vec![0u8; size as usize];
            let bytes_read = match &*io {
                Readable::Stdin(stdin) => (&**stdin).read(&mut buf)?,
                Readable::ChildStderr(stderr) => {
                    stderr.lock().unwrap().borrow_mut().read(&mut buf)?
                }
                Readable::ChildStdout(stdout) => {
                    stdout.lock().unwrap().borrow_mut().read(&mut buf)?
                }
                Readable::File(file) => file.lock().unwrap().read(&mut buf)?,
            };
            buf.truncate(bytes_read);
            Ok(Bytes::new(buf.as_slice()))
        }
    }

    /// Reads all bytes until EOF in this source and returns a string.
    ///
    /// If successful, this function will return all bytes as a string.
    fn read_to_string<'v>(this: values::Value) -> anyhow::Result<String> {
        let io = this.downcast_ref_err::<Readable>().into_anyhow_result()?;
        let mut buf = String::new();
        let _size = match &*io {
            Readable::Stdin(stdin) => (&**stdin).read_to_string(&mut buf)?,
            Readable::ChildStderr(stderr) => stderr
                .lock()
                .unwrap()
                .borrow_mut()
                .read_to_string(&mut buf)?,
            Readable::ChildStdout(stdout) => stdout
                .lock()
                .unwrap()
                .borrow_mut()
                .read_to_string(&mut buf)?,
            Readable::File(file) => file.lock().unwrap().read_to_string(&mut buf)?,
        };

        Ok(buf)
    }
}

#[starlark_value(type = "std.io.Writable")]
impl<'v> values::StarlarkValue<'v> for Writable {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("writable_methods", writable_methods);
        Some(RES.methods())
    }
}

starlark_simple_value!(Writable);

/// Swallow `BrokenPipe` on the inherited console streams.
///
/// A departed reader (`aspect build … | head`) is not a task failure: the task
/// must still finish and report its result, or its status-surface entry is
/// stranded showing "running". Applies to writes *and* flushes — stdout is
/// line-buffered, so a write with no newline is buffered and reports success,
/// and the `BrokenPipe` first surfaces at the flush.
///
/// Only for `Stdout`/`Stderr`. A `File` or a child's stdin failing is a real
/// error and still propagates.
fn ignore_broken_pipe(result: std::io::Result<()>) -> std::io::Result<()> {
    match result {
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

#[starlark_module]
fn writable_methods(registry: &mut MethodsBuilder) {
    /// Returns true if the underlying stream is connected to a terminal/tty.
    #[starlark(attribute)]
    fn is_tty<'v>(this: values::Value) -> anyhow::Result<bool> {
        let io = this.downcast_ref_err::<Writable>().into_anyhow_result()?;
        Ok(match &*io {
            Writable::ChildStdin(_) => false,
            Writable::Stdout(out, _) => {
                let guard = out.lock().unwrap();
                let borrowed = guard.borrow();
                borrowed.as_ref().map(|s| s.is_terminal()).unwrap_or(false)
            }
            Writable::Stderr(err, _) => {
                let guard = err.lock().unwrap();
                let borrowed = guard.borrow();
                borrowed.as_ref().map(|s| s.is_terminal()).unwrap_or(false)
            }
            Writable::File(_) => false,
        })
    }

    /// Whether lines written here may carry ANSI color, by the rules clap
    /// colors `--help` with: never under a non-empty `NO_COLOR`, always under
    /// a non-empty `CLICOLOR_FORCE`, never under `CLICOLOR=0`, and otherwise
    /// on a terminal whose `TERM` is not `dumb` (and, off Windows, is set), or
    /// with `CLICOLOR` or `CI` set. `std.io.stderr` also colors on a CI host
    /// whose log renders ANSI (GitHub Actions, Buildkite, CircleCI, GitLab
    /// CI). Never on any other stream (files, a child's stdin).
    #[starlark(attribute)]
    fn color<'v>(this: values::Value) -> anyhow::Result<bool> {
        let io = this.downcast_ref_err::<Writable>().into_anyhow_result()?;
        Ok(match &*io {
            Writable::Stdout(_, color) | Writable::Stderr(_, color) => *color,
            Writable::ChildStdin(_) | Writable::File(_) => false,
        })
    }

    /// Writes a buffer into this writer, returning how many bytes were written.
    ///
    /// What is written to stdout or stderr is also copied to the task
    /// history's `output.log` and `output.ansi`; `copy = False` leaves it
    /// out of them.
    fn write<'v>(
        this: values::Value,
        #[starlark(require = pos)] buf: values::Value,
        #[starlark(require = named, default = true)] copy: bool,
    ) -> anyhow::Result<u32> {
        let io = this.downcast_ref_err::<Writable>().into_anyhow_result()?;
        let data: &[u8] = if let Some(s) = buf.unpack_str() {
            s.as_bytes()
        } else if let Some(b) = buf.downcast_ref::<Bytes>() {
            b.as_bytes()
        } else {
            return Err(anyhow!("write() expects a string or bytes"));
        };
        match &*io {
            Writable::ChildStdin(stdin) => {
                let guard = stdin.lock().unwrap();
                let mut borrowed = guard.borrow_mut();
                let inner = borrowed
                    .as_mut()
                    .ok_or_else(|| anyhow!("stream is closed"))?;
                inner
                    .write(data)
                    .map(|f| f as u32)
                    .map_err(|err| anyhow!(err))
            }
            Writable::Stdout(stdout, _) => {
                let guard = stdout.lock().unwrap();
                let mut borrowed = guard.borrow_mut();
                let inner = borrowed
                    .as_mut()
                    .ok_or_else(|| anyhow!("stream is closed"))?;
                if !data.is_empty() {
                    STDOUT_WRITTEN.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                if ends_stream(data) {
                    STDOUT_ENDED.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                match inner.lock().write(data) {
                    Ok(n) => {
                        if copy {
                            copy_output(&data[..n]);
                        }
                        Ok(n as u32)
                    }
                    // Reported as written; see `ignore_broken_pipe`.
                    Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {
                        if copy {
                            copy_output(data);
                        }
                        Ok(data.len() as u32)
                    }
                    Err(err) => Err(anyhow!(err)),
                }
            }
            Writable::Stderr(stderr, _) => {
                let guard = stderr.lock().unwrap();
                let mut borrowed = guard.borrow_mut();
                let inner = borrowed
                    .as_mut()
                    .ok_or_else(|| anyhow!("stream is closed"))?;
                match inner.lock().write(data) {
                    Ok(n) => {
                        if copy {
                            copy_output(&data[..n]);
                        }
                        Ok(n as u32)
                    }
                    // Reported as written; see `ignore_broken_pipe`.
                    Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {
                        if copy {
                            copy_output(data);
                        }
                        Ok(data.len() as u32)
                    }
                    Err(err) => Err(anyhow!(err)),
                }
            }
            Writable::File(file) => {
                let mut guard = file.lock().unwrap();
                let inner = guard.as_mut().ok_or_else(|| anyhow!("stream is closed"))?;
                inner
                    .write(data)
                    .map(|f| f as u32)
                    .map_err(|err| anyhow!(err))
            }
        }
    }

    /// Flushes this output stream, ensuring that all intermediately buffered
    /// contents reach their destination.
    fn flush<'v>(this: values::Value) -> anyhow::Result<NoneType> {
        let io = this.downcast_ref_err::<Writable>().into_anyhow_result()?;
        match &*io {
            Writable::ChildStdin(stdin) => {
                let guard = stdin.lock().unwrap();
                let mut borrowed = guard.borrow_mut();
                if let Some(inner) = borrowed.as_mut() {
                    inner.flush()?;
                }
            }
            Writable::Stdout(stdout, _) => {
                let guard = stdout.lock().unwrap();
                let mut borrowed = guard.borrow_mut();
                if let Some(inner) = borrowed.as_mut() {
                    ignore_broken_pipe(inner.lock().flush())?;
                }
            }
            Writable::Stderr(stderr, _) => {
                let guard = stderr.lock().unwrap();
                let mut borrowed = guard.borrow_mut();
                if let Some(inner) = borrowed.as_mut() {
                    ignore_broken_pipe(inner.lock().flush())?;
                }
            }
            Writable::File(file) => {
                if let Some(inner) = file.lock().unwrap().as_mut() {
                    inner.flush()?;
                }
            }
        };
        Ok(NoneType)
    }

    /// Closes this output stream. This drops the underlying handle, and
    /// subsequent writes will fail with "stream is closed".
    fn close<'v>(this: values::Value) -> anyhow::Result<NoneType> {
        let io = this.downcast_ref_err::<Writable>().into_anyhow_result()?;
        match &*io {
            Writable::ChildStdin(stdin) => {
                stdin
                    .lock()
                    .unwrap()
                    .borrow_mut()
                    .take()
                    .ok_or_else(|| anyhow!("stream is already closed"))?;
            }
            Writable::Stdout(stdout, _) => {
                stdout
                    .lock()
                    .unwrap()
                    .borrow_mut()
                    .take()
                    .ok_or_else(|| anyhow!("stream is already closed"))?;
            }
            Writable::Stderr(stderr, _) => {
                stderr
                    .lock()
                    .unwrap()
                    .borrow_mut()
                    .take()
                    .ok_or_else(|| anyhow!("stream is already closed"))?;
            }
            Writable::File(file) => {
                file.lock()
                    .unwrap()
                    .take()
                    .ok_or_else(|| anyhow!("stream is already closed"))?;
            }
        };
        Ok(NoneType)
    }
}

#[cfg(test)]
mod broken_pipe_tests {
    use super::ignore_broken_pipe;
    use std::io::{Error, ErrorKind};

    #[test]
    fn broken_pipe_is_swallowed() {
        let r = ignore_broken_pipe(Err(Error::new(ErrorKind::BrokenPipe, "reader left")));
        assert!(
            r.is_ok(),
            "a departed console reader must not fail the task"
        );
    }

    #[test]
    fn other_errors_still_propagate() {
        // A full disk or a closed file descriptor is a real failure — only a
        // reader walking away from the console is benign.
        for kind in [ErrorKind::PermissionDenied, ErrorKind::Other] {
            let r = ignore_broken_pipe(Err(Error::new(kind, "real failure")));
            assert!(r.is_err(), "{kind:?} must propagate");
        }
    }

    #[test]
    fn success_passes_through() {
        assert!(ignore_broken_pipe(Ok(())).is_ok());
    }
}

#[cfg(test)]
mod stream_end_tests {
    use super::ends_stream;

    #[test]
    fn a_result_or_error_line_ends_a_stream() {
        assert!(ends_stream(br#"{"type":"result","schema_version":1}"#));
        assert!(ends_stream(br#"{"type":"error","error":"task_error"}"#));
        assert!(!ends_stream(br#"{"type":"attempt","attempt":0}"#));
        assert!(!ends_stream(br#"{"schema_version":1,"error":"x"}"#));
    }
}
