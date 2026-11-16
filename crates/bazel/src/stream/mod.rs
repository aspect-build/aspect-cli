//! Reading a stream that bazel's processes are still writing, and knowing
//! when it is over.
//!
//! * [`file::File`]: a regular file that fills while the build runs.
//! * [`pid_pair::PidPair`]: one bazel command's client and server, and the
//!   two answers a [`Gate`] needs from them.
//!
//! A [`file::File`] is a plain [`std::io::Read`]: bytes, or `Ok(0)` once the
//! stream is over, with the reason in `done()`. It also fails once with
//! [`file::StartedOver`] each time its reading starts over. The decision of
//! when to stop is [`Rules`].
//!
//! # When a stream is over
//!
//! Evaluated whenever a read finds nothing to return. `eof` is a read of zero
//! bytes: the file's current end. It does not mean *finished* by itself:
//! before the writer creates the file, and while it pauses between writes, a
//! read also returns nothing.
//!
//! | rule | condition | result |
//! |---|---|---|
//! | R1 | read error | [`Done::Broken`] |
//! | R2 | EOF, bytes seen, [`Expect::Unknown`], `held` says nobody holds it, and `ended` | [`Done::Drained`] |
//! | R2′ | EOF, bytes seen, [`Expect::NothingOnceEnded`], and `ended` | [`Done::Drained`] |
//! | R3 | `ended`, no bytes ever, `held` says nobody holds it | [`Done::Ended`] now |
//! | R4 | `ended`, and quiet for [`Gate::quiet_empty`] (no bytes ever) or [`Gate::quiet`] | [`Done::Ended`] |
//! | R5 | anything else | wait |
//!
//! R1 belongs to the file; [`Rules`] decides R2 to R4. Any bytes reset the
//! quiet timer, so a writer still flushing after its owner has ended is never
//! cut off.
//!
//! R2 needs `ended` as well as `held`. A file's EOF says nothing about
//! writers, so `held` would be the whole answer, and it comes from a server pid
//! read before the build that a restarted server makes stale. A stale pid holds
//! nothing, so trusted alone it would end a file at the first pause between
//! writes. Requiring `ended` too means a wrong `held` can only end the stream
//! early once the client is gone.
//!
//! R2′ is for when the protocol on top says what is written is whole, and
//! only a successor the owner starts could add to it. The owner having ended
//! settles that, and `held` is not asked: a server that still holds the file
//! open has nothing more to write to it.

use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

pub mod file;
pub mod pid_pair;
#[cfg(test)]
pub(crate) mod testing;

/// How long one wait for bytes lasts before the rules are evaluated again.
pub(crate) const TICK: Duration = Duration::from_millis(10);

/// [`Gate::quiet`] by default: how long a stream must stay quiet after its
/// owner ended before it is called over. The owner exiting is not the end of
/// the writing, since a server can still be flushing, so this is the window it
/// gets. It also bounds a writer that holds the stream open and never writes
/// again.
pub const QUIET: Duration = Duration::from_secs(5);

/// [`Gate::quiet_empty`] by default: the same window when nothing was ever
/// read. Shorter, because a writer that has not written a byte by the time its
/// owner is gone is not one that will start.
pub const QUIET_EMPTY: Duration = Duration::from_millis(250);

/// What a stream asks of whoever started the writers.
///
/// Both are called from the reading thread and only when a rule needs the
/// answer, or (`held`) while a [`file::File`] waits to move on to the next
/// file, so a costly `held` is not paid per byte.
pub struct Gate {
    /// The thing that owns this stream is over, so only what is already
    /// written, or being flushed, remains.
    pub ended: Box<dyn FnMut() -> bool + Send>,
    /// Whether any writer still holds open the path it is given: the one the
    /// stream reads from now. `None` when there is no way to know, which only
    /// removes the fast paths (R2, R3), never makes the stream wrong.
    pub held: Option<Box<dyn FnMut(&Path) -> bool + Send>>,
    /// See [`QUIET`].
    pub quiet: Duration,
    /// See [`QUIET_EMPTY`].
    pub quiet_empty: Duration,
}

impl Gate {
    pub fn new(ended: impl FnMut() -> bool + Send + 'static) -> Self {
        Self {
            ended: Box::new(ended),
            held: None,
            quiet: QUIET,
            quiet_empty: QUIET_EMPTY,
        }
    }

