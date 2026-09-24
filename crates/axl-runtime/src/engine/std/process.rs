use std::cell::RefCell;
use std::process;
use std::process::Stdio;

use allocative::Allocative;
use anyhow::anyhow;
use derive_more::Display;
use either::Either;

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

use starlark::StarlarkResultExt;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneOr;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::typing::StarlarkNever;

use super::live;
use super::stream;
use crate::eval::TaskExit;

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.process.Process>")]
pub struct Process {}

impl Process {
    pub fn new() -> Self {
        Self {}
    }
}

#[starlark_value(type = "std.process.Process")]
impl<'v> values::StarlarkValue<'v> for Process {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("process_methods", process_methods);
        Some(RES.methods())
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
            own_group: std::cell::Cell::new(false),
        })
    }

    /// Return the current (aspect-cli) process's ID. Stable for the
    /// lifetime of this AXL evaluator and unique within the host OS
    /// at any given moment, so it's a useful per-task identifier for
    /// state files under the shared job tmpdir: each task invocation
    /// gets its own aspect-cli process and therefore its own id.
    /// Available from any ctx flavor (FeatureContext / TaskContext)
    /// — `ctx.std` is on both, and `id()` doesn't depend on a task
    /// being in scope.
    fn id<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<i32> {
        Ok(process::id() as i32)
    }

    /// End the task with exit `code` (0..=255, no default, as in Rust's
    /// `std::process::exit`) and, optionally, a `message`, with no traceback.
    /// The message prints as an `ERROR:` line for a non-zero code and `INFO:`
    /// for 0. Use it for an expected refusal, or an early "nothing to do",
    /// from however deep in the call stack it is discovered; keep `fail()` for
    /// bugs, where the traceback helps. `ASPECT_DEBUG=1` prints the traceback
    /// after the message anyway.
    ///
    /// The exit unwinds through every caller like an error: `ctx.defer`
    /// callbacks still run, but whatever the task body had yet to run does
    /// not, including a status surface's final update. A task that reports to
    /// a surface ends through its final `phases.update` instead.
    fn exit<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] code: i32,
        #[starlark(default = NoneOr::None)] message: NoneOr<&'v str>,
    ) -> anyhow::Result<StarlarkNever> {
        let code =
            u8::try_from(code).map_err(|_| anyhow!("exit code must be 0..=255, got {code}"))?;
        Err(TaskExit::new(code, message.into_option().map(str::to_owned)).into())
    }

    /// The absolute path `command(name)` would run, or `None` when no `PATH`
    /// entry holds an executable of that name — `which` / `command -v`.
    ///
    /// A bare name is looked up along `PATH`; a name containing a path
    /// separator is checked as given. On Unix the file must carry an execute
    /// bit; on Windows the `PATHEXT` extensions are tried.
    ///
    /// **Examples**
    ///
    /// ```python
    /// helper = "aspect" if ctx.std.process.which("aspect") else ctx.std.env.current_exe()
    /// ```
    fn which<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] name: &str,
        heap: Heap<'v>,
    ) -> anyhow::Result<NoneOr<values::StringValue<'v>>> {
        Ok(
            match find_executable(name, std::env::var_os("PATH").as_deref()) {
                Some(path) => NoneOr::Other(
                    heap.alloc_str(
                        path.to_str()
                            .ok_or_else(|| anyhow::anyhow!("path of `{name}` is non utf-8"))?,
                    ),
                ),
                None => NoneOr::None,
            },
        )
    }
}

#[derive(Debug, Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.process.Command>")]
pub struct Command {
    #[allocative(skip)]
    inner: RefCell<process::Command>,
    #[allocative(skip)]
    own_group: std::cell::Cell<bool>,
}

impl Command {
    /// Format as `program "arg1" "arg2"` for use in error messages.
    /// Deliberately excludes environment variables to avoid leaking secrets.
    fn describe(&self) -> String {
        let inner = self.inner.borrow();
        inner.get_args().fold(
            inner.get_program().to_string_lossy().into_owned(),
            |acc, a| format!("{acc} {a:?}"),
        )
    }

    fn try_spawn(&self) -> anyhow::Result<(process::Child, live::LiveChildGuard)> {
        let result = live::spawn_registered(&mut self.inner.borrow_mut(), self.own_group.get());
        result.map_err(|e| self.spawn_error(e))
    }

