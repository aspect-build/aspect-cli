//! `ctx.std.process`: the child processes a task spawns.
//!
//! A stream the task does not set is the runtime's. An unmarked child reads
//! `/dev/null` and has its stdout and stderr piped through the CLI, flushed
//! as they come and copied to the task's output a line at a time; while a
//! full screen holds the terminal it has no controlling terminal either. One
//! marked `interactive(True)` gets the terminal, when there is one, and a
//! line before and after it in the task's output. A process that should
//! outlive the CLI sets its own stdout and stderr and leads its own group
//! (`process_group(0)`); one backgrounded by an unmarked child keeps its pipe,
//! and dies when it writes once the CLI, after a bounded wait, has gone.

use std::cell::{Cell, RefCell};
use std::process;
use std::process::Stdio;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

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

use super::stream;
use crate::engine::cancellation::{Signals, Token};
use crate::engine::children::{self, Bound};
use crate::engine::store::Env;
use crate::eval::TaskExit;
use bazel::process::signal::{self, Target};
use starlark::eval::Evaluator;
use std::sync::Arc;

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
            process_group: Cell::new(None),
            interactive: Cell::new(false),
            chosen: Cell::new(Chosen::default()),
            added_env: RefCell::new(vec![]),
            no_tty: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pre_exec_set: Cell::new(false),
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
    process_group: Cell<Option<i32>>,
    /// Marked `interactive(True)`: given the real terminal (see [`Command::try_spawn`]).
    #[allocative(skip)]
    interactive: Cell<bool>,
    /// Which of its streams the task chose itself.
    #[allocative(skip)]
    chosen: Cell<Chosen>,
    /// The colour variables a spawn added, for the next to take back.
    #[allocative(skip)]
    added_env: RefCell<Vec<String>>,
    /// Whether this spawn drops the controlling terminal, read by the one
    /// `pre_exec` closure the command gets ([`Command::unmarked`]).
    #[allocative(skip)]
    no_tty: Arc<std::sync::atomic::AtomicBool>,
    #[allocative(skip)]
    pre_exec_set: Cell<bool>,
}

/// How an unmarked child's output reaches the CLI ([`forward`]).
enum Forward {
    /// Its stdout and stderr in one pipe, the CLI's two being one file.
    Merged(std::io::PipeReader),
    /// Whether its stdout, and its stderr, are piped apart.
    Each(bool, bool),
}

/// Which of a command's streams the task set (`stdin()`, `stdout()`,
/// `stderr()`); the others are the runtime's to choose.
#[derive(Debug, Clone, Copy, Default)]
struct Chosen {
    stdin: bool,
    stdout: bool,
    stderr: bool,
}

impl Command {
    /// Wrap a command the runtime assembled (a runnable's), spawned by the
    /// same rules as any other. With `own_group` the child is spawned as the
    /// leader of a new process group, so stopping it stops everything it
    /// started.
    pub(crate) fn from_std(inner: process::Command, own_group: bool) -> Self {
        Self {
            inner: RefCell::new(inner),
            process_group: Cell::new(own_group.then_some(0)),
            interactive: Cell::new(false),
            chosen: Cell::new(Chosen::default()),
            added_env: RefCell::new(vec![]),
            no_tty: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pre_exec_set: Cell::new(false),
        }
    }