    pub fn with_held(mut self, held: impl FnMut(&Path) -> bool + Send + 'static) -> Self {
        self.held = Some(Box::new(held));
        self
    }
}

/// Why a followed stream stopped producing bytes.
#[derive(Debug)]
pub enum Done {
    /// Nothing more is coming: the writers closed (R2), or the stream was
    /// whole once the owner ended (R2′).
    Drained { bytes: u64 },
    /// The owner ended and the stream then stayed quiet for `quiet`.
    Ended { bytes: u64, quiet: Duration },
    /// The stream failed: a read error.
    Broken { bytes: u64, error: io::Error },
}

impl Done {
    /// How many bytes the stream delivered before it stopped.
    pub fn bytes(&self) -> u64 {
        match self {
            Done::Drained { bytes } | Done::Ended { bytes, .. } | Done::Broken { bytes, .. } => {
                *bytes
            }
        }
    }
}

impl std::fmt::Display for Done {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Done::Drained { bytes } => write!(f, "the stream ended after {bytes} bytes"),
            Done::Ended { bytes: 0, .. } => write!(f, "the writer ended without writing anything"),
            Done::Ended { bytes, quiet } => write!(
                f,
                "the writer ended and nothing more arrived for {}ms after {bytes} bytes",
                quiet.as_millis()
            ),
            Done::Broken { bytes, error } => {
                write!(f, "the stream failed after {bytes} bytes: {error}")
            }
        }
    }
}

/// What the protocol on top of a stream says about its writer, which neither
/// the file nor the [`Gate`] can. See R2 and R2′ in the [module docs](self).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Expect {
    /// Nothing: the rules decide alone.
    #[default]
    Unknown,
    /// The writer is not done: R2 is off, and the owner ending is what ends
    /// the stream (R4).
    More,
    /// What is written is whole. More can follow only while the owner runs,
    /// so once it has ended the current end is the end (R2′).
    NothingOnceEnded,
}

/// The decision of when a stream is over (R2 to R4), and the bookkeeping it
/// needs: bytes seen, the quiet timer, whether more is expected. Held by
/// [`file::File`].
pub(crate) struct Rules {
    gate: Gate,
    bytes: u64,
    /// When the stream first went quiet with the owner already ended.
    quiet_since: Option<Instant>,
    expect: Expect,
}

