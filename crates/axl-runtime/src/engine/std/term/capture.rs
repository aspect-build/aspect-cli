use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bazel::console::feed::{Feed, Source};

use super::fd::{STALL_LIMIT, dup, dup2, pipe_readable, write_patiently, write_when_writable};
use super::{HELD, lock_until};

/// Captured output kept for the replay; past it the oldest bytes go.
pub(super) const MAX_CAPTURED_BYTES: usize = 8 << 20;

/// One of the process's descriptors sent into a pipe a thread reads.
pub(super) struct Redirect {
    fd: RawFd,
    /// The descriptor as it was, put back by [`Redirect::restore`].
    pub(super) saved: OwnedFd,
    /// Hands back the pipe's read end when stopped with it still open.
    reader: Option<JoinHandle<Option<File>>>,
    stop: Arc<AtomicBool>,
    /// What was written to it, for the replay.
    written: Arc<Mutex<Vec<u8>>>,
    /// The pipe's read end a [`Redirect::drain`] got back, still open.
    left: Option<File>,
}

impl Redirect {
    /// Send `fd` into a pipe whose reader keeps what is written, for the
    /// replay, and hands it to `capture` as lines.
    /// With `copies` (a screen's, not a test's), what it reads is copied to
    /// the task's output files, all of it, as it is drained on release too.
    pub(super) fn new(
        fd: RawFd,
        capture: Arc<Feed>,
        source: Source,
        copies: bool,
    ) -> std::io::Result<Redirect> {
        let saved = dup(fd)?;
        // Closed on exec as it is made (`pipe2` on Linux; macOS sets it just
        // after): only `fd` is for children (`dup2` leaves it inheritable).
        let (read, write) = std::io::pipe()?;
        let (read, write) = (OwnedFd::from(read), OwnedFd::from(write));
        dup2(write.as_raw_fd(), fd)?;
        drop(write);
        let stop = Arc::new(AtomicBool::new(false));
        let written = Arc::new(Mutex::new(Vec::new()));
        let spawned = {
            let (stop, written) = (stop.clone(), written.clone());
            let mut file = File::from(read);
            std::thread::Builder::new()
                .name("term-capture".into())
                .spawn(move || {
                    let mut buf = [0u8; 8192];
                    // Once stopped, what is left is read for this long at
                    // most: a child writing without pause cannot hold up the
                    // release, and gets the rest copied through after it.
                    let mut drain_until = None;
                    loop {
                        let stopping = stop.load(Ordering::SeqCst);
                        if stopping {
                            let until = *drain_until
                                .get_or_insert_with(|| std::time::Instant::now() + DRAIN_BUDGET);
                            if std::time::Instant::now() >= until {
                                return Some(file);
                            }
                        }
                        let wait = if stopping {
                            Duration::ZERO
                        } else {
                            CAPTURE_POLL
                        };
                        if !pipe_readable(file.as_raw_fd(), wait) {
                            if stopping {
                                // A child may still hold the write end:
                                // `restore` keeps the pipe open for it.
                                return Some(file);
                            }
                            continue;
                        }
                        match file.read(&mut buf) {
                            Ok(0) => return None,
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                            // Read again after a pause: stopping reading would
                            // fill the pipe and block every writer to it.
                            Err(_) => std::thread::sleep(READ_RETRY),
                            Ok(n) => {
                                keep(&mut written.lock().unwrap(), &buf[..n]);
                                if copies {
                                    crate::out::copy_only(&buf[..n]);
                                }
                                capture.write(source, &buf[..n]);
                            }
                        }
                    }
                })
        };
        match spawned {
            Ok(reader) => Ok(Redirect {
                fd,
                saved,
                reader: Some(reader),
                stop,
                written,
                left: None,
            }),
            Err(e) => {
                let _ = dup2(saved.as_raw_fd(), fd);
                Err(e)
            }
        }
    }

    /// Put the descriptor back, and what was written to it while captured:
    /// stdout's to stdout, stderr's to stderr.
    ///
    /// A child spawned while captured may still hold the pipe: what it
    /// writes from here on is copied through to the descriptor by a detached
    /// thread until it closes, so it neither blocks nor dies of SIGPIPE.
    pub(super) fn restore(self) {
        self.put_back();
        self.finish(true);
    }

