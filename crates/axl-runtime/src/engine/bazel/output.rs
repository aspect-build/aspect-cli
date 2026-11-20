//! The `bazel.output` namespace: the stages a console pipeline is built from,
//! the processor a build captures its console with, and the lines an
//! `intercept` stage holds for the `BazelTrait.output_intercept` hooks
//! ([`TraitConsole`]). The capture and pipeline themselves are
//! [`bazel::console`].

use std::cell::RefCell;
use std::io::IsTerminal;
use std::sync::mpsc;
use std::time::Duration;

use allocative::Allocative;
use bazel::console::processors::{HeldLine, Verdict};
use bazel::console::report::Style;
use bazel::console::{Config, Mode, Output, Stage};
use derive_more::Display;
use starlark::StarlarkResultExt;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::values::dict::UnpackDictEntries;
use starlark::values::list::UnpackList;
use starlark::values::none::{NoneOr, NoneType};
use starlark::values::starlark_value_as_type::StarlarkValueAsType;
use starlark::values::{
    self, AllocValue, Heap, NoSerialize, ProvidesStaticType, Trace, UnpackValue, Value, ValueLike,
    starlark_value,
};

use starlark::eval::Evaluator;

/// One `bazel.output.<stage>(...)`: a [`Stage`] as a Starlark value, for a
/// processor's `stages` or `BazelTrait.output_stages`.
#[derive(Clone, Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.output.OutputStage {}>", stage.name())]
pub struct OutputStage {
    #[allocative(skip)]
    stage: Stage,
}

impl<'v> AllocValue<'v> for OutputStage {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> UnpackValue<'v> for OutputStage {
    type Error = anyhow::Error;
    fn unpack_value_impl(value: Value<'v>) -> Result<Option<Self>, Self::Error> {
        Ok(value.downcast_ref::<OutputStage>().cloned())
    }
}

#[starlark_value(type = "bazel.output.OutputStage")]
impl<'v> values::StarlarkValue<'v> for OutputStage {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // The constructor that made it: "collapse_similar", "drop", ...
            "name" => Some(heap.alloc_str(self.stage.name()).to_value()),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        attribute == "name"
    }
}

fn stage(stage: Result<Stage, bazel::console::PatternError>) -> anyhow::Result<OutputStage> {
    Ok(OutputStage { stage: stage? })
}

/// One `[kind, capture, max]` of `bazel.output.aggregate`'s `aggregations`.
fn aggregation(
    name: &str,
    spec: &[Value],
) -> anyhow::Result<bazel::console::aggregate::Aggregation> {
    use bazel::console::aggregate::Aggregation as A;
    let text = |i: usize| spec.get(i).and_then(|v| v.unpack_str()).map(str::to_string);
    let kind = text(0).ok_or_else(|| {
        anyhow::anyhow!("aggregation {name:?}: expected [kind, capture, max], got {spec:?}")
    })?;
    let most = match kind.as_str() {
        "count" => 1,
        "count_distinct" | "first" | "last" => 2,
        _ => 3,
    };
    if spec.len() > most {
        anyhow::bail!("aggregation {name:?}: {kind} takes at most {most} items, got {spec:?}");
    }
    if spec.get(1).is_some_and(|v| v.unpack_str().is_none()) {
        anyhow::bail!("aggregation {name:?}: its capture must be a string, got {spec:?}");
    }
    let capture =
        || text(1).ok_or_else(|| anyhow::anyhow!("aggregation {name:?}: {kind} needs a capture"));
    let max = || -> anyhow::Result<usize> {
        match spec.get(2) {
            None => Ok(1000),
            Some(v) => match v.unpack_i32() {
                Some(n) if n > 0 => Ok(n as usize),
                _ => anyhow::bail!("aggregation {name:?}: max must be a positive int, got {v}"),
            },
        }
    };
    Ok(match kind.as_str() {
        "count" => A::Count,
        "unique" => A::Unique {
            capture: capture()?,
            max: max()?,
        },
        "all" => A::All {
            capture: capture()?,
            max: max()?,
        },
        "count_distinct" => A::CountDistinct {
            capture: capture()?,
        },
        "count_by" => A::CountBy {
            capture: capture()?,
            max: max()?,
        },
        "first" => A::First {
            capture: capture()?,
        },
        "last" => A::Last {
            capture: capture()?,
        },
        other => anyhow::bail!(
            "aggregation {name:?}: unknown kind {other:?}; valid: count, unique, all, count_distinct, count_by, first, last"
        ),
    })
}

/// `bazel.output.processor(...)`: a [`Config`] as a Starlark value. Carries
/// configuration only, so one processor serves every attempt of a retried
/// build; each spawn builds its own pipeline from it.
#[derive(Clone, Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.output.OutputProcessor>")]
pub struct OutputProcessor {
    #[allocative(skip)]
    config: Config,
}

impl OutputProcessor {
    pub fn config(&self) -> &Config {
        &self.config
    }
}

impl<'v> AllocValue<'v> for OutputProcessor {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> UnpackValue<'v> for OutputProcessor {
    type Error = anyhow::Error;
    fn unpack_value_impl(value: Value<'v>) -> Result<Option<Self>, Self::Error> {
        Ok(value.downcast_ref::<OutputProcessor>().cloned())
    }
}

#[starlark_value(type = "bazel.output.OutputProcessor")]
impl<'v> values::StarlarkValue<'v> for OutputProcessor {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // Whether bazel's curses UI is kept in what is shown.
            "tty" => Some(Value::new_bool(self.config.output == Output::AsWritten)),
            // The `--isatty` / `--curses` (and, see `processor`, `--color`)
            // flags the capture needs.
            "flags" => Some(heap.alloc(self.config.bazel_flags())),
            // Whether it has an `intercept` stage, i.e. a build using it holds
            // lines for the `BazelTrait.output_intercept` hooks.
            "intercepts" => Some(Value::new_bool(self.config.intercepts())),
            // Whether `flags` asks bazel for colour itself.
            "colour" => Some(Value::new_bool(self.config.colour)),
            // The stage names, in the order they run.
            "stages" => Some(
                heap.alloc(
                    self.config
                        .stages()
                        .iter()
                        .map(|s| s.name())
                        .collect::<Vec<_>>(),
                ),
            ),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(
            attribute,
            "tty" | "flags" | "intercepts" | "stages" | "colour"
        )
    }
}

/// A captured line an `intercept` stage holds, as the hooks get it. Answer
/// it once with `keep()`, `replace(text)` or `drop()`; unanswered by every
/// hook, it is kept. `line` is the line's text, escape sequences stripped.
#[derive(Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.output.InterceptedLine id={id}>")]
pub struct InterceptedLine {
    id: String,
    line: String,
    #[allocative(skip)]
    reply: RefCell<Option<mpsc::SyncSender<Verdict>>>,
}

