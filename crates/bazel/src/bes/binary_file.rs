//! Bazel's `--build_event_binary_file`: the Build Event Protocol as
//! length-delimited `BuildEvent` messages, written by the server as the build
//! runs and, under `fully_async`, after the client has already exited.
//!
//! [`Reader`] turns the stream into frames and says, when it stops, whether
//! the stream was *complete* (bazel's final event, `last_message`, arrived)
//! or not, and if not, why. [`requested_path`] reads the file a caller asked
//! Bazel for, which the CLI's own flag would otherwise replace.
//!
//! The stream is a regular file rather than a FIFO, so a slow reader slows
//! only itself: a FIFO's full buffer would block bazel inside its own event
//! writer. A build bazel retries reopens the file with `O_TRUNC` for the next
//! attempt, which the [`File`] follows on its own (see its module docs). The
//! retry marker says one is coming, so that the marked attempt's
//! `last_message` does not end the stream. Bazel can also run out of retries:
//! that `last_message` then completes the stream once the client exits without
//! a new attempt's file having appeared.

use std::io::{self, ErrorKind};
use std::path::PathBuf;

use axl_proto::build_event_stream::BuildEvent;
use axl_proto::build_event_stream::build_event::Payload;
use prost::Message;
use prost::bytes::Bytes;

use crate::delimited;
use crate::stream::file::{File, StartedOver};
use crate::stream::{Done, Expect};

/// The flag that names where Bazel writes the binary BEP.
pub const FLAG: &str = "--build_event_binary_file";

/// What a build's file is called in its folder of the task invocation's
/// history: the stream as the CLI read it, redacted.
pub const FILE_NAME: &str = "build_events.binpb";

/// The BEP file the caller asked Bazel for in `flags`, if any. A relative one
/// is under the workspace root, where Bazel resolves it, or under `directory`
/// when no workspace can be found there; a leading `~` is literal, since the
/// option is a plain string. See [`crate::rc::requested_path`] for empty
/// values.
pub fn requested_path(flags: &[String], verb: &str, directory: Option<&str>) -> Option<PathBuf> {
    let path = crate::rc::requested_path(flags, FLAG, verb, None)?;
    if path.is_absolute() {
        return Some(path);
    }
    match crate::rc::workspace_root(directory) {
        Some(root) => Some(root.join(path)),
        None => crate::rc::requested_path(flags, FLAG, verb, directory),
    }
}

/// `REMOTE_CACHE_EVICTED`: the build ends with this code and bazel runs it
/// again in the same invocation, starting the file over for the next attempt.
pub(crate) const REMOTE_CACHE_EVICTED: i32 = 39;

/// One event as it was read.
#[derive(Debug)]
pub struct Frame {
    pub event: BuildEvent,
    /// The frame exactly as written, length prefix included: what a mirror of
    /// the stream writes back out.
    pub raw: Bytes,
    prefix_len: usize,
    retry: bool,
    retried: bool,
}

impl Frame {
    /// The serialized `BuildEvent`, without its length prefix.
    pub fn payload(&self) -> &[u8] {
        &self.raw[self.prefix_len..]
    }

    /// [`Frame::payload`] as shared bytes: the same allocation as `raw`, not
    /// a copy.
    pub fn payload_bytes(&self) -> Bytes {
        self.raw.slice(self.prefix_len..)
    }

    /// This is the `BuildFinished` that ends an attempt bazel is about to
    /// retry (`REMOTE_CACHE_EVICTED`): the next attempt starts the file over
    /// at the same path.
    pub fn is_retry_marker(&self) -> bool {
        self.retry
    }

    /// This is the `BuildStarted` of the attempt that follows a retry marker:
    /// the stream starts over here. Everything before it, the retried
    /// attempt's tail included, belongs to the attempt bazel abandoned.
    pub fn starts_retried_attempt(&self) -> bool {
        self.retried
    }
}

/// How the stream ended.
#[derive(Debug)]
pub enum End {
    /// Bazel's final event arrived. Nothing is missing.
    Complete,
    /// The stream stopped without it: the writer died, was cut off, or never
    /// wrote. Everything before the stop was delivered.
    Incomplete(Done),
    /// The bytes were not a valid stream. Everything before them was
    /// delivered.
    Corrupt(String),
}