    /// Point the descriptor at what it was again: quick, so every descriptor
    /// is back before any replay waits on a slow destination.
    fn put_back(&self) {
        let _ = dup2(self.saved.as_raw_fd(), self.fd);
    }

    /// Stop the reader once it has read (and, copying, copied) what the pipe
    /// holds; after [`Redirect::put_back`]. [`Redirect::finish`] does it too.
    fn drain(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(reader) = self.reader.take() {
            self.left = reader.join().ok().flatten();
        }
    }

    /// Stop capturing, replay what was written, and pass on what a child
    /// still holding the pipe writes, whole lines at a time, to `fd` as it is
    /// then (a capture again once a screen is taken back); after
    /// [`Redirect::put_back`].
    pub(super) fn finish(mut self, replay: bool) {
        self.drain();
        let open = self.left.take();
        let written = std::mem::take(&mut *self.written.lock().unwrap());
        let fd = self.fd;
        let mut out = File::from(self.saved);
        if replay {
            let _turn = stream_write_bounded(fd);
            let replayed = newest(&written);
            write_patiently(&mut out, replayed);
            axl_types::terminal::wrote(fd, replayed);
        }
        if let Some(mut pipe) = open {
            let _ = std::thread::Builder::new()
                .name("term-capture-after".into())
                .spawn(move || {
                    // Whole lines, each run of them one unit on the target;
                    // an unended tail waits for its line's end or the pipe's.
                    // To fd 1 or 2 as it is now ([`write_std_unit`]): the
                    // terminal while the screen is given back, a capture again
                    // once it is taken back, which then copies them; else
                    // copied here. Any other descriptor (a test's) gets them
                    // where it was.
                    let mut buf = [0u8; 8192];
                    let mut pending = Vec::new();
                    let std_stream = fd == 1 || fd == 2;
                    let mut put = |bytes: &[u8]| {
                        if !std_stream {
                            let _turn = stream_write_bounded(fd);
                            write_patiently(&mut out, bytes);
                            return;
                        }
                        if let Ok(false) = write_std_unit(fd == 2, bytes) {
                            crate::out::copy_only(bytes);
                        }
                    };
                    loop {
                        match pipe.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                pending.extend_from_slice(&buf[..n]);
                                if let Some(end) = pending.iter().rposition(|&b| b == b'\n') {
                                    put(&pending[..=end]);
                                    pending.drain(..=end);
                                }
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                            Err(_) => break,
                        }
                    }
                    if !pending.is_empty() {
                        put(&pending);
                    }
                });
        }
    }
}

/// How long a stopped capture reads what is left before handing the pipe on.
const DRAIN_BUDGET: Duration = Duration::from_millis(100);

/// How long a capture's reader waits for bytes before it looks again
/// whether it was stopped.
const CAPTURE_POLL: Duration = Duration::from_millis(20);

/// How long a capture's reader pauses after a failed read before it reads
/// again.
const READ_RETRY: Duration = Duration::from_millis(10);

/// How long moving fds 1 and 2 in or out of a capture waits for a writer to
/// finish its unit: one writing does in far less, and one stalled on a
/// destination nobody reads is not waited on.
pub(super) const SWITCH_WAIT: Duration = Duration::from_millis(100);

/// Append `bytes` to `buf`, keeping at least the newest
/// [`MAX_CAPTURED_BYTES`] and at most twice that, so trimming is rare.
fn keep(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(bytes);
    if buf.len() > 2 * MAX_CAPTURED_BYTES {
        let cut = buf.len() - MAX_CAPTURED_BYTES;
        buf.drain(..cut);
    }
}

/// The newest [`MAX_CAPTURED_BYTES`] of `buf`, for the replay.
fn newest(buf: &[u8]) -> &[u8] {
    &buf[buf.len().saturating_sub(MAX_CAPTURED_BYTES)..]
}

/// `fd`'s target ([`axl_types::terminal`]) for a whole dump or replay,
/// given up after [`STALL_LIMIT`]; None for any other descriptor, or given up.
fn stream_write_bounded(fd: RawFd) -> Option<axl_types::terminal::Turn> {
    axl_types::terminal::turn_within(fd, STALL_LIMIT)
}

