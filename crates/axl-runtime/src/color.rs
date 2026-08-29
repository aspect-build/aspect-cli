//! Whether hand-rendered output may carry ANSI color.
//!
//! Clap strips the escapes baked into our `--help` templates when it renders to
//! a non-terminal, so help output is already safe. The surfaces that print their
//! own layout (`aspect feature`, and anything else assembling SGR sequences by
//! hand) have no such protection and would otherwise emit raw escapes into a
//! pipe — a log file, a `| grep`, or an AI agent capturing stdout.

use std::io::IsTerminal;

/// Whether stdout may carry ANSI color.
///
/// True only on an interactive stdout, and never when `NO_COLOR` is set
/// (<https://no-color.org>).
pub fn stdout_supports_color() -> bool {
    supports_color_from(
        std::io::stdout().is_terminal(),
        std::env::var_os("NO_COLOR").is_some(),
    )
}

/// Pure core of [`stdout_supports_color`], parameterized over its inputs so the
/// gating is testable without a real TTY or process env.
fn supports_color_from(is_tty: bool, no_color_set: bool) -> bool {
    is_tty && !no_color_set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_only_on_a_tty_without_no_color() {
        assert!(supports_color_from(true, false));
        assert!(!supports_color_from(false, false));
        assert!(!supports_color_from(true, true));
        assert!(!supports_color_from(false, true));
    }
}