    /// Set, for this spawn, what the task left to the runtime: every stream
    /// it did not choose, the colour variables and whether the child drops
    /// the controlling terminal, so a reused command keeps nothing of the
    /// last spawn. An interactive child inherits the CLI's stdin, stdout and
    /// stderr. An unmarked one reads `/dev/null`, has its stdout and stderr
    /// piped (one pipe for both when the CLI's are one file, their order
    /// kept), colour forced on (`FORCE_COLOR`, `CLICOLOR_FORCE`) when its
    /// stdout reaches a colour terminal through the CLI unless the task set
    /// them or the CLI inherited them, and while a full screen holds the
    /// terminal, no controlling terminal.
    fn streams(&self, interactive: bool) -> anyhow::Result<Forward> {
        let chosen = self.chosen.get();
        let mut cmd = self.inner.borrow_mut();
        let forward = if interactive {
            for (set, which) in [(chosen.stdin, 0), (chosen.stdout, 1), (chosen.stderr, 2)] {
                if !set {
                    match which {
                        0 => cmd.stdin(Stdio::inherit()),
                        1 => cmd.stdout(Stdio::inherit()),
                        _ => cmd.stderr(Stdio::inherit()),
                    };
                }
            }
            Forward::Each(false, false)
        } else {
            if !chosen.stdin {
                cmd.stdin(Stdio::null());
            }
            let merged = !chosen.stdout
                && !chosen.stderr
                && bazel::console::capture::stdout_is_stderrs_file();
            if merged {
                let (read, write) = std::io::pipe()?;
                cmd.stdout(write.try_clone()?);
                cmd.stderr(write);
                Forward::Merged(read)
            } else {
                if !chosen.stdout {
                    cmd.stdout(Stdio::piped());
                }
                if !chosen.stderr {
                    cmd.stderr(Stdio::piped());
                }
                Forward::Each(!chosen.stdout, !chosen.stderr)
            }
        };

        // Colour as the stream the child's stdout reaches through the CLI
        // shows it; the task's own values, and what the CLI inherited, kept.
        let colour = !interactive && !chosen.stdout && crate::color::stdout_supports_color();
        let mut added = self.added_env.borrow_mut();
        for var in ["FORCE_COLOR", "CLICOLOR_FORCE"] {
            let ours = added.iter().any(|a| a == var);
            let theirs = !ours
                && (cmd.get_envs().any(|(k, _)| k.to_string_lossy() == var)
                    || std::env::var_os(var).is_some());
            if theirs {
                continue;
            }
            if colour {
                cmd.env(var, "1");
                if !ours {
                    added.push(var.to_string());
                }
            } else if ours {
                cmd.env_remove(var);
                added.retain(|a| a != var);
            }
        }

        // One closure for the command's life, which reads at each fork
        // whether this spawn drops the terminal.
        #[cfg(unix)]
        {
            self.no_tty.store(
                !interactive && crate::engine::std::term::screen_holds_terminal(),
                std::sync::atomic::Ordering::SeqCst,
            );
            if !self.pre_exec_set.replace(true) {
                let no_tty = self.no_tty.clone();
                // SAFETY: only async-signal-safe calls between fork and exec.
                // Dropping the controlling terminal leaves the process groups
                // as they are, so cancellation reaches the child as before.
                unsafe {
                    cmd.pre_exec(move || {
                        if no_tty.load(std::sync::atomic::Ordering::SeqCst) {
                            let fd =
                                libc::open(c"/dev/tty".as_ptr(), libc::O_RDWR | libc::O_NOCTTY);
                            if fd >= 0 {
                                libc::ioctl(fd, libc::TIOCNOTTY as _);
                                libc::close(fd);
                            }
                        }
                        Ok(())
                    });
                }
            }
        }
        Ok(forward)
    }

    /// The command for a marker line: its executable's name only, as an
    /// error names it ([`Command::describe`]); its arguments may hold a
    /// credential any tool takes its own way.
    fn summary(&self) -> String {
        let cmd = self.inner.borrow();
        let program = std::path::Path::new(cmd.get_program());
        program
            .file_name()
            .unwrap_or(program.as_os_str())
            .to_string_lossy()
            .into_owned()
    }

    /// Only the executable is safe to include in errors: arguments and
    /// environment variables may carry credentials (for example curl --user).
    fn describe(&self) -> String {
        self.inner
            .borrow()
            .get_program()
            .to_string_lossy()
            .into_owned()
    }

    /// Spawn, bound to `cancellation` (the root unless given): cancelling
    /// that token terminates the child, and kills it after the grace.
    fn try_spawn(
        &self,
        signals: &Arc<Signals>,
        cancellation: NoneOr<&Token>,
    ) -> anyhow::Result<Child> {
        let parent = match cancellation.into_option() {
            Some(token) => token.inner().clone(),
            None => signals.default_binding(),
        };
        // An interactive child is the terminal's while it runs, in the CLI's
        // own group, which the shell has in the foreground: Ctrl+Z, `fg` and
        // `bg` are the shell's as for the CLI. One leading its own group is
        // not in the foreground; a process meant to outlive the CLI is not
        // interactive.
        if self.interactive.get() && self.process_group.get().is_some() {
            return Err(anyhow!(
                "command {}: `interactive(True)` runs the child in the CLI's own process group, so it cannot take `process_group(...)`; a process meant to outlive the CLI sets its own stdout and stderr instead",
                self.describe()
            ));
        }
        // Interactive only where there is a terminal to give it: without one
        // (CI) it runs as any other child, piped and copied.
        let interactive = self.interactive.get() && has_terminal();
        let plan = self.streams(interactive)?;
        let marked = interactive.then(|| {
            let summary = self.summary();
            crate::errln!(
                "{}",
                marker_line(&format!("{summary} runs interactively"), false)
            );
            (
                summary,
                std::time::Instant::now(),
                crate::engine::std::term::suspend(),
            )
        });
        // The spawn's borrow of `inner` ends with this statement, before
        // `describe` borrows it again for the error.
        let spawned = children::spawn(
            &mut self.inner.borrow_mut(),
            signals,
            &parent,
            self.process_group.get(),
            children::Kind::Process,
        );
        if let Forward::Merged(_) = plan {
            // The command's own copies of the pipe's write end, closed so its
            // reader ends with the child's.
            let mut cmd = self.inner.borrow_mut();
            cmd.stdout(Stdio::null());
            cmd.stderr(Stdio::null());
        }
        let (mut child, bound) = match spawned {
            Ok(spawned) => spawned,
            Err(e) => {
                if marked.as_ref().is_some_and(|(_, _, suspended)| *suspended) {
                    crate::engine::std::term::resume();
                }
                return Err(anyhow!(
                    "failed to spawn command {}: {}",
                    self.describe(),
                    e
                ));
            }
        };
        let mut forwarders = vec![];
        match plan {
            Forward::Merged(read) => forwarders.push(forward(read, true)),
            Forward::Each(out, err) => {
                if out && let Some(pipe) = child.stdout.take() {
                    forwarders.push(forward(pipe, false));
                }
                if err && let Some(pipe) = child.stderr.take() {
                    forwarders.push(forward(pipe, true));
                }
            }
        }
        Ok(Child {
            inner: RefCell::new(Some(child)),
            bound,
            forwarders: RefCell::new(forwarders),
            interactive: RefCell::new(marked),
        })
    }

