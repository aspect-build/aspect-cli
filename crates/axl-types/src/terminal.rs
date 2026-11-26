//! One write lock per target the CLI's stdout and stderr write to, taken by
//! every path that writes there: the CLI's own lines, the AXL streams, the
//! build console, a child's forwarded output, and a screen's dump or replay.
//! A unit written under it is never spliced with another's: one write of
//! the CLI's, what a forwarder read, a whole dump or replay, a run of whole
//! lines a stopped capture still passes on, and the move of fds 1 and 2 in
//! or out of a screen's capture. Stdout and stderr share one lock when they
//! are one file or both terminals, so a stalled target holds up only those
//! writing to it.
//!
//! Lock order: this lock first, then std's own (`Stdout`/`Stderr` take it
//! inside a write). A dump or replay takes this one alone, for at most a
//! bound, so a writer stalled inside it never holds them up for good.
//!
//! Each target also keeps whether its last write ended mid-line, so a dump
//! can wait a moment for the line's end, and else start a line of its own.
//!
//! The CLI's own output (its lines, the AXL streams, the build console) is
//! one ordered channel: each write, under its target, is first offered to
//! the route a screen installs ([`route_to`]), and copied to the task's
//! output files under the same turn ([`write_std`]; the console holds the
//! turn across its copy and its [`Routed`] write). While a screen holds the
//! terminal the route takes it (into the screen's console and its replay),
//! so it never goes through fds 1 and 2 while a capture has them; else it
//! is written there. The capture of fds 1 and 2 is left for stray writers
//! and children given the CLI's own stdout.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

static LOCKS: [Mutex<()>; 2] = [Mutex::new(()), Mutex::new(())];
static MID_LINE: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];
static SHARED: OnceLock<bool> = OnceLock::new();
static ROUTE: OnceLock<fn(i32, &[u8]) -> bool> = OnceLock::new();

/// Install the route the CLI's own writes are offered first: whether it took
/// `bytes` for fd 1 or 2 (a screen holding the terminal). Once, at start.
pub fn route_to(route: fn(i32, &[u8]) -> bool) {
    let _ = ROUTE.set(route);
}

/// Whether the route took `bytes` for `fd`; call it under `fd`'s [`turn`].
pub fn routed(fd: i32, bytes: &[u8]) -> bool {
    !bytes.is_empty() && ROUTE.get().is_some_and(|route| route(fd, bytes))
}

/// A held target, for one unit of writing.
pub type Turn = MutexGuard<'static, ()>;

/// Settle whether stdout and stderr are one target, from fds 1 and 2 as they
/// are now: before anything redirects them (a screen's capture), else the
/// first lock settles it.
pub fn settle() {
    SHARED.get_or_init(|| one_target(1, 2));
}

/// Whether descriptors `a` and `b` are one file (`2>&1`) or both terminals,
/// taken as one (`/dev/tty` and the pty it is are two inodes).
pub fn one_target(a: i32, b: i32) -> bool {
    #[cfg(unix)]
    // SAFETY: isatty, and fstat into zeroed stat buffers, on open descriptors.
    unsafe {
        if libc::isatty(a) == 1 && libc::isatty(b) == 1 {
            return true;
        }
        let (mut x, mut y) = (
            std::mem::zeroed::<libc::stat>(),
            std::mem::zeroed::<libc::stat>(),
        );
        libc::fstat(a, &mut x) == 0
            && libc::fstat(b, &mut y) == 0
            && x.st_dev == y.st_dev
            && x.st_ino == y.st_ino
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        false
    }
}

/// The target for `fd` (1 stdout, 2 stderr); None for any other.
fn target(fd: i32) -> Option<usize> {
    match fd {
        1 => Some(0),
        2 => Some(if *SHARED.get_or_init(|| one_target(1, 2)) {
            0
        } else {
            1
        }),
        _ => None,
    }
}

/// `fd`'s target held, waiting as long as it takes; None for a descriptor
/// other than 1 and 2.
pub fn turn(fd: i32) -> Option<Turn> {
    let lock = &LOCKS[target(fd)?];
    Some(lock.lock().unwrap_or_else(|p| p.into_inner()))
}

