//! The `Aspect CLI v<version>` identity line with the docs URL, in the theme's
//! `muted`.
//!
//! Shown in two places — at the top of every `--help` screen and above each
//! `▸ Running …` task header at runtime — so users always know which CLI
//! they're holding and where to find docs, even when aspect is invoked through
//! a `tools/bazel` wrapper or a custom org alias. Both call sites render the
//! same string in the same color from here so the two can't drift. At runtime
//! the line reaches the console's `banner` slot, which a `Console` layout may
//! leave blank (see [`crate::console`]).

use std::io::IsTerminal;

use aspect_telemetry::{cargo_pkg_short_version, is_debug_build};

use crate::ci::on_recognized_ci;

/// Docs landing page advertised by the banner.
const DOCS_URL: &str = crate::links::CLI;

/// The banner line for `version`, e.g.
/// `Aspect CLI v1.2.3 — https://aspect.build/go/cli`, in `muted`.
///
/// A `-debug-` build is marked here too: it is slower and larger than the primary
/// binary, so a run that unexpectedly used one should be obvious from its output.
///
/// No trailing newline: callers place it within their own layout. Styled
/// in the process's theme ([`crate::theme::of_process`]).
pub fn line(version: &str) -> String {
    line_colored(version, Some(&crate::theme::of_process()))
}

/// [`line`], in `palette`'s `muted`, or plain with none.
///
/// Call sites that print the banner themselves (rather than handing it to clap,
/// which strips escapes by the rules [`crate::color`] follows) pass a palette
/// only when their stream takes color ([`crate::color::stdout_supports_color`]
/// for stdout, the console's color for the runtime banner), so a piped run
/// stays plain text.
pub fn line_colored(version: &str, palette: Option<&aspect_theme::Palette>) -> String {
    let debug = if is_debug_build() {
        " (debug build)"
    } else {
        ""
    };
    let body = format!("Aspect CLI v{version}{debug} — {DOCS_URL}");
    match palette {
        Some(p) => p.paint("muted", &body),
        None => body,
    }
}

/// The console's `banner` slot context: the built-in line whole, and its pieces
/// for a template that lays the line out itself, `highlight_style` being
/// `muted`. `palette` is the run's theme when the console colors, as for the
/// header and verdict, so the style is left out wherever they print plain.
pub fn console_context(palette: Option<&aspect_theme::Palette>) -> minijinja::Value {
    let highlight_style = palette.map(|p| p.sgr("muted")).unwrap_or_default();
    let highlight_reset = if highlight_style.is_empty() {
        ""
    } else {
        aspect_theme::RESET
    };
    minijinja::context! {
        line => line_colored(&cargo_pkg_short_version(), palette),
        version => cargo_pkg_short_version(),
        debug_build => is_debug_build(),
        docs_url => DOCS_URL,
        highlight_style => highlight_style,
        highlight_reset => highlight_reset,
    }
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

    const MUTED: &str = "\x1b[38;5;247m";
    const RESET: &str = aspect_theme::RESET;

    fn dark() -> aspect_theme::Palette {
        aspect_theme::Palette::aspect_dark(aspect_theme::Depth::Ansi256)
    }

    #[test]
    fn the_console_context_follows_color() {
        let plain = console_context(None);
        assert_eq!(plain.get_attr("highlight_style").unwrap().to_string(), "");
        assert_eq!(plain.get_attr("highlight_reset").unwrap().to_string(), "");
        assert!(!plain.get_attr("line").unwrap().to_string().contains('\x1b'));
        let muted = console_context(Some(&dark()));
        assert_eq!(
            muted.get_attr("highlight_style").unwrap().to_string(),
            MUTED
        );
        assert!(
            muted
                .get_attr("line")
                .unwrap()
                .to_string()
                .starts_with(MUTED)
        );
    }

    #[test]
    fn line_renders_muted_version_and_docs_url() {
        let s = line_colored("9.9.9", Some(&dark()));
        // Tests build with debug assertions on, so the marker is expected here; the
        // release binary renders the same line without it.
        let debug = if is_debug_build() {
            " (debug build)"
        } else {
            ""
        };
        assert_eq!(
            s,
            format!("{MUTED}Aspect CLI v9.9.9{debug} — {DOCS_URL}{RESET}")
        );
        assert!(s.starts_with(MUTED) && s.ends_with(RESET));
    }

    #[test]
    fn line_colored_drops_the_style_when_uncolored() {
        let plain = line_colored("9.9.9", None);
        assert!(!plain.contains('\x1b'), "no escapes when uncolored");
        assert!(plain.starts_with("Aspect CLI v9.9.9"));
        assert_eq!(line("9.9.9"), plain, "a unit test's process theme is plain");
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