    fn try_status(
        &self,
        signals: &Arc<Signals>,
        cancellation: NoneOr<&Token>,
    ) -> anyhow::Result<process::ExitStatus> {
        let child = self.try_spawn(signals, cancellation)?;
        child
            .wait(None)
            .map_err(|e| anyhow!("failed to execute command {}: {}", self.describe(), e))?
            .ok_or_else(|| anyhow!("a wait without a timeout reports a status"))
    }
}

/// A muted `▹ text` line, as the console's updates are, in `error` for one
/// saying something failed; plain without colour.
fn marker_line(text: &str, failed: bool) -> String {
    let palette = crate::color::console_supports_color().then(crate::theme::of_process);
    let sgr = palette
        .map(|p| p.sgr(if failed { "error" } else { "muted" }))
        .unwrap_or_default();
    if sgr.is_empty() {
        format!("▹ {text}")
    } else {
        format!("{sgr}▹ {text}{}", aspect_theme::RESET)
    }
}

/// Whether the CLI has a terminal to give an interactive child: stdin and
/// stdout both one (through a full screen's capture, the screen's).
fn has_terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && crate::engine::stdout_is_terminal()
}

/// A thread passing a child's output on ([`forward`]).
struct Forwarder {
    handle: std::thread::JoinHandle<()>,
    activity: Arc<Activity>,
}

/// What a [`forward`] thread is doing, for a wait deciding whether the
/// child's output has all been passed on.
#[derive(Default)]
struct Activity {
    /// Writing what it read: a slow reader is progress.
    busy: std::sync::atomic::AtomicBool,
    /// When it last read, in ms of [`Activity::now`].
    read_ms: std::sync::atomic::AtomicU64,
}

impl Activity {
    fn now() -> u64 {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64
    }

    /// Neither writing nor having read for `grace`, counted from `since`
    /// (the reap) at the earliest: what the child wrote before it exited is
    /// in the pipe by then, so a quiet pipe after it is a grandchild's.
    fn quiet_since(&self, since: u64, grace: Duration) -> bool {
        use std::sync::atomic::Ordering::SeqCst;
        let last = self.read_ms.load(SeqCst).max(since);
        !self.busy.load(SeqCst) && Self::now().saturating_sub(last) >= grace.as_millis() as u64
    }
}

/// How long a pipe stays quiet, after its child is reaped, before its wait
/// stops waiting on it: a grandchild holding it, writing nothing.
const FORWARD_GRACE: Duration = Duration::from_millis(250);

impl std::fmt::Debug for Forwarder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Forwarder")
    }
}

/// Forwarders a wait left running, a grandchild holding the pipe: finished
/// at the CLI's exit ([`finish_forwarders`]).
static LEFT: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>> = std::sync::Mutex::new(Vec::new());

/// How long the output of a child that has exited is passed on before what
/// is left is let go: after its wait (a backgrounded grandchild holding the
/// pipe, writing or not), and again at the CLI's exit.
pub const FORWARD_BOUND: Duration = Duration::from_secs(2);

