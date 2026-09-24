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

#[derive(Debug, ProvidesStaticType, Dupe, Clone, NoSerialize, Allocative)]
pub enum Readable {
    Stdin(#[allocative(skip)] Arc<Stdin>),
    ChildStderr(#[allocative(skip)] Arc<Mutex<RefCell<ChildStderr>>>),
    ChildStdout(#[allocative(skip)] Arc<Mutex<RefCell<ChildStdout>>>),
    File(#[allocative(skip)] Arc<Mutex<std::fs::File>>),
}

#[derive(Debug, ProvidesStaticType, Dupe, Clone, NoSerialize, Allocative)]
pub enum Writable {
    ChildStdin(#[allocative(skip)] Arc<Mutex<RefCell<Option<ChildStdin>>>>),
    Stdout(#[allocative(skip)] Arc<Mutex<RefCell<Option<Stdout>>>>),
    Stderr(#[allocative(skip)] Arc<Mutex<RefCell<Option<Stderr>>>>),
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
            Self::Stderr(_) => write!(f, "stream<stderr>"),
            Self::Stdout(_) => write!(f, "stream<stdout>"),
            Self::File(_) => write!(f, "stream<file>"),
        }
    }
}

impl From<Stdin> for Readable {
    fn from(stdin: Stdin) -> Self {
        Self::Stdin(Arc::new(stdin))
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

impl From<Stdout> for Writable {
    fn from(stdout: Stdout) -> Self {
        Self::Stdout(Arc::new(Mutex::new(RefCell::new(Some(stdout)))))
    }
}

impl From<Stderr> for Writable {
    fn from(stderr: Stderr) -> Self {
        Self::Stderr(Arc::new(Mutex::new(RefCell::new(Some(stderr)))))
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

struct StdinReader(Arc<Stdin>);

impl Read for StdinReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.lock().read(buf)
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
                    stdin.lock().read_to_end(&mut buf)?;
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
                Readable::Stdin(stdin) => stdin.lock().read(&mut buf)?,
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
            Readable::Stdin(stdin) => stdin.lock().read_to_string(&mut buf)?,
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

#[cfg(unix)]
struct NonblockingFd {
    fd: std::os::fd::RawFd,
    flags: libc::c_int,
    changed: bool,
}

#[cfg(unix)]
impl NonblockingFd {
    fn new(fd: std::os::fd::RawFd) -> std::io::Result<Self> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(std::io::Error::last_os_error());
        }

        let changed = flags & libc::O_NONBLOCK == 0;
        if changed && unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(Self { fd, flags, changed })
    }
}

#[cfg(unix)]
impl Drop for NonblockingFd {
    fn drop(&mut self) {
        if self.changed {
            unsafe {
                libc::fcntl(self.fd, libc::F_SETFL, self.flags);
            }
        }
    }
}

#[cfg(unix)]
fn nonblocking_write<W>(writer: &mut W, data: &[u8]) -> std::io::Result<usize>
where
    W: Write + std::os::fd::AsRawFd,
{
    let _guard = NonblockingFd::new(writer.as_raw_fd())?;
    loop {
        match writer.write(data) {
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            result => return result,
        }
    }
}

#[cfg(not(unix))]
fn nonblocking_write<W: Write>(_writer: &mut W, _data: &[u8]) -> std::io::Result<usize> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "nonblocking pipe writes are unsupported on this platform",
    ))
}

#[starlark_module]
fn writable_methods(registry: &mut MethodsBuilder) {
    /// Returns true if the underlying stream is connected to a terminal/tty.
    #[starlark(attribute)]
    fn is_tty<'v>(this: values::Value) -> anyhow::Result<bool> {
        let io = this.downcast_ref_err::<Writable>().into_anyhow_result()?;
        Ok(match &*io {
            Writable::ChildStdin(_) => false,
            Writable::Stdout(out) => {
                let guard = out.lock().unwrap();
                let borrowed = guard.borrow();
                borrowed.as_ref().map(|s| s.is_terminal()).unwrap_or(false)
            }
            Writable::Stderr(err) => {
                let guard = err.lock().unwrap();
                let borrowed = guard.borrow();
                borrowed.as_ref().map(|s| s.is_terminal()).unwrap_or(false)
            }
            Writable::File(_) => false,
        })
    }

    /// Writes a buffer into this writer, returning how many bytes were written.
    fn write<'v>(
        this: values::Value,
        #[starlark(require = pos)] buf: values::Value,
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
            Writable::Stdout(stdout) => {
                let guard = stdout.lock().unwrap();
                let mut borrowed = guard.borrow_mut();
                let inner = borrowed
                    .as_mut()
                    .ok_or_else(|| anyhow!("stream is closed"))?;
                match inner.lock().write(data) {
                    Ok(n) => Ok(n as u32),
                    // Reported as written; see `ignore_broken_pipe`.
                    Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {
                        Ok(data.len() as u32)
                    }
                    Err(err) => Err(anyhow!(err)),
                }
            }
            Writable::Stderr(stderr) => {
                let guard = stderr.lock().unwrap();
                let mut borrowed = guard.borrow_mut();
                let inner = borrowed
                    .as_mut()
                    .ok_or_else(|| anyhow!("stream is closed"))?;
                match inner.lock().write(data) {
                    Ok(n) => Ok(n as u32),
                    // Reported as written; see `ignore_broken_pipe`.
                    Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {
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

    /// Attempts one nonblocking write and returns the number of bytes
    /// accepted. Returns 0 when the stream is closed, the pipe is full, or
    /// the write fails. A short write may occur; retry only the unwritten
    /// suffix. A regular file never blocks, so its write is the plain one.
    ///
    /// On non-Unix platforms nonblocking pipe writes are unsupported and
    /// every call on a pipe-backed stream returns 0. That is deliberate:
    /// callers treat 0 as "this stream cannot be reached this way" and fall
    /// back to another path (e.g. the watch loop restarts the child instead
    /// of notifying it over stdin).
    fn try_write<'v>(
        this: values::Value,
        #[starlark(require = pos)] buf: values::Value,
    ) -> anyhow::Result<u32> {
        let io = this.downcast_ref_err::<Writable>().into_anyhow_result()?;
        let data: &[u8] = if let Some(s) = buf.unpack_str() {
            s.as_bytes()
        } else if let Some(b) = buf.downcast_ref::<Bytes>() {
            b.as_bytes()
        } else {
            return Err(anyhow!("try_write() expects a string or bytes"));
        };
        Ok(match &*io {
            Writable::ChildStdin(stdin) => {
                let guard = stdin.lock().unwrap();
                let mut borrowed = guard.borrow_mut();
                borrowed
                    .as_mut()
                    .and_then(|writer| nonblocking_write(writer, data).ok())
                    .unwrap_or(0) as u32
            }
            Writable::Stdout(stdout) => {
                let guard = stdout.lock().unwrap();
                let mut borrowed = guard.borrow_mut();
                borrowed
                    .as_mut()
                    .and_then(|inner| nonblocking_write(&mut inner.lock(), data).ok())
                    .unwrap_or(0) as u32
            }
            Writable::Stderr(stderr) => {
                let guard = stderr.lock().unwrap();
                let mut borrowed = guard.borrow_mut();
                borrowed
                    .as_mut()
                    .and_then(|inner| nonblocking_write(&mut inner.lock(), data).ok())
                    .unwrap_or(0) as u32
            }
            Writable::File(file) => {
                let mut guard = file.lock().unwrap();
                guard
                    .as_mut()
                    .and_then(|inner| inner.write(data).ok())
                    .unwrap_or(0) as u32
            }
        })
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
            Writable::Stdout(stdout) => {
                let guard = stdout.lock().unwrap();
                let mut borrowed = guard.borrow_mut();
                if let Some(inner) = borrowed.as_mut() {
                    ignore_broken_pipe(inner.lock().flush())?;
                }
            }
            Writable::Stderr(stderr) => {
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
            Writable::Stdout(stdout) => {
                stdout
                    .lock()
                    .unwrap()
                    .borrow_mut()
                    .take()
                    .ok_or_else(|| anyhow!("stream is already closed"))?;
            }
            Writable::Stderr(stderr) => {
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

#[cfg(all(test, unix))]
mod nonblocking_write_tests {
    use super::nonblocking_write;
    use std::fs::File;
    use std::io::{self, Write};
    use std::os::fd::{AsRawFd, FromRawFd, RawFd};

    fn pipe() -> (File, File) {
        let mut fds = [-1; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        unsafe { (File::from_raw_fd(fds[1]), File::from_raw_fd(fds[0])) }
    }

    fn flags(fd: RawFd) -> libc::c_int {
        let result = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        assert!(result >= 0);
        result
    }

    #[test]
    fn reports_partial_writes_and_restores_flags() {
        struct PartialWriter {
            fd: File,
            limit: usize,
        }

        impl AsRawFd for PartialWriter {
            fn as_raw_fd(&self) -> RawFd {
                self.fd.as_raw_fd()
            }
        }

        impl Write for PartialWriter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                Ok(buf.len().min(self.limit))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let (fd, _reader) = pipe();
        let mut writer = PartialWriter { fd, limit: 3 };
        let original_flags = flags(writer.as_raw_fd());

        assert_eq!(nonblocking_write(&mut writer, b"abcdef").unwrap(), 3);
        assert_eq!(flags(writer.as_raw_fd()), original_flags);
    }

    #[test]
    fn full_pipe_returns_would_block() {
        let (mut writer, _reader) = pipe();
        let chunk = [0_u8; 64 * 1024];

        for _ in 0..4096 {
            match nonblocking_write(&mut writer, &chunk) {
                Ok(_) => continue,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => return,
                Err(err) => panic!("unexpected pipe write error: {err}"),
            }
        }

        panic!("pipe did not fill after 256 MiB of writes");
    }

    #[test]
    fn dead_pipe_returns_broken_pipe() {
        let (mut writer, reader) = pipe();
        drop(reader);

        let err = nonblocking_write(&mut writer, b"notification").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }
}
