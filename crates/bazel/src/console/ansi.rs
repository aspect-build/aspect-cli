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
//! to BEL or `ESC \`), and two-byte escapes. One left unterminated at the end
//! of a record runs to its end.

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
                if b[i] == ESC && b.get(i + 1) == Some(&b'\\') {
                    return i + 2;
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
pub(crate) fn segments(b: &[u8]) -> impl Iterator<Item = (bool, &[u8])> {
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

/// The characters of `b`'s text, escape sequences aside: its width on a
/// terminal for the text bazel writes, though a wide (CJK) character or a
/// tab takes more than one column.
pub fn width(b: &[u8]) -> usize {
    String::from_utf8_lossy(&strip(b)).chars().count()
}

/// `b` with its text cut to `width` characters ([`width`]), every escape
/// sequence kept (a cursor movement or a colour reset after the cut still
/// applies) and every byte kept is as it was, valid UTF-8 or not.
pub fn truncate(b: &[u8], width: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut left = width;
    for (is_escape, seg) in segments(b) {
        if is_escape {
            out.extend_from_slice(seg);
            continue;
        }
        for chunk in seg.utf8_chunks() {
            for c in chunk.valid().chars() {
                if left == 0 {
                    break;
                }
                let mut buf = [0; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                left -= 1;
            }
            // An invalid sequence counts as one character, as in `width`.
            if left > 0 && !chunk.invalid().is_empty() {
                out.extend_from_slice(chunk.invalid());
                left -= 1;
            }
        }
    }
    out
}

/// Whether `seg`, an escape sequence, sets colours or text attributes (SGR:
/// `ESC [ … m`) rather than moving the cursor or erasing.
fn is_sgr(seg: &[u8]) -> bool {
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
        assert_eq!(
            truncate("漢字漢字".as_bytes(), 2),
            "漢字".as_bytes().to_vec()
        );
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
