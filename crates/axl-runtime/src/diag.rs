//! Severity-prefixed user-facing diagnostics for the Rust side of the CLI.
//!
//! The Rust counterpart of the `debug` / `info` / `warn` / `error` std helpers
//! in `lib/environment.axl`: the same `DEBUG:` / `INFO:` / `WARNING:` /
//! `ERROR:` prefix style (mirroring Bazel's uppercase severity labels), the
//! same ANSI colors, and the same "degrade to plain text when color is off"
//! behavior - so a line printed from Rust is indistinguishable from one printed
//! by an AXL task.
//!
//! Use this for user-facing subsystem messages (a backend became unreachable,
//! a fallback fired). It is **not** the structured channel - that is
//! `tracing`; the runtime's `trace!` macro emits a log record there and the
//! [`debug`] line here.
//!
//! Both write to stderr and color by the same answer,
//! [`crate::color::console_supports_color`]: the process's memoized
//! [`crate::color::StreamColor`], the one value a run's `Env` carries as
//! `stream_color`, which the console ([`crate::console::Host`]) and AXL's
//! `std.io.stderr.color` read.
//!
//! Levels, quietest first: debug, detail, info, warning, error. The AXL
//! `detail` helper follows [`verbose`], and [`debug`] (which the runtime's
//! `trace!` macro calls) and the AXL `debug` helper print their `DEBUG:` line
//! only when [`debugging`]. The `Console` feature sets the level for the
//! running task when the console resolves; before that, early in start-up, it
//! is [`Verbose::On`] unless the CLI set it from `ASPECT_VERBOSE` /
//! `ASPECT_DEBUG`. Everything else prints under every preset: [`info`],
//! `WARNING:` and `ERROR:` lines, and [`emit`], which carries a task's own exit
//! message.

use std::sync::atomic::{AtomicU8, Ordering};

use crate::errln;

/// How much of the optional output prints: the `--console:verbose` setting
/// once `auto` is resolved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Verbose {
    /// Detail lines hidden (`false`).
    Off,
    /// Detail lines shown (`true`).
    #[default]
    On,
    /// Detail lines and debug lines shown (`debug`).
    Debug,
}

impl Verbose {
    /// Every value `--console:verbose` and `ASPECT_VERBOSE` take, `auto`
    /// (the preset decides) first.
    pub const NAMES: [&'static str; 4] = ["auto", "true", "false", "debug"];

    /// `name` as a level, `Ok(None)` for `auto`, or `Err` for a name that is
    /// none of [`Self::NAMES`].
    pub fn parse(name: &str) -> Result<Option<Verbose>, ()> {
        match name {
            "auto" => Ok(None),
            "true" => Ok(Some(Verbose::On)),
            "false" => Ok(Some(Verbose::Off)),
            "debug" => Ok(Some(Verbose::Debug)),
            _ => Err(()),
        }
    }

    /// The level the environment `var` sets: `debug` for a non-empty
    /// `ASPECT_DEBUG`, else `ASPECT_VERBOSE` (unset, empty or `auto` set
    /// none). Fails on any other `ASPECT_VERBOSE` value, naming the closest.
    pub fn from_env(
        var: impl Fn(&str) -> Option<std::ffi::OsString>,
    ) -> Result<Option<Verbose>, String> {
        if var("ASPECT_DEBUG").is_some_and(|v| !v.is_empty()) {
            return Ok(Some(Verbose::Debug));
        }
        let Some(value) = var("ASPECT_VERBOSE").filter(|v| !v.is_empty()) else {
            return Ok(None);
        };
        let value = value.to_string_lossy();
        Verbose::parse(&value).map_err(|()| {
            format!(
                "ASPECT_VERBOSE must be one of {}, got {value:?}{}",
                Verbose::NAMES.join(", "),
                crate::suggest::did_you_mean(&value, Verbose::NAMES)
            )
        })
    }

    fn index(self) -> u8 {
        match self {
            Verbose::Off => 0,
            Verbose::On => 1,
            Verbose::Debug => 2,
        }
    }

    fn from_index(index: u8) -> Verbose {
        match index {
            0 => Verbose::Off,
            2 => Verbose::Debug,
            _ => Verbose::On,
        }
    }

    /// Whether detail lines print at this level.
    pub fn shows_detail(self) -> bool {
        self != Verbose::Off
    }
}

/// The run's [`Verbose`] level. [`Verbose::On`] until set.
static VERBOSE: AtomicU8 = AtomicU8::new(1);

/// Set the level for the rest of the run, from Rust and AXL alike.
pub fn set_verbose(level: Verbose) {
    VERBOSE.store(level.index(), Ordering::Relaxed);
}

/// The run's level; see [`set_verbose`].
pub fn verbosity() -> Verbose {
    Verbose::from_index(VERBOSE.load(Ordering::Relaxed))
}

/// Whether detail lines print; see [`set_verbose`].
pub fn verbose() -> bool {
    verbosity().shows_detail()
}

/// Whether debug lines print: the level is [`Verbose::Debug`].
pub fn debugging() -> bool {
    verbosity() == Verbose::Debug
}

/// Severity of a diagnostic line, carrying its label and color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Severity {
    /// Bold grey `DEBUG:` - internals a bug report needs.
    Debug,
    /// Bold cyan `INFO:` - a notable but expected runtime branch fired.
    Info,
    /// Bold yellow `WARNING:` - a subsystem problem; the task continues.
    Warning,
    /// Bold red `ERROR:` - a recoverable error, usually preceding a non-zero exit.
    Error,
}

