use anyhow::anyhow;
use starlark::values::list::AllocList;
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

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::ValueLike;

use crate::engine::std::stream_iter;

#[derive(Debug, ProvidesStaticType, Dupe, Clone, NoSerialize, Allocative)]
pub enum Readable {
    Stdin(#[allocative(skip)] Arc<Stdin>),
    ChildStderr(#[allocative(skip)] Arc<Mutex<RefCell<ChildStderr>>>),
    ChildStdout(#[allocative(skip)] Arc<Mutex<RefCell<ChildStdout>>>),
}

#[derive(Debug, ProvidesStaticType, Dupe, Clone, NoSerialize, Allocative)]
pub enum Writable {
    ChildStdin(#[allocative(skip)] Arc<Mutex<RefCell<Option<ChildStdin>>>>),
    Stdout(#[allocative(skip)] Arc<Mutex<RefCell<Option<Stdout>>>>),
    Stderr(#[allocative(skip)] Arc<Mutex<RefCell<Option<Stderr>>>>),
}

impl Display for Readable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stdin(_) => write!(f, "stream<stdin>"),
            Self::ChildStderr(_) => write!(f, "stream<child_stderr>"),
            Self::ChildStdout(_) => write!(f, "stream<child_stdout>"),
        }
    }
}

impl Display for Writable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChildStdin(_) => write!(f, "stream<child_stdin>"),
            Self::Stderr(_) => write!(f, "stream<stderr>"),
            Self::Stdout(_) => write!(f, "stream<stdout>"),
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

#[starlark_value(type = "std.io.Readable")]
impl<'v> values::StarlarkValue<'v> for Readable {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(readable_methods)
    }

    unsafe fn iterate(
        &self,
        _me: values::Value<'v>,
        heap: &'v Heap,
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
        let io = this.downcast_ref_err::<Readable>()?;
        Ok(match &*io {
            Readable::Stdin(stdin) => stdin.is_terminal(),
            Readable::ChildStderr(_) => false,
            Readable::ChildStdout(_) => false,
        })
    }

    /// Reads all bytes until EOF in this source, placing them into a list of bytes.
    ///
    /// If successful, this function will return the list of bytes read.
    fn read<'v>(this: values::Value) -> anyhow::Result<AllocList<Vec<u32>>> {
        let io = this.downcast_ref_err::<Readable>()?;
        let mut buf = vec![];
        let _size = match &*io {
            Readable::Stdin(stdin) => stdin.lock().read(&mut buf)?,
            Readable::ChildStderr(stderr) => stderr.lock().unwrap().borrow_mut().read(&mut buf)?,
            Readable::ChildStdout(stdout) => stdout.lock().unwrap().borrow_mut().read(&mut buf)?,
        };

        Ok(AllocList(
            buf.iter().map(|b| *b as u32).collect::<Vec<u32>>(),
        ))
    }

    /// Reads all bytes until EOF in this source and returns a string.
    ///
    /// If successful, this function will return all bytes as a string.
    fn read_to_string<'v>(this: values::Value) -> anyhow::Result<String> {
        let io = this.downcast_ref_err::<Readable>()?;
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
        };

        Ok(buf)
    }
}

#[starlark_value(type = "std.io.Writable")]
impl<'v> values::StarlarkValue<'v> for Writable {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(writable_methods)
    }
}

starlark_simple_value!(Writable);

#[starlark_module]
fn writable_methods(registry: &mut MethodsBuilder) {
    /// Returns true if the underlying stream is connected to a terminal/tty.
    #[starlark(attribute)]
    fn is_tty<'v>(this: values::Value) -> anyhow::Result<bool> {
        let io = this.downcast_ref_err::<Writable>()?;
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
        })
    }

    /// Writes a buffer into this writer, returning how many bytes were written.
    ///
    /// This function will attempt to write the entire contents of `buf`, but
    /// the entire write may not succeed, or the write may also generate an
    /// error. Typically data is written from the start of the buffer and then
    /// caller should call `flush` to finish the write operation.
    fn write<'v>(
        this: values::Value,
        #[starlark(require = pos)] buf: values::StringValue,
    ) -> anyhow::Result<u32> {
        let io = this.downcast_ref_err::<Writable>()?;
        match &*io {
            Writable::ChildStdin(stdin) => {
                let guard = stdin.lock().unwrap();
                let mut borrowed = guard.borrow_mut();
                let inner = borrowed
                    .as_mut()
                    .ok_or_else(|| anyhow!("stream is closed"))?;
                inner
                    .write(buf.as_bytes())
                    .map(|f| f as u32)
                    .map_err(|err| anyhow!(err))
            }
            Writable::Stdout(stdout) => {
                let guard = stdout.lock().unwrap();
                let mut borrowed = guard.borrow_mut();
                let inner = borrowed
                    .as_mut()
                    .ok_or_else(|| anyhow!("stream is closed"))?;
                inner
                    .lock()
                    .write(buf.as_bytes())
                    .map(|f| f as u32)
                    .map_err(|err| anyhow!(err))
            }
            Writable::Stderr(stderr) => {
                let guard = stderr.lock().unwrap();
                let mut borrowed = guard.borrow_mut();
                let inner = borrowed
                    .as_mut()
                    .ok_or_else(|| anyhow!("stream is closed"))?;
                inner
                    .lock()
                    .write(buf.as_bytes())
                    .map(|f| f as u32)
                    .map_err(|err| anyhow!(err))
            }
        }
    }

    /// Flushes this output stream, ensuring that all intermediately buffered
    /// contents reach their destination.
    fn flush<'v>(this: values::Value) -> anyhow::Result<NoneType> {
        let io = this.downcast_ref_err::<Writable>()?;
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
                    inner.lock().flush()?;
                }
            }
            Writable::Stderr(stderr) => {
                let guard = stderr.lock().unwrap();
                let mut borrowed = guard.borrow_mut();
                if let Some(inner) = borrowed.as_mut() {
                    inner.lock().flush()?;
                }
            }
        };
        Ok(NoneType)
    }

    /// Closes this output stream. This drops the underlying handle, and
    /// subsequent writes will fail with "stream is closed".
    fn close<'v>(this: values::Value) -> anyhow::Result<NoneType> {
        let io = this.downcast_ref_err::<Writable>()?;
        match &*io {
            Writable::ChildStdin(stdin) => {
                // Take the inner value out, replacing with None. Dropping it closes the stream.
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
        };
        Ok(NoneType)
    }
}