    /// A spawn refused because shutdown began is an expected refusal, not a
    /// bug: map it to a quiet `TaskExit` instead of a traceback while the
    /// process winds down.
    fn spawn_error(&self, e: std::io::Error) -> anyhow::Error {
        if e.kind() == std::io::ErrorKind::Interrupted {
            return TaskExit::error(format!(
                "aspect-cli is shutting down; not spawning {}",
                self.describe()
            ))
            .into();
        }
        anyhow!("failed to spawn command {}: {}", self.describe(), e)
    }

    fn try_status(&self) -> anyhow::Result<process::ExitStatus> {
        let (mut child, _guard) = self.try_spawn()?;
        child
            .wait()
            .map_err(|e| anyhow!("failed to execute command {}: {}", self.describe(), e))
    }
}

impl<'v> AllocValue<'v> for Command {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "std.process.Command")]
impl<'v> values::StarlarkValue<'v> for Command {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("command_methods", command_methods);
        Some(RES.methods())
    }
}

/// Resolve a `stdout`/`stderr` argument to a [`Stdio`]. Accepts the mode
/// strings `"null"`/`"piped"`/`"inherit"`, or a writable file stream from
/// `fs.create(...)` — whose owned file is taken and handed to the child so its
/// output lands directly on disk. `field` names the caller for error messages.
fn stdio_from<'v>(
    io: Either<values::StringValue<'v>, stream::Writable>,
    field: &str,
) -> anyhow::Result<Stdio> {
    match io {
        Either::Left(mode) => match mode.as_str() {
            "null" => Ok(Stdio::null()),
            "piped" => Ok(Stdio::piped()),
            "inherit" => Ok(Stdio::inherit()),
            other => Err(anyhow!("invalid {field} type {other}")),
        },
        Either::Right(stream::Writable::File(file)) => {
            let file = file
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| anyhow!("{field} file stream has already been consumed"))?;
            Ok(Stdio::from(file))
        }
        Either::Right(_) => Err(anyhow!(
            "{field} redirect requires a file stream from fs.create(...)"
        )),
    }
}

#[starlark_module]
pub(crate) fn command_methods(registry: &mut MethodsBuilder) {
    fn arg<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] arg: values::StringValue,
    ) -> anyhow::Result<values::Value<'v>> {
        let cmd = this.downcast_ref_err::<Command>().into_anyhow_result()?;
        cmd.inner.borrow_mut().arg(arg.as_str());
        Ok(this)
    }
    fn args<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] args: UnpackList<values::StringValue>,
    ) -> anyhow::Result<values::Value<'v>> {
        let cmd = this.downcast_ref_err::<Command>().into_anyhow_result()?;
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
        let cmd = this.downcast_ref_err::<Command>().into_anyhow_result()?;
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
        let cmd = this.downcast_ref_err::<Command>().into_anyhow_result()?;
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
        let cmd = this.downcast_ref_err::<Command>().into_anyhow_result()?;
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
    ///
    /// Accepts the strings `"null"`, `"piped"`, `"inherit"`, or a writable file
    /// stream from `fs.create(...)` to redirect output straight to that file —
    /// letting a caller poll the child's liveness with `try_wait` and read the
    /// file afterwards instead of draining a pipe with `wait_with_output`.
    fn stdout<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] io: Either<values::StringValue<'v>, stream::Writable>,
    ) -> anyhow::Result<values::Value<'v>> {
        let cmd = this.downcast_ref_err::<Command>().into_anyhow_result()?;
        cmd.inner.borrow_mut().stdout(stdio_from(io, "stdout")?);
        Ok(this)
    }

    /// Configuration for the child process's standard error (stderr) handle.
    ///
    /// Defaults to [`inherit`] when used with [`spawn`] or [`status`], and
    /// defaults to [`piped`] when used with [`output`].
    ///
    /// Accepts the same values as [`stdout`].
    fn stderr<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] io: Either<values::StringValue<'v>, stream::Writable>,
    ) -> anyhow::Result<values::Value<'v>> {
        let cmd = this.downcast_ref_err::<Command>().into_anyhow_result()?;
        cmd.inner.borrow_mut().stderr(stdio_from(io, "stderr")?);
        Ok(this)
    }

    /// Place the child in a new process group (Unix; no-op elsewhere).
    /// `terminate()` and `kill()` then signal the whole group, catching
    /// grandchildren a wrapper script leaves behind, and the OS shutdown
    /// handler does the same. The child no longer receives terminal signals
    /// (Ctrl-C) — the caller owns its lifecycle.
    fn process_group<'v>(this: values::Value<'v>) -> anyhow::Result<values::Value<'v>> {
        let cmd = this.downcast_ref_err::<Command>().into_anyhow_result()?;
        #[cfg(unix)]
        {
            std::os::unix::process::CommandExt::process_group(&mut *cmd.inner.borrow_mut(), 0);
            cmd.own_group.set(true);
        }
        Ok(this)
    }

    /// Executes the command as a child process, returning a handle to it.
    ///
    /// By default, stdin, stdout and stderr are inherited from the parent.
    ///
    /// On a cancelled run (Ctrl-C, or a CI job cancellation) spawned children
    /// are SIGTERMed and, after a short grace, SIGKILLed. Launch bazel through
    /// `ctx.bazel`, not here: the shutdown sequence never hard-kills the bazel
    /// clients it knows about (protecting sandbox state), but a `bazel`
    /// spawned as a plain command is killed like any other child.
    fn spawn<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Child> {
        let cmd = this.downcast_ref_err::<Command>().into_anyhow_result()?;
        let (child, guard) = cmd.try_spawn()?;
        Ok(Child {
            inner: RefCell::new(Some(child)),
            guard: RefCell::new(Some(guard)),
            own_group: cmd.own_group.get(),
        })
    }

    /// Executes a command as a child process, waiting for it to finish and collecting its status.
    /// Unlike `cmd.spawn().wait()` and `cmd.spawn().wait_with_output()`, this function does not
    /// close the stdin handle.
    ///
    /// By default, stdin, stdout and stderr are inherited from the parent.
    fn status<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<ExitStatus> {
        let cmd = this.downcast_ref_err::<Command>().into_anyhow_result()?;
        let status = cmd.try_status()?;
        Ok(ExitStatus(status))
    }
}

