//! Whether hand-rendered output may carry ANSI color.
//!
//! Clap strips the escapes baked into our `--help` templates when it renders to
//! a non-terminal, so help output is already safe. The surfaces that print their
//! own layout (`aspect feature`, the console lines around a task, and anything
//! else assembling SGR sequences by hand) have no such protection and would
//! otherwise emit raw escapes into a pipe — a log file, a `| grep`, or an AI
//! agent capturing output.
//!
//! A non-empty `NO_COLOR` (<https://no-color.org>) turns color off on every
//! surface, CI hosts whose logs render ANSI included. `color_enabled` in
//! `lib/environment.axl` applies the same rule to AXL's own stderr lines.

use std::ffi::OsStr;
use std::io::IsTerminal;

/// Whether stdout may carry ANSI color: only on an interactive stdout, and
/// never under [`no_color_requested`].
pub fn stdout_supports_color() -> bool {
    color_from(std::io::stdout().is_terminal(), no_color_requested())
}

/// Whether the lines the runtime prints to stderr (the console slots, `WARNING:`
/// and other severity prefixes, the grey Bazel trace line) may carry ANSI
/// color: on an interactive stderr, or on a CI host whose log viewer renders it
/// ([`crate::ci::ci_renders_ansi`]), and never under [`no_color_requested`].
/// The console resolves it once for every slot; AXL reads it as
/// `ctx.console.color`.
pub fn console_supports_color() -> bool {
    color_from(
        std::io::stderr().is_terminal() || crate::ci::ci_renders_ansi(),
        no_color_requested(),
    )
}

/// Whether `NO_COLOR` asks for no color: set to a non-empty value, as the
/// no-color.org spec reads it.
fn no_color_requested() -> bool {
    no_color_from(std::env::var_os("NO_COLOR").as_deref())
}

/// Pure core of [`no_color_requested`], over the variable's value.
fn no_color_from(value: Option<&OsStr>) -> bool {
    value.is_some_and(|v| !v.is_empty())
}

/// Pure core of both predicates: color where the stream renders it, unless
/// `NO_COLOR` asks otherwise.
fn color_from(renders: bool, no_color: bool) -> bool {
    renders && !no_color
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_only_where_it_renders_without_no_color() {
        assert!(color_from(true, false));
        assert!(!color_from(false, false));
        assert!(!color_from(true, true));
        assert!(!color_from(false, true));
    }

    #[test]
    fn only_a_non_empty_no_color_counts() {
        assert!(!no_color_from(None));
        assert!(!no_color_from(Some(OsStr::new(""))));
        assert!(no_color_from(Some(OsStr::new("1"))));
        assert!(no_color_from(Some(OsStr::new("0"))));
    }
}
