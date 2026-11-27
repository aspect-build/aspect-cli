use allocative::Allocative;
use derive_more::Display;

use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::starlark_value;

use starlark::{
    environment::GlobalsBuilder, starlark_module,
    values::starlark_value_as_type::StarlarkValueAsType,
};

use std::sync::Arc;

use crate::color::StreamColor;
use crate::engine::cancellation::Signals;
use crate::engine::error::{IoError, NativeError};
use starlark::StarlarkResultExt;
use starlark::values::ValueLike;

mod env;
pub(crate) mod fs;
mod history;
pub mod io;
mod net;
pub(crate) mod process;
pub mod re;
pub mod stream;
pub mod syntax;
pub mod term;
pub mod theme;
pub(crate) mod watch;

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.Std>")]
pub struct Std {
    /// The run's cancellation state, for the members that block.
    #[allocative(skip)]
    signals: Arc<Signals>,
    /// What `std.io.stdout.color` and `std.io.stderr.color` report.
    #[allocative(skip)]
    color: StreamColor,
}

impl Std {
    pub fn new(signals: Arc<Signals>, color: StreamColor) -> Self {
        Self { signals, color }
    }
}

starlark_simple_value!(Std);

#[starlark_value(type = "std.Std")]
impl<'v> values::StarlarkValue<'v> for Std {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("std_methods", std_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
pub(crate) fn std_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn env<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<env::Env> {
        Ok(env::Env::new())
    }

    #[starlark(attribute)]
    fn io<'v>(this: values::Value<'v>) -> anyhow::Result<io::Stdio> {
        let this = this.downcast_ref_err::<Std>().into_anyhow_result()?;
        Ok(io::Stdio::new(this.signals.clone(), this.color))
    }

    #[starlark(attribute)]
    fn fs<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<fs::Filesystem> {
        Ok(fs::Filesystem::new())
    }

    #[starlark(attribute)]
    fn process<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<process::Process> {
        Ok(process::Process::new())
    }

    #[starlark(attribute)]
    fn re<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<re::Re> {
        Ok(re::Re)
    }

    /// Syntax highlighting for a source file shown on a screen; see
    /// `std.Syntax`.
    #[starlark(attribute)]
    fn syntax<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<syntax::Syntax> {
        Ok(syntax::Syntax)
    }

    /// The terminal, for a feature that draws the whole screen: `take()`
    /// hands it out as a `std.term.Screen` (the alternate screen, keys read
    /// one at a time, the mouse when the screen takes it, the CLI's output
    /// kept off it), `size()` and `taken` read it without taking it, and
    /// `dump_output()` and `keep_output()` write the scrollback a screen
    /// leaves. What needs no terminal is in the `std.term` namespace, which
    /// a module's top level reads too: `dump_lines`, the dump's size, and
    /// `width` and `fit`, which measure text in cells.
    #[starlark(attribute)]
    fn term<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<term::Term> {
        Ok(term::Term)
    }

    #[starlark(attribute)]
    fn net<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<net::Net> {
        Ok(net::Net::new())
    }

    /// The run's colour theme, a role at a time; see `std.Theme`. The same
    /// value as `load("@std//theme.axl", "theme")`.
    #[starlark(attribute)]
    fn theme<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<theme::Theme> {
        Ok(theme::Theme)
    }

    /// The task history's records of past runs; see `std.History`.
    #[starlark(attribute)]
    fn history<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<history::History> {
        Ok(history::History::new())
    }

