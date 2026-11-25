//! ANSI escape sequences in a console record.
//!
//! Under a PTY, bazel's curses UI prefixes each message with the sequences
//! that erase its progress area (`\r` then `ESC[1A ESC[K` per line) and
//! colours the severity. Patterns are matched against a record's text with
//! these removed ([`strip`]), and a record a stage suppresses or replaces
//! keeps them ([`controls`], [`leading`]): the cursor and colour state bazel
//! tracks then stays the one it assumes, and its next redraw lands where it
//! should.
//!
//! Sequences are recognised per ECMA-48: CSI (`ESC [` … final byte), OSC and
//! the other string sequences (`ESC ]`, `ESC P`, `ESC X`, `ESC ^`, `ESC _`, up
//! to BEL or `ESC \`, or before any other ESC), and two-byte escapes. One
//! left unterminated at the end of a record runs to its end.

use std::borrow::Cow;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

/// Length of the escape sequence at the start of `b`, which starts with ESC.
fn escape_len(b: &[u8]) -> usize {
    debug_assert_eq!(b.first(), Some(&ESC));
    match b.get(1) {
        None => 1,
        Some(b'[') => {
            let mut i = 2;
            while i < b.len() && (0x20..=0x3f).contains(&b[i]) {
                i += 1;
            }
            if i < b.len() && (0x40..=0x7e).contains(&b[i]) {
                i + 1
            } else {
                i
            }
        }
        Some(b']' | b'P' | b'X' | b'^' | b'_') => {
            let mut i = 2;
            while i < b.len() {
                if b[i] == BEL {
                    return i + 1;
                }
                if b[i] == ESC {
                    // Another sequence, as a terminal's parser takes it,
                    // where the ESC is not ST's.
                    return if b.get(i + 1) == Some(&b'\\') {
                        i + 2
                    } else {
                        i
                    };
                }
                i += 1;
            }
            i
        }
        // A lone ESC before another ESC or a control byte is just itself.
        Some(&next) if next == ESC || next < 0x20 => 1,
        Some(_) => {
            let mut i = 1;
            while i < b.len() && (0x20..=0x2f).contains(&b[i]) {
                i += 1;
            }
            (i + 1).min(b.len())
        }
    }
}

/// Split `b` into its runs of text (`false`) and escape sequences (`true`).
pub fn segments(b: &[u8]) -> impl Iterator<Item = (bool, &[u8])> {
    let mut rest = b;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let (is_escape, len) = if rest[0] == ESC {
            (true, escape_len(rest))
        } else {
            (
                false,
                rest.iter().position(|&c| c == ESC).unwrap_or(rest.len()),
            )
        };
        let (seg, tail) = rest.split_at(len);
        rest = tail;
        Some((is_escape, seg))
    })
}

/// `b` without its escape sequences.
pub fn strip(b: &[u8]) -> Cow<'_, [u8]> {
    if !b.contains(&ESC) {
        return Cow::Borrowed(b);
    }
    Cow::Owned(
        segments(b)
            .filter(|(esc, _)| !esc)
            .flat_map(|(_, s)| s.iter().copied())
            .collect(),
    )
}

