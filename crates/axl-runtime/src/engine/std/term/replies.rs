//! The terminal's answers to the screen's queries: its background (OSC 11,
//! with DA1 to end the wait), and its alternate scroll mode (DECRQM), read
//! as they come or late among the keys, and dropped before the terminal
//! goes to a child.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nix::sys::termios;

use super::fd::tty_readable;
use super::keys::{Keys, find};

/// How long the screen waits for the terminal to say its background.
pub(super) const BACKGROUND_WAIT: Duration = Duration::from_millis(150);

/// The background query: OSC 11 asking the background colour, then a DA1
/// request (`ESC [ c`), which every terminal answers.
const BACKGROUND_QUERY: &[u8] = b"\x1b]11;?\x1b\\\x1b[c";

/// An OSC 11 answer's start, its colour and BEL or ST after it.
pub(super) const OSC11_REPLY: &[u8] = b"\x1b]11;";

/// A DA1 answer's start, its parameters and `c` after it.
const DA1_REPLY: &[u8] = b"\x1b[?";

/// BEL, which ends an OSC answer as ST does.
pub(super) const BEL: u8 = 0x07;

/// String Terminator (`ESC \`), which ends an OSC answer.
pub(super) const ST: &[u8] = b"\x1b\\";

/// DECRQM asking for alternate scroll's state (`?1007`).
const WHEEL_QUERY: &[u8] = b"\x1b[?1007$p";

/// The DECRQM answer's start for alternate scroll, its state after it.
const WHEEL_REPLY: &[u8] = b"\x1b[?1007;";

/// What ends a DECRQM answer.
const DECRQM_END: &[u8] = b"$y";

/// Alternate scroll on: the wheel arrives as up and down keys on the
/// alternate screen.
pub(super) const WHEEL_KEYS_ON: &[u8] = b"\x1b[?1007h";

/// Alternate scroll off: the wheel scrolls as the terminal has it.
pub(super) const WHEEL_KEYS_OFF: &[u8] = b"\x1b[?1007l";

/// The terminal's background as it answers an OSC 11 query (`Some(true)`
/// when dark), with what else arrived meanwhile, for the keys. A DA1
/// request follows the query: every terminal answers it, so one that
/// ignores OSC 11 ends the wait at once rather than at `wait`.
pub(super) fn query_background(tty: &mut File, wait: Duration) -> (Option<bool>, Vec<u8>) {
    if tty
        .write_all(BACKGROUND_QUERY)
        .and_then(|()| tty.flush())
        .is_err()
    {
        return (None, vec![]);
    }
    let mut got = vec![];
    ANSWER_DUE.store(true, Ordering::SeqCst);
    if read_until(tty, &mut got, Instant::now() + wait, |got| {
        da1_reply(got).is_some()
    }) {
        ANSWER_DUE.store(false, Ordering::SeqCst);
    }
    let mut rest = got.clone();
    let dark = osc11_reply(&got).and_then(|(range, dark)| {
        rest.drain(range);
        dark
    });
    if let Some(range) = da1_reply(&rest) {
        rest.drain(range);
    }
    (dark, rest)
}

/// Whether the background query's DA1 answer is still to come: asked, and
/// not answered within its wait (a slow link). The keys clear it when it
/// arrives late; a hand-off to a child reads it out first
/// ([`drop_answers_due`]).
pub(super) static ANSWER_DUE: AtomicBool = AtomicBool::new(false);

/// Before the terminal goes to a child or a prompt: read and drop the
/// answers to the screen's own queries still on their way (the background
/// query's DA1, the wheel query's), for at most [`BACKGROUND_WAIT`], so the
/// child never gets them as typed input; then drop what else is unread,
/// keys typed for the screen.
pub(super) fn drop_answers_due(tty: &mut File) {
    let mut got = vec![];
    let due = |got: &[u8]| {
        (ANSWER_DUE.load(Ordering::SeqCst) && da1_reply(got).is_none())
            || (WHEEL.load(Ordering::SeqCst) == WHEEL_ASKED && find(got, DECRQM_END).is_none())
    };
    read_until(tty, &mut got, Instant::now() + BACKGROUND_WAIT, |got| {
        !due(got)
    });
    if da1_reply(&got).is_some() {
        ANSWER_DUE.store(false, Ordering::SeqCst);
    }
    // A wheel answer read here still says what release puts back.
    if let Some(at) = find(&got, WHEEL_REPLY) {
        let _ = late_wheel_answer(&got[at..]);
    }
    let _ = termios::tcflush(&*tty, termios::FlushArg::TCIFLUSH);
}

