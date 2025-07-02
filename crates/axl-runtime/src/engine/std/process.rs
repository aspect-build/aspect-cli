use std::cell::RefCell;
use std::process;
use std::process::Stdio;
use std::rc::Rc;

use allocative::Allocative;
use anyhow::anyhow;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::ValueLike;

use starlark::values::list::UnpackList;
use starlark::values::none::NoneOr;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;

use super::stream;

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<process>")]
pub struct Process {}

impl Process {
    pub fn new() -> Self {
        Self {}
    }
}

#[starlark_value(type = "process")]
impl<'v> values::StarlarkValue<'v> for Process {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(process_methods)
    }
}

starlark_simple_value!(Process);

#[starlark_module]
pub(crate) fn process_methods(registry: &mut MethodsBuilder) {
    fn command<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] program: values::StringValue,
    ) -> anyhow::Result<Command> {
        Ok(Command {
            inner: RefCell::new(process::Command::new(program.as_str())),
        })
    }
}

#[derive(Debug, Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<command>")]
pub struct Command {
    #[allocative(skip)]
    inner: RefCell<process::Command>,
}

impl<'v> AllocValue<'v> for Command {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "command")]
impl<'v> values::StarlarkValue<'v> for Command {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(command_methods)
    }
}

#[starlark_module]
pub(crate) fn command_methods(registry: &mut MethodsBuilder) {
    fn arg<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] arg: values::StringValue,
    ) -> anyhow::Result<values::Value<'v>> {
        let cmd = this.downcast_ref_err::<Command>()?;
        cmd.inner.borrow_mut().arg(arg.as_str());
        Ok(this)
    }
    fn args<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] args: UnpackList<values::StringValue>,
    ) -> anyhow::Result<values::Value<'v>> {
        let cmd = this.downcast_ref_err::<Command>()?;
        cmd.inner
            .borrow_mut()
            .args(args.items.iter().map(|f| f.as_str()));
        Ok(this)
    }

    fn env<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] key: values::StringValue,
        #[starlark(require = pos)] value: NoneOr<values::StringValue>,
    ) -> anyhow::Result<values::Value<'v>> {
        let cmd = this.downcast_ref_err::<Command>()?;
        match value {
            NoneOr::None => cmd.inner.borrow_mut().env_remove(key.as_str()),
            NoneOr::Other(v) => cmd.inner.borrow_mut().env(key.as_str(), v.as_str()),
        };
        Ok(this)
    }

    fn current_dir<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] dir: values::StringValue,
    ) -> anyhow::Result<values::Value<'v>> {
        let cmd = this.downcast_ref_err::<Command>()?;
        cmd.inner.borrow_mut().current_dir(dir.as_str());
        Ok(this)
    }

    /// Configuration for the child process's standard input (stdin) handle.
    ///
    /// Defaults to [`inherit`] when used with [`spawn`] or [`status`], and
    /// defaults to [`piped`] when used with [`output`].
    fn stdin<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] io: values::StringValue,
    ) -> anyhow::Result<values::Value<'v>> {
        let cmd = this.downcast_ref_err::<Command>()?;
        match io.as_str() {
            "null" => cmd.inner.borrow_mut().stdin(Stdio::null()),
            "piped" => cmd.inner.borrow_mut().stdin(Stdio::piped()),
            "inherit" => cmd.inner.borrow_mut().stdin(Stdio::inherit()),
            v => return Err(anyhow::anyhow!("invalid stdin type {v}")),
        };
        Ok(this)
    }

    /// Configuration for the child process's standard output (stdout) handle.
    ///
    /// Defaults to [`inherit`] when used with [`spawn`] or [`status`], and
    /// defaults to [`piped`] when used with [`output`].
    fn stdout<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] io: values::StringValue,
    ) -> anyhow::Result<values::Value<'v>> {
        let cmd = this.downcast_ref_err::<Command>()?;
        match io.as_str() {
            "null" => cmd.inner.borrow_mut().stdout(Stdio::null()),
            "piped" => cmd.inner.borrow_mut().stdout(Stdio::piped()),
            "inherit" => cmd.inner.borrow_mut().stdout(Stdio::inherit()),
            v => return Err(anyhow::anyhow!("invalid stdout type {v}")),
        };
        Ok(this)
    }

    /// Configuration for the child process's standard error (stderr) handle.
    ///
    /// Defaults to [`inherit`] when used with [`spawn`] or [`status`], and
    /// defaults to [`piped`] when used with [`output`].
    fn stderr<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] io: values::StringValue,
    ) -> anyhow::Result<values::Value<'v>> {
        let cmd = this.downcast_ref_err::<Command>()?;
        match io.as_str() {
            "null" => cmd.inner.borrow_mut().stderr(Stdio::null()),
            "piped" => cmd.inner.borrow_mut().stderr(Stdio::piped()),
            "inherit" => cmd.inner.borrow_mut().stderr(Stdio::inherit()),
            v => return Err(anyhow::anyhow!("invalid stderr type {v}")),
        };
        Ok(this)
    }

    fn spawn<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Child> {
        let cmd = this.downcast_ref_err::<Command>()?;
        let child = cmd.inner.borrow_mut().spawn()?;
        Ok(Child {
            inner: Rc::new(RefCell::new(Some(child))),
        })
    }
}