/// Let the forwarders a wait left running finish, for [`FORWARD_BOUND`] at
/// most: at the CLI's exit, so what they read reaches the terminal and
/// output.log.
pub fn finish_forwarders() {
    let deadline = std::time::Instant::now() + FORWARD_BOUND;
    let left = std::mem::take(&mut *LEFT.lock().unwrap());
    for f in left {
        while !f.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// The longest unended line a forwarder holds back from the output files.
const MAX_FORWARD_LINE: usize = 64 * 1024;

/// Pass what a child writes to `pipe` on to the CLI's stderr (or stdout),
/// flushed as it comes, a partial line (a progress line) as soon as nothing
/// more is waiting in the pipe,
/// and copied to the task's output a whole line at a time (an unended one
/// held back, up to [`MAX_FORWARD_LINE`], ended at the pipe's end), so it
/// never lands inside the CLI's own line. While a full screen holds the
/// terminal it goes into the screen's capture instead, whole lines too, so
/// neither its console nor the output files splice it with the CLI's. A
/// stream that takes no more (a closed pipe, `| head -1`) ends it and closes
/// `pipe`, so the child's next write fails, as it would have written there
/// itself.
fn forward(
    mut pipe: impl std::io::Read + std::os::fd::AsRawFd + Send + 'static,
    stderr: bool,
) -> Forwarder {
    use std::sync::atomic::Ordering::SeqCst;
    let activity = Arc::new(Activity::default());
    let mine = activity.clone();
    let handle = std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        // Not yet copied; its first `shown` bytes written out already.
        let mut line = Vec::new();
        let mut shown = 0;
        // Whether a show of the line being gathered went into a screen's
        // capture, which copies it: then this forwarder does not.
        let mut captured = false;
        let show = |bytes: &[u8]| crate::engine::std::term::write_std_unit(stderr, bytes);
        loop {
            match pipe.read(&mut buf) {
                Ok(0) => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
                Ok(n) => {
                    mine.busy.store(true, SeqCst);
                    mine.read_ms.store(Activity::now(), SeqCst);
                    line.extend_from_slice(&buf[..n]);
                    let whole = line
                        .iter()
                        .rposition(|&b| b == b'\n')
                        .map(|end| end + 1)
                        .or((line.len() > MAX_FORWARD_LINE).then_some(line.len()));
                    // A screen takes whole lines (a line begun before it was
                    // taken, from its start); without one, it is shown as it
                    // comes. Never while a screen is being released.
                    // The route is decided, and a screen fed, under the order
                    // lock, which a release holds while it replays; a write to
                    // the terminal is made after, without it, so a stream
                    // nobody reads holds up only this child's output.
                    let fed = crate::engine::std::term::forwarding(stderr, || {
                        crate::engine::std::term::feed_screen(
                            stderr,
                            &line[..whole.unwrap_or(0)],
                            shown,
                        )
                    });
                    // An unended tail waits while more is already there to
                    // read: it is shown once the child goes quiet (a progress
                    // line), so a flood leaves its stream at a line's end.
                    let upto = if crate::engine::std::term::pipe_readable(
                        pipe.as_raw_fd(),
                        Duration::ZERO,
                    ) {
                        whole.unwrap_or(0).max(shown)
                    } else {
                        line.len()
                    };
                    let written = if fed || upto == shown {
                        Ok(())
                    } else {
                        let r = show(&line[shown..upto]);
                        shown = upto;
                        r.map(|into_capture| captured |= into_capture)
                    };
                    if let Some(end) = whole {
                        if !captured {
                            crate::out::copy_only(&line[..end]);
                        }
                        captured = false;
                        line.drain(..end);
                        shown = shown.saturating_sub(end);
                    }
                    mine.busy.store(false, SeqCst);
                    if written.is_err() {
                        break;
                    }
                }
            }
        }
        if !line.is_empty() {
            let mut ended = line.clone();
            ended.push(b'\n');
            let fed = crate::engine::std::term::forwarding(stderr, || {
                crate::engine::std::term::feed_screen(stderr, &ended, shown)
            });
            if !fed && let Ok(into_capture) = show(&line[shown..]) {
                captured |= into_capture;
            }
            if !captured {
                crate::out::copy_only(&ended);
            }
        }
    });
    Forwarder { handle, activity }
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
pub(crate) fn stdio_from<'v>(
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

    /// Assign the command to process group `group`.
    ///
    /// On Unix, `0` starts the command as the leader of a new process group.
    /// Cancelling a grouped child's token, or calling [`Child.kill`],
    /// reaches every process in the group, not just the child. This setting
    /// has no effect on Windows.
    fn process_group<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] group: i32,
    ) -> anyhow::Result<values::Value<'v>> {
        let cmd = this.downcast_ref_err::<Command>().into_anyhow_result()?;
        if group < 0 {
            return Err(anyhow!("process group must be non-negative, got {group}"));
        }
        #[cfg(unix)]
        cmd.inner.borrow_mut().process_group(group);
        cmd.process_group.set(Some(group));
        Ok(this)
    }

    /// Mark the child interactive: it may prompt, so it gets the real
    /// terminal on stdin, stdout and stderr. While a full screen (the
    /// Dashboard) holds the terminal, the screen is given back for it as
    /// it starts and taken again as its wait ends. Its output is then the
    /// terminal's alone: a muted `▹ <program> runs interactively` line and
    /// `▹ … ran interactively (exit N, 1.2s); its output is in the
    /// terminal` after it are what the task's output keeps. An unmarked
    /// child has no terminal: stdin from `/dev/null`, stdout and stderr
    /// piped through the CLI (copied to the task's output, then passed on),
    /// and while a full screen holds the terminal, no controlling terminal
    /// either, so a surprise prompt fails rather than hangs. Mark a spawn
    /// interactive if it may prompt. It runs in the CLI's own process group,
    /// which the shell has in the foreground, so it cannot also take
    /// `process_group(...)`: spawning both fails.
    fn interactive<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] on: bool,
    ) -> anyhow::Result<values::Value<'v>> {
        let cmd = this.downcast_ref_err::<Command>().into_anyhow_result()?;
        cmd.interactive.set(on);
        Ok(this)
    }

    /// Configuration for the child process's standard input (stdin) handle.
    ///
    /// Unset, it is `"null"`, or for a child marked `interactive(True)` on a
    /// terminal, `"inherit"`.
    fn stdin<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] io: values::StringValue,
    ) -> anyhow::Result<values::Value<'v>> {
        let cmd = this.downcast_ref_err::<Command>().into_anyhow_result()?;
        cmd.chosen.set(Chosen {
            stdin: true,
            ..cmd.chosen.get()
        });
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
    /// Unset, it is piped through the CLI (copied to the task's output, then
    /// passed on to the CLI's own), or for a child marked `interactive(True)`
    /// on a terminal, `"inherit"`. A process that should outlive the CLI sets
    /// it (a file, `"null"` or `"inherit"`): a piped one dies when it writes
    /// once the CLI has gone.
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
        cmd.chosen.set(Chosen {
            stdout: true,
            ..cmd.chosen.get()
        });
        cmd.inner.borrow_mut().stdout(stdio_from(io, "stdout")?);
        Ok(this)
    }

    /// Configuration for the child process's standard error (stderr) handle.
    ///
    /// Unset, it is piped through the CLI (copied to the task's output, then
    /// passed on to the CLI's own), or for a child marked `interactive(True)`
    /// on a terminal, `"inherit"`. A process that should outlive the CLI sets
    /// it (a file, `"null"` or `"inherit"`): a piped one dies when it writes
    /// once the CLI has gone.
    ///
    /// Accepts the same values as [`stdout`].
    fn stderr<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] io: Either<values::StringValue<'v>, stream::Writable>,
    ) -> anyhow::Result<values::Value<'v>> {
        let cmd = this.downcast_ref_err::<Command>().into_anyhow_result()?;
        cmd.chosen.set(Chosen {
            stderr: true,
            ..cmd.chosen.get()
        });
        cmd.inner.borrow_mut().stderr(stdio_from(io, "stderr")?);
        Ok(this)
    }

    /// Executes the command as a child process, returning a handle to it.
    ///
    /// Stdin, stdout and stderr the task did not set are the runtime's: see
    /// [`interactive`] and [`stdin`]/[`stdout`]/[`stderr`]. A process that
    /// should outlive the CLI sets its stdout and stderr and leads its own
    /// group (`process_group(0)`), so a Ctrl+C on the CLI does not reach it.
    ///
    /// The child is bound to `cancellation`, `ctx.cancellation.root` unless
    /// given: when that token is cancelled (Ctrl+C for the root) the child is
    /// sent a terminate signal, to its whole process group if it leads one,
    /// and killed three seconds later if it is still running. Bind to
    /// `ctx.cancellation.new()` to keep a child out of that and stop it
    /// yourself. A child spawned from a post-task hook or `ctx.defer` after
    /// a cancel is cleanup, not part of what was cancelled: without an
    /// explicit token it gets one of its own and runs to completion.
    fn spawn<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] cancellation: NoneOr<&'v Token>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Child> {
        let cmd = this.downcast_ref_err::<Command>().into_anyhow_result()?;
        cmd.try_spawn(&Env::from_eval(eval)?.signals, cancellation)
    }

    /// Executes a command as a child process, waiting for it to finish and collecting its status.
    ///
    /// Its streams are chosen as for [`spawn`].
    /// The child is bound to `cancellation` as in [`spawn`], and the wait
    /// ends the task if the root is cancelled meanwhile.
    fn status<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] cancellation: NoneOr<&'v Token>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<ExitStatus> {
        let cmd = this.downcast_ref_err::<Command>().into_anyhow_result()?;
        if crate::engine::task_hooks::has_idle(eval) {
            let child = cmd.try_spawn(&Env::from_eval(eval)?.signals, cancellation)?;
            let status = child
                .wait_idle(None, eval)?
                .ok_or_else(|| anyhow!("a wait without a timeout reports a status"))?;
            return Ok(ExitStatus(status));
        }
        let status = cmd.try_status(&Env::from_eval(eval)?.signals, cancellation)?;
        Ok(ExitStatus(status))
    }
}