#[derive(Debug, Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.process.Child>")]
pub struct Child {
    #[allocative(skip)]
    inner: RefCell<Option<process::Child>>,
    #[allocative(skip)]
    guard: RefCell<Option<live::LiveChildGuard>>,
    own_group: bool,
}

impl<'v> AllocValue<'v> for Child {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "std.process.Child")]
impl<'v> values::StarlarkValue<'v> for Child {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("child_methods", child_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
pub(crate) fn child_methods(registry: &mut MethodsBuilder) {
    /// The handle for reading from the child’s standard output (stdout), if it has been captured.
    /// Calling this function more than once will yield error.
    fn stdout<'v>(this: values::Value<'v>) -> anyhow::Result<stream::Readable> {
        let child = this.downcast_ref_err::<Child>().into_anyhow_result()?;

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
        let child = this.downcast_ref_err::<Child>().into_anyhow_result()?;

        let mut inner = child.inner.borrow_mut();

        let inner = inner
            .as_mut()
            .ok_or(anyhow::anyhow!("child is no longer active"))?;

        let child_stderr = inner.stderr.take().ok_or(anyhow!(
            r#"stderr is not available. spawn the process with stderr("piped")."#
        ))?;

        Ok(stream::Readable::from(child_stderr))
    }

    /// The handle for writing to the child’s standard input (stdin), if it has been captured.
    /// Calling this function more than once will yield error.
    fn stdin<'v>(this: values::Value<'v>) -> anyhow::Result<stream::Writable> {
        let child = this.downcast_ref_err::<Child>().into_anyhow_result()?;

        let mut inner = child.inner.borrow_mut();

        let inner = inner
            .as_mut()
            .ok_or(anyhow::anyhow!("child is no longer active"))?;

        let child_stdin = inner.stdin.take().ok_or(anyhow!(
            r#"stdin is not available. spawn the process with stdin("piped")."#
        ))?;

        Ok(stream::Writable::from(child_stdin))
    }

