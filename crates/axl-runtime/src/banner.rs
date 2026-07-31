//! The grey "Aspect CLI v… — docs URL" identity line.
//!
//! Shown in two places — at the top of every `--help` screen and above each
//! `→ 🎬 Running …` task header at runtime — so users always know which CLI
//! they're holding and where to find docs, even when aspect is invoked through
//! a `tools/bazel` wrapper or a custom org alias. Both call sites render the
//! same string in the same color from here so the two can't drift.

use std::io::IsTerminal;

use aspect_telemetry::{cargo_pkg_short_version, is_debug_build};

use crate::ci::on_recognized_ci;

/// SGR sequence for the banner's grey (256-color 244).
const GREY: &str = "\x1b[38;5;244m";
/// SGR reset.
const RESET: &str = "\x1b[0m";

/// Docs landing page advertised by the banner.
const DOCS_URL: &str = "https://aspect.build/docs/cli";

/// The grey banner line for `version`, e.g.
/// `Aspect CLI v1.2.3 — https://aspect.build/docs/cli`, wrapped in ANSI grey.
///
/// A `-debug-` build is marked here too: it is slower and larger than the primary
/// binary, so a run that unexpectedly used one should be obvious from its output.
///
/// No trailing newline — callers place it within their own layout.
pub fn line(version: &str) -> String {
    let debug = if is_debug_build() {
        " (debug build)"
    } else {
        ""
    };
    format!("{GREY}Aspect CLI v{version}{debug} — {DOCS_URL}{RESET}")
}

/// [`line`] resolving the version from workspace crate metadata. For call
/// sites that don't have the resolved version handy.
pub fn line_from_pkg() -> String {
    line(&cargo_pkg_short_version())
}

/// Whether to print the banner above a runtime task header.
///
/// Shown only on an interactive TTY or a recognized CI host — never when
/// stderr is piped to a non-CI consumer (e.g. a script parsing task output) —
/// and suppressible everywhere via `ASPECT_CLI_NO_BANNER`.
///
/// (The `--help` banner is not gated this way: `--help` is an explicit request
/// for output, so it always prints.)
pub fn show_runtime_banner() -> bool {
    show_runtime_banner_from(
        std::io::stderr().is_terminal(),
        on_recognized_ci(),
        std::env::var_os("ASPECT_CLI_NO_BANNER").is_some(),
    )
}

/// Pure core of [`show_runtime_banner`], parameterized over its inputs so the
/// gating is testable without a real TTY or process env.
fn show_runtime_banner_from(stderr_is_tty: bool, on_ci: bool, no_banner_set: bool) -> bool {
    (stderr_is_tty || on_ci) && !no_banner_set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_renders_grey_version_and_docs_url() {
        let s = line("9.9.9");
        // Tests build with debug assertions on, so the marker is expected here; the
        // release binary renders the same line without it.
        let debug = if is_debug_build() {
            " (debug build)"
        } else {
            ""
        };
        assert_eq!(
            s,
            format!("{GREY}Aspect CLI v9.9.9{debug} — {DOCS_URL}{RESET}")
        );
        assert!(s.starts_with(GREY) && s.ends_with(RESET));
    }

    #[test]
    fn line_marks_debug_builds() {
        assert_eq!(line("9.9.9").contains("(debug build)"), is_debug_build());
    }

    #[test]
    fn runtime_banner_shows_on_tty_or_ci_unless_suppressed() {
        // Shown on an interactive TTY or a recognized CI host…
        assert!(show_runtime_banner_from(true, false, false));
        assert!(show_runtime_banner_from(false, true, false));
        assert!(show_runtime_banner_from(true, true, false));

        // …but NOT when stderr is piped to a non-CI consumer (a scripted/piped
        // run must not get an extra line).
        assert!(!show_runtime_banner_from(false, false, false));

        // ASPECT_CLI_NO_BANNER suppresses it on every surface.
        assert!(!show_runtime_banner_from(true, false, true));
        assert!(!show_runtime_banner_from(false, true, true));
        assert!(!show_runtime_banner_from(false, false, true));
    }
}