#[derive(Debug, Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.process.Child>")]
pub struct Child {
    #[allocative(skip)]
    inner: RefCell<Option<process::Child>>,
    /// The child's binding to its cancellation token; `child.cancellation`.
    #[allocative(skip)]
    bound: Bound,
    /// The threads passing on its piped stdout and stderr ([`forward`]).
    #[allocative(skip)]
    forwarders: RefCell<Vec<Forwarder>>,

    /// An interactive child's command, start, and whether it suspended a
    /// screen: for the line after it, and to take the screen again.
    #[allocative(skip)]
    interactive: RefCell<Option<(String, std::time::Instant, bool)>>,
}

impl Child {
    /// A child the runtime spawned through `children::spawn`, as the value
    /// AXL handles it by.
    pub(crate) fn from_parts(child: process::Child, bound: Bound) -> Self {
        Self {
            inner: RefCell::new(Some(child)),
            bound,
            forwarders: RefCell::new(vec![]),
            interactive: RefCell::new(None),
        }
    }

    /// Wait for the child to exit, a safe point: once the root is cancelled
    /// the task's exit comes back instead. `Ok(None)` when `timeout` passes
    /// first. Once it has exited, what it wrote is passed on in full, and an
    /// interactive one's screen taken again and its line written.
    fn wait(&self, timeout: Option<Duration>) -> std::io::Result<Option<process::ExitStatus>> {
        let status = children::wait(&self.inner, &self.bound, timeout)?;
        if let Some(status) = &status {
            self.ended(status);
        }
        Ok(status)
    }

