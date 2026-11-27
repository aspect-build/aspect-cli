//! Text as a terminal lays it out: how many cells a string takes, and a
//! string cut or padded to a number of cells, for a screen that draws rows
//! exactly as wide as it is (`std.term.width`, `std.term.fit`).
//!
//! A code point's cells are Unicode's (East Asian Width, through
//! `unicode-width`): two for a wide character (CJK, most emoji), none for a
//! combining mark or a zero-width one. Every code point below U+0300 is one,
//! a control character among them. U+FE0F, the emoji presentation selector,
//! makes a one-cell character before it two (`⚠️`), as terminals draw it.

use std::borrow::Cow;

use unicode_width::UnicodeWidthChar;

/// U+FE0F, the emoji presentation selector.
const EMOJI_SELECTOR: char = '\u{FE0F}';

/// What [`fit`] puts in place of what it cuts.
const ELLIPSIS: char = '…';

/// What [`fit`] writes for a tab: one would move the cursor off the row.
const TAB: &str = "  ";

/// How many cells a terminal draws `c` in, alone.
fn cells(c: char) -> usize {
    if (c as u32) < 0x300 {
        return 1;
    }
    c.width().unwrap_or(1)
}

/// Each code point of `text`, at its byte offset, with the cells it adds:
/// [`cells`], but for [`EMOJI_SELECTOR`] after a one-cell character, which
/// adds one, making it two.
fn laid_out(text: &str) -> impl Iterator<Item = (usize, char, usize)> + '_ {
    let mut last = 0;
    text.char_indices().map(move |(at, c)| {
        let n = if c == EMOJI_SELECTOR && last == 1 {
            1
        } else {
            cells(c)
        };
        if n > 0 {
            last = n;
        }
        (at, c, n)
    })
}

/// How many cells a terminal draws `text` in.
pub(super) fn width(text: &str) -> usize {
    laid_out(text).map(|(_, _, n)| n).sum()
}

/// `text` cut or padded with spaces to exactly `width` cells, a tab as two
/// spaces: "" for a `width` of 0 or less. Cut, it keeps what fits in one
/// cell less, then [`ELLIPSIS`] (no ellipsis at a `width` of 1); a wide
/// character that would cross the edge is left out, padding in its place,
/// and so is everything after it; zero-width characters right after the
/// last character kept stay with it. Only as much of `text` is measured as
/// could show.
pub(super) fn fit(text: &str, width: i64) -> String {
    let Ok(width) = usize::try_from(width) else {
        return String::new();
    };
    if width == 0 {
        return String::new();
    }
    let text = if text.contains('\t') {
        Cow::Owned(text.replace('\t', TAB))
    } else {
        Cow::Borrowed(text)
    };
    let room = if width > 1 { width - 1 } else { width };
    // Where a cut would fall, and the cells before it.
    let mut cut = None;
    let mut used = 0;
    for (at, _, n) in laid_out(&text) {
        if cut.is_none() && used + n > room {
            cut = Some((at, used));
        }
        used += n;
        if used > width {
            break;
        }
    }
    let mut out = String::with_capacity(text.len() + width);
    let used = match cut.filter(|_| used > width) {
        None => {
            out.push_str(&text);
            used
        }
        Some((at, kept)) => {
            out.push_str(&text[..at]);
            if width > 1 {
                out.push(ELLIPSIS);
                kept + 1
            } else {
                kept
            }
        }
    };
    out.extend(std::iter::repeat_n(' ', width - used));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_character_is_two_cells_and_a_combining_mark_none() {
        assert_eq!(width("✅"), 2);
        assert_eq!(width("→ ✅ ok ✨📋"), 12);
        assert_eq!(width("e\u{301}✔"), 2);
        assert_eq!(width("中文"), 4);
        assert_eq!(width(""), 0);
    }

    /// U+FE0F after a one-cell character makes it two; after a wide one, or
    /// with nothing before it, it adds nothing.
    #[test]
    fn the_emoji_selector_widens_a_narrow_character() {
        assert_eq!(width("⚠️ x"), 4);
        assert_eq!(width("⚠"), 1);
        assert_eq!(width("✅\u{FE0F}"), 2);
        assert_eq!(width("\u{FE0F}"), 0);
        assert_eq!(width("⚠\u{301}\u{FE0F}"), 2, "past a combining mark");
    }

    /// A code point below U+0300 is one cell, a control character among them.
    #[test]
    fn latin_and_controls_are_one_cell() {
        assert_eq!(width("abc"), 3);
        assert_eq!(width("é"), 1);
        assert_eq!(width("\u{1b}[1m"), 4);
        assert_eq!(width("\u{ad}"), 1);
    }

    #[test]
    fn fit_pads_to_the_width() {
        assert_eq!(fit("ab", 4), "ab  ");
        assert_eq!(fit("✅ a", 6), "✅ a  ");
        assert_eq!(fit("abcd", 4), "abcd");
        assert_eq!(fit("", 3), "   ");
    }

    #[test]
    fn fit_cuts_to_one_cell_less_and_an_ellipsis() {
        assert_eq!(fit("abcdef", 4), "abc…");
        assert_eq!(fit("ab✅✅✅", 6), "ab✅… ");
        assert_eq!(fit("✅".repeat(5000).as_str(), 7), "✅✅✅…");
    }

    /// A wide character that would cross the edge is left out, padding in
    /// its place, and what follows it with it.
    #[test]
    fn fit_never_splits_a_wide_character() {
        assert_eq!(fit("a✅b", 2), "a…");
        assert_eq!(fit("a✅b", 3), "a… ");
        assert_eq!(fit("✅b", 1), " ");
    }

    /// At a width of 1 a cut has no ellipsis.
    #[test]
    fn fit_at_one_cell_has_no_ellipsis() {
        assert_eq!(fit("ab", 1), "a");
        assert_eq!(fit("a", 1), "a");
    }

    #[test]
    fn fit_with_no_room_is_empty() {
        assert_eq!(fit("ab", 0), "");
        assert_eq!(fit("ab", -3), "");
    }

    #[test]
    fn fit_writes_a_tab_as_two_spaces() {
        assert_eq!(fit("a\tb", 5), "a  b ");
        assert_eq!(fit("a\tb", 3), "a …");
    }

    /// Zero-width characters right after the last character kept stay; a
    /// line of them is measured whole.
    #[test]
    fn fit_keeps_zero_width_characters_after_the_last_kept() {
        let marks = "\u{301}".repeat(50);
        assert_eq!(fit(&format!("a{marks}b"), 3), format!("a{marks}b "));
        assert_eq!(fit(&format!("ab{marks}cd"), 3), format!("ab{marks}…"));
        assert_eq!(fit("ab\u{301}c\u{301}", 2), "a…");
    }
}
