//! Severity-prefixed user-facing diagnostics for the Rust side of the CLI.
//!
//! The Rust counterpart of the `info` / `warn` / `error` std helpers in
//! `lib/environment.axl`: the same `INFO:` / `WARNING:` / `ERROR:` prefix
//! style (mirroring Bazel's uppercase severity labels), the same ANSI colors,
//! and the same "degrade to plain text when color is off" behavior — so a line
//! printed from Rust is indistinguishable from one printed by an AXL task.
//!
//! Use this for user-facing subsystem messages (a backend became unreachable,
//! a fallback fired). It is **not** the structured/debug channel — that is
//! `tracing` (and the `ASPECT_DEBUG`-gated ad-hoc logging some subsystems keep).
//!
//! Both write to stderr and color by the same answer,
//! [`crate::color::console_supports_color`]: the process's memoized
//! [`crate::color::StreamColor`], the one value a run's `Env` carries as
//! `stream_color`, which the console ([`crate::console::Host`]) and AXL's
//! `std.io.stderr.color` read.
//!
//! [`info`] and the AXL `info` helper follow [`info_lines`], which the
//! `Console` feature sets for the running task when the console resolves; lines
//! printed before that, early in start-up, always show. [`emit`] does not: a
//! task's own exit message is its answer, not an aside, so it prints under
//! every preset. `WARNING:` and `ERROR:` lines always print.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::errln;

/// Whether `INFO:` lines print. True until the console resolves.
static INFO_LINES: AtomicBool = AtomicBool::new(true);

/// Show or hide `INFO:` lines for the rest of the run, from Rust and AXL alike.
pub fn set_info_lines(show: bool) {
    INFO_LINES.store(show, Ordering::Relaxed);
}

/// Whether `INFO:` lines print; see [`set_info_lines`].
pub fn info_lines() -> bool {
    INFO_LINES.load(Ordering::Relaxed)
}

/// Severity of a diagnostic line, carrying its label and color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Severity {
    /// Cyan `INFO:` — a notable but expected runtime branch fired.
    Info,
    /// Yellow `WARNING:` — a subsystem problem; the task continues.
    Warning,
    /// Red `ERROR:` — a recoverable error, usually preceding a non-zero exit.
    Error,
}

impl Severity {
    /// The uppercase label (without the trailing colon).
    fn label(self) -> &'static str {
        match self {
            Severity::Info => "INFO",
            Severity::Warning => "WARNING",
            Severity::Error => "ERROR",
        }
    }

    /// The SGR color-open sequence for the label, matching the AXL helpers
    /// (`INFO` bold cyan, `WARNING`/`ERROR` plain yellow/red).
    fn color(self) -> &'static str {
        match self {
            Severity::Info => "\x1b[1;36m",
            Severity::Warning => "\x1b[0;33m",
            Severity::Error => "\x1b[0;31m",
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

/// Print one severity-prefixed line to stderr, colorized per [`colorize`],
/// whatever [`info_lines`] says. The named helpers below cover the fixed
/// severities; call this when the severity is computed.
pub(crate) fn emit(sev: Severity, msg: &str) {
    errln!("{}", format_line(sev, colorize(), msg));
}

/// Print a cyan `INFO:` line for a notable runtime branch, unless
/// [`info_lines`] hides them.
pub fn info(msg: &str) {
    if info_lines() {
        emit(Severity::Info, msg);
    }
}

/// Print a yellow `WARNING:` line for a non-fatal subsystem failure.
pub fn warn(msg: &str) {
    emit(Severity::Warning, msg);
}

/// Print a red `ERROR:` line for a recoverable error (usually preceding a
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
            "\x1b[0;33mWARNING\x1b[0m: hello"
        );
        assert_eq!(
            format_line(Severity::Error, true, "hello"),
            "\x1b[0;31mERROR\x1b[0m: hello"
        );
    }

    #[test]
    fn plain_omits_ansi() {
        for sev in [Severity::Info, Severity::Warning, Severity::Error] {
            let line = format_line(sev, false, "hello");
            assert_eq!(line, format!("{}: hello", sev.label()));
            assert!(!line.contains('\x1b'), "{sev:?} leaked an escape: {line:?}");
        }
    }
}