    /// Returns the OS-assigned process identifier associated with this child.
    #[starlark(attribute)]
    fn id<'v>(this: values::Value<'v>) -> anyhow::Result<u32> {
        let child = this.downcast_ref_err::<Child>().into_anyhow_result()?;
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
        let child = this.downcast_ref_err::<Child>().into_anyhow_result()?;
        let mut inner = child.inner.borrow_mut();
        let inner = inner
            .as_mut()
            .ok_or(anyhow::anyhow!("child is no longer active"))?;
        // A group is signaled before the leader is reaped: its members can
        // outlive the leader, and `try_wait` frees the leader's pid for reuse
        // as a pgid — so this must precede the exit check below. The group
        // signal already reaches the leader, so no separate `inner.kill()`.
        if child.own_group {
            crate::engine::process::sigkill_group(inner.id());
            return Ok(NoneType);
        }
        if inner.try_wait()?.is_some() {
            let _ = child.guard.borrow_mut().take();
            return Ok(NoneType);
        }
        inner.kill()?;
        Ok(NoneType)
    }

    /// Asks the child process to exit gracefully. Sends SIGTERM on Unix and
    /// falls back to a forced kill on other platforms.
    ///
    /// The child is not reaped by this call — follow up with `wait()` or
    /// `try_wait()` to collect the exit status.
    fn terminate<'v>(this: values::Value<'v>) -> anyhow::Result<NoneType> {
        let child = this.downcast_ref_err::<Child>().into_anyhow_result()?;
        let mut inner = child.inner.borrow_mut();
        let inner = inner
            .as_mut()
            .ok_or(anyhow::anyhow!("child is no longer active"))?;
        // Signal the group before the exit check: members can outlive the
        // leader, and the `try_wait` below reaps the leader — freeing its pid
        // for reuse as a pgid, which a later group signal must not hit.
        #[cfg(unix)]
        if child.own_group {
            crate::engine::process::sigterm_group(inner.id());
        }
        // Avoid signalling a reused PID after the child exits.
        if inner.try_wait()?.is_some() {
            let _ = child.guard.borrow_mut().take();
            return Ok(NoneType);
        }
        #[cfg(unix)]
        if !child.own_group && !crate::engine::process::sigterm(inner.id()) {
            // Treat exit between try_wait and signal as success.
            if inner.try_wait()?.is_some() {
                let _ = child.guard.borrow_mut().take();
                return Ok(NoneType);
            }
            return Err(anyhow::anyhow!(
                "failed to deliver SIGTERM to pid {}",
                inner.id()
            ));
        }
        #[cfg(not(unix))]
        inner.kill()?;
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
        let child = this.downcast_ref_err::<Child>().into_anyhow_result()?;
        let status = child
            .inner
            .borrow_mut()
            .as_mut()
            .ok_or(anyhow::anyhow!("child is no longer active"))?
            .wait()?;
        let _ = child.guard.borrow_mut().take();
        Ok(ExitStatus(status))
    }

    /// Non-blocking check for child exit. Returns None if the child is still running,
    /// or ExitStatus if it has exited. Does not consume the child — stdout/stderr
    /// stream accessors remain callable after this returns a status, allowing pipe
    /// contents to be drained via child.stdout().read_to_string() etc.
    fn try_wait<'v>(
        this: values::Value<'v>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let child = this.downcast_ref_err::<Child>().into_anyhow_result()?;
        let status = child
            .inner
            .borrow_mut()
            .as_mut()
            .ok_or(anyhow::anyhow!("child is no longer active"))?
            .try_wait()?;
        Ok(match status {
            None => values::Value::new_none(),
            Some(s) => {
                let _ = child.guard.borrow_mut().take();
                heap.alloc(ExitStatus(s))
            }
        })
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
        let child = this.downcast_ref_err::<Child>().into_anyhow_result()?;
        let result = child
            .inner
            .replace(None)
            .ok_or(anyhow::anyhow!("child is no longer active"))?
            .wait_with_output();
        // The handle is consumed even on failure; drop the registration
        // with it.
        let _ = child.guard.borrow_mut().take();
        Ok(Output(result?))
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.process.ExitStatus>")]
pub struct ExitStatus(#[allocative(skip)] pub process::ExitStatus);

#[starlark_value(type = "std.process.ExitStatus")]
impl<'v> values::StarlarkValue<'v> for ExitStatus {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("exit_status_methods", exit_status_methods);
        Some(RES.methods())
    }
}
starlark_simple_value!(ExitStatus);