impl std::fmt::Display for End {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            End::Complete => write!(f, "complete"),
            End::Incomplete(done) => write!(f, "{done}"),
            End::Corrupt(why) => write!(f, "the stream was corrupt: {why}"),
        }
    }
}

/// Reads frames from a [`File`].
///
/// An iterator over frames: `None` is the end, and [`Reader::end`] says which
/// kind. An `Err` item is a corrupt stream and is followed by `None`. A stream
/// cut off in the middle of a frame is an end, not an error: the frames before
/// it were good, and [`End::Incomplete`] says how it stopped. A frame cut off
/// where bazel started the file over is dropped, and reading goes on in the
/// new file.
pub struct Reader {
    file: File,
    buf: Vec<u8>,
    /// An attempt ended with `REMOTE_CACHE_EVICTED`, or the file started over,
    /// and the next attempt's `BuildStarted` has not arrived. Its
    /// `last_message` is not the end of the stream.
    retrying: bool,
    /// That attempt's `last_message` arrived. Should no new attempt's file
    /// appear, that attempt was the build's last, and the stream is complete.
    retried_whole: bool,
    end: Option<End>,
}

impl Reader {
    pub fn new(file: File) -> Self {
        Self {
            file,
            buf: Vec::with_capacity(5 * 1024),
            retrying: false,
            retried_whole: false,
            end: None,
        }
    }

    /// How the stream ended, once iteration has returned `None`.
    pub fn end(&self) -> Option<&End> {
        self.end.as_ref()
    }

    /// [`Reader::end`], moved out.
    pub fn into_end(self) -> Option<End> {
        self.end
    }

    /// How the stream ended at the zero-byte read that ended a frame read:
    /// the file's reason for stopping, unless a retry bazel did not make
    /// leaves the stream whole.
    fn end_of_file(&mut self) -> End {
        let done = self.file.take_done().unwrap_or(Done::Drained {
            bytes: self.file.bytes_read(),
        });
        let out_of_retries = self.retried_whole
            && self.file.awaiting_replacement()
            && matches!(done, Done::Drained { .. });
        if out_of_retries {
            End::Complete
        } else {
            End::Incomplete(done)
        }
    }
}

fn is_retry_marker(event: &BuildEvent) -> bool {
    matches!(
        &event.payload,
        Some(Payload::Finished(f)) if f.exit_code.as_ref().is_some_and(|c| c.code == REMOTE_CACHE_EVICTED)
    )
}

