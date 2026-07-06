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
//! Two deliberate differences from the AXL helpers:
//!   - **Writes to stderr, not stdout.** These are diagnostics about a task's
//!     machinery; keeping them off stdout avoids interleaving with piped build
//!     output. Color therefore keys off *stderr's* TTY, matching `banner`.
//!   - **No `std` handle.** Rust callers have no Starlark `ctx`, so the color
//!     predicate reads the ambient environment directly (via [`crate::ci`] and
//!     `IsTerminal`) instead of `std.io.stdout.is_tty`.

use std::io::IsTerminal;

use crate::ci::on_recognized_ci;

/// Severity of a diagnostic line, carrying its label and color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Severity {
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

/// Whether to emit ANSI color, using the same predicate as the rest of the CLI:
/// an interactive stderr, or a recognized CI host (whose log viewers render
/// ANSI despite a non-TTY pipe).
fn colorize() -> bool {
    std::io::stderr().is_terminal() || on_recognized_ci()
}

/// Print one severity-prefixed line to stderr, colorized per [`colorize`].
fn emit(sev: Severity, msg: &str) {
    eprintln!("{}", format_line(sev, colorize(), msg));
}

/// Print a cyan `INFO:` line for a notable runtime branch.
pub fn info(msg: &str) {
    emit(Severity::Info, msg);
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