#[starlark_module]
pub(crate) fn exit_status_methods(registry: &mut MethodsBuilder) {
    /// Was termination successful? Signal termination is not considered a
    /// success, and success is defined as a zero exit status.
    #[starlark(attribute)]
    fn success<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let out = this.downcast_ref_err::<ExitStatus>().into_anyhow_result()?;
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
        let out = this.downcast_ref_err::<ExitStatus>().into_anyhow_result()?;
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
            let out = this.downcast_ref_err::<ExitStatus>().into_anyhow_result()?;
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
            let out = this.downcast_ref_err::<ExitStatus>().into_anyhow_result()?;
            Ok(NoneOr::from_option(out.0.stopped_signal()))
        }
        #[cfg(not(unix))]
        {
            Ok(NoneOr::None)
        }
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.process.Output>")]
pub struct Output(#[allocative(skip)] pub process::Output);

#[starlark_value(type = "std.process.Output")]
impl<'v> values::StarlarkValue<'v> for Output {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("output_methods", output_methods);
        Some(RES.methods())
    }
}
starlark_simple_value!(Output);

#[starlark_module]
pub(crate) fn output_methods(registry: &mut MethodsBuilder) {
    /// The status (exit code) of the process.
    #[starlark(attribute)]
    fn status<'v>(this: values::Value<'v>) -> anyhow::Result<ExitStatus> {
        let out = this.downcast_ref_err::<Output>().into_anyhow_result()?;
        Ok(ExitStatus(out.0.status))
    }

    /// The data that the process wrote to stderr.
    #[starlark(attribute)]
    fn stderr<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        let out = this.downcast_ref_err::<Output>().into_anyhow_result()?;
        Ok(String::from_utf8(out.0.stderr.clone())?)
    }

    /// The data that the process wrote to stdout.
    #[starlark(attribute)]
    fn stdout<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        let out = this.downcast_ref_err::<Output>().into_anyhow_result()?;
        Ok(String::from_utf8(out.0.stdout.clone())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_error_includes_command_and_args() {
        let program = "/nonexistent/program___axl_test";
        let cmd = Command {
            inner: RefCell::new(process::Command::new(program)),
            own_group: std::cell::Cell::new(false),
        };
        cmd.inner.borrow_mut().args(["--flag", "value"]);
        let err_msg = cmd.try_spawn().unwrap_err().to_string();
        assert!(err_msg.contains(program));
        assert!(err_msg.contains("--flag") && err_msg.contains("value"));
    }

    #[test]
    fn status_error_includes_command_and_args() {
        let program = "/nonexistent/program___axl_test";
        let cmd = Command {
            inner: RefCell::new(process::Command::new(program)),
            own_group: std::cell::Cell::new(false),
        };
        cmd.inner.borrow_mut().args(["--flag", "value"]);
        let err_msg = cmd.try_status().unwrap_err().to_string();
        assert!(err_msg.contains(program));
        assert!(err_msg.contains("--flag") && err_msg.contains("value"));
    }

    #[cfg(unix)]
    #[test]
    fn terminate_and_kill_are_noops_after_wait() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    c = ctx.std.process.command("sleep").arg("30").spawn()
    c.terminate()
    status = c.wait()
    if status.success:
        fail("SIGTERM'd child must not report success")
    if status.signal != 15:
        fail("expected SIGTERM (15), got %s" % status.signal)
    c.terminate()
    c.kill()
    return 0

Test = task(implementation = _impl)
"#,
        )
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// A wrapper that backgrounds a worker into its own process group and
    /// exits leaves the worker as an orphaned group member. `terminate()`
    /// must still signal the group even though the leader already exited — the
    /// P2 regression where the exit check short-circuited the group signal.
    #[cfg(unix)]
    #[test]
    fn terminate_reaches_orphaned_group_member() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    d = ctx.std.fs.mkdtemp(prefix = "pg-orphan")
    marker = d + "/worker.alive"
    # Leader backgrounds a worker that traps SIGTERM to clear the marker,
    # then exits immediately — so at terminate() time the leader is gone but
    # the worker still holds the group.
    script = "sh -c 'trap \"rm -f %s; exit 0\" TERM; touch %s; while true; do sleep 1; done' & exit 0" % (marker, marker)
    c = ctx.std.process.command("sh").arg("-c").arg(script).process_group().spawn()

    # Spin until the worker is up (it created the marker).
    up = False
    for _ in range(2000000):
        if ctx.std.fs.exists(marker):
            up = True
            break
    if not up:
        fail("worker never started")

    c.terminate()

    # Spin until the worker handled SIGTERM (it removed the marker). If the
    # group signal were skipped after the leader exited, this never clears.
    for _ in range(2000000):
        if not ctx.std.fs.exists(marker):
            return 0
    fail("orphaned group member survived terminate()")