impl InterceptedLine {
    fn new(held: HeldLine) -> Self {
        Self {
            id: held.id,
            line: held.line,
            reply: RefCell::new(Some(held.reply)),
        }
    }

    fn answer(&self, verdict: Verdict) -> anyhow::Result<NoneType> {
        let reply = self.reply.borrow_mut().take().ok_or_else(|| {
            anyhow::anyhow!("this InterceptedLine was already answered; answer each one once")
        })?;
        // A closed channel means the line already timed out and was forwarded.
        let _ = reply.send(verdict);
        Ok(NoneType)
    }

    fn answered(&self) -> bool {
        self.reply.borrow().is_none()
    }
}

impl<'v> AllocValue<'v> for InterceptedLine {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "bazel.output.InterceptedLine")]
impl<'v> values::StarlarkValue<'v> for InterceptedLine {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("intercepted_line_methods", intercepted_line_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "id" => Some(heap.alloc_str(&self.id).to_value()),
            "line" => Some(heap.alloc_str(&self.line).to_value()),
            // Whether `keep()`, `replace()` or `drop()` was called.
            "answered" => Some(Value::new_bool(self.answered())),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "id" | "line" | "answered")
    }
}

#[starlark_module]
fn intercepted_line_methods(registry: &mut MethodsBuilder) {
    /// Release the held line unchanged.
    fn keep<'v>(this: Value<'v>) -> anyhow::Result<NoneType> {
        let ev = this
            .downcast_ref_err::<InterceptedLine>()
            .into_anyhow_result()?;
        ev.answer(Verdict::Keep)
    }

    /// Forward `text` in place of the held line's text, before its original
    /// line ending; escape sequences around the text (a PTY's cursor movement
    /// and colours) are kept. `text` may contain newlines.
    fn replace<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] text: &str,
    ) -> anyhow::Result<NoneType> {
        let ev = this
            .downcast_ref_err::<InterceptedLine>()
            .into_anyhow_result()?;
        ev.answer(Verdict::Replace(text.as_bytes().to_vec()))
    }

    /// Suppress the held line, line ending included.
    fn drop<'v>(this: Value<'v>) -> anyhow::Result<NoneType> {
        let ev = this
            .downcast_ref_err::<InterceptedLine>()
            .into_anyhow_result()?;
        ev.answer(Verdict::Drop)
    }
}

/// What the running task's `BazelTrait` (the trait with its `output_*`
/// fields, [`CONSOLE_FIELDS`]) says about the console: the stages a build
/// with no `output =` and no redirected stdout or stderr is processed with,
/// and the hooks every build's console calls. Read from the task context the runtime records
/// ([`crate::engine::module_values::task_context`]); a task without the trait has
/// none of it.
pub(crate) struct TraitConsole<'v> {
    pub ctx: Value<'v>,
    pub stages: Vec<Stage>,
    pub tty: Option<bool>,
    /// `output_progress`: rendered lines show Bazel's status line.
    pub progress: bool,
    /// `output_intercept`: `fn(ctx, line)`, answering held lines.
    pub intercept: Vec<Value<'v>>,
    /// `output_end`: `fn(ctx, build)`, once a build's console has ended.
    pub end: Vec<Value<'v>>,
    /// `output_start`: `fn(ctx)` returning more stages for a build about
    /// to start ([`TraitConsole::started`]).
    pub start: Vec<Value<'v>>,
}

/// The fields that make a trait the console's: `BazelTrait`'s. Others it
/// reads (`output_progress`) are optional, so a trait need not declare them.
const CONSOLE_FIELDS: [&str; 5] = [
    "output_stages",
    "output_tty",
    "output_intercept",
    "output_end",
    "output_start",
];

impl<'v> TraitConsole<'v> {
    pub(crate) fn of(eval: &Evaluator<'v, '_, '_>) -> Option<TraitConsole<'v>> {
        let ctx = crate::engine::module_values::task_context(eval)?;
        let traits = ctx
            .downcast_ref::<crate::engine::task_context::TaskContext>()?
            .traits;
        let heap = eval.heap();
        let instance = traits
            .downcast_ref::<crate::engine::trait_map::TraitMap>()?
            .with_fields(&CONSOLE_FIELDS, heap)?;
        let field = |name: &str| instance.get_attr(name, heap).ok().flatten();
        let list = |name: &str| -> Vec<Value<'v>> {
            field(name)
                .and_then(|v| v.iterate(heap).ok().map(|it| it.collect()))
                .unwrap_or_default()
        };
        Some(TraitConsole {
            ctx,
            stages: list("output_stages")
                .into_iter()
                .filter_map(|v| v.downcast_ref::<OutputStage>().map(|s| s.stage.clone()))
                .collect(),
            tty: field("output_tty").and_then(|v| v.unpack_bool()),
            progress: field("output_progress")
                .and_then(|v| v.unpack_bool())
                .unwrap_or(true),
            intercept: list("output_intercept"),
            end: list("output_end"),
            start: list("output_start"),
        })
    }

    /// The stages the `start` hooks return for a build about to start, read
    /// then rather than when the trait was configured, so they reflect
    /// whatever every feature and config set.
    pub(crate) fn started(&self, eval: &mut Evaluator<'v, '_, '_>) -> anyhow::Result<Vec<Stage>> {
        let mut stages = vec![];
        for hook in &self.start {
            let got = eval
                .eval_function(*hook, &[self.ctx], &[])
                .map_err(starlark::Error::into_anyhow)?;
            if got.is_none() {
                continue;
            }
            for v in got
                .iterate(eval.heap())
                .map_err(starlark::Error::into_anyhow)?
            {
                let stage = v.downcast_ref::<OutputStage>().ok_or_else(|| {
                    anyhow::anyhow!(
                        "a `BazelTrait.output_start` hook returned {}, not a `bazel.output` stage",
                        v.get_type()
                    )
                })?;
                stages.push(stage.stage.clone());
            }
        }
        Ok(stages)
    }