impl Iterator for Reader {
    type Item = io::Result<Frame>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.end.is_some() {
            return None;
        }
        let header = loop {
            match delimited::next_frame(&mut self.file, &mut self.buf) {
                // Drop a frame the old file left half written. Bazel starts
                // over only to retry, even when the dropped frame was its
                // marker, so reset what was expected of the old file.
                Err(e) if StartedOver::is(&e) => {
                    self.retrying = true;
                    self.file.expect(Expect::More);
                }
                read => break read,
            }
        };
        let header = match header {
            Ok(Some(header)) => header,
            Ok(None) => {
                self.end = Some(self.end_of_file());
                return None;
            }
            // Cut off inside a frame: the file returned its zero-byte read.
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                self.end = Some(self.end_of_file());
                return None;
            }
            Err(e) => {
                self.end = Some(End::Corrupt(e.to_string()));
                return Some(Err(e));
            }
        };

        let payload = &self.buf[..header.len];
        let event = match BuildEvent::decode(payload) {
            Ok(event) => event,
            Err(e) => {
                self.end = Some(End::Corrupt(format!("undecodable build event: {e}")));
                return Some(Err(io::Error::new(ErrorKind::InvalidData, e)));
            }
        };

        let retry = is_retry_marker(&event);
        let mut retried = false;
        if retry {
            self.retrying = true;
            self.retried_whole = false;
            self.file.expect(Expect::More);
            self.file.expect_replacement();
        } else if matches!(event.payload, Some(Payload::Started(_))) {
            retried = self.retrying;
            self.retrying = false;
            self.retried_whole = false;
            self.file.expect(Expect::Unknown);
        }
        if event.last_message {
            if self.retrying {
                self.retried_whole = true;
                self.file.expect(Expect::NothingOnceEnded);
            } else {
                self.end = Some(End::Complete);
            }
        }

        let mut raw = Vec::with_capacity(header.prefix.len() + header.len);
        raw.extend_from_slice(&header.prefix);
        raw.extend_from_slice(payload);
        Some(Ok(Frame {
            event,
            raw: Bytes::from(raw),
            prefix_len: header.prefix.len(),
            retry,
            retried,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bes::testing::{build_finished, encoded, plain, retry_marker, started};
    use crate::stream::file::moved_name;
    use crate::stream::testing::{Flags, QUIET_TEST};
    use axl_proto::build_event_stream::BuildFinished;
    use axl_proto::build_event_stream::build_finished::ExitCode;
    use std::fs::File;
    use std::io::Write;
    use std::time::{Duration, Instant};

    /// A stream under test: the flags stand in for the client and server.
    struct Rig {
        reader: Reader,
        path: std::path::PathBuf,
        flags: Flags,
    }

    impl Rig {
        fn new() -> Self {
            let file = crate::stream::file::Reservation::temp("bazel-bes-test");
            let path = file.path().to_path_buf();
            let flags = Flags::new();
            let reader = Reader::new(file.follow(flags.gate(true)));
            Self {
                reader,
                path,
                flags,
            }
        }

        /// A new attempt, as bazel opens one: the path created or truncated.
        fn next_attempt(&self) -> File {
            File::create(&self.path).unwrap()
        }

        /// Read `n` frames, as the reader does before bazel starts over.
        fn take(&mut self, n: usize) -> Vec<BuildEvent> {
            (0..n)
                .map(|_| self.reader.next().unwrap().unwrap().event)
                .collect()
        }

        fn client_exits(&self) {
            self.flags.end();
        }

        fn server_lets_go(&self) {
            self.flags.release();
        }

        fn frames(&mut self) -> Vec<BuildEvent> {
            self.reader
                .by_ref()
                .map(|f| f.expect("a good frame").event)
                .collect()
        }
    }

    #[test]
    fn frames_come_back_decoded_in_order() {
        let mut rig = Rig::new();
        let mut w = rig.next_attempt();
        for e in [started(), plain(false), build_finished(0, true)] {
            w.write_all(&encoded(&e)).unwrap();
        }
        let events = rig.frames();
        assert_eq!(
            events,
            vec![started(), plain(false), build_finished(0, true)]
        );
    }

    #[test]
    fn a_frame_carries_its_raw_bytes_and_its_payload() {
        let mut rig = Rig::new();
        let event = build_finished(7, true);
        rig.next_attempt().write_all(&encoded(&event)).unwrap();
        let frame = rig.reader.next().unwrap().unwrap();
        assert_eq!(&frame.raw[..], &encoded(&event)[..]);
        assert_eq!(frame.payload(), &event.encode_to_vec()[..]);
        assert_eq!(BuildEvent::decode(frame.payload()).unwrap(), event);
    }

    #[test]
    fn payload_bytes_is_the_payload_without_a_copy() {
        let mut rig = Rig::new();
        rig.next_attempt()
            .write_all(&encoded(&build_finished(3, true)))
            .unwrap();
        let frame = rig.reader.next().unwrap().unwrap();
        let shared = frame.payload_bytes();
        assert_eq!(&shared[..], frame.payload());
        assert_eq!(
            shared.as_ptr(),
            frame.payload().as_ptr(),
            "a slice of the same allocation"
        );
    }

    #[test]
    fn an_end_says_what_happened_in_a_sentence() {
        assert_eq!(End::Complete.to_string(), "complete");
        assert_eq!(
            End::Incomplete(Done::Drained { bytes: 5 }).to_string(),
            "the stream ended after 5 bytes"
        );
        assert_eq!(
            End::Corrupt("bad".into()).to_string(),
            "the stream was corrupt: bad"
        );
    }

    #[test]
    fn last_message_completes_the_stream_while_the_writer_is_still_attached() {
        let mut rig = Rig::new();
        let mut w = rig.next_attempt();
        w.write_all(&encoded(&started())).unwrap();
        w.write_all(&encoded(&build_finished(0, true))).unwrap();
        // `w` stays open: only the protocol can end this stream.
        let events = rig.frames();
        assert_eq!(events.len(), 2);
        assert!(events[1].last_message, "the final event is delivered too");
        assert!(matches!(rig.reader.end(), Some(End::Complete)));
        drop(w);
    }

    #[test]
    fn nothing_after_last_message_is_read() {
        let mut rig = Rig::new();
        let mut w = rig.next_attempt();
        w.write_all(&encoded(&build_finished(0, true))).unwrap();
        w.write_all(&encoded(&plain(false))).unwrap();
        assert_eq!(rig.frames().len(), 1);
        assert!(rig.reader.next().is_none(), "the iterator is fused");
    }

    #[test]
    fn the_end_is_not_known_until_iteration_finishes() {
        let mut rig = Rig::new();
        rig.next_attempt().write_all(&encoded(&started())).unwrap();
        assert!(rig.reader.end().is_none());
        rig.reader.next().unwrap().unwrap();
        assert!(rig.reader.end().is_none(), "mid-stream");
    }

    #[test]
    fn a_stream_that_stops_without_last_message_is_incomplete_not_failed() {
        let mut rig = Rig::new();
        {
            let mut w = rig.next_attempt();
            w.write_all(&encoded(&started())).unwrap();
            w.write_all(&encoded(&plain(false))).unwrap();
        } // the writer closes without the final event
        rig.server_lets_go();
        rig.client_exits();
        assert_eq!(rig.frames().len(), 2);
        match rig.reader.into_end() {
            Some(End::Incomplete(Done::Drained { bytes })) => {
                assert_eq!(
                    bytes as usize,
                    encoded(&started()).len() + encoded(&plain(false)).len()
                )
            }
            other => panic!("expected Incomplete(Drained), got {other:?}"),
        }
    }

    #[test]
    fn a_client_that_exits_leaves_a_server_still_flushing_its_tail() {
        // The fully_async shape: client gone, final event written later.
        let mut rig = Rig::new();
        let mut w = rig.next_attempt();
        w.write_all(&encoded(&started())).unwrap();
        rig.client_exits();
        let tail = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            w.write_all(&encoded(&build_finished(0, true))).unwrap();
            w
        });
        let events = rig.frames();
        let _w = tail.join().unwrap();
        assert_eq!(events.len(), 2, "the tail written after the exit arrives");
        assert!(matches!(rig.reader.end(), Some(End::Complete)));
    }

    #[test]
    fn a_client_that_exits_without_a_tail_ends_the_stream_after_the_quiet_window() {
        let mut rig = Rig::new();
        let mut w = rig.next_attempt();
        w.write_all(&encoded(&started())).unwrap();
        rig.client_exits();
        let t = Instant::now();
        assert_eq!(rig.frames().len(), 1);
        assert!(t.elapsed() >= QUIET_TEST);
        match rig.reader.end() {
            Some(End::Incomplete(Done::Ended { bytes, .. })) => assert!(*bytes > 0),
            other => panic!("expected Incomplete(Ended), got {other:?}"),
        }
        drop(w);
    }

    #[test]
    fn a_stream_with_no_writer_at_all_ends_empty_once_the_client_is_gone() {
        let mut rig = Rig::new();
        rig.client_exits();
        rig.server_lets_go();
        assert!(rig.reader.next().is_none());
        match rig.reader.end() {
            Some(End::Incomplete(Done::Ended { bytes: 0, .. })) => {}
            other => panic!("expected an empty Incomplete(Ended), got {other:?}"),
        }
    }

    #[test]
    fn a_stream_cut_off_inside_a_frame_is_an_end_with_the_good_frames_kept() {
        let mut rig = Rig::new();
        {
            let mut w = rig.next_attempt();
            w.write_all(&encoded(&started())).unwrap();
            let second = encoded(&build_finished(0, true));
            w.write_all(&second[..second.len() - 1]).unwrap();
        }
        rig.server_lets_go();
        rig.client_exits();
        assert_eq!(rig.frames(), vec![started()]);
        assert!(matches!(
            rig.reader.end(),
            Some(End::Incomplete(Done::Drained { .. }))
        ));
    }

    #[test]
    fn a_frame_that_does_not_decode_is_a_corrupt_stream() {
        let mut rig = Rig::new();
        let mut w = rig.next_attempt();
        w.write_all(&encoded(&started())).unwrap();
        // A valid length followed by bytes that are not a BuildEvent.
        w.write_all(&[4, 0xff, 0xff, 0xff, 0xff]).unwrap();
        w.write_all(&encoded(&plain(false))).unwrap();

        assert!(rig.reader.next().unwrap().is_ok());
        let err = rig.reader.next().unwrap().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(
            rig.reader.next().is_none(),
            "nothing is read past corruption"
        );
        assert!(matches!(rig.reader.end(), Some(End::Corrupt(_))));
    }

    #[test]
    fn an_absurd_frame_length_is_a_corrupt_stream_not_an_allocation() {
        let mut rig = Rig::new();
        let mut w = rig.next_attempt();
        // A varint for 2^40.
        w.write_all(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x20]).unwrap();
        let err = rig.reader.next().unwrap().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(matches!(rig.reader.end(), Some(End::Corrupt(_))));
    }

    #[test]
    fn an_unreadable_file_is_an_incomplete_stream() {
        let mut rig = Rig::new();
        std::fs::create_dir(&rig.path).unwrap();
        assert!(rig.reader.next().is_none());
        assert!(matches!(
            rig.reader.end(),
            Some(End::Incomplete(Done::Broken { .. }))
        ));
        let _ = std::fs::remove_dir(&rig.path);
        let _ = std::fs::remove_dir(moved_name(&rig.path, 1));
    }

    #[test]
    fn a_pause_between_events_is_not_the_end_while_the_client_runs() {
        // A stale server pid reads as "nobody holds the file" at every pause.
        let mut rig = Rig::new();
        rig.server_lets_go();
        let mut w = rig.next_attempt();
        w.write_all(&encoded(&started())).unwrap();
        assert_eq!(rig.take(1), vec![started()]);
        let later = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            w.write_all(&encoded(&build_finished(0, true))).unwrap();
            w
        });
        assert_eq!(rig.frames(), vec![build_finished(0, true)]);
        drop(later.join().unwrap());
        assert!(matches!(rig.reader.end(), Some(End::Complete)));
    }

    #[test]
    fn the_retry_marker_is_recognised_only_for_remote_cache_evicted() {
        let mut rig = Rig::new();
        let mut w = rig.next_attempt();
        for e in [retry_marker(false), build_finished(0, false), plain(false)] {
            w.write_all(&encoded(&e)).unwrap();
        }
        let marks: Vec<bool> = (0..3)
            .map(|_| rig.reader.next().unwrap().unwrap().is_retry_marker())
            .collect();
        assert_eq!(marks, vec![true, false, false]);
    }

    #[test]
    fn only_the_build_started_after_a_retry_marker_starts_a_retried_attempt() {
        let mut rig = Rig::new();
        let mut w = rig.next_attempt();
        for e in [started(), retry_marker(false), plain(true)] {
            w.write_all(&encoded(&e)).unwrap();
        }
        let starts: Vec<bool> = (0..3)
            .map(|_| rig.reader.next().unwrap().unwrap().starts_retried_attempt())
            .collect();
        assert_eq!(starts, vec![false, false, false], "the tail is not a start");
        drop(w);
        rig.next_attempt().write_all(&encoded(&started())).unwrap();
        assert!(rig.reader.next().unwrap().unwrap().starts_retried_attempt());
    }

    #[test]
    fn a_build_finished_without_an_exit_code_is_not_a_retry() {
        let mut rig = Rig::new();
        let event = BuildEvent {
            payload: Some(Payload::Finished(BuildFinished::default())),
            ..Default::default()
        };
        rig.next_attempt().write_all(&encoded(&event)).unwrap();
        assert!(!rig.reader.next().unwrap().unwrap().is_retry_marker());
    }

    #[test]
    fn last_message_on_a_retry_marker_does_not_end_the_stream() {
        let mut rig = Rig::new();
        let mut w = rig.next_attempt();
        w.write_all(&encoded(&started())).unwrap();
        w.write_all(&encoded(&retry_marker(true))).unwrap();
        for _ in 0..2 {
            rig.reader.next().unwrap().unwrap();
        }
        assert!(rig.reader.end().is_none(), "bazel is about to retry");
        drop(w);
    }

    #[test]
    fn both_attempts_of_a_retry_are_delivered() {
        let mut rig = Rig::new();
        {
            let mut w = rig.next_attempt();
            w.write_all(&encoded(&started())).unwrap();
            w.write_all(&encoded(&retry_marker(false))).unwrap();
        } // attempt one closes the file
        assert_eq!(rig.take(2).len(), 2);
        {
            let mut w = rig.next_attempt();
            w.write_all(&encoded(&started())).unwrap();
            w.write_all(&encoded(&build_finished(0, true))).unwrap();
        }
        assert_eq!(rig.frames(), vec![started(), build_finished(0, true)]);
        assert!(matches!(rig.reader.end(), Some(End::Complete)));
    }

    #[test]
    fn the_tail_of_a_retried_attempt_comes_before_the_next_attempt() {
        // The retry marker is not the attempt's last event: bazel goes on to
        // write metrics and its final event before it starts over.
        let mut rig = Rig::new();
        let mut w = rig.next_attempt();
        w.write_all(&encoded(&started())).unwrap();
        w.write_all(&encoded(&retry_marker(false))).unwrap();
        assert_eq!(rig.take(2), vec![started(), retry_marker(false)]);
        assert!(!rig.path.exists(), "the path is free for the next attempt");
        w.write_all(&encoded(&plain(false))).unwrap();
        w.write_all(&encoded(&plain(true))).unwrap();
        drop(w);
        let mut w = rig.next_attempt();
        w.write_all(&encoded(&started())).unwrap();
        w.write_all(&encoded(&build_finished(0, true))).unwrap();
        assert_eq!(
            rig.frames(),
            vec![
                plain(false),
                plain(true),
                started(),
                build_finished(0, true)
            ]
        );
        assert!(matches!(rig.reader.end(), Some(End::Complete)));
    }

    #[test]
    fn the_gap_between_attempts_is_a_pause_not_an_end() {
        let mut rig = Rig::new();
        rig.server_lets_go(); // nobody holds the file in the gap
        {
            let mut w = rig.next_attempt();
            w.write_all(&encoded(&started())).unwrap();
            w.write_all(&encoded(&retry_marker(false))).unwrap();
        }
        let path = rig.path.clone();
        let second_attempt = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            let mut w = File::create(&path).unwrap();
            w.write_all(&encoded(&started())).unwrap();
            w.write_all(&encoded(&build_finished(0, true))).unwrap();
        });
        let events = rig.frames();
        second_attempt.join().unwrap();
        assert_eq!(events.len(), 4, "the reader waited out the gap");
        assert!(matches!(rig.reader.end(), Some(End::Complete)));
    }

    #[test]
    fn a_retry_that_never_comes_ends_when_the_client_does() {
        let mut rig = Rig::new();
        rig.server_lets_go();
        {
            let mut w = rig.next_attempt();
            w.write_all(&encoded(&started())).unwrap();
            w.write_all(&encoded(&retry_marker(false))).unwrap();
        }
        rig.client_exits();
        assert_eq!(rig.frames().len(), 2);
        assert!(matches!(
            rig.reader.end(),
            Some(End::Incomplete(Done::Ended { .. }))
        ));
    }

    /// Bazel ran out of retries: the attempt that ended on the marker is the
    /// build's last. Its `last_message` completes the stream once the client
    /// has exited without starting another, whether or not `held` says the
    /// server still has the file open.
    #[test]
    fn a_retry_bazel_does_not_make_is_complete_once_the_client_exits() {
        let mut rig = Rig::new();
        let mut w = rig.next_attempt();
        for e in [started(), retry_marker(false), plain(true)] {
            w.write_all(&encoded(&e)).unwrap();
        }
        assert_eq!(rig.take(3).len(), 3);
        assert!(rig.reader.end().is_none(), "a retry can still start");
        rig.client_exits();
        assert!(rig.frames().is_empty());
        assert!(
            matches!(rig.reader.end(), Some(End::Complete)),
            "{:?}",
            rig.reader.end()
        );
        drop(w);
    }

    #[test]
    fn a_stream_that_breaks_after_an_exhausted_retry_is_not_complete() {
        let mut rig = Rig::new();
        rig.server_lets_go();
        rig.next_attempt()
            .write_all(&encoded(&retry_marker(true)))
            .unwrap();
        assert_eq!(rig.take(1).len(), 1);
        // The next file cannot be moved to its own name.
        let taken = moved_name(&rig.path, 2);
        std::fs::create_dir_all(taken.join("in-the-way")).unwrap();
        drop(rig.next_attempt());
        rig.client_exits();
        assert!(rig.frames().is_empty());
        assert!(
            matches!(rig.reader.end(), Some(End::Incomplete(Done::Broken { .. }))),
            "{:?}",
            rig.reader.end()
        );
        let _ = std::fs::remove_dir_all(&taken);
    }

    #[test]
    fn a_frame_the_old_attempt_left_half_written_is_dropped_not_spliced() {
        let mut rig = Rig::new();
        rig.server_lets_go();
        let half = encoded(&plain(false));
        {
            let mut w = rig.next_attempt();
            w.write_all(&encoded(&started())).unwrap();
            w.write_all(&encoded(&retry_marker(false))).unwrap();
            w.write_all(&half[..half.len() / 2]).unwrap();
        }
        assert_eq!(rig.take(2).len(), 2);
        {
            let mut w = rig.next_attempt();
            w.write_all(&encoded(&started())).unwrap();
            w.write_all(&encoded(&build_finished(0, true))).unwrap();
        }
        rig.client_exits();
        let frames: Vec<_> = rig.reader.by_ref().map(|f| f.unwrap()).collect();
        assert_eq!(frames.len(), 2);
        assert!(frames[0].starts_retried_attempt());
        assert_eq!(frames[1].event, build_finished(0, true));
        assert!(matches!(rig.reader.end(), Some(End::Complete)));
    }

    #[test]
    fn a_retry_whose_marker_was_cut_off_still_starts_a_retried_attempt() {
        let mut rig = Rig::new();
        rig.server_lets_go();
        let marker = encoded(&retry_marker(false));
        {
            let mut w = rig.next_attempt();
            w.write_all(&encoded(&started())).unwrap();
            w.write_all(&marker[..marker.len() - 1]).unwrap();
        }
        assert_eq!(rig.take(1), vec![started()]);
        {
            let mut w = rig.next_attempt();
            w.write_all(&encoded(&started())).unwrap();
            w.write_all(&encoded(&build_finished(0, true))).unwrap();
        }
        rig.client_exits();
        let frames: Vec<_> = rig.reader.by_ref().map(|f| f.unwrap()).collect();
        assert_eq!(frames.len(), 2);
        assert!(frames[0].starts_retried_attempt());
        assert!(matches!(rig.reader.end(), Some(End::Complete)));
    }

    #[test]
    fn a_new_attempt_s_empty_file_waits_out_the_window_once_the_client_exits() {
        let mut rig = Rig::new();
        {
            let mut w = rig.next_attempt();
            for e in [started(), retry_marker(true)] {
                w.write_all(&encoded(&e)).unwrap();
            }
        }
        assert_eq!(rig.take(2).len(), 2);
        rig.server_lets_go();
        drop(rig.next_attempt());
        rig.client_exits();
        let t = std::time::Instant::now();
        assert!(rig.frames().is_empty());
        assert!(
            matches!(rig.reader.end(), Some(End::Incomplete(Done::Ended { .. }))),
            "R4, not R2′ carried over from the old file: {:?}",
            rig.reader.end()
        );
        assert!(t.elapsed() >= QUIET_TEST);
    }

    #[test]
    fn a_retried_attempt_cut_off_before_its_first_event_is_incomplete() {
        let mut rig = Rig::new();
        rig.server_lets_go();
        {
            let mut w = rig.next_attempt();
            for e in [started(), retry_marker(true)] {
                w.write_all(&encoded(&e)).unwrap();
            }
        }
        assert_eq!(rig.take(2).len(), 2);
        drop(rig.next_attempt());
        rig.client_exits();
        assert!(rig.frames().is_empty());
        assert!(matches!(rig.reader.end(), Some(End::Incomplete(_))));
    }

    #[test]
    fn a_large_stream_arrives_whole_and_in_order() {
        let mut rig = Rig::new();
        let mut w = rig.next_attempt();
        let big = |i: u32| BuildEvent {
            id: None,
            children: Vec::new(),
            last_message: false,
            payload: Some(Payload::Finished(BuildFinished {
                exit_code: Some(ExitCode {
                    // Offset: 39 would be a retry marker.
                    code: 1000 + i as i32,
                    name: "x".repeat(10_000),
                }),
                ..Default::default()
            })),
            ..Default::default()
        };
        let writer = std::thread::spawn(move || {
            for i in 0..200 {
                w.write_all(&encoded(&big(i))).unwrap();
            }
            w.write_all(&encoded(&plain(true))).unwrap();
        });
        let events = rig.frames();
        writer.join().unwrap();
        assert_eq!(events.len(), 201, "well past one read's worth");
        for (i, e) in events[..200].iter().enumerate() {
            match &e.payload {
                Some(Payload::Finished(f)) => {
                    assert_eq!(f.exit_code.as_ref().unwrap().code, 1000 + i as i32)
                }
                other => panic!("unexpected payload {other:?}"),
            }
        }
    }

    /// The caller's BEP file is read from the command line and from a `common`
    /// rc entry alike, since Bazel honours both.
    #[test]
    fn requested_path_reads_the_binary_file_flag() {
        let line = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            requested_path(
                &line(&["--build_event_binary_file=/tmp/ide.bep"]),
                "build",
                None
            ),
            Some(PathBuf::from("/tmp/ide.bep")),
        );
        assert_eq!(
            requested_path(
                &line(&["--default_override=0:common=--build_event_binary_file=/tmp/rc.bep"]),
                "test",
                None,
            ),
            Some(PathBuf::from("/tmp/rc.bep")),
        );
        assert_eq!(
            requested_path(
                &line(&["--build_event_json_file=/tmp/x.json"]),
                "build",
                None
            ),
            None
        );
    }

    /// A relative path is under the workspace root, as Bazel resolves it, and
    /// a `~` in it is literal.
    #[test]
    fn a_relative_requested_path_is_under_the_workspace_root() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("MODULE.bazel"), "").unwrap();
        let pkg = ws.path().join("pkg");
        std::fs::create_dir(&pkg).unwrap();
        let root = ws.path().canonicalize().unwrap();
        for (value, under_root) in [("out/ide.bep", "out/ide.bep"), ("~/ide.bep", "~/ide.bep")] {
            let flags = vec![format!("{FLAG}={value}")];
            assert_eq!(
                requested_path(&flags, "build", pkg.to_str()),
                Some(root.join(under_root)),
                "{value}"
            );
        }
    }

    /// Outside any workspace a relative path is under the spawn directory.
    #[test]
    fn a_relative_requested_path_outside_a_workspace_is_under_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let flags = vec![format!("{FLAG}=ide.bep")];
        let directory = dir.path().to_str().unwrap();
        assert_eq!(
            requested_path(&flags, "build", Some(directory)),
            Some(dir.path().join("ide.bep"))
        );
    }
}