/// Where a DA1 answer (`ESC [ ? ... c`) is in `bytes`.
pub(super) fn da1_reply(bytes: &[u8]) -> Option<std::ops::Range<usize>> {
    let start = find(bytes, DA1_REPLY)?;
    let params = start + DA1_REPLY.len();
    let end = bytes[params..]
        .iter()
        .position(|&b| !(b.is_ascii_digit() || b == b';'))?;
    (bytes[params + end] == b'c').then(|| start..params + end + 1)
}

/// Where an OSC 11 answer (`ESC ] 11 ; rgb:R/G/B`, ended by BEL or ST) is
/// in `bytes`, and whether its colour is dark (None for one unread).
fn osc11_reply(bytes: &[u8]) -> Option<(std::ops::Range<usize>, Option<bool>)> {
    let start = find(bytes, OSC11_REPLY)?;
    let body = &bytes[start + OSC11_REPLY.len()..];
    let (len, end) = body.iter().enumerate().find_map(|(i, &b)| match b {
        BEL => Some((i, 1)),
        _ if body[i..].starts_with(ST) => Some((i, ST.len())),
        _ => None,
    })?;
    let text = std::str::from_utf8(&body[..len]).ok();
    let dark = text.and_then(|t| t.strip_prefix("rgb:")).and_then(|rgb| {
        let channel = |h: &str| {
            let v = u32::from_str_radix(h, 16).ok()?;
            (!h.is_empty() && h.len() <= 4).then(|| v as f64 / ((1u32 << (4 * h.len())) - 1) as f64)
        };
        let mut parts = rgb.split('/').map(channel);
        let (r, g, b) = (parts.next()??, parts.next()??, parts.next()??);
        Some(0.2126 * r + 0.7152 * g + 0.0722 * b < 0.5)
    });
    Some((start..start + OSC11_REPLY.len() + len + end, dark))
}

/// What a screen's `wheel_keys` did to the terminal's alternate scroll
/// (`?1007`): nothing, or changed it from on or from off (or from a state
/// the terminal did not say). One atomic, for the crash path's restore.
pub(super) static WHEEL: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(WHEEL_UNTOUCHED);
pub(super) const WHEEL_UNTOUCHED: u8 = 0;
pub(super) const WHEEL_WAS_ON: u8 = 1;
pub(super) const WHEEL_WAS_OFF: u8 = 2;
/// Asked, and not answered in time: taken as off, unless the answer comes
/// later among the keys ([`late_wheel_answer`]).
pub(super) const WHEEL_ASKED: u8 = 3;

/// What puts the terminal's alternate scroll back as it was before
/// `wheel_keys` changed it: nothing when untouched or it was on (a VTE
/// terminal's default), off otherwise, as it is in most terminals by
/// default and in one that did not answer the query.
pub(super) fn wheel_restore() -> &'static [u8] {
    match WHEEL.load(Ordering::SeqCst) {
        WHEEL_WAS_ON => WHEEL_KEYS_ON,
        WHEEL_WAS_OFF | WHEEL_ASKED => WHEEL_KEYS_OFF,
        _ => b"",
    }
}