#[derive(Debug, Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<child>")]
pub struct Child {
    #[allocative(skip)]
    inner: Rc<RefCell<Option<process::Child>>>,
}

impl<'v> AllocValue<'v> for Child {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "child")]
impl<'v> values::StarlarkValue<'v> for Child {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(child_methods)
    }
}

#[starlark_module]
pub(crate) fn child_methods(registry: &mut MethodsBuilder) {
    /// The handle for reading from the child’s standard output (stdout), if it has been captured.
    /// Calling this function more than once will yield error.
    fn stdout<'v>(this: values::Value<'v>) -> anyhow::Result<stream::Readable> {
        let child = this.downcast_ref_err::<Child>()?;

        let mut inner = child.inner.borrow_mut();

        let inner = inner
            .as_mut()
            .ok_or(anyhow::anyhow!("child is no longer active"))?;

        let child_stdout = inner.stdout.take().ok_or(anyhow!(
            r#"stdout is not available. spawn the process with stdout("piped")."#
        ))?;

        Ok(stream::Readable::from(child_stdout))
    }

    /// The handle for reading from the child’s standard error (stderr), if it has been captured.
    /// Calling this function more than once will yield error.
    fn stderr<'v>(this: values::Value<'v>) -> anyhow::Result<stream::Readable> {
        let child = this.downcast_ref_err::<Child>()?;

        let mut inner = child.inner.borrow_mut();

        let inner = inner
            .as_mut()
            .ok_or(anyhow::anyhow!("child is no longer active"))?;

        let child_stderr = inner.stderr.take().ok_or(anyhow!(
            r#"stderr is not available. spawn the process with stderr("piped")."#
        ))?;

        Ok(stream::Readable::from(child_stderr))
    }

    /// Returns the OS-assigned process identifier associated with this child.
    #[starlark(attribute)]
    fn id<'v>(this: values::Value<'v>) -> anyhow::Result<u32> {
        let child = this.downcast_ref_err::<Child>()?;
        Ok(child
            .inner
            .borrow()
            .as_ref()
            .ok_or(anyhow::anyhow!("child is no longer active"))?
            .id())
    }

    /// Forces the child process to exit. If the child has already exited, its a no-op.
    ///
    /// This is equivalent to sending a SIGKILL on Unix platforms.
    fn kill<'v>(this: values::Value<'v>) -> anyhow::Result<NoneType> {
        let child = this.downcast_ref_err::<Child>()?;
        child
            .inner
            .borrow_mut()
            .as_mut()
            .ok_or(anyhow::anyhow!("child is no longer active"))?
            .kill()?;
        Ok(NoneType)
    }

    /// Waits for the child to exit completely, returning the status that it
    /// exited with. This function will continue to have the same return value
    /// after it has been called at least once.
    ///
    /// The stdin handle to the child process, if any, will be closed
    /// before waiting. This helps avoid deadlock: it ensures that the
    /// child does not block waiting for input from the parent, while
    /// the parent waits for the child to exit.
    fn wait<'v>(this: values::Value<'v>) -> anyhow::Result<ExitStatus> {
        let child = this.downcast_ref_err::<Child>()?;
        let status = child
            .inner
            .borrow_mut()
            .as_mut()
            .ok_or(anyhow::anyhow!("child is no longer active"))?
            .wait()?;
        Ok(ExitStatus(status))
    }

    /// WARNING: Calling `wait_with_output` consumes the child instance,
    /// causing errors on subsequent calls to other methods.
    ///
    /// Simultaneously waits for the child to exit and collect all remaining
    /// output on the stdout/stderr handles, returning an `Output`
    /// instance.
    ///
    /// The stdin handle to the child process, if any, will be closed
    /// before waiting. This helps avoid deadlock: it ensures that the
    /// child does not block waiting for input from the parent, while
    /// the parent waits for the child to exit.
    ///
    /// By default, stdin, stdout and stderr are inherited from the parent.
    /// In order to capture the output into this `Result<Output>` it is
    /// necessary to create new pipes between parent and child. Use
    /// `stdout('piped')` or `stderr('piped')`, respectively.
    fn wait_with_output<'v>(this: values::Value<'v>) -> anyhow::Result<Output> {
        let child = this.downcast_ref_err::<Child>()?;
        let output = child
            .inner
            .replace(None)
            .ok_or(anyhow::anyhow!("child is no longer active"))?
            .wait_with_output()?;
        Ok(Output(output))
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<exit_status>")]
pub struct ExitStatus(#[allocative(skip)] pub process::ExitStatus);

#[starlark_value(type = "exit_status")]
impl<'v> values::StarlarkValue<'v> for ExitStatus {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(exit_status_methods)
    }
}
starlark_simple_value!(ExitStatus);

#[starlark_module]
pub(crate) fn exit_status_methods(registry: &mut MethodsBuilder) {
    /// Was termination successful? Signal termination is not considered a
    /// success, and success is defined as a zero exit status.
    #[starlark(attribute)]
    fn success<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let out = this.downcast_ref_err::<ExitStatus>()?;
        Ok(out.0.success())
    }

    /// Returns the exit code of the process, if any.
    ///
    /// In Unix terms the return value is the **exit status**: the value passed to `exit`, if the
    /// process finished by calling `exit`.  Note that on Unix the exit status is truncated to 8
    /// bits, and that values that didn't come from a program's call to `exit` may be invented by the
    /// runtime system (often, for example, 255, 254, 127 or 126).
    ///
    /// On Unix, this will return `None` if the process was terminated by a signal.
    #[starlark(attribute)]
    fn code<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<i32>> {
        let out = this.downcast_ref_err::<ExitStatus>()?;
        Ok(NoneOr::from_option(out.0.code()))
    }

    /// If the process was terminated by a signal, returns that signal.
    ///
    /// In other words, if `WIFSIGNALED`, this returns `WTERMSIG`.
    ///
    /// Avability: UNIX
    #[starlark(attribute)]
    fn signal<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<i32>> {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            let out = this.downcast_ref_err::<ExitStatus>()?;
            Ok(NoneOr::from_option(out.0.signal()))
        }
        #[cfg(not(unix))]
        {
            Ok(NoneOr::None)
        }
    }

    /// If the process was stopped by a signal, returns that signal.
    ///
    /// In other words, if `WIFSTOPPED`, this returns `WSTOPSIG`.  This is only possible if the status came from
    /// a `wait` system call which was passed `WUNTRACED`, and was then converted into an `ExitStatus`.
    ///
    /// Avability: UNIX
    #[starlark(attribute)]
    fn stopped_signal<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<i32>> {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            let out = this.downcast_ref_err::<ExitStatus>()?;
            Ok(NoneOr::from_option(out.0.stopped_signal()))
        }
        #[cfg(not(unix))]
        {
            Ok(NoneOr::None)
        }
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<output>")]
pub struct Output(#[allocative(skip)] pub process::Output);

#[starlark_value(type = "output")]
impl<'v> values::StarlarkValue<'v> for Output {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(output_methods)
    }
}
starlark_simple_value!(Output);

#[starlark_module]
pub(crate) fn output_methods(registry: &mut MethodsBuilder) {
    /// Returns the OS-assigned process identifier associated with this child.
    #[starlark(attribute)]
    fn status<'v>(this: values::Value<'v>) -> anyhow::Result<ExitStatus> {
        let out = this.downcast_ref_err::<Output>()?;
        Ok(ExitStatus(out.0.status))
    }

    /// The data that the process wrote to stderr.
    #[starlark(attribute)]
    fn stderr<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        let out = this.downcast_ref_err::<Output>()?;
        Ok(String::from_utf8(out.0.stderr.clone())?)
    }

    /// The data that the process wrote to stdout.
    #[starlark(attribute)]
    fn stdout<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        let out = this.downcast_ref_err::<Output>()?;
        Ok(String::from_utf8(out.0.stdout.clone())?)
    }
}