/// Write `bytes` to stdout (or, with `stderr`, stderr) as one unit, for a
/// child's output passed on: a forwarder's, or what a child still writes into
/// a stopped capture. Whether it went into a screen's capture, which copies
/// it, so the caller copies it only when not.
///
/// The target is held for the whole, waited on for as long as it takes (a
/// capture's replay holds it, and gives up on its own after
/// [`STALL_LIMIT`] without progress): never written alongside another's.
/// What std's stdout buffers goes first. Then `bytes` go to the stream's
/// file as it is now, through a descriptor of their own, never inside a
/// blocking write ([`write_when_writable`]): a screen taken or given back
/// meanwhile moves fds 1 and 2 without waiting on this write or failing it,
/// and a `select` cut short by that move backs off and goes on. Whether a
/// capture copies is read under the target, where no move happens.
pub(crate) fn write_std_unit(stderr: bool, bytes: &[u8]) -> std::io::Result<bool> {
    let fd = if stderr { 2 } else { 1 };
    let _turn = axl_types::terminal::turn(fd);
    if !stderr {
        std::io::stdout().lock().flush()?;
    }
    let into_capture = crate::out::capture_copies();
    // SAFETY: F_DUPFD_CLOEXEC on fd 1 or 2 returns a new descriptor this
    // function owns, closed when `own` drops.
    let own = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if own < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let own = unsafe { OwnedFd::from_raw_fd(own) };
    write_when_writable(own.as_raw_fd(), bytes)?;
    axl_types::terminal::wrote(fd, bytes);
    Ok(into_capture)
}

/// Stdout's and stderr's: each held by a release from before the screen is
/// taken out of `HELD` until that stream's capture is replayed, and by a
/// forwarder to that stream while it decides a chunk's route and feeds a
/// screen ([`forwarding`]), never across a write to the terminal: so a
/// child's output never lands inside the replay, nor goes through the
/// capture's pipe as the screen goes, where it would be copied twice, and a
/// stream nobody reads stalls no one else (stderr's is let go before
/// stdout's replay, unless the two are one file). A line a forwarder feeds
/// the screen is copied to the output files under it too, so a release's
/// dump has it.
pub(super) static OUTPUT_ORDER: [Mutex<()>; 2] = [Mutex::new(()), Mutex::new(())];

/// [`OUTPUT_ORDER`] as a release holds it, a lock it gave up on `None`.
pub(super) struct Order {
    _out: Option<std::sync::MutexGuard<'static, ()>>,
    pub(super) err: Option<std::sync::MutexGuard<'static, ()>>,
}

impl Order {
    /// Both, stdout's first, as every release takes them.
    pub(super) fn lock() -> Order {
        let lock = |m: &'static Mutex<()>| Some(m.lock().unwrap_or_else(|p| p.into_inner()));
        Order {
            _out: lock(&OUTPUT_ORDER[0]),
            err: lock(&OUTPUT_ORDER[1]),
        }
    }

    /// Both, stdout's first, each given up when still held at `deadline`.
    pub(super) fn lock_until(deadline: Instant) -> Order {
        Order {
            _out: lock_until(&OUTPUT_ORDER[0], deadline),
            err: lock_until(&OUTPUT_ORDER[1], deadline),
        }
    }

    /// Neither: for a release nothing else can be writing beside.
    pub(super) fn none() -> Order {
        Order {
            _out: None,
            err: None,
        }
    }
}

/// Run `f`, a forwarder's handling of one chunk to stderr (or stdout),
/// between that stream's releases.
pub(crate) fn forwarding<T>(stderr: bool, f: impl FnOnce() -> T) -> T {
    let _order = OUTPUT_ORDER[stderr as usize]
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    f()
}