    /// Offer `held` to the `intercept` hooks in order until one answers;
    /// keep it when none does.
    pub(crate) fn answer(
        &self,
        held: HeldLine,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<()> {
        let line = eval.heap().alloc(InterceptedLine::new(held));
        let held = line
            .downcast_ref::<InterceptedLine>()
            .expect("allocated above");
        for hook in &self.intercept {
            if let Err(e) = eval.eval_function(*hook, &[self.ctx, line], &[]) {
                // The reply lives as long as the heap does: unanswered, the
                // line would wait out its whole timeout.
                if !held.answered() {
                    held.answer(Verdict::Keep)?;
                }
                return Err(e.into_anyhow());
            }
            if held.answered() {
                return Ok(());
            }
        }
        held.answer(Verdict::Keep)?;
        Ok(())
    }

    /// Call the `end` hooks with `build`.
    pub(crate) fn ended(
        &self,
        build: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<()> {
        for hook in &self.end {
            eval.eval_function(*hook, &[self.ctx, build], &[])
                .map_err(starlark::Error::into_anyhow)?;
        }
        Ok(())
    }
}

/// How often rendered lines show Bazel's status line, with `progress`.
const STATUS_EVERY: Duration = Duration::from_secs(5);

/// What a console processor is made from: `processor(...)`'s arguments, and
/// what a build without `output =` or a redirect takes from the trait and
/// the user's flags.
pub(crate) struct ProcessorSettings {
    /// Keep Bazel's curses UI in the output; None: whether this process's
    /// stdout and stderr are both terminals.
    pub tty: Option<bool>,
    pub stages: Vec<Stage>,
    /// Ask Bazel for colour; None asks.
    pub colour: Option<bool>,
    /// The user's own `--color`; None: colour shows on a terminal.
    pub show_colour: Option<bool>,
    /// In rendered lines, Bazel's status line every few seconds.
    pub progress: bool,
}

/// The console processor `processor(...)` makes; see its docs.
pub(crate) fn processor_config(s: ProcessorSettings) -> Config {
    let stderr_tty = std::io::stderr().is_terminal();
    let curses_out = s
        .tty
        .unwrap_or_else(|| std::io::stdout().is_terminal() && stderr_tty);
    let style = if crate::color::console_supports_color() {
        Style::dim()
    } else {
        Style::default()
    };
    // Curses always: its redraw after every message is what ends an action's
    // output for the stages. The output then follows the destination.
    // Where its raw log goes is the build's to say, at its spawn.
    let mut config = Config::new(Mode::Pty, s.stages, style);
    config.pty = curses_out && stderr_tty;
    config.output = if curses_out {
        Output::AsWritten
    } else {
        Output::Lines {
            status_every: s.progress.then_some(STATUS_EVERY),
        }
    };
    // Colour lets the stages tell bazel's messages from tool output; what is
    // shown keeps it as the user asked, else on a terminal.
    config.colour = s.colour.unwrap_or(true);
    config.user_colour = s.show_colour;
    config.strip_colour = !s.show_colour.unwrap_or(stderr_tty);
    config.links = stderr_tty;
    config
}

/// The user's own choice of a tri-state flag such as `--curses` or `--color`
/// in `flags`, the last one given: `yes`/`true`/`1` or the bare flag, `no`/
/// `false`/`0` or `--no<name>`; None when unset or `auto`.
pub(crate) fn user_choice(flags: &[String], name: &str) -> Option<bool> {
    let mut choice = None;
    for f in flags {
        let Some(rest) = f.strip_prefix("--") else {
            continue;
        };
        if rest == name {
            choice = Some(true);
        } else if rest.strip_prefix("no") == Some(name) {
            choice = Some(false);
        } else if let Some(v) = rest.strip_prefix(name).and_then(|r| r.strip_prefix('=')) {
            choice = match v.to_ascii_lowercase().as_str() {
                "yes" | "true" | "1" => Some(true),
                "no" | "false" | "0" => Some(false),
                _ => None,
            };
        }
    }
    choice
}

#[starlark_module]
pub(crate) fn register_output(globals: &mut GlobalsBuilder) {
    const OutputProcessor: StarlarkValueAsType<OutputProcessor> = StarlarkValueAsType::new();
    const InterceptedLine: StarlarkValueAsType<InterceptedLine> = StarlarkValueAsType::new();

    const OutputStage: StarlarkValueAsType<OutputStage> = StarlarkValueAsType::new();

    /// A console processor. Pass it as `output =` to `ctx.bazel.build` or
    /// `ctx.bazel.test`: the client's stderr is then captured, run through
    /// `stages` and forwarded to this process's stderr, instead of being
    /// inherited. A build given none, and redirecting neither stdout nor
    /// stderr, is processed with the task's `BazelTrait.output_stages`, when
    /// it has any.
    ///
    /// Bazel is always asked for its curses UI (`--curses=yes`), and for
    /// colour unless `colour = False`:
    /// its redraw after every message is what ends an action's output, and
    /// its colour is what tells its messages from the output, so the stages
    /// can tell them apart in any build. What is shown then follows the
    /// destination: the curses UI as Bazel drew it (`tty`), or plain lines,
    /// the progress area it drew and erased left out (with `progress`, its
    /// status line every few seconds). Colour shows as `show_colour` says.
    /// The build must carry `processor.flags` after the user's own, so they
    /// win (`--isatty`, `--curses=yes`, `--color=yes` unless `colour =
    /// False`; they assume the
    /// build's stdout is inherited); the user's `--curses` and `--color` are
    /// what `tty` and `show_colour` are for. The `bazel_invocation` handles
    /// and a build without `output =` or a redirect do this themselves.
    ///
    /// Stages run in a fixed order of kinds, whatever order they are listed
    /// in, and in list order within a kind: `observe`/`aggregate`/`fatal`, then
    /// `intercept`, then `rewrite`/`relativize_paths`, then `drop`, then
    /// `recap_errors`, then `collapse_similar`, then
    /// `collapse_repeats`. Observers so see every line as Bazel wrote it, and
    /// a collapser folds what is left to show. Patterns use
    /// [regex](https://docs.rs/regex) syntax and match a line's text with
    /// escape sequences and a `--show_timestamps` prefix stripped.
    ///
    /// A processor with a stage that changes what is shown keeps the console
    /// as Bazel wrote it (its stderr, and its stdout when the two are one
    /// file) in a raw log (`build.output_log`), and ends the
    /// console with a summary naming it: how many lines were hidden, and for
    /// each kind `collapse_similar` folded, a file beside the log with every
    /// message of that kind. On a terminal the paths are links. The log is
    /// `console.log` in the Bazel invocation's folder in the task's history,
    /// with the folded kinds' `console.collapsed-<n>.log` beside it, kept and
    /// pruned as the rest of the history is (in the temp dir when no history
    /// folder can be made). It holds whatever a stage redacted, so it and its
    /// files are readable by their owner only.
    ///
    /// # Arguments
    /// * `tty` - Show Bazel's curses UI as it drew it, rather than plain
    ///   lines. Defaults to whether this process's stdout and stderr are both
    ///   terminals. Pass the user's `--curses` when they gave one: it is
    ///   their choice of what they see. Captured through a PTY when shown on
    ///   a terminal, otherwise a pipe.
    /// * `stages` - `bazel.output.<stage>(...)` values.
    /// * `colour` - Ask Bazel for colour (`--color=yes` in `flags`). Default
    ///   True; for a build whose stdout is not inherited, pass False and
    ///   `tty = False`: nothing strips colour from that stdout, and a PTY
    ///   would have Bazel colour it too.
    /// * `show_colour` - Whether what is shown keeps colour: the user's own
    ///   `--color`, when they gave one. Defaults to whether stderr (and, for
    ///   a separate stdout, stdout) is a terminal.
    /// * `progress` - In plain lines, show Bazel's status line (`[n / m] …`)
    ///   every few seconds. Default True; the agent profile passes False.
    ///
    /// **Example**
    ///
    /// ```python
    /// processor = bazel.output.processor(stages = [
    ///     bazel.output.relativize_paths(root = ctx.std.env.bazel_root_dir()),
    ///     bazel.output.collapse_similar(),
    ///     bazel.output.fatal([r"Server terminated abruptly"]),
    /// ])
    /// build = ctx.bazel.build("//...", output = processor, flags = processor.flags)
    /// status = build.wait()
    /// if build.output_fatal:
    ///     print("bazel server died: " + build.output_fatal_line)
    /// ```
    fn processor(
        #[starlark(require = named, default = NoneOr::None)] tty: NoneOr<bool>,
        #[starlark(require = named, default = UnpackList::default())] stages: UnpackList<
            OutputStage,
        >,
        #[starlark(require = named, default = NoneOr::None)] colour: NoneOr<bool>,
        #[starlark(require = named, default = NoneOr::None)] show_colour: NoneOr<bool>,
        #[starlark(require = named, default = true)] progress: bool,
    ) -> anyhow::Result<OutputProcessor> {
        let config = processor_config(ProcessorSettings {
            tty: tty.into_option(),
            stages: stages.items.into_iter().map(|s| s.stage).collect(),
            colour: colour.into_option(),
            show_colour: show_colour.into_option(),
            progress,
        });
        Ok(OutputProcessor { config })
    }

    /// Why `pattern` is not a valid regex for a stage, or None when it is: for
    /// a caller that would rather warn about a bad pattern than fail.
    fn pattern_error(#[starlark(require = pos)] pattern: &str) -> anyhow::Result<NoneOr<String>> {
        Ok(match bazel::console::compile(pattern, pattern) {
            Ok(_) => NoneOr::None,
            Err(e) => NoneOr::Other(e.to_string()),
        })
    }

    /// The named groups of `pattern`, a valid regex, in order: the captures
    /// an `aggregate` stage can group by or aggregate. Fails on a pattern
    /// that does not compile.
    fn pattern_groups(#[starlark(require = pos)] pattern: &str) -> anyhow::Result<Vec<String>> {
        let re = bazel::console::compile(pattern, pattern)?;
        Ok(re.capture_names().flatten().map(str::to_string).collect())
    }

    /// Whether `pattern` matches `line` as a stage would match it: against
    /// the line's text with escape sequences stripped. Fails on a pattern that
    /// does not compile. For checking a pattern against a line it is meant for.
    fn pattern_matches(
        #[starlark(require = pos)] pattern: &str,
        #[starlark(require = pos)] line: &str,
    ) -> anyhow::Result<bool> {
        let re = bazel::console::compile(pattern, pattern)?;
        Ok(re.is_match(&bazel::console::ansi::text(line.as_bytes())))
    }

    /// Record each line matching a `{id: regex}` pattern as an `(id, line)`
    /// hit in `build.output_matches()`, during or after the build (at most
    /// 1024 hits, though each id's first is always kept; lines cut to 2048
    /// bytes). Like `aggregate` and `fatal`, it never sees a row of Bazel's
    /// progress UI, in any mode: curses redraws those (one would match on
    /// every redraw), and they say nothing a message does not. An action's
    /// output shaped like progress is matched.
    fn observe(
        #[starlark(require = pos)] patterns: UnpackDictEntries<String, String>,
    ) -> anyhow::Result<OutputStage> {
        stage(Stage::observe(patterns.entries))
    }

    /// Aggregate the lines `pattern` matches, for `build.output_aggregates()`:
    /// exact over every line, with memory bounded whatever the console's
    /// size. Matches split into groups by the values of the `group_by`
    /// named captures (one group without), each counted, with
    /// `aggregations`, `{name: [kind, capture, max]}`, over a named capture
    /// or `"line"`, the whole line:
    ///
    /// * `["count"]`: the group's matches.
    /// * `["unique", capture, max]`: distinct values, first seen first, up
    ///   to `max` (default 1000).
    /// * `["all", capture, max]`: every value, repeats included, up to `max`.
    /// * `["count_distinct", capture]`: how many distinct values.
    /// * `["count_by", capture, max]`: each value and its matches, most
    ///   first: the first `max` distinct values seen, each counted exactly.
    /// * `["first", capture]`, `["last", capture]`: one value.
    ///
    /// A list that stopped at its `max` is named in the group's
    /// `truncated`. At most `max_groups` groups are kept (the matches of
    /// the rest are counted in `dropped_matches`), and the first `samples`
    /// matches of the kept groups as they were, with their captures (at
    /// most 10000). Values are cut to 1024 bytes. A capture the pattern
    /// does not name fails, as does a named group `line`, which would hide
    /// the whole line. Bazel's progress UI is never matched (see
    /// `observe`).
    fn aggregate<'v>(
        #[starlark(require = pos)] id: &str,
        #[starlark(require = pos)] pattern: &str,
        #[starlark(require = named, default = UnpackList::default())] group_by: UnpackList<String>,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        aggregations: UnpackDictEntries<String, UnpackList<Value<'v>>>,
        #[starlark(require = named, default = 20)] samples: i32,
        #[starlark(require = named, default = 100)] max_groups: i32,
    ) -> anyhow::Result<OutputStage> {
        let mut aggs = vec![];
        for (name, spec) in aggregations.entries {
            aggs.push((name.clone(), aggregation(&name, &spec.items)?));
        }
        if samples < 0 || max_groups < 1 {
            anyhow::bail!(
                "aggregate {id:?}: samples must be >= 0 and max_groups >= 1, got {samples} and {max_groups}"
            );
        }
        let spec = bazel::console::aggregate::Spec::new(
            id.to_string(),
            pattern,
            group_by.items,
            aggs,
            samples as usize,
            max_groups as usize,
        )?;
        Ok(OutputStage {
            stage: Stage::Aggregate(Box::new(spec)),
        })
    }

    /// Latch `build.output_fatal` on the first line matching one of
    /// `patterns`, keeping it in `build.output_fatal_line`: a server that
    /// died, say. `build.output_idle_ms` complements it for a silent stall.
    fn fatal(
        #[starlark(require = pos)] patterns: UnpackList<String>,
    ) -> anyhow::Result<OutputStage> {
        stage(Stage::fatal(patterns.items))
    }

    /// Hold each line matching a `{id: regex}` pattern, not yet shown, for
    /// the `BazelTrait.output_intercept` hooks, `fn(ctx, line)`, to answer
    /// with `line.keep()`, `line.replace(text)` or `line.drop()`. Each line
    /// goes to the hooks in order until one answers; a line none answers is
    /// kept. A build with this stage and no hooks fails to start.
    ///
    /// Everything after a held line waits behind it. The runtime runs the
    /// hooks inside the build's `wait()`, and whenever the task calls
    /// `build.run_output_hooks()` (the `build`/`test` handles do, for every
    /// build event), so hooks must be quick. A line no hook has answered
    /// after `timeout_ms` is shown unchanged and its hook's later answer
    /// ignored: the stage fails open, so a slow hook never stalls Bazel for
    /// long, and a pattern meant to redact belongs in `rewrite`, which never
    /// holds a line. Where curses is shown, the rows of Bazel's live progress
    /// area are never held: curses redraws them, and one dropped or made
    /// taller would make it erase real output. In plain lines the stages
    /// never see that area.
    fn intercept(
        #[starlark(require = pos)] patterns: UnpackDictEntries<String, String>,
        #[starlark(require = named, default = 1000)] timeout_ms: i32,
    ) -> anyhow::Result<OutputStage> {
        if timeout_ms <= 0 {
            anyhow::bail!("timeout_ms must be > 0, got {timeout_ms}");
        }
        stage(Stage::intercept(
            patterns.entries,
            Duration::from_millis(timeout_ms as u64),
        ))
    }

    /// Replace every match of `pattern` in a line with `replacement`, in
    /// which `$1` and `${name}` expand to capture groups. Unlike the other
    /// patterns, `pattern` sees the line's escape sequences too. It sees the
    /// line as the stages before it left it: after `intercept`, and after any
    /// earlier `rewrite`/`relativize_paths`. Where curses is shown, a row of
    /// Bazel's live progress area stays one row: newlines in the result become
    /// spaces, and one longer than Bazel wrote is cut to that width. In plain
    /// lines the stages never see that area.
    fn rewrite(
        #[starlark(require = pos)] pattern: &str,
        #[starlark(require = pos)] replacement: &str,
    ) -> anyhow::Result<OutputStage> {
        stage(Stage::rewrite(pattern, replacement.to_string()))
    }

    /// Shorten paths: a sandbox path becomes the workspace path it mirrors
    /// (`bazel-out/...`, `src/...`), as does an execroot path to a source
    /// file, and so does a path under `root`, the workspace directory; each
    /// is written relative to `cwd` (default `root`), or absolute when `cwd`
    /// is outside the workspace. An execroot path into `bazel-out/` or
    /// `external/` stays absolute, since that is where it exists.
    fn relativize_paths(
        #[starlark(require = named, default = NoneOr::None)] root: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] cwd: NoneOr<String>,
    ) -> anyhow::Result<OutputStage> {
        Ok(OutputStage {
            stage: Stage::RelativizePaths {
                root: root.into_option(),
                cwd: cwd.into_option(),
            },
        })
    }

    /// Hide each of Bazel's messages whose first line matches one of
    /// `patterns`, with the indented lines right under it. Only a message's
    /// first line is matched, never a line of an action's or a test's
    /// output, nor a header that opens one (`INFO: From …`, an action's
    /// `ERROR:`). Where curses is shown, its progress area and blank lines
    /// are never hidden; in plain lines the stages never see that area.
    fn drop(
        #[starlark(require = pos)] patterns: UnpackList<String>,
    ) -> anyhow::Result<OutputStage> {
        stage(Stage::drop(patterns.items))
    }

    /// Repeat the first `max` distinct `ERROR:` lines once the console ends,
    /// when at least `after` lines followed the first error, enough to have
    /// scrolled it away (blank and progress lines do not count; 0 always
    /// recaps). Bazel's closing `Build did NOT complete successfully` and
    /// `Build failed. Not running tests` are never repeated.
    fn recap_errors(
        #[starlark(require = named, default = bazel::console::report::RECAP_MAX as i32)] max: i32,
        #[starlark(require = named, default = bazel::console::report::RECAP_AFTER_RECORDS as i32)]
        after: i32,
    ) -> anyhow::Result<OutputStage> {
        if max <= 0 {
            anyhow::bail!("max must be > 0, got {max}");
        }
        if after < 0 {
            anyhow::bail!("after must be >= 0, got {after}");
        }
        Ok(OutputStage {
            stage: Stage::RecapErrors {
                max: max as usize,
                after: after as usize,
            },
        })
    }

    /// Fold each run of identical lines into the first plus a `(last line
    /// repeated N more times)` line. Unlike the other hiding stages it folds
    /// any line, an action's or a test's output included; it is not one of
    /// the `Console` defaults. Plain lines only: where curses is shown the
    /// annotation would land inside Bazel's redraw of its progress area, so
    /// the stage is left out.
    fn collapse_repeats() -> anyhow::Result<OutputStage> {
        Ok(OutputStage {
            stage: Stage::CollapseRepeats,
        })
    }

    /// Show the first of Bazel's messages of each kind and hide the rest,
    /// with the indented lines right under them; the summary at the end of
    /// the console counts what each kind hid. Like `drop`, it only ever
    /// matches a message's first line, never action or test output nor a
    /// header that opens one (`INFO: From …`).
    ///
    /// # Arguments
    /// * `severities` - Messages of these severities (`WARNING: ...`) are
    ///   alike when they differ only in labels, paths, numbers, hashes and
    ///   quoted text.
    ///   The default, `WARNING` alone, leaves a Starlark `print()` (`DEBUG`)
    ///   whole: its values differ, and so may the lines under it.
    /// * `groups` - `{id: regex}`: every message matching a group's pattern
    ///   is of that kind, whatever its severity. Checked first.
    fn collapse_similar(
        #[starlark(require = named, default = UnpackList { items: vec!["WARNING".to_string()] })]
        severities: UnpackList<String>,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        groups: UnpackDictEntries<String, String>,
    ) -> anyhow::Result<OutputStage> {
        stage(Stage::collapse_similar(severities.items, groups.entries))
    }
}

#[cfg(all(test, unix))]
mod tests {
    //! `output =` end to end through the basil fake bazel, which writes a
    //! scenario's console lines to its stderr.