impl Severity {
    /// The uppercase label (without the trailing colon).
    fn label(self) -> &'static str {
        match self {
            Severity::Debug => "DEBUG",
            Severity::Info => "INFO",
            Severity::Warning => "WARNING",
            Severity::Error => "ERROR",
        }
    }

    /// The SGR color-open sequence for the label, matching the AXL helpers:
    /// every label bold, `DEBUG` grey, `INFO` cyan, `WARNING` yellow, `ERROR`
    /// red.
    fn color(self) -> &'static str {
        match self {
            Severity::Debug => "\x1b[1;90m",
            Severity::Info => "\x1b[1;36m",
            Severity::Warning => "\x1b[1;33m",
            Severity::Error => "\x1b[1;31m",
        }
    }
}

/// SGR reset.
const RESET: &str = "\x1b[0m";

/// Format a severity line: `<LABEL>: <msg>`. The colon and message sit outside
/// the color span, so the label degrades to a plain `LABEL:` when `colorize`
/// is false (a non-rendering log gets no stray escapes).
///
/// Pure (no I/O) so both the wording and the color gating are unit-testable;
/// [`emit`] wraps it for the stderr side effect.
fn format_line(sev: Severity, colorize: bool, msg: &str) -> String {
    if colorize {
        format!("{}{}{}: {msg}", sev.color(), sev.label(), RESET)
    } else {
        format!("{}: {msg}", sev.label())
    }
}

/// Whether to emit ANSI color: [`crate::color::console_supports_color`], the
/// single memoized answer every line the runtime prints to stderr uses, the
/// console's included.
fn colorize() -> bool {
    crate::color::console_supports_color()
}

/// Print one severity-prefixed line to stderr, colorized per [`colorize`].
/// The named helpers below cover the fixed severities; call this when the
/// severity is computed.
pub(crate) fn emit(sev: Severity, msg: &str) {
    errln!("{}", format_line(sev, colorize(), msg));
}

/// Print a bold grey `DEBUG:` line when [`debugging`]. The `trace!` macro calls
/// it beside the log record it emits.
pub fn debug(msg: &str) {
    if debugging() {
        emit(Severity::Debug, msg);
    }
}

/// Print a bold cyan `INFO:` line for a notable runtime branch.
pub fn info(msg: &str) {
    emit(Severity::Info, msg);
}

/// Print a bold yellow `WARNING:` line for a non-fatal subsystem failure.
pub fn warn(msg: &str) {
    emit(Severity::Warning, msg);
}

/// Print a bold red `ERROR:` line for a recoverable error (usually preceding a
/// non-zero exit).
pub fn error(msg: &str) {
    emit(Severity::Error, msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorized_matches_axl_severity_style() {
        // Colon and message outside the color span; colors match lib/environment.axl.
        assert_eq!(
            format_line(Severity::Info, true, "hello"),
            "\x1b[1;36mINFO\x1b[0m: hello"
        );
        assert_eq!(
            format_line(Severity::Warning, true, "hello"),
            "\x1b[1;33mWARNING\x1b[0m: hello"
        );
        assert_eq!(
            format_line(Severity::Error, true, "hello"),
            "\x1b[1;31mERROR\x1b[0m: hello"
        );
    }

    #[test]
    fn verbose_parses_every_name_and_suggests_the_closest() {
        assert_eq!(Verbose::parse("auto"), Ok(None));
        assert_eq!(Verbose::parse("true"), Ok(Some(Verbose::On)));
        assert_eq!(Verbose::parse("false"), Ok(Some(Verbose::Off)));
        assert_eq!(Verbose::parse("debug"), Ok(Some(Verbose::Debug)));
        assert_eq!(Verbose::parse("on"), Err(()));
        for level in [Verbose::Off, Verbose::On, Verbose::Debug] {
            assert_eq!(Verbose::from_index(level.index()), level);
        }
    }

    #[test]
    fn the_environment_sets_the_level_and_aspect_debug_wins() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            Verbose::from_env(move |name| {
                vars.iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| (*v).into())
            })
        };
        assert_eq!(env(&[]), Ok(None));
        assert_eq!(env(&[("ASPECT_VERBOSE", "")]), Ok(None));
        assert_eq!(env(&[("ASPECT_VERBOSE", "auto")]), Ok(None));
        assert_eq!(env(&[("ASPECT_VERBOSE", "false")]), Ok(Some(Verbose::Off)));
        assert_eq!(
            env(&[("ASPECT_VERBOSE", "debug")]),
            Ok(Some(Verbose::Debug))
        );
        assert_eq!(env(&[("ASPECT_DEBUG", "1")]), Ok(Some(Verbose::Debug)));
        assert_eq!(
            env(&[("ASPECT_DEBUG", "1"), ("ASPECT_VERBOSE", "false")]),
            Ok(Some(Verbose::Debug))
        );
        assert_eq!(env(&[("ASPECT_DEBUG", "")]), Ok(None));
        assert_eq!(
            env(&[("ASPECT_VERBOSE", "debgu")]),
            Err(r#"ASPECT_VERBOSE must be one of auto, true, false, debug, got "debgu"; did you mean "debug"?"#.to_owned())
        );
    }

    #[test]
    fn plain_omits_ansi() {
        for sev in [
            Severity::Debug,
            Severity::Info,
            Severity::Warning,
            Severity::Error,
        ] {
            let line = format_line(sev, false, "hello");
            assert_eq!(line, format!("{}: hello", sev.label()));
            assert!(!line.contains('\x1b'), "{sev:?} leaked an escape: {line:?}");
        }
    }
}
