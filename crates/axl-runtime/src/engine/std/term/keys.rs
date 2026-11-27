use std::fs::File;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::fd::{hung_up, tty_readable};
use super::replies::{ANSWER_DUE, da1_reply, late_background_answer, late_wheel_answer};

/// What has been read from the terminal and not yet handed out as keys.
#[derive(Debug, Default)]
pub(super) struct Keys {
    pub(super) pending: Vec<u8>,
    /// Inside a paste whose end marker has not come yet: what arrives is
    /// more of it ([`paste_chunk`]), never keys.
    pub(super) in_paste: bool,
    /// Since when, inside a paste, a look at the terminal has found nothing
    /// waiting (cleared by anything read): to end a paste whose end marker
    /// never comes ([`PASTE_IDLE`]). Time unpolled (a frame late) is not
    /// quiet, since what came meanwhile is still waiting.
    pub(super) quiet_since: Option<std::time::Instant>,
    /// Since when a background answer has been arriving in pieces (a slow
    /// link): held whole, never read as keys ([`ANSWER_WAIT`] at most).
    pub(super) answer_since: Option<std::time::Instant>,
    /// The terminal is gone (a hangup): a read failed so ([`hung_up`]), or
    /// found nothing where a wait said something was. From then on there is
    /// no key to wait for.
    pub(super) gone: bool,
}

/// How long a paste may go without a byte before what follows is keys
/// again: a stream that lost its end marker would otherwise take every key
/// after it as pasted text, the finished screen's leave keys among them.
const PASTE_IDLE: Duration = Duration::from_secs(1);

impl Keys {
    /// The next key `tty` has, without waiting: one it cannot name is
    /// skipped, not taken for none. None when nothing has come, or, inside a
    /// paste, when its rest has not yet, which the caller waits for under its
    /// own deadline.
    pub(super) fn take(&mut self, tty: &mut File) -> std::io::Result<Option<String>> {
        loop {
            match self.next(tty, Duration::ZERO)? {
                Some(name) => return Ok(Some(name)),
                None if self.pending.is_empty() || self.in_paste => return Ok(None),
                None => {}
            }
        }
    }

    /// The next key typed within `timeout`, named as the Starlark `key()`
    /// says; None for one it cannot name, or nothing.
    fn next(&mut self, tty: &mut File, timeout: Duration) -> std::io::Result<Option<String>> {
        let mut buf = [0u8; 4096];
        // Gone quiet inside a paste: its end is lost, and a fragment held back
        // for it (part of the marker, half a character) is dropped with it.
        if self.in_paste {
            if tty_readable(tty.as_raw_fd(), Duration::ZERO) {
                self.quiet_since = None;
            } else if self
                .quiet_since
                .get_or_insert_with(std::time::Instant::now)
                .elapsed()
                >= PASTE_IDLE
            {
                self.in_paste = false;
                self.pending.clear();
                self.quiet_since = None;
            }
        }
        if self.pending.is_empty() {
            if !tty_readable(tty.as_raw_fd(), timeout) {
                return Ok(None);
            }
            let n = match tty.read(&mut buf) {
                Err(e) if hung_up(&e) => 0,
                read => read?,
            };
            if n == 0 {
                self.gone = true;
                return Ok(None);
            }
            self.pending.extend_from_slice(&buf[..n]);
            self.quiet_since = None;
        }
        // A key split across reads (an escape sequence on a slow link, a
        // UTF-8 character or a paste cut by a full read): wait briefly for
        // the rest, `KEY_SLICE` at most in all, so a paste trickling in
        // never holds the screen's lock long; its rest comes as more pieces.
        let until = std::time::Instant::now() + KEY_SLICE;
        while unfinished(&self.pending) {
            let left = until.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() || !tty_readable(tty.as_raw_fd(), left.min(ESCAPE_WAIT)) {
                break;
            }
            let n = match tty.read(&mut buf) {
                Err(e) if hung_up(&e) => break,
                read => read?,
            };
            if n == 0 {
                break;
            }
            self.pending.extend_from_slice(&buf[..n]);
            self.quiet_since = None;
        }
        if self.in_paste {
            return Ok(paste_chunk(&mut self.pending, &mut self.in_paste));
        }
        // A paste still coming after the wait (a terminal that sends a long
        // one in pieces): what came now, the rest as more of it.
        if self.pending.starts_with(PASTE_START) && find(&self.pending, PASTE_END).is_none() {
            self.pending.drain(..PASTE_START.len());
            self.in_paste = true;
            return Ok(paste_chunk(&mut self.pending, &mut self.in_paste));
        }
        // A late background answer still coming: held until it is whole.
        if self.pending.starts_with(b"\x1b]11;") && unfinished(&self.pending) {
            let since = *self
                .answer_since
                .get_or_insert_with(std::time::Instant::now);
            if since.elapsed() < ANSWER_WAIT {
                return Ok(None);
            }
        }
        self.answer_since = None;
        if let Some(used) = late_wheel_answer(&self.pending).or_else(|| {
            late_background_answer(&self.pending).inspect(|_| {
                if da1_reply(&self.pending).is_some_and(|r| r.start == 0) {
                    ANSWER_DUE.store(false, Ordering::SeqCst);
                }
            })
        }) {
            self.pending.drain(..used);
            return Ok(None);
        }
        let (name, used) = parse_key(&self.pending);
        self.pending.drain(..used);
        Ok(name)
    }
}