    use std::time::Duration;

    /// Run `body` as a task's implementation, failing instead of hanging.
    /// `HISTORY` in it names a fresh directory, the task's history root, where
    /// raw logs go.
    fn run(body: &str) -> anyhow::Result<Option<u8>> {
        let logs = tempfile::tempdir().expect("temp dir");
        let root = logs.path().to_path_buf();
        let body = body.replace("HISTORY", &root.display().to_string());
        let script = format!("def _impl(ctx):\n{body}\nTest = task(implementation = _impl)\n");
        crate::test::with_timeout(Duration::from_secs(60), move || {
            crate::test::eval(&script)
                .with_fake_bazel()
                .with_history(root)
                .run_task(0)
        })
        .expect("the task hung")
    }

    /// A trait with the builtin one's console fields: the runtime finds it
    /// by them.
    const TRAIT: &str = r#"
BazelTrait = trait(
    output_stages = attr(list, default = []),
    output_tty = attr(bool | None, default = None),
    output_intercept = attr(list, default = []),
    output_end = attr(list, default = []),
    output_start = attr(list, default = []),
)
"#;

    /// Run `body` as the implementation of a task with that trait, after
    /// `prelude` at the top level; `t` in `body` is the task's instance.
    fn run_with_trait(prelude: &str, body: &str) -> anyhow::Result<Option<u8>> {
        let logs = tempfile::tempdir().expect("temp dir");
        let root = logs.path().to_path_buf();
        let body = body.replace("HISTORY", &root.display().to_string());
        let script = format!(
            "{TRAIT}\n{prelude}\ndef _impl(ctx):\n    t = ctx.traits[BazelTrait]\n{body}\nTest = task(implementation = _impl, traits = [BazelTrait])\n"
        );
        crate::test::with_timeout(Duration::from_secs(60), move || {
            crate::test::eval(&script)
                .with_fake_bazel()
                .with_history(root)
                .with_config("def config(ctx):\n    pass\n")
                .run_task(0)
        })
        .expect("the task hung")
    }