/// [`turn`], given up after `bound`: for a dump or replay, which goes ahead
/// without it rather than wait on a writer stalled on its target.
pub fn turn_within(fd: i32, bound: Duration) -> Option<Turn> {
    let lock = &LOCKS[target(fd)?];
    let deadline = Instant::now() + bound;
    loop {
        match lock.try_lock() {
            Ok(g) => return Some(g),
            Err(std::sync::TryLockError::Poisoned(p)) => return Some(p.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(std::sync::TryLockError::WouldBlock) => return None,
        }
    }
}

/// [`turn_within`], at a line's end where the target gets there within
/// `settle`: a writer left mid-line (a child's line cut where its read
/// ended) is given that long to end it before the turn is taken anyway.
pub fn turn_at_line_end(fd: i32, bound: Duration, settle: Duration) -> Option<Turn> {
    let until = Instant::now() + settle;
    loop {
        let turn = turn_within(fd, bound)?;
        if !mid_line(fd) || Instant::now() >= until {
            return Some(turn);
        }
        drop(turn);
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Every target held, stdout's first, each given up after `bound`: for
/// moving fds 1 and 2 (a screen's capture taken or given back), so no write
/// lands half on either side of the move.
pub fn every_turn_within(bound: Duration) -> Vec<Turn> {
    let mut held: Vec<Turn> = turn_within(1, bound).into_iter().collect();
    if target(2) != target(1) {
        held.extend(turn_within(2, bound));
    }
    held
}

/// Whether `turns` (from [`every_turn_within`]) holds every target: none
/// was given up on.
pub fn holds_every(turns: &[Turn]) -> bool {
    turns.len() == if target(2) == target(1) { 1 } else { 2 }
}

/// Note `bytes` written to `fd`'s target: whether it is left mid-line.
pub fn wrote(fd: i32, bytes: &[u8]) {
    if let (Some(t), Some(&last)) = (target(fd), bytes.last()) {
        MID_LINE[t].store(last != b'\n', Ordering::SeqCst);
    }
}

/// Whether `fd`'s target was left mid-line by its last write.
pub fn mid_line(fd: i32) -> bool {
    target(fd).is_some_and(|t| MID_LINE[t].load(Ordering::SeqCst))
}

/// A writer to fd `fd` (1 or 2) that offers each write to the route first
/// ([`routed`]) and notes where it left the target ([`wrote`]), for a caller
/// that holds the target's [`turn`] itself (across a copy and the write).
pub struct Routed<W> {
    inner: W,
    fd: i32,
}

impl<W> Routed<W> {
    pub fn new(inner: W, fd: i32) -> Self {
        Routed { inner, fd }
    }
}

impl<W: std::io::Write> std::io::Write for Routed<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if routed(self.fd, buf) {
            return Ok(buf.len());
        }
        let n = self.inner.write(buf)?;
        wrote(self.fd, &buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// The target of stdout held, as a guard of any type: for a writer that
/// holds it across more than its write ([`Routed`]).
pub fn hold_stdout() -> Box<dyn std::any::Any> {
    Box::new(turn(1))
}

/// [`hold_stdout`] for stderr.
pub fn hold_stderr() -> Box<dyn std::any::Any> {
    Box::new(turn(2))
}

/// Write all of `bytes` to std's stdout (1) or stderr (2) as one unit, or
/// to the route that takes it ([`routed`]), and `copy` them (to the task's
/// output files) under the same turn: no screen switch comes between where
/// they went and their copy.
pub fn write_std(fd: i32, bytes: &[u8], copy: fn(&[u8])) -> std::io::Result<()> {
    use std::io::Write;
    let _turn = turn(fd);
    if routed(fd, bytes) {
        copy(bytes);
        return Ok(());
    }
    let r = if fd == 1 {
        std::io::stdout().write_all(bytes)
    } else {
        std::io::stderr().write_all(bytes)
    };
    wrote(fd, bytes);
    copy(bytes);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_notes_whether_it_left_its_target_mid_line() {
        wrote(1, b"partial");
        assert!(mid_line(1));
        wrote(1, b"rest\n");
        assert!(!mid_line(1));
        wrote(1, b"");
        assert!(!mid_line(1), "an empty write changes nothing");
    }

    #[test]
    fn a_turn_at_a_line_end_waits_out_a_line_left_mid_way_for_a_while() {
        // Stderr's target, which these other tests leave alone where the two
        // are one target only in the sense of locks; mid-line is per target.
        wrote(2, b"cut");
        let start = Instant::now();
        let turn = turn_at_line_end(2, Duration::from_secs(1), Duration::from_millis(30));
        assert!(turn.is_some() && start.elapsed() >= Duration::from_millis(30));
        drop(turn);
        wrote(2, b"\n");
        let start = Instant::now();
        assert!(turn_at_line_end(2, Duration::from_secs(1), Duration::from_secs(5)).is_some());
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "at a line's end at once"
        );
    }

    #[test]
    fn only_stdout_and_stderr_have_targets() {
        assert!(turn(3).is_none());
        assert!(turn_within(0, Duration::ZERO).is_none());
    }

    #[test]
    fn a_held_target_is_given_up_on_after_the_bound() {
        let held = turn(1).unwrap();
        let start = Instant::now();
        assert!(turn_within(1, Duration::from_millis(50)).is_none());
        assert!(start.elapsed() >= Duration::from_millis(50));
        drop(held);
        assert!(turn_within(1, Duration::from_millis(50)).is_some());
    }
}