impl Rules {
    pub(crate) fn new(gate: Gate) -> Self {
        Self {
            gate,
            bytes: 0,
            quiet_since: None,
            expect: Expect::Unknown,
        }
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    /// `n` bytes were delivered: the stream is not quiet.
    pub(crate) fn arrived(&mut self, n: usize) {
        self.bytes += n as u64;
        self.quiet_since = None;
    }

    pub(crate) fn expect(&mut self, what: Expect) {
        self.expect = what;
    }

    /// `held`, asked about `path` outside the rules.
    pub(crate) fn held(&mut self, path: &Path) -> Option<bool> {
        self.gate.held.as_mut().map(|f| f(path))
    }

    pub(crate) fn quiet(&self) -> Duration {
        self.gate.quiet
    }

    /// R1: the read failed.
    pub(crate) fn broken(&self, error: io::Error) -> Done {
        Done::Broken {
            bytes: self.bytes,
            error,
        }
    }

    /// Apply R2 to R4 to a read that returned nothing. `eof` is whether the
    /// read reached the end of the file, which R2 needs; the file follower
    /// always passes `true`, and tests pass `false` to exercise R3 and R4 on
    /// their own. `reading` is the path `held` is asked about.
    pub(crate) fn judge(&mut self, eof: bool, reading: &Path) -> Option<Done> {
        let bytes = self.bytes;

        let mut held: Option<Option<bool>> = None;
        let mut held = |gate: &mut Gate| -> Option<bool> {
            *held.get_or_insert_with(|| gate.held.as_mut().map(|f| f(reading)))
        };

        let mut ended: Option<bool> = None;
        let mut ended = |gate: &mut Gate| -> bool { *ended.get_or_insert_with(|| (gate.ended)()) };

        let drained = eof
            && bytes > 0
            && match self.expect {
                Expect::Unknown => ended(&mut self.gate) && held(&mut self.gate) == Some(false),
                Expect::More => false,
                Expect::NothingOnceEnded => ended(&mut self.gate),
            };
        if drained {
            return Some(Done::Drained { bytes });
        }

        if !ended(&mut self.gate) {
            self.quiet_since = None;
            return None;
        }

        if bytes == 0 && held(&mut self.gate) == Some(false) {
            return Some(Done::Ended {
                bytes,
                quiet: Duration::ZERO,
            });
        }

        let since = *self.quiet_since.get_or_insert_with(Instant::now);
        let need = if bytes == 0 {
            self.gate.quiet_empty
        } else {
            self.gate.quiet
        };
        let quiet = since.elapsed();
        (quiet >= need).then_some(Done::Ended { bytes, quiet })
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Flags;
    use super::*;
    use std::sync::atomic::Ordering;

    /// The path `held` is asked about: these rules never look at it.
    const STREAM: &str = "/stream";

    /// Rules on [`Flags`], with windows the sleeps below are sized to.
    fn rules(with_held: bool) -> (Rules, Flags) {
        let flags = Flags::new();
        let mut gate = flags.gate(with_held);
        gate.quiet = Duration::from_millis(100);
        gate.quiet_empty = Duration::from_millis(40);
        (Rules::new(gate), flags)
    }

    #[test]
    fn nothing_is_decided_while_the_owner_lives_and_the_file_is_held() {
        let (mut r, _f) = rules(true);
        assert!(r.judge(true, Path::new(STREAM)).is_none());
        r.arrived(5);
        assert!(r.judge(true, Path::new(STREAM)).is_none());
        assert!(r.judge(false, Path::new(STREAM)).is_none());
    }

    #[test]
    fn r2_needs_eof_bytes_no_expectation_and_nobody_holding() {
        let (mut r, f) = rules(true);
        f.release();
        // No bytes yet.
        assert!(r.judge(true, Path::new(STREAM)).is_none());
        r.arrived(3);
        f.end();
        // Without EOF, R2 does not apply.
        assert!(r.judge(false, Path::new(STREAM)).is_none());
        // Expecting more switches it off.
        r.expect(Expect::More);
        assert!(r.judge(true, Path::new(STREAM)).is_none());
        r.expect(Expect::Unknown);
        assert!(matches!(
            r.judge(true, Path::new(STREAM)),
            Some(Done::Drained { bytes: 3 })
        ));
    }

    #[test]
    fn r2_also_needs_the_owner_ended() {
        // A stale server pid holds nothing: while the client lives, "nobody
        // holds the file" must not end a file at a pause between writes.
        let (mut r, f) = rules(true);
        f.release();
        r.arrived(3);
        assert!(
            r.judge(true, Path::new(STREAM)).is_none(),
            "the client is still running"
        );
        f.end();
        assert!(matches!(
            r.judge(true, Path::new(STREAM)),
            Some(Done::Drained { bytes: 3 })
        ));
    }

    #[test]
    fn r2_prime_needs_only_eof_bytes_and_the_owner_ended() {
        // `held` says the file is held, and is not asked.
        let (mut r, f) = rules(true);
        r.expect(Expect::NothingOnceEnded);
        assert!(r.judge(true, Path::new(STREAM)).is_none(), "no bytes yet");
        r.arrived(3);
        assert!(
            r.judge(true, Path::new(STREAM)).is_none(),
            "the owner can still add to it"
        );
        f.end();
        assert!(
            r.judge(false, Path::new(STREAM)).is_none(),
            "a writer attached is not EOF"
        );
        assert!(matches!(
            r.judge(true, Path::new(STREAM)),
            Some(Done::Drained { bytes: 3 })
        ));
    }

    #[test]
    fn r2_without_a_held_hint_never_applies() {
        let (mut r, f) = rules(false);
        r.arrived(3);
        f.end();
        assert!(r.judge(true, Path::new(STREAM)).is_none());
    }

    #[test]
    fn r3_is_immediate_only_with_no_bytes_and_nobody_holding() {
        let (mut r, f) = rules(true);
        f.end();
        f.release();
        match r.judge(true, Path::new(STREAM)) {
            Some(Done::Ended { bytes: 0, quiet }) => assert_eq!(quiet, Duration::ZERO),
            other => panic!("expected an immediate Ended, got {other:?}"),
        }
        // With bytes it is not R3: the window applies.
        let (mut r, f) = rules(true);
        f.end();
        f.release();
        r.arrived(1);
        assert!(
            r.judge(false, Path::new(STREAM)).is_none(),
            "R2 needs eof; the window has not passed"
        );
    }

    #[test]
    fn r4_waits_the_window_that_matches_whether_bytes_were_seen() {
        let (mut r, f) = rules(true);
        f.end();
        assert!(
            r.judge(false, Path::new(STREAM)).is_none(),
            "the window just started"
        );
        std::thread::sleep(Duration::from_millis(60));
        assert!(
            matches!(
                r.judge(false, Path::new(STREAM)),
                Some(Done::Ended { bytes: 0, .. })
            ),
            "the empty window is 40ms"
        );

        let (mut r, f) = rules(true);
        f.end();
        r.arrived(9);
        assert!(r.judge(false, Path::new(STREAM)).is_none());
        std::thread::sleep(Duration::from_millis(60));
        assert!(
            r.judge(false, Path::new(STREAM)).is_none(),
            "the non-empty window is 100ms"
        );
        std::thread::sleep(Duration::from_millis(60));
        match r.judge(false, Path::new(STREAM)) {
            Some(Done::Ended { bytes: 9, quiet }) => assert!(quiet >= Duration::from_millis(100)),
            other => panic!("expected Ended, got {other:?}"),
        }
    }

    #[test]
    fn bytes_restart_the_window_and_a_living_owner_clears_it() {
        let (mut r, f) = rules(true);
        f.end();
        r.arrived(1);
        r.judge(false, Path::new(STREAM));
        assert!(r.quiet_since.is_some());
        r.arrived(1);
        assert!(r.quiet_since.is_none(), "bytes restart the window");
        r.judge(false, Path::new(STREAM));
        assert!(r.quiet_since.is_some());
        f.ended.store(false, Ordering::SeqCst);
        r.judge(false, Path::new(STREAM));
        assert!(
            r.quiet_since.is_none(),
            "an owner that is not ended clears it"
        );
    }

    #[test]
    fn held_is_asked_lazily_and_at_most_once_per_judgement() {
        let (mut r, f) = rules(true);
        // Owner alive, no EOF: nothing needs it.
        r.judge(false, Path::new(STREAM));
        assert_eq!(f.held_calls.load(Ordering::SeqCst), 0);
        // EOF with no bytes and an ended owner: R3 asks once.
        f.end();
        f.release();
        r.judge(true, Path::new(STREAM));
        assert_eq!(f.held_calls.load(Ordering::SeqCst), 1);
        // EOF with bytes, owner ended: R2 asks, R3 does not ask again.
        let (mut r, f) = rules(true);
        f.end();
        r.arrived(1);
        r.judge(true, Path::new(STREAM));
        assert_eq!(f.held_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn arrived_counts_bytes_and_broken_reports_them() {
        let (mut r, _f) = rules(true);
        r.arrived(4);
        r.arrived(6);
        assert_eq!(r.bytes(), 10);
        match r.broken(io::Error::other("boom")) {
            Done::Broken { bytes: 10, error } => assert_eq!(error.to_string(), "boom"),
            other => panic!("expected Broken, got {other:?}"),
        }
    }

    #[test]
    fn a_gate_starts_with_the_documented_windows_and_no_hint() {
        let gate = Gate::new(|| false);
        assert_eq!(gate.quiet, QUIET);
        assert_eq!(gate.quiet_empty, QUIET_EMPTY);
        assert!(gate.held.is_none());
        let mut gate = gate.with_held(|_| true);
        assert!((gate.held.as_mut().unwrap())(Path::new(STREAM)));
    }

    #[test]
    fn done_reports_its_bytes_and_a_sentence() {
        let drained = Done::Drained { bytes: 10 };
        let ended = Done::Ended {
            bytes: 20,
            quiet: Duration::from_millis(5000),
        };
        let empty = Done::Ended {
            bytes: 0,
            quiet: Duration::ZERO,
        };
        let broken = Done::Broken {
            bytes: 30,
            error: io::Error::other("boom"),
        };
        assert_eq!(
            [
                drained.bytes(),
                ended.bytes(),
                empty.bytes(),
                broken.bytes()
            ],
            [10, 20, 0, 30]
        );
        assert_eq!(drained.to_string(), "the stream ended after 10 bytes");
        assert_eq!(
            ended.to_string(),
            "the writer ended and nothing more arrived for 5000ms after 20 bytes"
        );
        assert_eq!(
            empty.to_string(),
            "the writer ended without writing anything"
        );
        assert_eq!(broken.to_string(), "the stream failed after 30 bytes: boom");
    }
}