/// The terminal columns `c` takes: 2 for a wide character (CJK, fullwidth
/// forms, and emoji shown as emoji by default), 0 for one that joins or
/// changes the one before it (combining marks, zero-width joiners and
/// spaces, variation selectors) and for control characters, else 1. The
/// common cases, without a full Unicode width table.
pub fn char_width(c: char) -> usize {
    let u = c as u32;
    let zero = [
        (0x0000, 0x001F),
        (0x007F, 0x009F),
        (0x0300, 0x036F),
        (0x1AB0, 0x1AFF),
        (0x1DC0, 0x1DFF),
        (0x200B, 0x200F),
        (0x20D0, 0x20FF),
        (0xFE00, 0xFE0F),
        (0xFE20, 0xFE2F),
        (0xE0100, 0xE01EF),
    ];
    if zero.iter().any(|&(a, b)| (a..=b).contains(&u)) {
        return 0;
    }
    let wide = [
        // CJK and Hangul.
        (0x1100, 0x115F),
        (0x2E80, 0x303E),
        (0x3041, 0x33FF),
        (0x3400, 0x4DBF),
        (0x4E00, 0x9FFF),
        (0xA000, 0xA4CF),
        (0xAC00, 0xD7A3),
        (0xF900, 0xFAFF),
        (0xFE30, 0xFE4F),
        (0xFF00, 0xFF60),
        (0xFFE0, 0xFFE6),
        (0x20000, 0x3FFFD),
        // Emoji.
        (0x1F1E6, 0x1F1FF),
        (0x1F300, 0x1F64F),
        (0x1F680, 0x1F6FF),
        (0x1F900, 0x1F9FF),
        (0x1FA70, 0x1FAFF),
        // Symbols shown as emoji by default.
        (0x231A, 0x231B),
        (0x23E9, 0x23EC),
        (0x23F0, 0x23F0),
        (0x23F3, 0x23F3),
        (0x25FD, 0x25FE),
        (0x2614, 0x2615),
        (0x2648, 0x2653),
        (0x267F, 0x267F),
        (0x2693, 0x2693),
        (0x26A1, 0x26A1),
        (0x26AA, 0x26AB),
        (0x26BD, 0x26BE),
        (0x26C4, 0x26C5),
        (0x26CE, 0x26CE),
        (0x26D4, 0x26D4),
        (0x26EA, 0x26EA),
        (0x26F2, 0x26F3),
        (0x26F5, 0x26F5),
        (0x26FA, 0x26FA),
        (0x26FD, 0x26FD),
        (0x2705, 0x2705),
        (0x270A, 0x270B),
        (0x2728, 0x2728),
        (0x274C, 0x274C),
        (0x274E, 0x274E),
        (0x2753, 0x2755),
        (0x2757, 0x2757),
        (0x2795, 0x2797),
        (0x27B0, 0x27B0),
        (0x27BF, 0x27BF),
        (0x2B1B, 0x2B1C),
        (0x2B50, 0x2B50),
        (0x2B55, 0x2B55),
    ];
    if wide.iter().any(|&(a, b)| (a..=b).contains(&u)) {
        2
    } else {
        1
    }
}

/// The terminal columns `b`'s text takes, escape sequences aside
/// ([`char_width`]); an invalid UTF-8 sequence counts as one.
pub fn width(b: &[u8]) -> usize {
    let text = strip(b);
    text.utf8_chunks()
        .map(|chunk| {
            chunk.valid().chars().map(char_width).sum::<usize>()
                + usize::from(!chunk.invalid().is_empty())
        })
        .sum()
}

/// `b` with its text cut to `width` columns ([`width`]): a wide character
/// that would cross the edge is left out, and so is what follows it. Every
/// escape sequence is kept (a cursor movement or a colour reset after the
/// cut still applies), and every byte kept is as it was, valid UTF-8 or not.
pub fn truncate(b: &[u8], width: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut left = width;
    // Past the cut: no more text, though escape sequences still go out.
    let mut cut = false;
    for (is_escape, seg) in segments(b) {
        if is_escape {
            out.extend_from_slice(seg);
            continue;
        }
        for chunk in seg.utf8_chunks() {
            for c in chunk.valid().chars() {
                // A zero-width character goes with the one before it, so one
                // right at the edge (an emoji's variation selector) is kept.
                let w = char_width(c);
                if cut || w > left {
                    cut = true;
                    break;
                }
                let mut buf = [0; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                left -= w;
            }
            // An invalid sequence counts as one column, as in `width`.
            if !cut && !chunk.invalid().is_empty() {
                if left == 0 {
                    cut = true;
                } else {
                    out.extend_from_slice(chunk.invalid());
                    left -= 1;
                }
            }
        }
    }
    out
}

/// Whether `seg`, an escape sequence, sets colours or text attributes (SGR:
/// `ESC [ … m`) rather than moving the cursor or erasing.
pub fn is_sgr(seg: &[u8]) -> bool {
    seg.starts_with(b"\x1b[") && seg.ends_with(b"m")
}

/// `b` without its colour sequences (SGR) when `colour`, and without every
/// other escape sequence (cursor movement, erasing, OSC…) when `cursor`;
/// every other byte kept.
pub fn without(b: &[u8], colour: bool, cursor: bool) -> Cow<'_, [u8]> {
    if !b.contains(&ESC) || (!colour && !cursor) {
        return Cow::Borrowed(b);
    }
    Cow::Owned(
        segments(b)
            .filter(|(esc, seg)| !esc || if is_sgr(seg) { !colour } else { !cursor })
            .flat_map(|(_, s)| s.iter().copied())
            .collect(),
    )
}