/// The terminal's alternate scroll mode now, asked with DECRQM: Some(true)
/// or Some(false) as it answers within [`WHEEL_QUERY_WAIT`], None when it
/// does not. What arrives meanwhile is kept for `key()`, the answer among
/// it (an unknown sequence there, skipped).
pub(super) fn alternate_scroll(tty: &mut File, keys: &mut Keys) -> Option<bool> {
    tty.write_all(WHEEL_QUERY).ok()?;
    tty.flush().ok()?;
    let from = keys.pending.len();
    // Some(the state) once an answer has come, None until then.
    let answer = |pending: &[u8]| {
        let at = find(pending, WHEEL_REPLY)?;
        let rest = &pending[at + WHEEL_REPLY.len()..];
        let end = find(rest, DECRQM_END)?;
        Some(match &rest[..end] {
            b"1" | b"3" => Some(true),
            b"2" | b"4" => Some(false),
            _ => None,
        })
    };
    let deadline = Instant::now() + WHEEL_QUERY_WAIT;
    read_until(tty, &mut keys.pending, deadline, |pending| {
        answer(&pending[from..]).is_some()
    });
    answer(&keys.pending[from..]).flatten()
}

/// Read what the terminal sends into `got` until `done(got)` holds, or
/// `deadline` passes, or a read ends (the terminal closed, an error): for
/// the answers to the screen's queries. Whether `done` held.
fn read_until(
    tty: &mut File,
    got: &mut Vec<u8>,
    deadline: Instant,
    done: impl Fn(&[u8]) -> bool,
) -> bool {
    let mut buf = [0u8; 256];
    loop {
        if done(got) {
            return true;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || !tty_readable(tty.as_raw_fd(), left) {
            return false;
        }
        match tty.read(&mut buf) {
            Ok(0) | Err(_) => return false,
            Ok(n) => got.extend_from_slice(&buf[..n]),
        }
    }
}

/// How long `wheel_keys` waits for the terminal to say its alternate scroll.
const WHEEL_QUERY_WAIT: Duration = Duration::from_millis(150);

/// An answer to the background query (`take`'s OSC 11, then DA1) at the
/// start of `pending`, come after its wait (a slow link): never keys. Its
/// length, or None when `pending` does not start with one.
pub(super) fn late_background_answer(pending: &[u8]) -> Option<usize> {
    osc11_reply(pending)
        .map(|(range, _)| range)
        .or_else(|| da1_reply(pending))
        .filter(|range| range.start == 0)
        .map(|range| range.end)
}

/// The answer to the alternate scroll query (`ESC[?1007;<n>$y`) at the
/// start of `pending`, come after its wait (a slow link): what release puts
/// back is then what the terminal had (1 or 3 set), not off. Its length,
/// or None when `pending` does not start with one.
pub(super) fn late_wheel_answer(pending: &[u8]) -> Option<usize> {
    let rest = pending.strip_prefix(WHEEL_REPLY)?;
    let end = find(rest, DECRQM_END)?;
    let n = std::str::from_utf8(&rest[..end]).ok()?.parse::<u8>().ok()?;
    let _ = WHEEL.compare_exchange(
        WHEEL_ASKED,
        if matches!(n, 1 | 3) {
            WHEEL_WAS_ON
        } else {
            WHEEL_WAS_OFF
        },
        Ordering::SeqCst,
        Ordering::SeqCst,
    );
    Some(WHEEL_REPLY.len() + end + DECRQM_END.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::sys::termios::SetArg;

    use super::super::fd::tests::private_pty;
    use super::super::keys::unfinished;

    #[test]
    fn the_background_is_read_from_its_answer() {
        let dark = b"\x1b]11;rgb:1e1e/1e1e/1e1e\x1b\\\x1b[?62;22cq";
        let (range, is_dark) = osc11_reply(dark).unwrap();
        assert_eq!((range, is_dark), (0..25, Some(true)));
        assert_eq!(da1_reply(dark), Some(25..34));
        let light = b"\x1b]11;rgb:ffff/ffff/ffff\x07";
        assert_eq!(osc11_reply(light).unwrap().1, Some(false));
        assert_eq!(
            osc11_reply(b"\x1b]11;rgb:f/f/f\x07").unwrap().1,
            Some(false)
        );
        assert_eq!(osc11_reply(b"\x1b]11;bogus\x07").unwrap().1, None);
        assert!(osc11_reply(b"\x1b]11;rgb:00/00").is_none(), "unended");
        assert!(da1_reply(b"\x1b[?62;22").is_none());
    }

    /// A terminal that answers both: the background, and a key typed
    /// meanwhile kept for the keys. One that answers DA1 alone: unknown, at
    /// once.
    #[test]
    fn a_background_query_keeps_what_was_typed() {
        let pty = private_pty();
        let mut master = File::from(pty.master);
        let mut tty = File::from(pty.slave);
        let answer = std::thread::spawn(move || {
            let mut asked = [0u8; 64];
            let _ = std::io::Read::read(&mut master, &mut asked);
            master
                .write_all(b"x\x1b]11;rgb:0000/0000/0000\x07\x1b[?1;2c")
                .unwrap();
            master
        });
        let mut raw = termios::tcgetattr(&tty).unwrap();
        termios::cfmakeraw(&mut raw);
        termios::tcsetattr(&tty, SetArg::TCSANOW, &raw).unwrap();
        let (dark, typed) = query_background(&mut tty, Duration::from_secs(5));
        assert_eq!((dark, typed), (Some(true), b"x".to_vec()));
        let mut master = answer.join().unwrap();
        let answer = std::thread::spawn(move || {
            let mut asked = [0u8; 64];
            let _ = std::io::Read::read(&mut master, &mut asked);
            master.write_all(b"\x1b[?1;2c").unwrap();
            master
        });
        let started = std::time::Instant::now();
        assert_eq!(
            query_background(&mut tty, Duration::from_secs(5)),
            (None, vec![])
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "ended by the DA1 answer"
        );
        drop(answer.join());
    }

    /// The alternate scroll query's answer, after a key typed meanwhile: the
    /// state it says, all of what came kept for the keys. With no answer,
    /// unknown once the wait is out.
    #[test]
    fn the_alternate_scroll_is_read_from_its_answer() {
        let pty = private_pty();
        let mut master = File::from(pty.master);
        let mut tty = File::from(pty.slave);
        let mut raw = termios::tcgetattr(&tty).unwrap();
        termios::cfmakeraw(&mut raw);
        termios::tcsetattr(&tty, SetArg::TCSANOW, &raw).unwrap();
        let answer = std::thread::spawn(move || {
            let mut asked = [0u8; 64];
            let _ = std::io::Read::read(&mut master, &mut asked);
            master.write_all(b"q\x1b[?1007;2$y").unwrap();
            master
        });
        let mut keys = Keys::default();
        assert_eq!(alternate_scroll(&mut tty, &mut keys), Some(false));
        assert_eq!(keys.pending, b"q\x1b[?1007;2$y");
        let _master = answer.join().unwrap();
        let mut keys = Keys::default();
        assert_eq!(alternate_scroll(&mut tty, &mut keys), None, "no answer");
        assert!(keys.pending.is_empty());
    }

    /// The alternate scroll answer, come late among the keys, is taken for
    /// what release puts back and not read as a key.
    #[test]
    fn a_late_wheel_answer_settles_what_release_puts_back() {
        WHEEL.store(WHEEL_ASKED, Ordering::SeqCst);
        assert_eq!(late_wheel_answer(b"\x1b[?1007;1$yq"), Some(11));
        assert_eq!(wheel_restore(), b"\x1b[?1007h");
        assert_eq!(late_wheel_answer(b"q"), None);
        assert_eq!(
            late_background_answer(b"\x1b]11;rgb:1e1e/1e1e/1e1e\x07q"),
            Some(24)
        );
        assert_eq!(late_background_answer(b"\x1b[?62;22cq"), Some(9));
        assert_eq!(
            late_background_answer(b"q\x1b[?62;22c"),
            None,
            "only at the start"
        );
        assert!(
            unfinished(b"\x1b]11;rgb:1e1"),
            "an answer split across reads waits"
        );
        assert!(unfinished(b"\x1b]1"));
        assert!(!unfinished(b"\x1b]11;rgb:1e1e/1e1e/1e1e\x07"));
        WHEEL.store(WHEEL_UNTOUCHED, Ordering::SeqCst);
    }
}
