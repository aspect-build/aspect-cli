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

/// Remove ANSI escape sequences from `s`.
///
/// [`stdout_supports_color`] gates output we assemble ourselves, but prose
/// authored in AXL can carry its own escapes — `aspect delivery`'s description
/// contains `\x1b[3m…\x1b[23m` — and that prose is copied verbatim into
/// `aspect describe`, which is machine-readable by construction. Escapes are
/// stripped there rather than at the author's keyboard, so a task may still be
/// italic in `--help` and clean in JSON.
///
/// Handles CSI (`ESC [ … 0x40..=0x7E`), OSC (`ESC ] … BEL | ESC \`) and
/// two-character escapes.
pub fn strip_ansi(s: &str) -> String {
    if !s.contains('\x1b') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                let mut prev_esc = false;
                for c in chars.by_ref() {
                    if c == '\u{7}' || (prev_esc && c == '\\') {
                        break;
                    }
                    prev_esc = c == '\x1b';
                }
            }
            // A two-character escape is fully consumed by taking the second char.
            Some(_) | None => {}
        }
    }
    out
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

    #[test]
    fn strip_ansi_removes_escapes_and_leaves_plain_text_untouched() {
        // The real case: `aspect delivery`'s description.
        assert_eq!(
            strip_ansi("Currently \x1b[3monly\x1b[23m supported on runners."),
            "Currently only supported on runners."
        );
        // Untouched, and allocation-cheap because it short-circuits.
        assert_eq!(strip_ansi("no escapes here"), "no escapes here");
        // Multi-parameter SGR, and a reset.
        assert_eq!(strip_ansi("\x1b[1;4mbold\x1b[0m"), "bold");
        // OSC 8 hyperlink, terminated by BEL and by ST.
        assert_eq!(strip_ansi("\x1b]8;;http://x\x07link"), "link");
        assert_eq!(strip_ansi("\x1b]0;title\x1b\\rest"), "rest");
        // Two-character escape.
        assert_eq!(strip_ansi("a\x1bMb"), "ab");
        // A trailing lone ESC must not panic or emit anything.
        assert_eq!(strip_ansi("tail\x1b"), "tail");
        // An unterminated CSI swallows the remainder rather than leaking it.
        assert_eq!(strip_ansi("x\x1b[1;2"), "x");
    }
}