/// The next piece of a paste being read, as `paste:<text>`: up to its end
/// marker, which ends it, or what has come, less a tail that may be the
/// start of the end marker or of a UTF-8 character. None when that leaves
/// nothing yet.
fn paste_chunk(pending: &mut Vec<u8>, in_paste: &mut bool) -> Option<String> {
    let (text_len, used) = match find(pending, PASTE_END) {
        Some(end) => {
            *in_paste = false;
            (end, end + PASTE_END.len())
        }
        None => {
            let keep = held_back(pending);
            (pending.len() - keep, pending.len() - keep)
        }
    };
    let text = String::from_utf8_lossy(&pending[..text_len]).into_owned();
    pending.drain(..used);
    (!text.is_empty() || !*in_paste).then(|| format!("paste:{text}"))
}

/// How many bytes at the end of `bytes` to keep for the next read: a
/// proper prefix of the paste's end marker, or an unfinished UTF-8
/// character.
fn held_back(bytes: &[u8]) -> usize {
    let marker = (1..PASTE_END.len())
        .rev()
        .find(|&n| bytes.ends_with(&PASTE_END[..n]))
        .unwrap_or(0);
    let utf8 = (1..=3.min(bytes.len()))
        .find(|&n| {
            let start = bytes.len() - n;
            utf8_len(bytes[start]).is_some_and(|len| len > n)
        })
        .unwrap_or(0);
    marker.max(utf8)
}

/// The longest a key wait goes without looking at the screen again (and
/// failing once it was released).
pub(super) const KEY_SLICE: Duration = Duration::from_millis(50);

/// How long a background answer arriving in pieces is held for its rest.
const ANSWER_WAIT: Duration = Duration::from_secs(1);

/// How long [`Keys::next`] waits for the rest of a key between reads.
const ESCAPE_WAIT: Duration = Duration::from_millis(30);

/// Bracketed paste's markers around pasted text (`take()` turns it on).
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// The most of a paste waited for at once; what came is handed out, and
/// the rest follows as more of it.
const MAX_PASTE: usize = 64 * 1024;

/// Whether `bytes` start a key that has not all arrived: `ESC` alone, `ESC
/// O`, `ESC [` without its final byte, a UTF-8 character short of its
/// continuation bytes, or a paste without its end marker.
pub(super) fn unfinished(bytes: &[u8]) -> bool {
    if bytes.starts_with(PASTE_START) {
        return find(bytes, PASTE_END).is_none() && bytes.len() < MAX_PASTE;
    }
    // The background query's answer (`take`), whole before it is dropped.
    if bytes.starts_with(b"\x1b]11;") {
        return !bytes.contains(&0x07) && find(bytes, b"\x1b\\").is_none() && bytes.len() < 64;
    }
    match bytes {
        [0x1b] | [0x1b, b'O'] | [0x1b, b'O', b'2'] => true,
        [0x1b, b']'] | [0x1b, b']', b'1'] | [0x1b, b']', b'1', b'1'] => true,
        // A legacy mouse report: three raw bytes after `ESC [ M`.
        [0x1b, b'[', b'M', rest @ ..] => rest.len() < 3,
        [0x1b, b'[', rest @ ..] => {
            !rest.iter().any(|b| (0x40..=0x7e).contains(b)) || PASTE_START.starts_with(bytes)
        }
        [lead, ..] => utf8_len(*lead).is_some_and(|n| bytes.len() < n),
        [] => false,
    }
}