    /// What follows the child's exit: its output passed on, and an
    /// interactive one's screen back and the line after it.
    fn ended(&self, status: &process::ExitStatus) {
        // Its output passed on until its pipes end, a pipe still open (a
        // grandchild holding it) until it has been quiet for [`FORWARD_GRACE`]
        // since the reap, for [`FORWARD_BOUND`] at most, or until the wait is
        // cancelled; what is left is finished at exit.
        let reaped = Activity::now();
        let deadline = std::time::Instant::now() + FORWARD_BOUND;
        for f in self.forwarders.borrow_mut().drain(..) {
            while !f.handle.is_finished()
                && std::time::Instant::now() < deadline
                && !f.activity.quiet_since(reaped, FORWARD_GRACE)
                && !self.bound.token().is_cancelled()
            {
                std::thread::sleep(Duration::from_millis(2));
            }
            if f.handle.is_finished() {
                let _ = f.handle.join();
            } else {
                LEFT.lock().unwrap().push(f.handle);
            }
        }
        if let Some((summary, started, suspended)) = self.interactive.borrow_mut().take() {
            if suspended {
                crate::engine::std::term::resume();
            }
            let code = status
                .code()
                .map_or_else(|| "signal".to_string(), |c| c.to_string());
            let took = started.elapsed();
            let text = format!(
                "{summary} ran interactively (exit {code}, {}.{}s); its output is in the terminal",
                took.as_secs(),
                took.subsec_millis() / 100
            );
            crate::errln!("{}", marker_line(&text, !status.success()));
        }
    }

    /// [`Child::wait`], running the task's idle hooks every
    /// [`IDLE_EVERY`](crate::engine::task_hooks::IDLE_EVERY) meanwhile when
    /// it has any.
    fn wait_idle<'v>(
        &self,
        timeout: Option<Duration>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> std::io::Result<Option<process::ExitStatus>> {
        use crate::engine::task_hooks::{IDLE_EVERY, has_idle, idle};
        if !has_idle(eval) {
            return self.wait(timeout);
        }
        let deadline = timeout.map(|t| std::time::Instant::now() + t);
        loop {
            let slice = deadline.map_or(IDLE_EVERY, |d| {
                d.saturating_duration_since(std::time::Instant::now())
                    .min(IDLE_EVERY)
            });
            if let Some(status) = self.wait(Some(slice))? {
                return Ok(Some(status));
            }
            if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                return Ok(None);
            }
            idle(eval);
        }
    }

    /// [`Child::wait`], for runtime code outside this module.
    pub(crate) fn wait_status(
        &self,
        timeout: Option<Duration>,
    ) -> std::io::Result<Option<process::ExitStatus>> {
        self.wait(timeout)
    }
}

impl crate::engine::r#async::future::FutureAlloc for Child {
    fn alloc_value_fut<'v>(self: Box<Self>, heap: Heap<'v>) -> values::Value<'v> {
        self.alloc_value(heap)
    }
}

impl Child {
    /// The child's pid, or the group it leads, for a signal sent now. `None`
    /// once the child was reaped: its pid may be someone else's by then, so a
    /// signal after `wait()` goes nowhere.
    fn target(&self) -> anyhow::Result<Option<Target>> {
        if self.inner.borrow().is_none() {
            return Err(anyhow::anyhow!("child is no longer active"));
        }
        Ok((!self.bound.exited()).then(|| self.bound.target()))
    }
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