/// [`strip`] as text, lossily decoded.
pub fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(&strip(b)).into_owned()
}

/// Only the escape sequences of `b`, in order: what a suppressed record
/// leaves behind.
pub fn controls(b: &[u8]) -> Vec<u8> {
    segments(b)
        .filter(|(esc, _)| *esc)
        .flat_map(|(_, s)| s.iter().copied())
        .collect()
}

/// `b` with `f` applied to each run of text, escape sequences kept as they
/// are.
pub fn map_text(b: &[u8], mut f: impl FnMut(&[u8]) -> Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    for (is_escape, seg) in segments(b) {
        if is_escape {
            out.extend_from_slice(seg);
        } else {
            out.extend(f(seg));
        }
    }
    out
}

/// Whether `controls` (escape sequences only) do nothing a terminal would
/// carry forward: colour changes ending in a reset, or nothing at all.
pub fn is_neutral(controls: &[u8]) -> bool {
    let mut last_reset = true;
    for (is_escape, seg) in segments(controls) {
        let sgr = is_escape && is_sgr(seg);
        if !sgr {
            return false;
        }
        let params = &seg[2..seg.len() - 1];
        last_reset = params.is_empty() || params == b"0";
    }
    last_reset
}

/// The escape sequences `b` starts with, before its first text: what a
/// replaced record keeps in front of its replacement.
pub fn leading(b: &[u8]) -> &[u8] {
    let len = segments(b)
        .take_while(|(esc, _)| *esc)
        .map(|(_, s)| s.len())
        .sum();
    &b[..len]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncating_keeps_escapes_and_bytes_as_they_were() {
        let row = b"\x1b[32m[1 / 2]\x1b[0m Fetching \xffxyz\x1b[0m";
        assert_eq!(truncate(row, 100), row.to_vec(), "nothing to cut");
        assert_eq!(
            truncate(row, 9),
            b"\x1b[32m[1 / 2]\x1b[0m F\x1b[0m".to_vec()
        );
        assert_eq!(
            truncate(row, 18),
            b"\x1b[32m[1 / 2]\x1b[0m Fetching \xff\x1b[0m".to_vec()
        );
        assert_eq!(truncate("漢字漢字".as_bytes(), 2), "漢".as_bytes().to_vec());
        assert_eq!(
            truncate("漢字漢字".as_bytes(), 3),
            "漢".as_bytes().to_vec(),
            "a wide character that would cross the edge is left out"
        );
        assert_eq!(
            truncate("a⚡\u{FE0F}b".as_bytes(), 3),
            "a⚡\u{FE0F}".as_bytes().to_vec(),
            "a variation selector stays with its emoji"
        );
    }

    #[test]
    fn width_counts_terminal_columns() {
        assert_eq!(width(b"plain"), 5);
        assert_eq!(width("\x1b[1m🔨\x1b[0m Build".as_bytes()), 8);
        assert_eq!(width("⚡\u{FE0F} x".as_bytes()), 4);
        assert_eq!(width("漢字".as_bytes()), 4);
        assert_eq!(
            width("─┐…│".as_bytes()),
            4,
            "box drawing and the ellipsis are narrow"
        );
        assert_eq!(
            width("e\u{301}".as_bytes()),
            1,
            "a combining accent takes no column"
        );
        assert_eq!(width(b"a\xffb"), 3);
    }

    /// A bazel 9 curses message, as a PTY capture records it.
    const WARNING: &[u8] = b"\x1b[1A\x1b[K\x1b[35mWARNING: \x1b[0mtarget '//:g0' is deprecated";

    #[test]
    fn strip_leaves_the_text() {
        assert_eq!(&*strip(WARNING), b"WARNING: target '//:g0' is deprecated");
        assert_eq!(&*strip(b"plain"), b"plain");
        assert!(matches!(strip(b"plain"), Cow::Borrowed(_)));
    }

    #[test]
    fn controls_keep_every_sequence_in_order() {
        assert_eq!(controls(WARNING), b"\x1b[1A\x1b[K\x1b[35m\x1b[0m");
        assert_eq!(controls(b"plain"), b"");
    }

    #[test]
    fn leading_stops_at_the_first_text() {
        assert_eq!(leading(WARNING), b"\x1b[1A\x1b[K\x1b[35m");
        assert_eq!(leading(b"text\x1b[0m"), b"");
    }

    #[test]
    fn string_sequences_and_two_byte_escapes_are_recognised() {
        let osc = b"\x1b]8;;https://example.com\x07link\x1b]8;;\x1b\\ \x1b7end";
        assert_eq!(&*strip(osc), b"link end");
        assert_eq!(
            controls(osc),
            b"\x1b]8;;https://example.com\x07\x1b]8;;\x1b\\\x1b7"
        );
    }

    /// A string sequence ends before an ESC that is not ST's, which starts
    /// the next sequence, as a terminal reads it.
    #[test]
    fn a_string_sequence_ends_before_another_escape() {
        let b = b"\x1b]8;;\x1b]8;;file:///x\x07ls";
        let segs: Vec<_> = segments(b).collect();
        assert_eq!(
            segs,
            [
                (true, &b"\x1b]8;;"[..]),
                (true, &b"\x1b]8;;file:///x\x07"[..]),
                (false, &b"ls"[..]),
            ]
        );
    }

    #[test]
    fn a_lone_escape_before_another_is_one_byte() {
        assert_eq!(&*strip(b"\x1b\x1b[31mred"), b"red");
        assert_eq!(controls(b"\x1b\x1b[31mred"), b"\x1b\x1b[31m");
    }

    #[test]
    fn colours_that_end_reset_are_neutral() {
        assert!(is_neutral(b""));
        assert!(is_neutral(b"\x1b[35m\x1b[0m"));
        assert!(is_neutral(b"\x1b[31m\x1b[1m\x1b[m"));
        assert!(!is_neutral(b"\x1b[35m"), "a colour left on");
        assert!(
            !is_neutral(b"\x1b[1A\x1b[K\x1b[35m\x1b[0m"),
            "cursor movement"
        );
    }

    #[test]
    fn without_drops_colour_or_cursor_sequences_alone() {
        assert_eq!(
            &*without(WARNING, false, true),
            b"\x1b[35mWARNING: \x1b[0mtarget '//:g0' is deprecated"
        );
        assert_eq!(
            &*without(WARNING, true, false),
            b"\x1b[1A\x1b[KWARNING: target '//:g0' is deprecated"
        );
        assert_eq!(&*without(WARNING, true, true), &*strip(WARNING));
        assert_eq!(&*without(WARNING, false, false), WARNING);
    }

    #[test]
    fn map_text_leaves_escapes_alone() {
        assert_eq!(
            map_text(b"\x1b[1mab\x1b[0mcd", |t| t.to_ascii_uppercase()),
            b"\x1b[1mAB\x1b[0mCD"
        );
    }

    #[test]
    fn an_unterminated_sequence_runs_to_the_end() {
        assert_eq!(&*strip(b"ok\x1b[12"), b"ok");
        assert_eq!(&*strip(b"ok\x1b"), b"ok");
        assert_eq!(&*strip(b"ok\x1b]title"), b"ok");
    }
}