Test = task(implementation = _impl)
"#,
        )
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    #[cfg(unix)]
    #[test]
    fn try_write_survives_dead_child() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    c = ctx.std.process.command("cat").stdin("piped").stdout("null").spawn()
    w = c.stdin()
    if w.try_write("ping\n") != 5:
        fail("write to a live child must succeed")
    c.kill()
    c.wait()
    if w.try_write("ping\n") != 0:
        fail("write to a dead child's pipe must report zero, not raise")
    return 0

Test = task(implementation = _impl)
"#,
        )
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    #[test]
    fn try_write_accepts_file_and_stdout_streams() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    d = ctx.std.fs.mkdtemp(prefix = "trywrite")
    f = ctx.std.fs.create(d + "/out.txt")
    if f.try_write("data") != 4:
        fail("file try_write must accept the buffer")
    f.close()
    if ctx.std.fs.read_to_string(d + "/out.txt") != "data":
        fail("file try_write must land on disk")
    ctx.std.io.stdout.try_write("")
    ctx.std.io.stderr.try_write("")
    return 0

Test = task(implementation = _impl)
"#,
        )
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    #[test]
    fn shutdown_spawn_refusal_is_a_quiet_task_exit() {
        let cmd = Command {
            inner: RefCell::new(process::Command::new("program")),
            own_group: std::cell::Cell::new(false),
        };
        let refusal = cmd.spawn_error(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "process shutdown is already in progress",
        ));
        assert!(refusal.downcast_ref::<TaskExit>().is_some());

        let failure = cmd.spawn_error(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no such file",
        ));
        assert!(failure.downcast_ref::<TaskExit>().is_none());
        assert!(failure.to_string().contains("program"));
    }

    #[test]
    fn describe_excludes_env_vars() {
        let cmd = Command {
            inner: RefCell::new(process::Command::new("program")),
            own_group: std::cell::Cell::new(false),
        };
        cmd.inner
            .borrow_mut()
            .args(["--flag1", "--flag2", "with spaces"])
            .env("SECRET_TOKEN", "super_secret");
        let description = cmd.describe();
        assert_eq!(description, r#"program "--flag1" "--flag2" "with spaces""#);
    }
}

/// Resolve `name` the way a shell does against `path` (the `PATH` value): a
/// bare name against each entry in order, a name with a separator as given.
/// The first existing executable wins. Behind `which` above.
fn find_executable(name: &str, path: Option<&std::ffi::OsStr>) -> Option<std::path::PathBuf> {
    let candidate = std::path::Path::new(name);
    if candidate.components().count() > 1 {
        return executable_variants(candidate)
            .into_iter()
            .find(|p| is_executable(p));
    }
    std::env::split_paths(path?)
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| executable_variants(&dir.join(name)))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
fn executable_variants(base: &std::path::Path) -> Vec<std::path::PathBuf> {
    vec![base.to_path_buf()]
}

#[cfg(windows)]
fn executable_variants(base: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut variants = vec![base.to_path_buf()];
    if let Some(exts) = std::env::var_os("PATHEXT") {
        for ext in std::env::split_paths(&exts) {
            let mut with_ext = base.as_os_str().to_owned();
            with_ext.push(ext.as_os_str());
            variants.push(std::path::PathBuf::from(with_ext));
        }
    }
    variants
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

#[cfg(all(test, unix))]
mod which_tests {
    use super::find_executable;
    use std::os::unix::fs::PermissionsExt;

    fn file(dir: &std::path::Path, name: &str, mode: u32) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        p
    }

    #[test]
    fn which_walks_path_in_order_and_requires_an_execute_bit() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        file(first.path(), "aspect", 0o644);
        let runnable = file(second.path(), "aspect", 0o755);
        let path = std::env::join_paths([first.path(), second.path()]).unwrap();

        // The non-executable file in the first entry is skipped; the second wins.
        assert_eq!(
            find_executable("aspect", Some(&path)),
            Some(runnable.clone())
        );
        assert_eq!(find_executable("bazel", Some(&path)), None);
        assert_eq!(find_executable("aspect", None), None);

        // A name with a separator is checked as given, not along PATH.
        assert_eq!(
            find_executable(runnable.to_str().unwrap(), Some(&path)),
            Some(runnable)
        );
        assert_eq!(
            find_executable(first.path().join("aspect").to_str().unwrap(), Some(&path)),
            None
        );
    }
}