    /// The child's cancellation token, a `child()` of the one passed as
    /// `cancellation =` at the spawn (`ctx.cancellation.root` by default).
    /// `child.cancellation.cancel()` stops this one child the way
    /// cancellation does: a terminate signal (to its whole process group when
    /// it leads one), then a kill three seconds later if needed.
    #[starlark(attribute)]
    fn cancellation<'v>(this: values::Value<'v>) -> anyhow::Result<Token> {
        let child = this.downcast_ref_err::<Child>().into_anyhow_result()?;
        Ok(Token::new(
            child.bound.token().clone(),
            child.bound.signals().clone(),
        ))
    }

    /// Send the child an interrupt now, what Ctrl+C at a terminal delivers
    /// (SIGINT; a console break on Windows), to its whole process group when
    /// it leads one. One signal, no escalation; `wait()` to see the result.
    fn interrupt<'v>(this: values::Value<'v>) -> anyhow::Result<NoneType> {
        let child = this.downcast_ref_err::<Child>().into_anyhow_result()?;
        if let Some(target) = child.target()? {
            signal::interrupt(target);
        }
        Ok(NoneType)
    }

    /// Ask the child to exit now (SIGTERM; a console break on Windows), to
    /// its whole process group when it leads one. One signal, no escalation;
    /// `wait()` to see the result.
    fn terminate<'v>(this: values::Value<'v>) -> anyhow::Result<NoneType> {
        let child = this.downcast_ref_err::<Child>().into_anyhow_result()?;
        if let Some(target) = child.target()? {
            signal::terminate(target);
        }
        Ok(NoneType)
    }

    /// Forces the child process to exit (SIGKILL), and with it every process
    /// in its process group when it leads one. If the child has already
    /// exited, it is a no-op.
    fn kill<'v>(this: values::Value<'v>) -> anyhow::Result<NoneType> {
        let child = this.downcast_ref_err::<Child>().into_anyhow_result()?;
        if let Some(target) = child.target()? {
            signal::kill(target);
        }
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
    ///
    /// The wait ends the task instead if `ctx.cancellation.root` is
    /// cancelled meanwhile and no `intercept()` is in effect; the child is
    /// already stopping by then. The task's idle hooks (`ctx.hooks.idle`)
    /// run meanwhile, as in every wait on a child.
    fn wait<'v>(
        this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<ExitStatus> {
        let child = this.downcast_ref_err::<Child>().into_anyhow_result()?;
        child
            .wait_idle(None, eval)?
            .map(ExitStatus)
            .ok_or_else(|| anyhow!("a wait without a timeout reports a status"))
    }

    /// Check for child exit. Returns None if the child is still running,
    /// or ExitStatus if it has exited. Does not consume the child — stdout/stderr
    /// stream accessors remain callable after this returns a status, allowing pipe
    /// contents to be drained via child.stdout().read_to_string() etc.
    ///
    /// Non-blocking by default; with `timeout_ms` it waits up to that long
    /// for the child to exit before returning `None`, a bounded wait for a
    /// loop that also watches a cancellation token.
    fn try_wait<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = 0)] timeout_ms: i32,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<values::Value<'v>> {
        let heap = eval.heap();
        let child = this.downcast_ref_err::<Child>().into_anyhow_result()?;
        if timeout_ms < 0 {
            anyhow::bail!("timeout_ms must not be negative: {timeout_ms}");
        }
        let status = if timeout_ms == 0 {
            let mut inner = child.inner.borrow_mut();
            let inner = inner
                .as_mut()
                .ok_or(anyhow::anyhow!("child is no longer active"))?;
            let status = children::try_wait(inner, &child.bound)?;
            if let Some(status) = &status {
                child.ended(status);
            }
            status
        } else {
            child.wait_idle(Some(Duration::from_millis(timeout_ms as u64)), eval)?
        };
        Ok(match status {
            None => values::Value::new_none(),
            Some(s) => heap.alloc(ExitStatus(s)),
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
    /// A stream the task did not set is passed on through the CLI, not
    /// collected: to collect it into this `Output`, set `stdout('piped')` or
    /// `stderr('piped')`.
    fn wait_with_output<'v>(
        this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Output> {
        use crate::engine::task_hooks::{IDLE_EVERY, has_idle, idle};
        let child = this.downcast_ref_err::<Child>().into_anyhow_result()?;
        let inner = child
            .inner
            .replace(None)
            .ok_or(anyhow::anyhow!("child is no longer active"))?;
        let output = if has_idle(eval) {
            children::wait_with_output_while(inner, &child.bound, IDLE_EVERY, || idle(eval))?
        } else {
            children::wait_with_output(inner, &child.bound)?
        };
        child.ended(&output.status);
        Ok(Output(output))
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

    /// A task's run of `script` under `sh`, as `command(...)` spawns it,
    /// `chain` appended to the command; what the script wrote to its file.
    fn run_child(script: &str, chain: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out");
        let path = path.display();
        let result = crate::test::eval(&format!(
            r#"
def _impl(ctx):
    ctx.std.process.command("sh").args(["-c", """{script}""", "sh", "{path}"]){chain}.spawn().wait()
    return 0

t = task(implementation = _impl)
"#
        ))
        .run_task(0);
        assert_eq!(result.expect("run_task"), Some(0));
        std::fs::read_to_string(dir.path().join("out")).unwrap_or_default()
    }

    /// An unmarked child reads no terminal: stdin is `/dev/null`, at its end
    /// at once.
    #[test]
    fn an_unmarked_child_reads_dev_null() {
        let got = run_child(
            r#"if [ -t 0 ]; then echo tty; else echo notty; fi > "$1"; if read x; then echo read >> "$1"; else echo eof >> "$1"; fi"#,
            "",
        );
        assert_eq!(got, "notty\neof\n");
    }

    /// A grandchild still holding the pipe does not hold the child's wait:
    /// once the child exits its output is waited on only briefly.
    #[test]
    fn a_grandchild_holding_the_pipe_does_not_hold_the_wait() {
        let started = std::time::Instant::now();
        let got = run_child(r#"sleep 5 & echo started > $1"#, "");
        assert_eq!(got, "started\n");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
    }

    /// A grandchild that keeps writing holds the child's wait for
    /// `FORWARD_BOUND` at most: the task moves on.
    #[test]
    fn a_chatty_grandchild_holds_the_wait_for_the_bound_at_most() {
        let started = std::time::Instant::now();
        let got = run_child(
            r#"(i=0; while [ $i -lt 50 ]; do echo x; sleep 0.2; i=$((i+1)); done) & echo started > $1"#,
            "",
        );
        assert_eq!(got, "started\n");
        assert!(
            started.elapsed() < FORWARD_BOUND + Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    /// An interactive child runs in the CLI's own group: one asked to lead
    /// its own is refused at spawn, saying why.
    #[test]
    fn interactive_and_a_process_group_are_refused() {
        let result = crate::test::eval(
            r#"
def _impl(ctx):
    ctx.std.process.command("true").interactive(True).process_group(0).spawn().wait()
    return 0

t = task(implementation = _impl)
"#,
        )
        .run_task(0);
        let err = format!("{:#}", result.expect_err("refused"));
        assert!(err.contains("cannot take `process_group(...)`"), "{err}");
    }

    /// A command spawned twice decides each spawn afresh, its exit polled
    /// with `try_wait` as well as waited.
    #[test]
    fn a_reused_command_spawns_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out");
        let path = path.display();
        let result = crate::test::eval(&format!(
            r#"
load("@std//time.axl", "sleep")

def _impl(ctx):
    cmd = ctx.std.process.command("sh").args(["-c", "echo x >> $1", "sh", "{path}"])
    cmd.spawn().wait()
    child = cmd.spawn()
    for _ in range(1000):
        if child.try_wait() != None:
            break
        sleep(5)
    return 0

t = task(implementation = _impl)
"#
        ))
        .run_task(0);
        assert_eq!(result.expect("run_task"), Some(0));
        let got = std::fs::read_to_string(dir.path().join("out")).unwrap();
        assert_eq!(got, "x\nx\n");
    }

    /// A task's own `FORCE_COLOR` is the child's, never overridden.
    #[test]
    fn a_tasks_own_force_color_is_kept() {
        let got = run_child(r#"echo "$FORCE_COLOR" > $1"#, r#".env("FORCE_COLOR", "0")"#);
        assert_eq!(got, "0\n");
    }

    /// An interactive child's marker names only its executable: an argument
    /// may hold a credential.
    #[test]
    fn a_marker_names_only_the_executable() {
        let cmd = Command::from_std(process::Command::new("/usr/bin/curl"), false);
        cmd.inner
            .borrow_mut()
            .args(["--user", "me:s3cret", "https://example.com"]);
        assert_eq!(cmd.summary(), "curl");
    }

    #[test]
    fn spawn_error_excludes_sensitive_args() {
        let program = "/nonexistent/program___axl_test";
        let cmd = Command {
            inner: RefCell::new(process::Command::new(program)),
            process_group: Cell::new(None),
            interactive: Cell::new(false),
            chosen: Cell::new(Chosen::default()),
            added_env: RefCell::new(vec![]),
            no_tty: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pre_exec_set: Cell::new(false),
        };
        cmd.inner.borrow_mut().args([
            "--aws-sigv4",
            "aws:amz:us-gov-west-1:s3",
            "--user",
            "ASIAFAKE:fake-secret-key",
            "-H",
            "x-amz-security-token: fake-session-token",
        ]);
        let err_msg = cmd
            .try_spawn(&Signals::new(), NoneOr::None)
            .unwrap_err()
            .to_string();
        assert!(err_msg.contains(program));
        assert!(!err_msg.contains("ASIAFAKE"));
        assert!(!err_msg.contains("fake-secret-key"));
        assert!(!err_msg.contains("fake-session-token"));
        assert!(!err_msg.contains("--user"));
    }

    #[test]
    fn status_error_excludes_sensitive_args() {
        let program = "/nonexistent/program___axl_test";
        let cmd = Command {
            inner: RefCell::new(process::Command::new(program)),
            process_group: Cell::new(None),
            interactive: Cell::new(false),
            chosen: Cell::new(Chosen::default()),
            added_env: RefCell::new(vec![]),
            no_tty: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pre_exec_set: Cell::new(false),
        };
        cmd.inner
            .borrow_mut()
            .args(["--user", "ASIAFAKE:fake-secret-key"]);
        let err_msg = cmd
            .try_status(&Signals::new(), NoneOr::None)
            .unwrap_err()
            .to_string();
        assert!(err_msg.contains(program));
        assert!(!err_msg.contains("--user"));
        assert!(!err_msg.contains("fake-secret-key"));
    }

    #[test]
    fn describe_excludes_args_and_env_vars() {
        let cmd = Command {
            inner: RefCell::new(process::Command::new("program")),
            process_group: Cell::new(None),
            interactive: Cell::new(false),
            chosen: Cell::new(Chosen::default()),
            added_env: RefCell::new(vec![]),
            no_tty: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pre_exec_set: Cell::new(false),
        };
        cmd.inner
            .borrow_mut()
            .args(["--flag1", "--flag2", "with spaces"])
            .env("SECRET_TOKEN", "super_secret");
        let description = cmd.describe();
        assert_eq!(description, "program");
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