/// A mouse report's button as a wheel key; None for any other button. Shift,
/// Alt and Ctrl held (bits 4, 8 and 16) do not change it.
fn wheel(button: u32) -> Option<&'static str> {
    match button & !0b11100 {
        64 => Some("wheel-up"),
        65 => Some("wheel-down"),
        _ => None,
    }
}

/// A mouse report as a key: the wheel's, or the left button's
/// `mouse-press`, `mouse-drag` (moved with it held) and `mouse-release`, and
/// a move with no button held (`mouse-move`), at
/// `:<column>:<row>` from 0 (the report's `x` and `y` count from 1). Shift,
/// Alt and Ctrl held do not change it. None for another button. `release`
/// is SGR's own (`m`); a legacy report's release is button 3, which names
/// no button.
fn mouse_key(button: u32, x: u32, y: u32, release: bool) -> Option<String> {
    if let Some(name) = wheel(button) {
        return (!release).then(|| name.to_string());
    }
    let b = button & !0b11100;
    let at = |kind: &str| {
        format!(
            "mouse-{kind}:{}:{}",
            x.saturating_sub(1),
            y.saturating_sub(1)
        )
    };
    match b {
        _ if release && b & 0b11 == 0 => Some(at("release")),
        3 => Some(at("release")),
        0 => Some(at("press")),
        32 => Some(at("drag")),
        35 => Some(at("move")),
        _ => None,
    }
}

/// An arrow's direction from its final byte, `A` to `D`.
fn arrow(d: u8) -> &'static str {
    match d {
        b'A' => "up",
        b'B' => "down",
        b'C' => "right",
        _ => "left",
    }
}

/// How many bytes the UTF-8 character `lead` starts takes; None for a byte
/// that starts none (a stray continuation byte).
fn utf8_len(lead: u8) -> Option<usize> {
    match lead {
        0x00..=0x7f => Some(1),
        0xc0..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf7 => Some(4),
        _ => None,
    }
}