    /// The lines of a failure's output that say what went wrong, as the
    /// console's recap of an error picks them: the first line that reads as
    /// an error, with `before` and `after` lines around it, and the last
    /// `tail` lines, at most `max_lines` in all; with no such line, the last
    /// `max_lines`. `text` is cleaned first (escapes removed, a line redrawn
    /// in place only as last drawn, a run of the same line once).
    ///
    /// Returns `{"lines": [...], "cut_head": bool, "cut_tail": bool,
    /// "cut_between": bool}`: at most `max_lines` lines of the output, plus a
    /// `… N more lines` line where lines between two spans are left out; and
    /// whether lines before the first, after the last, or between are.
    fn excerpt<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] text: &str,
        #[starlark(require = named, default = 20)] max_lines: u32,
        #[starlark(require = named, default = 4)] before: u32,
        #[starlark(require = named, default = 10)] after: u32,
        #[starlark(require = named, default = 5)] tail: u32,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        use bazel::console::excerpt;
        let lines = excerpt::clean_lines(text.as_bytes());
        let spans = excerpt::select(
            &lines,
            excerpt::Shape {
                max_lines: max_lines as usize,
                before: before as usize,
                after: after as usize,
                tail: tail as usize,
            },
        );
        let mut out: Vec<String> = Vec::new();
        let mut at = None;
        for span in &spans {
            if let Some(end) = at
                && span.start > end
            {
                let n = span.start - end;
                let s = if n == 1 { "" } else { "s" };
                out.push(format!("… {n} more line{s}"));
            }
            out.extend(lines[span.clone()].iter().cloned());
            at = Some(span.end);
        }
        let cut_head = spans.first().is_some_and(|s| s.start > 0);
        let cut_tail = spans.last().is_some_and(|s| s.end < lines.len());
        let cut_between = spans.len() > 1;
        Ok(heap.alloc(starlark::values::dict::AllocDict([
            ("lines", heap.alloc(starlark::values::list::AllocList(out))),
            ("cut_head", heap.alloc(cut_head)),
            ("cut_tail", heap.alloc(cut_tail)),
            ("cut_between", heap.alloc(cut_between)),
        ])))
    }
}

#[starlark_module]
fn register_types(globals: &mut GlobalsBuilder) {
    const Env: StarlarkValueAsType<env::Env> = StarlarkValueAsType::new();
    const FileSystem: StarlarkValueAsType<fs::Filesystem> = StarlarkValueAsType::new();
    const Net: StarlarkValueAsType<net::Net> = StarlarkValueAsType::new();
    const History: StarlarkValueAsType<history::History> = StarlarkValueAsType::new();
    const Theme: StarlarkValueAsType<theme::Theme> = StarlarkValueAsType::new();
    const Std: StarlarkValueAsType<Std> = StarlarkValueAsType::new();
}

#[starlark_module]
fn register_process_types(globals: &mut GlobalsBuilder) {
    const Child: StarlarkValueAsType<process::Child> = StarlarkValueAsType::new();
    const Command: StarlarkValueAsType<process::Command> = StarlarkValueAsType::new();
    const ExitStatus: StarlarkValueAsType<process::ExitStatus> = StarlarkValueAsType::new();
    const Output: StarlarkValueAsType<process::Output> = StarlarkValueAsType::new();
    const Process: StarlarkValueAsType<process::Process> = StarlarkValueAsType::new();
}

#[starlark_module]
fn register_io_types(globals: &mut GlobalsBuilder) {
    const Stdio: StarlarkValueAsType<io::Stdio> = StarlarkValueAsType::new();
    const Readable: StarlarkValueAsType<stream::Readable> = StarlarkValueAsType::new();
    const Writable: StarlarkValueAsType<stream::Writable> = StarlarkValueAsType::new();
}

#[starlark_module]
fn register_term_types(globals: &mut GlobalsBuilder) {
    const Screen: StarlarkValueAsType<term::Screen> = StarlarkValueAsType::new();
}

#[starlark_module]
fn register_re_types(globals: &mut GlobalsBuilder) {
    const Pattern: StarlarkValueAsType<re::Pattern> = StarlarkValueAsType::new();
    const Re: StarlarkValueAsType<re::Re> = StarlarkValueAsType::new();
}

#[starlark_module]
fn register_watch_types(globals: &mut GlobalsBuilder) {
    const CreatedEvent: StarlarkValueAsType<watch::WatchCreated> = StarlarkValueAsType::new();
    const ModifiedEvent: StarlarkValueAsType<watch::WatchModified> = StarlarkValueAsType::new();
    const RemovedEvent: StarlarkValueAsType<watch::WatchRemoved> = StarlarkValueAsType::new();
}

pub fn register_globals(globals: &mut GlobalsBuilder) {
    register_types(globals);

    globals.namespace("process", register_process_types);
    globals.namespace("net", net::register_net_types);
    globals.namespace("re", register_re_types);
    globals.namespace("term", |g| {
        register_term_types(g);
        term::register_globals(g);
    });
    globals.namespace("io", |g| {
        register_io_types(g);
        g.set("Error", IoError::native_type().error_type());
    });
    // `std.fs` is a module (the `fs.watch` event types live here); the
    // filesystem value type stays `std.FileSystem`.
    globals.namespace("fs", |g| g.namespace("watch", register_watch_types));
}