/// Hand `bytes` (whole lines) a child wrote to the screen holding the
/// terminal, as its stderr (or stdout) capture would: a forwarder's lines
/// go to the screen's console here, and are kept for its replay as the
/// capture's are, not through the process's own stderr, where the capture
/// would copy them a second time and splice them with the CLI's. Whether a
/// screen took them (one held, not given back); `bytes` empty asks
/// only that. Its first `shown` bytes, a line's start written before the
/// screen was taken, are on the main screen already: not kept to replay.
pub(crate) fn feed_screen(stderr: bool, bytes: &[u8], shown: usize) -> bool {
    let Ok(held) = HELD.lock() else {
        return false;
    };
    let Some(h) = held.as_ref().filter(|h| !h.suspended) else {
        return false;
    };
    if !bytes.is_empty() {
        h.capture.write(
            if stderr {
                Source::Stderr
            } else {
                Source::Stdout
            },
            bytes,
        );
        if let Some(r) = if stderr { &h.stderr } else { &h.stdout } {
            keep(
                &mut r.written.lock().unwrap(),
                &bytes[shown.min(bytes.len())..],
            );
        }
    }
    true
}

/// Where the CLI's own output goes while a screen holds the terminal
/// ([`route`]): its console's feed, and what its captures keep for
/// the replay. Set and cleared with every target held, where the capture's
/// copies go on and off, so a write is routed exactly when fds 1 and 2 are
/// captured.
pub(super) struct Sink {
    capture: Arc<Feed>,
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

pub(super) static SINK: Mutex<Option<Sink>> = Mutex::new(None);

/// Route the CLI's writes to `capture` and the replays `out` and `err`
/// keep, or (None) to fds 1 and 2 again.
pub(super) fn set_sink(sink: Option<(&Arc<Feed>, &Redirect, &Redirect)>) {
    *SINK.lock().unwrap_or_else(|p| p.into_inner()) = sink.map(|(capture, out, err)| Sink {
        capture: capture.clone(),
        stdout: out.written.clone(),
        stderr: err.written.clone(),
    });
}

/// The route the CLI's own writes are offered ([`axl_types::terminal`]):
/// while a screen holds the terminal, `bytes` for fd 1 or 2 go to
/// its console and are kept for its replay, as its capture would have them,
/// without passing through the captured descriptors; the writer copies them
/// to the output files itself. Whether it took them.
fn route(fd: i32, bytes: &[u8]) -> bool {
    let sink = SINK.lock().unwrap_or_else(|p| p.into_inner());
    let Some(s) = sink.as_ref() else {
        return false;
    };
    let (source, kept) = if fd == 2 {
        (Source::Stderr, &s.stderr)
    } else {
        (Source::Stdout, &s.stdout)
    };
    s.capture.write(source, bytes);
    keep(&mut kept.lock().unwrap_or_else(|p| p.into_inner()), bytes);
    true
}

/// Install [`route`] for the CLI's own output. Once, at start-up.
pub fn route_cli_output() {
    axl_types::terminal::route_to(route);
}

/// Put fds 1 and 2 back from a screen's captures, and the output files'
/// copies back to the print paths, with every target held the while (each
/// waited on for [`SWITCH_WAIT`]; returned, for a caller that dumps before
/// it lets them go): no write lands half in a capture and half on the terminal, and
/// none is copied between the capture's last chunks (its readers drained
/// first), or goes uncopied.
pub(super) fn give_back_descriptors(
    redirects: [Option<&mut Redirect>; 2],
) -> axl_types::terminal::Turns {
    let moving = axl_types::terminal::every_turn_within(SWITCH_WAIT);
    let mut redirects: Vec<&mut Redirect> = redirects.into_iter().flatten().collect();
    for r in &redirects {
        r.put_back();
    }
    for r in &mut redirects {
        r.drain();
    }
    crate::out::set_capture_copies(false);
    set_sink(None);
    moving
}

/// Whether descriptors `a` and `b` are one target ([`axl_types::terminal::one_target`]).
pub(super) fn same_file(a: RawFd, b: RawFd) -> bool {
    axl_types::terminal::one_target(a, b)
}

/// Remember the terminal state from before a screen captured stdout and
/// stderr (`Some`), or forget it (`None`): for AXL's `is_tty`, the console's
/// display and colour, and the size of the PTY Bazel draws into.
pub(super) fn mark_captured(state: Option<(bool, bool)>) {
    axl_types::stream::remember_tty(state);
    bazel::console::capture::set_stderr_captured(state.is_some());
}

#[cfg(test)]
mod tests {
    use super::super::fd::tests::private_pipe;
    use super::*;