pub(super) fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The first key in `bytes` and how many bytes it took.
fn parse_key(bytes: &[u8]) -> (Option<String>, usize) {
    let named = |s: &str, n: usize| (Some(s.to_string()), n);
    match bytes {
        [] => (None, 0),
        // A bracketed paste, whole: `paste:<text>`, so a newline in it is
        // never read as Enter. (`key()` hands out one not yet ended in
        // pieces, before this.)
        _ if bytes.starts_with(PASTE_START) => {
            let body = &bytes[PASTE_START.len()..];
            let (text, used) = match find(body, PASTE_END) {
                Some(end) => (&body[..end], PASTE_START.len() + end + PASTE_END.len()),
                None => (body, bytes.len()),
            };
            (
                Some(format!("paste:{}", String::from_utf8_lossy(text))),
                used,
            )
        }
        [0x1b, b'[', b'A', ..] | [0x1b, b'O', b'A', ..] => named("up", 3),
        [0x1b, b'[', b'B', ..] | [0x1b, b'O', b'B', ..] => named("down", 3),
        [0x1b, b'[', b'C', ..] | [0x1b, b'O', b'C', ..] => named("right", 3),
        [0x1b, b'[', b'D', ..] | [0x1b, b'O', b'D', ..] => named("left", 3),
        [0x1b, b'[', b'H', ..] | [0x1b, b'O', b'H', ..] => named("home", 3),
        [0x1b, b'[', b'Z', ..] => named("shift-tab", 3),
        [0x1b, b'[', b'F', ..] | [0x1b, b'O', b'F', ..] => named("end", 3),
        // xterm's modified arrows: `ESC [ 1 ; <mod> <dir>`, 2 for Shift.
        [
            0x1b,
            b'[',
            b'1',
            b';',
            b'2',
            d @ (b'A' | b'B' | b'C' | b'D'),
            ..,
        ] => (Some(format!("shift-{}", arrow(*d))), 6),
        // Alt (Option) with an arrow: xterm's `ESC [ 1 ; 3 <dir>`, and an ESC
        // before the arrow where Option is Meta (the macOS Terminal).
        [
            0x1b,
            b'[',
            b'1',
            b';',
            b'3',
            d @ (b'A' | b'B' | b'C' | b'D'),
            ..,
        ] => (Some(format!("alt-{}", arrow(*d))), 6),
        [0x1b, 0x1b, b'[' | b'O', d @ (b'A' | b'B' | b'C' | b'D'), ..] => {
            (Some(format!("alt-{}", arrow(*d))), 4)
        }
        // Shift+arrows as other terminals send them: older xterm's
        // `ESC [ 2 <dir>` and `ESC O 2 <dir>`, rxvt's `ESC [ a`…`d`.
        [0x1b, b'[' | b'O', b'2', d @ (b'A' | b'B' | b'C' | b'D'), ..] => {
            (Some(format!("shift-{}", arrow(*d))), 4)
        }
        [0x1b, b'[', d @ (b'a' | b'b' | b'c' | b'd'), ..] => {
            (Some(format!("shift-{}", arrow(d.to_ascii_uppercase()))), 3)
        }
        // SS3 (`ESC O x`), F1-F4 among them: three bytes, unnamed unless
        // above.
        [0x1b, b'O', _, ..] => (None, 3),
        [0x1b, b'[', b'1' | b'7', b'~', ..] => named("home", 4),
        [0x1b, b'[', b'4' | b'8', b'~', ..] => named("end", 4),
        [0x1b, b'[', b'5', b'~', ..] => named("pageup", 4),
        [0x1b, b'[', b'6', b'~', ..] => named("pagedown", 4),
        // A mouse report (`mouse(True)`): SGR's `ESC [ < b ; x ; y M` (or
        // `m`, a release), or the legacy `ESC [ M` and three bytes, each
        // offset by 32, from a terminal without SGR ([`mouse_key`]).
        [0x1b, b'[', b'<', rest @ ..] => {
            let Some(end) = rest.iter().position(|b| (0x40..=0x7e).contains(b)) else {
                return (None, bytes.len());
            };
            let fields: Vec<u32> = std::str::from_utf8(&rest[..end])
                .ok()
                .map(|p| p.split(';').filter_map(|f| f.parse().ok()).collect())
                .unwrap_or_default();
            let name = match (fields.as_slice(), rest[end]) {
                (&[b, x, y], f @ (b'M' | b'm')) => mouse_key(b, x, y, f == b'm'),
                _ => None,
            };
            (name, end + 4)
        }
        [0x1b, b'[', b'M', b, x, y, ..] => {
            let field = |v: u8| u32::from(v.wrapping_sub(32));
            (mouse_key(field(*b), field(*x), field(*y), false), 6)
        }
        [0x1b, b'[', rest @ ..] => {
            // Some other sequence: skip it whole, to its final byte.
            let end = rest.iter().position(|b| (0x40..=0x7e).contains(b));
            (None, end.map_or(bytes.len(), |i| i + 3))
        }
        // Alt (Option, where the terminal sends it as Meta) with a key: ESC
        // then the key, read together.
        [0x1b, c @ 0x20..=0x7e, ..] => (Some(format!("alt-{}", *c as char)), 2),
        [0x1b, 0x7f | 0x08, ..] => named("alt-backspace", 2),
        [0x1b, ..] => named("esc", 1),
        [b'\r', ..] | [b'\n', ..] => named("enter", 1),
        [b'\t', ..] => named("tab", 1),
        [0x7f, ..] | [0x08, ..] => named("backspace", 1),
        [c @ 1..=26, ..] => (Some(format!("ctrl-{}", (b'a' + c - 1) as char)), 1),
        [0, ..] => named("ctrl-@", 1),
        [0x1c, ..] => named("ctrl-\\", 1),
        [0x1d, ..] => named("ctrl-]", 1),
        [0x1e, ..] => named("ctrl-^", 1),
        [0x1f, ..] => named("ctrl-_", 1),
        // A continuation byte with no lead: skipped alone, never taking the
        // bytes after it.
        [lead, ..] if utf8_len(*lead).is_none() => (None, 1),
        _ => {
            let len = utf8_len(bytes[0]).unwrap_or(1).min(bytes.len());
            (
                Some(String::from_utf8_lossy(&bytes[..len]).into_owned()),
                len,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    use nix::sys::termios::{self, SetArg};

    use super::super::fd::tests::private_pty;

    /// Keys on a terminal gone remember it, so `key()` sleeps out its wait
    /// rather than spinning on a terminal that reads as ready.
    #[test]
    fn keys_on_a_hung_up_terminal_remember_it() {
        let pty = private_pty();
        drop(pty.master);
        let mut tty = std::fs::File::from(pty.slave);
        let mut keys = Keys::default();
        assert_eq!(
            keys.next(&mut tty, Duration::from_millis(50)).unwrap(),
            None
        );
        assert!(keys.gone);
    }

    /// A late background answer split across reads is held until whole,
    /// then dropped: none of it reads as keys; a key after it does.
    #[test]
    fn a_late_answer_in_pieces_is_no_keys() {
        let pty = private_pty();
        let mut master = File::from(pty.master);
        let mut tty = File::from(pty.slave);
        let mut raw = termios::tcgetattr(&tty).unwrap();
        termios::cfmakeraw(&mut raw);
        termios::tcsetattr(&tty, SetArg::TCSANOW, &raw).unwrap();
        let mut keys = Keys::default();
        master.write_all(b"\x1b]11;rgb:1e1").unwrap();
        let mut got = vec![];
        for _ in 0..3 {
            got.extend(keys.next(&mut tty, Duration::from_millis(30)).unwrap());
        }
        master.write_all(b"e/1e1e/1e1e\x1b\\\x1b[?62;22cq").unwrap();
        for _ in 0..6 {
            got.extend(keys.next(&mut tty, Duration::from_millis(30)).unwrap());
        }
        assert_eq!(got, vec!["q".to_string()]);
    }

    #[test]
    fn the_wheel_and_the_left_button_are_named_and_other_buttons_skipped() {
        let k = |b: &[u8]| parse_key(b);
        let some = |s: &str| Some(s.to_string());
        assert_eq!(k(b"\x1b[<64;10;5Mq"), (some("wheel-up"), 11));
        assert_eq!(k(b"\x1b[<65;1;1M"), (some("wheel-down"), 10));
        assert_eq!(k(b"\x1b[<69;1;1M"), (some("wheel-down"), 10), "with Shift");
        assert_eq!(k(b"\x1b[<0;3;4Mq"), (some("mouse-press:2:3"), 9), "a click");
        assert_eq!(k(b"\x1b[<32;5;4M"), (some("mouse-drag:4:3"), 10), "a drag");
        assert_eq!(
            k(b"\x1b[<0;6;4m"),
            (some("mouse-release:5:3"), 9),
            "a release"
        );
        assert_eq!(
            k(b"\x1b[<4;3;4M"),
            (some("mouse-press:2:3"), 9),
            "with Shift"
        );
        assert_eq!(k(b"\x1b[<2;3;4M"), (None, 9), "the right button");
        assert_eq!(k(b"\x1b[<2;3;4m"), (None, 9), "its release");
        assert_eq!(
            k(b"\x1b[<35;3;4M"),
            (some("mouse-move:2:3"), 10),
            "a move with no button"
        );
        assert_eq!(k(b"\x1b[<64;3;4m"), (None, 10));
        assert_eq!(k(b"\x1b[M`!!q"), (some("wheel-up"), 6), "legacy");
        assert_eq!(
            k(b"\x1b[M !!q"),
            (some("mouse-press:0:0"), 6),
            "a legacy click"
        );
        assert_eq!(
            k(b"\x1b[M#\"!"),
            (some("mouse-release:1:0"), 6),
            "a legacy release"
        );
        assert_eq!(
            k(b"\x1b[M@#\""),
            (some("mouse-drag:2:1"), 6),
            "a legacy drag"
        );
        assert!(unfinished(b"\x1b[<64;10"));
        assert!(unfinished(b"\x1b[M`!"));
        assert!(!unfinished(b"\x1b[M`!!"));
    }

    #[test]
    fn keys_are_named() {
        let k = |b: &[u8]| parse_key(b);
        assert_eq!(k(b"q"), (Some("q".into()), 1));
        assert_eq!(k(b"\x1b[A"), (Some("up".into()), 3));
        assert_eq!(k(b"\x1b[6~x"), (Some("pagedown".into()), 4));
        assert_eq!(k(b"\x03"), (Some("ctrl-c".into()), 1));
        assert_eq!(k(b"\r"), (Some("enter".into()), 1));
        assert_eq!(k(b"\x1bc"), (Some("alt-c".into()), 2));
        assert_eq!(
            k(b"\x1b[200~a\nq\x1b[201~j"),
            (Some("paste:a\nq".into()), 15)
        );
        // A stray continuation byte takes itself alone.
        assert_eq!(k(b"\x80jk"), (None, 1));
        assert_eq!(k("é".as_bytes()), (Some("é".into()), 2));
        assert_eq!(k(b"\x1b"), (Some("esc".into()), 1));
        assert_eq!(k(b"\t"), (Some("tab".into()), 1));
        assert_eq!(k(b"\x1b"), (Some("esc".into()), 1));
        assert_eq!(k(b"\x1b[1;2A"), (Some("shift-up".into()), 6));
        assert_eq!(k(b"\x1b[2A"), (Some("shift-up".into()), 4));
        assert_eq!(k(b"\x1bO2B"), (Some("shift-down".into()), 4));
        assert_eq!(k(b"\x1b[a"), (Some("shift-up".into()), 3));
        assert_eq!(k(b"\x1b[b"), (Some("shift-down".into()), 3));
        // `ESC [ 2 ~` is still Insert, unnamed, not Shift.
        assert_eq!(k(b"\x1b[2~").0, None);
        assert_eq!(k(b"\x1bOP"), (None, 3), "F1 is not Esc");
        assert_eq!(
            k(b"\x1c"),
            (Some("ctrl-\\".into()), 1),
            "Ctrl+\\ with interrupts off"
        );
        assert_eq!(k(b"\x1b[1~"), (Some("home".into()), 4), "tmux");
        assert_eq!(k(b"\x1b[4~"), (Some("end".into()), 4), "tmux");
        assert_eq!(k(b"\x1b[8~"), (Some("end".into()), 4), "rxvt");
        assert_eq!(k(b"\x1b[1;2Bx"), (Some("shift-down".into()), 6));
        assert_eq!(k(b"\x1b[1;3A"), (Some("alt-up".into()), 6), "xterm's Alt");
        assert_eq!(
            k(b"\x1b\x1b[Bx"),
            (Some("alt-down".into()), 4),
            "Option as Meta"
        );
        assert_eq!(k(b"\x1b[5~"), (Some("pageup".into()), 4), "Fn+up on a Mac");
        assert_eq!(k(b"\x1b[Z"), (Some("shift-tab".into()), 3), "Shift+Tab");
        assert_eq!(k("é".as_bytes()), (Some("é".into()), 2));
        assert_eq!(
            k(b"\x1b[1;5Cz"),
            (None, 6),
            "an unknown sequence is skipped whole"
        );
    }

    #[test]
    fn an_escape_sequence_without_its_end_is_unfinished() {
        assert!(unfinished(b"\x1b"));
        assert!(unfinished(b"\x1b["));
        assert!(unfinished(b"\x1b[1;5"));
        assert!(!unfinished(b"\x1b[A"));
        assert!(!unfinished(b"q"));
        assert!(unfinished(&"é".as_bytes()[..1]));
        assert!(unfinished(b"\x1b[200~half a paste"));
        assert!(unfinished(b"\x1b[20"));
        assert!(!unfinished(b"\x1b[200~done\x1b[201~"));
    }

    /// A paste whose end marker never comes ends once the terminal has been
    /// found quiet for `PASTE_IDLE`: what is typed after is keys again. What
    /// waited unread through a gap without polls is still the paste.
    #[test]
    fn a_paste_without_its_end_ends_when_input_stops() {
        use std::io::Write;
        let (read, write) = std::io::pipe().unwrap();
        let mut tty = File::from(std::os::fd::OwnedFd::from(read));
        let mut write = File::from(std::os::fd::OwnedFd::from(write));
        let mut keys = Keys::default();
        write.write_all(b"\x1b[200~lost\x1b[20").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("paste:lost".into()));
        // Its end marker arrives split across reads, the start held back.
        std::thread::sleep(Duration::from_millis(50));
        write.write_all(b"1~").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("paste:".into()));
        assert!(!keys.in_paste);

        write.write_all(b"\x1b[200~again\x1b[20").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("paste:again".into()));
        assert_eq!(keys.take(&mut tty).unwrap(), None, "found quiet");
        keys.quiet_since = Some(std::time::Instant::now() - PASTE_IDLE);
        assert_eq!(
            keys.take(&mut tty).unwrap(),
            None,
            "quiet long enough: ended"
        );
        assert!(!keys.in_paste);
        write.write_all(b"q").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("q".into()));
    }

    /// Input waiting when a paste would have gone stale keeps it going.
    #[test]
    fn waiting_input_is_not_quiet() {
        use std::io::Write;
        let (read, write) = std::io::pipe().unwrap();
        let mut tty = File::from(std::os::fd::OwnedFd::from(read));
        let mut write = File::from(std::os::fd::OwnedFd::from(write));
        let mut keys = Keys::default();
        write.write_all(b"\x1b[200~abc").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("paste:abc".into()));
        keys.quiet_since = Some(std::time::Instant::now() - PASTE_IDLE);
        write.write_all(b"q").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("paste:q".into()));
    }

    /// An Esc alone is Esc once the wait for the rest of a sequence is out;
    /// an Esc and a key read together are Alt with it.
    #[test]
    fn keys_read_esc_alone_and_with_a_key() {
        use std::io::Write;
        let (read, write) = std::io::pipe().unwrap();
        let mut tty = File::from(std::os::fd::OwnedFd::from(read));
        let mut write = File::from(std::os::fd::OwnedFd::from(write));
        let mut keys = Keys::default();
        write.write_all(b"\x1b").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("esc".into()));
        write.write_all(b"\x1bq").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("alt-q".into()));
        write.write_all(b"\x1b\x7f").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("alt-backspace".into()));
        assert_eq!(keys.take(&mut tty).unwrap(), None);
    }

    /// Reading from a pipe as the terminal: a paste with no end yet comes as
    /// a piece, then nothing (not a spin) while its rest has not come, then
    /// the rest, and keys after it.
    #[test]
    fn keys_read_a_paste_across_gaps() {
        use std::io::Write;
        let (read, write) = std::io::pipe().unwrap();
        let mut tty = File::from(std::os::fd::OwnedFd::from(read));
        let mut write = File::from(std::os::fd::OwnedFd::from(write));
        let mut keys = Keys::default();
        write.write_all(b"\x1b[200~abc\x1b[20").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("paste:abc".into()));
        let started = std::time::Instant::now();
        assert_eq!(keys.take(&mut tty).unwrap(), None, "the rest has not come");
        assert!(started.elapsed() < Duration::from_secs(1), "no spin");
        write.write_all(b"1~q").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("paste:".into()));
        assert_eq!(keys.take(&mut tty).unwrap(), Some("q".into()));
        assert_eq!(keys.take(&mut tty).unwrap(), None);
    }

    /// A paste longer than a read comes as pieces, keys never, its end
    /// marker or a UTF-8 character split across reads held for the next.
    #[test]
    fn a_long_paste_comes_in_pieces() {
        let mut pending = b"abc\x1b[20".to_vec();
        let mut in_paste = true;
        assert_eq!(
            paste_chunk(&mut pending, &mut in_paste),
            Some("paste:abc".into())
        );
        assert!(in_paste);
        assert_eq!(pending, b"\x1b[20");
        pending.extend_from_slice(b"1~qj");
        assert_eq!(
            paste_chunk(&mut pending, &mut in_paste),
            Some("paste:".into())
        );
        assert!(!in_paste, "the end marker ends it");
        assert_eq!(pending, b"qj", "what follows is keys again");
        let mut pending = "xé".as_bytes()[..2].to_vec();
        let mut in_paste = true;
        assert_eq!(
            paste_chunk(&mut pending, &mut in_paste),
            Some("paste:x".into())
        );
        assert_eq!(pending.len(), 1, "half a character waits");
    }
}