    #[test]
    fn a_processor_states_its_mode_and_the_flags_it_needs() {
        let exit = run(r#"
    shown = bazel.output.processor(tty = True)
    lines = bazel.output.processor(tty = False)
    if not shown.tty or "--curses=yes" not in shown.flags: fail("shown %r %r" % (shown.tty, shown.flags))
    if lines.tty or lines.flags != ["--isatty=0", "--curses=yes", "--color=yes"]: fail("flags %r" % (lines.flags,))
    if "--color=yes" in bazel.output.processor(colour = False).flags: return 5
    if lines.intercepts: return 3
    if not bazel.output.processor(stages = [bazel.output.intercept({"s": "x"})]).intercepts: return 4
    return 0"#)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// Listed in any order, stages run by kind.
    #[test]
    fn a_processor_runs_its_stages_in_phase_order() {
        let exit = run(r#"
    o = bazel.output
    processor = o.processor(tty = False, stages = [
        o.collapse_similar(), o.drop(["d"]), o.relativize_paths(),
        o.intercept({"r": "r"}), o.fatal(["f"]), o.rewrite("a", "b"), o.observe({"m": "m"}),
        o.recap_errors(),
    ])
    if processor.stages != [
        "fatal", "observe", "intercept", "relativize_paths", "rewrite", "drop",
        "recap_errors", "collapse_similar",
    ]: return 1
    if o.drop(["x"]).name != "drop": return 2
    return 0"#)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    #[test]
    fn a_build_without_console_output_reports_nothing() {
        let exit = run(r#"
    processor = bazel.output.processor(tty = False, stages = [
        bazel.output.collapse_repeats(),
        bazel.output.observe({"warn": "WARNING:"}),
        bazel.output.fatal(["Server terminated abruptly"]),
    ])
    build = ctx.bazel.build(flags = ["--scenario=success"] + processor.flags, output = processor)
    if build.output_idle_ms < 0: return 1
    status = build.wait()
    if not status.success: return 2
    if build.output_fatal: return 3
    if build.output_fatal_line != None: return 4
    if build.output_matches() != []: return 5
    if build.output_idle_ms != 0: return 6
    return 0"#)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    #[test]
    fn matches_and_fatal_lines_are_recorded_from_the_console() {
        let exit = run(r#"
    processor = bazel.output.processor(tty = False, stages = [
        bazel.output.observe({"warn": "^WARNING:"}),
        bazel.output.fatal(["completed", "Build"]),
    ])
    build = ctx.bazel.build(flags = ["--scenario=stderr_chatter"], output = processor)
    if not build.wait().success: return 1
    matches = build.output_matches()
    if len(matches) != 2: return 2
    if matches[0] != ("warn", "WARNING: deprecated flag used"): return 3
    if not build.output_fatal: return 4
    if build.output_fatal_line != "INFO: Build completed successfully": return 5
    if build.output_log != None: return 6
    return 0"#)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// A processor that changes what is shown keeps the console as Bazel
    /// wrote it, as `console.log` in the Bazel invocation's history folder:
    /// one per build.
    #[test]
    fn a_changing_processor_keeps_a_raw_log_in_the_history() {
        let exit = run(r#"
    processor = bazel.output.processor(tty = False, stages = [
        bazel.output.collapse_similar(),
        bazel.output.rewrite("hunter2", "***"),
    ])
    logs = []
    for _ in range(2):
        build = ctx.bazel.build(flags = ["--scenario=stderr_chatter"], output = processor)
        if not build.wait().success: return 1
        logs.append(build.output_log)
    for n, log in enumerate(logs):
        if log == None or not log.startswith("HISTORY/"): return 2
        if not log.endswith("/bazel/%d/console.log" % (n + 1)): return 3
        if "password=hunter2 leaked" not in ctx.std.fs.read_to_string(log): return 4
    return 0"#)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// The runtime knows the console's trait by its fields: one that has
    /// `output_stages` but not the rest is someone else's, and its stages
    /// process nothing.
    #[test]
    fn a_trait_without_every_console_field_is_not_the_consoles() {
        let script = r#"
Lookalike = trait(output_stages = attr(list, default = []))

def _impl(ctx):
    ctx.traits[Lookalike].output_stages.append(bazel.output.rewrite("hunter2", "***"))
    build = ctx.bazel.build(flags = ["--scenario=stderr_chatter"])
    if not build.wait().success: return 1
    if build.output_log != None: return 2
    return 0
Test = task(implementation = _impl, traits = [Lookalike])
"#;
        let exit = crate::test::with_timeout(Duration::from_secs(60), move || {
            crate::test::eval(script)
                .with_fake_bazel()
                .with_config("def config(ctx):\n    pass\n")
                .run_task(0)
        })
        .expect("the task hung")
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// With no history folder to be had (its root is a file), the raw log
    /// goes to a temp folder of its own, never straight into the shared
    /// temp dir, and the build goes on.
    #[test]
    fn a_raw_log_without_a_history_goes_to_a_private_temp_folder() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("not-a-dir");
        std::fs::write(&root, "").unwrap();
        let script = r#"
def _impl(ctx):
    processor = bazel.output.processor(tty = False, stages = [bazel.output.rewrite("hunter2", "***")])
    build = ctx.bazel.build(flags = ["--scenario=stderr_chatter"], output = processor)
    if not build.wait().success: return 1
    log = build.output_log
    if log == None or not log.endswith("/console.log"): return 2
    if "/aspect-console-" not in log: return 3
    if "password=hunter2 leaked" not in ctx.std.fs.read_to_string(log): return 4
    return 0
Test = task(implementation = _impl)
"#;
        let exit = crate::test::with_timeout(Duration::from_secs(60), move || {
            crate::test::eval(script)
                .with_fake_bazel()
                .with_history(root)
                .run_task(0)
        })
        .expect("the task hung")
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// Held lines go to the hooks in order until one answers; one no hook
    /// answers is kept. `wait()` answering them at all proves it stops with
    /// the console, and the end hooks then see the whole console.
    #[test]
    fn held_lines_are_answered_by_the_hooks_and_the_end_hooks_follow() {
        let exit = run_with_trait(
            "",
            r#"
    seen = []
    def _note(ctx, line):
        seen.append(line.id)
    def _answer(ctx, line):
        if line.answered: fail("answered before its turn")
        if line.id == "secret":
            line.replace("(password elided)")
    def _end(ctx, build):
        seen.append("end:%d" % len(build.output_matches()))
    t.output_intercept.extend([_note, _answer])
    t.output_end.append(_end)
    processor = bazel.output.processor(tty = False, stages = [
        bazel.output.intercept({"secret": "password", "info": "^INFO:"}),
        bazel.output.observe({"warn": "^WARNING:"}),
    ])
    build = ctx.bazel.build(flags = ["--scenario=stderr_chatter"], output = processor)
    if not build.wait().success: return 2
    if seen != ["info", "secret", "info", "end:2"]: return 3
    return 0"#,
        )
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// With `until_ended`, every line is answered before the call returns,
    /// so the build's `wait()` answers none.
    #[test]
    fn run_output_hooks_until_ended_answers_the_whole_console() {
        let exit = run_with_trait(
            "",
            r#"
    seen = []
    def _keep(ctx, line):
        seen.append(line.id)
        line.keep()
    t.output_intercept.append(_keep)
    processor = bazel.output.processor(tty = False, stages = [
        bazel.output.intercept({"secret": "password"}),
    ])
    build = ctx.bazel.build(flags = ["--scenario=stderr_chatter"], output = processor)
    build.run_output_hooks()
    build.run_output_hooks(until_ended = True)
    if seen != ["secret"]: return 1
    if not build.wait().success: return 2
    if seen != ["secret"]: return 3
    return 0"#,
        )
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// A build given no `output =` is processed with the trait's stages.
    #[test]
    fn a_build_without_output_takes_the_traits_stages() {
        let exit = run_with_trait(
            "",
            r#"
    t.output_stages.append(bazel.output.observe({"warn": "^WARNING:"}))
    t.output_tty = False
    build = ctx.bazel.build(flags = ["--scenario=stderr_chatter"])
    if not build.wait().success: return 1
    if build.output_matches()[:1] != [("warn", "WARNING: deprecated flag used")]: return 2
    plain = ctx.bazel.build(flags = ["--scenario=stderr_chatter"], stderr = None)
    if not plain.wait().success: return 3
    if plain.output_matches() != []: return 4
    return 0"#,
        )
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// `output_start` hooks add stages to each build as it starts; a build
    /// whose stdout the task redirects gets no processing at all: no hooks,
    /// no stages, and none of the capture's flags.
    #[test]
    fn start_hooks_add_stages_and_a_redirected_stdout_turns_processing_off() {
        let argv_log = tempfile::NamedTempFile::new().unwrap();
        // SAFETY: process-wide, as in build.rs's argv tests; a concurrent
        // test's basil lines are told apart by the marker below.
        unsafe { std::env::set_var("BASIL_ARGV_LOG", argv_log.path()) };
        let exit = run_with_trait(
            "",
            r#"
    calls = []
    def _start(ctx):
        calls.append(1)
        return [bazel.output.observe({"warn": "^WARNING:"})]
    t.output_start.append(_start)
    build = ctx.bazel.build(flags = ["--scenario=stderr_chatter"])
    if not build.wait().success: return 1
    if build.output_matches()[:1] != [("warn", "WARNING: deprecated flag used")]: return 2
    out = ctx.std.fs.create("HISTORY/out.txt")
    t.output_tty = True
    redirected = ctx.bazel.build(flags = ["--scenario=stderr_chatter", "--define=probe=redirected-stdout"], stdout = out)
    if not redirected.wait().success: return 3
    if len(calls) != 1: return 4
    if redirected.output_matches(): return 5
    both = ctx.bazel.build(flags = ["--scenario=stderr_chatter"], stdio = ctx.std.io)
    if not both.wait().success: return 6
    if len(calls) != 1 or both.output_matches(): return 7
    return 0"#,
        )
        .expect("run_task");
        assert_eq!(exit, Some(0));
        let argv = std::fs::read_to_string(argv_log.path()).unwrap();
        let line = argv
            .lines()
            .find(|l| l.contains("probe=redirected-stdout"))
            .unwrap_or_else(|| panic!("no redirected build in:\n{argv}"));
        for flag in ["--color=yes", "--curses=yes", "--isatty"] {
            assert!(!line.contains(flag), "{flag} in {line}");
        }
    }

    /// The end hooks run once per build: a second `wait()`, or one from an
    /// end hook, does not run them again.
    #[test]
    fn end_hooks_run_once_however_often_wait_is_called() {
        let exit = run_with_trait(
            "",
            r#"
    ends = []
    def _end(ctx, build):
        ends.append(1)
        build.wait()
    t.output_end.append(_end)
    processor = bazel.output.processor(tty = False, stages = [bazel.output.observe({"x": "x"})])
    build = ctx.bazel.build(flags = ["--scenario=stderr_chatter"], output = processor)
    build.wait()
    build.wait()
    if ends != [1]: return 1
    return 0"#,
        )
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// An `aggregate` stage counts and groups what its pattern matched, for
    /// `build.output_aggregates()`; a capture its pattern lacks, or an
    /// unknown kind, fails where the stage is made.
    #[test]
    fn aggregate_stages_report_what_they_matched() {
        let exit = run_with_trait(
            "",
            r#"
    stage = bazel.output.aggregate(
        "w",
        r"^(?P<sev>WARNING): (?P<what>\w+)",
        group_by = ["sev"],
        aggregations = {"whats": ["unique", "what"], "n": ["count"], "first": ["first", "line"]},
        samples = 1,
    )
    processor = bazel.output.processor(tty = False, stages = [stage])
    build = ctx.bazel.build(flags = ["--scenario=stderr_chatter"], output = processor)
    if not build.wait().success: return 1
    got = build.output_aggregates()
    if len(got) != 1 or got[0]["id"] != "w" or got[0]["count"] < 1: return 2
    g = got[0]["groups"][0]
    if g["key"] != {"sev": "WARNING"} or g["values"]["n"] != g["count"]: return 3
    if g["values"]["whats"][0] != "deprecated" or not g["values"]["first"].startswith("WARNING: deprecated"): return 4
    if len(got[0]["samples"]) != 1 or got[0]["samples"][0]["captures"]["what"] != "deprecated": return 5
    if bazel.output.pattern_groups(r"(?P<a>x)(y)(?P<b>z)") != ["a", "b"]: return 6
    return 0"#,
        )
        .expect("run_task");
        assert_eq!(exit, Some(0));
        for (bad, want) in [
            (
                r#"bazel.output.aggregate("w", "x", group_by = ["pkg"])"#,
                "no named group `pkg`",
            ),
            (
                r#"bazel.output.aggregate("w", "(?P<a>x)", aggregations = {"n": ["median", "a"]})"#,
                "unknown kind \"median\"",
            ),
        ] {
            let err = run_with_trait(
                "",
                &format!(
                    "    {bad}
    return 0"
                ),
            )
            .expect_err("an invalid aggregate stage fails");
            assert!(format!("{err:#}").contains(want), "{err:#}");
        }
    }

    /// A hook failing while `run_output_hooks(until_ended = True)` answers
    /// does not raise there, with the client still running: `wait()` raises
    /// it, once the build is joined.
    #[test]
    fn a_hook_error_waits_for_the_build() {
        let err = run_with_trait(
            "",
            r#"
    def _boom(ctx, line):
        fail("hook kaboom")
    t.output_intercept.append(_boom)
    processor = bazel.output.processor(tty = False, stages = [
        bazel.output.intercept({"m": "^WARNING:|^INFO:|password"}, timeout_ms = 120000),
    ])
    build = ctx.bazel.build(flags = ["--scenario=stderr_chatter"], output = processor)
    build.run_output_hooks(until_ended = True)
    print("after the hooks")
    build.wait()
    return 0"#,
        )
        .expect_err("the hook's failure must be raised")
        .to_string();
        assert!(err.contains("hook kaboom"), "{err}");
    }

    /// Lines held for hooks nobody registered would each wait out their
    /// timeout, so the build is refused.
    #[test]
    fn an_intercept_stage_needs_hooks() {
        for script in [
            (
                false,
                "bazel.output.processor(tty = False, stages = [bazel.output.intercept({\"s\": \"x\"})])",
            ),
            (
                true,
                "bazel.output.processor(tty = False, stages = [bazel.output.intercept({\"s\": \"x\"})])",
            ),
        ] {
            let body = format!(
                "    ctx.bazel.build(flags = [\"--scenario=success\"], output = {})\n    return 0",
                script.1
            );
            let err = if script.0 {
                run_with_trait("", &body)
            } else {
                run(&body)
            }
            .expect_err("the build must be refused")
            .to_string();
            assert!(err.contains("`BazelTrait.output_intercept` hooks"), "{err}");
        }
    }

    /// A hook's error ends `wait()` with it, once the build is joined; the
    /// lines after it are shown at once rather than each waiting its timeout.
    #[test]
    fn a_failing_hook_fails_the_wait() {
        let err = run_with_trait(
            r#"
def _boom(ctx, line):
    fail("hook kaboom")
"#,
            r#"
    t.output_intercept.append(_boom)
    processor = bazel.output.processor(tty = False, stages = [
        bazel.output.intercept({"m": "^WARNING:|^INFO:|password"}, timeout_ms = 120000),
    ])
    build = ctx.bazel.build(flags = ["--scenario=stderr_chatter"], output = processor)
    build.wait()
    return 0"#,
        )
        .expect_err("the hook's failure must be raised")
        .to_string();
        assert!(err.contains("hook kaboom"), "{err}");
    }

    #[test]
    fn a_line_answered_twice_is_refused() {
        let err = run_with_trait(
            r#"
def _twice(ctx, line):
    line.keep()
    line.drop()
"#,
            r#"
    t.output_intercept.append(_twice)
    processor = bazel.output.processor(tty = False, stages = [bazel.output.intercept({"secret": "password"})])
    ctx.bazel.build(flags = ["--scenario=stderr_chatter"], output = processor).wait()
    return 0"#,
        )
        .expect_err("a second answer must be refused")
        .to_string();
        assert!(err.contains("already answered"), "{err}");
    }

    #[test]
    fn output_cannot_be_combined_with_stderr_or_stdio() {
        for arg in ["stderr = None", "stdio = ctx.std.io"] {
            let err = run(&format!(
                r#"
    ctx.bazel.build(output = bazel.output.processor(tty = False), {arg})
    return 0"#
            ))
            .expect_err("the combination must be refused")
            .to_string();
            assert!(
                err.contains("cannot be combined with stderr or stdio"),
                "{err}"
            );
        }
    }

    #[test]
    fn pattern_error_explains_a_bad_pattern_without_failing() {
        let exit = run(r#"
    if bazel.output.pattern_error("^ok$") != None: return 1
    err = bazel.output.pattern_error("(")
    if err == None or not err.startswith("invalid regex for '('"): return 2
    return 0"#)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    #[test]
    fn an_invalid_pattern_or_bound_fails_the_stage() {
        for (call, expected) in [
            ("observe({\"bad\": \"(\"})", "invalid regex for 'bad'"),
            ("drop([\"(\"])", "invalid regex for '('"),
            (
                "collapse_similar(groups = {\"g\": \"[\"})",
                "invalid regex for 'g'",
            ),
            ("rewrite(\"(\", \"x\")", "invalid regex for '('"),
            (
                "intercept({\"r\": \"r\"}, timeout_ms = 0)",
                "timeout_ms must be > 0",
            ),
            ("recap_errors(max = 0)", "max must be > 0"),
        ] {
            let err = run(&format!("    bazel.output.{call}\n    return 0"))
                .expect_err("the stage must be refused")
                .to_string();
            assert!(err.contains(expected), "{call}: {err}");
        }
    }
}