    /// A child that holds the pipe past `restore()` still gets its writes
    /// through.
    #[test]
    fn a_child_outliving_the_capture_still_writes() {
        let (read, write) = private_pipe();
        let capture = Arc::new(Feed::default());
        let r = Redirect::new(write.as_raw_fd(), capture.clone(), Source::Stdout, false).unwrap();
        let mut child = std::process::Command::new("sh")
            .args(["-c", "sleep 0.3; echo late"])
            .stdout(std::process::Stdio::from(dup(write.as_raw_fd()).unwrap()))
            .spawn()
            .unwrap();
        r.restore();
        drop(write);
        assert!(child.wait().unwrap().success(), "not killed by SIGPIPE");
        let mut got = String::new();
        File::from(read).read_to_string(&mut got).unwrap();
        assert_eq!(got, "late\n");
    }

    /// The CLI's own writes, with a screen's sink set, go to the screen's
    /// feed and its replay, never through the captured descriptors; with
    /// none, nowhere but the descriptor.
    #[test]
    fn the_clis_own_output_is_routed_past_a_capture() {
        route_cli_output();
        let (_read, write) = private_pipe();
        let capture = Arc::new(Feed::default());
        let out = Redirect::new(write.as_raw_fd(), capture.clone(), Source::Stdout, false).unwrap();
        let err = Redirect::new(write.as_raw_fd(), capture.clone(), Source::Stderr, false).unwrap();
        set_sink(Some((&capture, &out, &err)));
        assert!(axl_types::terminal::routed(2, b"routed line\n"));
        set_sink(None);
        assert!(!axl_types::terminal::routed(2, b"not routed\n"));
        assert_eq!(capture.drain().lines, vec!["routed line"]);
        assert_eq!(&*err.written.lock().unwrap(), b"routed line\n");
        assert!(out.written.lock().unwrap().is_empty());
        err.restore();
        out.restore();
    }

    /// What is written to a redirected descriptor is captured as lines and
    /// written back to it on restore.
    #[test]
    fn a_redirect_captures_and_replays() {
        let (read, write) = private_pipe();
        let capture = Arc::new(Feed::default());
        let r = Redirect::new(write.as_raw_fd(), capture.clone(), Source::Stdout, false).unwrap();
        let mut target = File::from(dup(write.as_raw_fd()).unwrap());
        target.write_all(b"one\ntwo\n").unwrap();
        drop(target);
        std::thread::sleep(Duration::from_millis(100));
        r.restore();
        drop(write);
        assert_eq!(capture.drain().lines, vec!["one", "two"]);
        let mut replayed = String::new();
        File::from(read).read_to_string(&mut replayed).unwrap();
        assert_eq!(replayed, "one\ntwo\n");
    }

    #[test]
    fn the_replay_keeps_the_newest_bytes() {
        let mut buf = vec![b'a'; 2 * MAX_CAPTURED_BYTES];
        keep(&mut buf, b"xyz");
        assert_eq!(buf.len(), MAX_CAPTURED_BYTES);
        assert!(buf.ends_with(b"axyz"));
        let mut small = b"ab".to_vec();
        keep(&mut small, b"c");
        assert_eq!(newest(&small), b"abc");
    }

    /// A child writing without pause does not hold up the restore.
    #[test]
    fn a_restore_does_not_wait_on_a_busy_child() {
        let (read, write) = private_pipe();
        let capture = Arc::new(Feed::default());
        let r = Redirect::new(write.as_raw_fd(), capture, Source::Stdout, false).unwrap();
        let mut child = std::process::Command::new("sh")
            // `exec`, so the kill below reaches `yes` itself.
            .args(["-c", "exec yes"])
            .stdout(std::process::Stdio::from(dup(write.as_raw_fd()).unwrap()))
            .spawn()
            .unwrap();
        // Something drains the restored descriptor, as a terminal would.
        let drain = std::thread::spawn(move || {
            let _ = std::io::copy(&mut File::from(read), &mut std::io::sink());
        });
        std::thread::sleep(Duration::from_millis(50));
        let started = std::time::Instant::now();
        r.restore();
        let took = started.elapsed();
        child.kill().unwrap();
        child.wait().unwrap();
        drop(write);
        drain.join().unwrap();
        assert!(took < Duration::from_secs(2), "restore took {took:?}");
    }
}
