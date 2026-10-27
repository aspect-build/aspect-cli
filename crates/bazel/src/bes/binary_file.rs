//! Bazel's `--build_event_binary_file`: the Build Event Protocol as
//! length-delimited `BuildEvent` messages, written by the server as the build
//! runs and, under `fully_async`, after the client has already exited.
//!
//! [`Reader`] turns the stream into frames and says, when it stops, whether
//! the stream was *complete* (bazel's final event, `last_message`, arrived)
//! or not, and if not, why.

use std::io::{self, ErrorKind};

use axl_proto::build_event_stream::BuildEvent;
use axl_proto::build_event_stream::build_event::Payload;
use prost::Message;
use prost::bytes::Bytes;

use crate::delimited;
use crate::stream::Done;
use crate::stream::pipe::Pipe;

/// `REMOTE_CACHE_EVICTED`: the build ends with this code and bazel runs it
/// again in the same invocation, reopening the file for the next attempt.
const REMOTE_CACHE_EVICTED: i32 = 39;

/// One event as it was read.
#[derive(Debug)]
pub struct Frame {
    pub event: BuildEvent,
    /// The frame exactly as written, length prefix included: what a mirror of
    /// the stream writes back out.
    pub raw: Bytes,
    prefix_len: usize,
    retry: bool,
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
    /// retry (`REMOTE_CACHE_EVICTED`): the next attempt starts a new stream
    /// in the same file.
    pub fn is_retry_marker(&self) -> bool {
        self.retry
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

/// Reads frames from a [`Pipe`].
///
/// An iterator over frames: `None` is the end, and [`Reader::end`] says which
/// kind. An `Err` item is a corrupt stream and is followed by `None`. A stream
/// cut off in the middle of a frame is an end, not an error: the frames before
/// it were good, and [`End::Incomplete`] says how it stopped.
pub struct Reader {
    pipe: Pipe,
    buf: Vec<u8>,
    /// An attempt ended with `REMOTE_CACHE_EVICTED` and the next has not
    /// started. Its `last_message` is not the end of the stream.
    retrying: bool,
    end: Option<End>,
}

impl Reader {
    pub fn new(pipe: Pipe) -> Self {
        Self {
            pipe,
            buf: Vec::with_capacity(5 * 1024),
            retrying: false,
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

    fn stop(&mut self, end: End) {
        self.end = Some(end);
    }

    /// The pipe's reason for stopping. It has one whenever it returned the
    /// zero-byte read that ended a frame read.
    fn incomplete(&mut self) -> End {
        End::Incomplete(self.pipe.take_done().unwrap_or(Done::Drained {
            bytes: self.pipe.bytes_read(),
        }))
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
        let header = match delimited::next_frame(&mut self.pipe, &mut self.buf) {
            Ok(Some(header)) => header,
            Ok(None) => {
                let end = self.incomplete();
                self.stop(end);
                return None;
            }
            // Cut off inside a frame: the pipe returned its zero-byte read.
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                let end = self.incomplete();
                self.stop(end);
                return None;
            }
            Err(e) => {
                self.stop(End::Corrupt(e.to_string()));
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
        if retry {
            self.retrying = true;
            self.pipe.expect_more(true);
        } else if matches!(event.payload, Some(Payload::Started(_))) {
            self.retrying = false;
            self.pipe.expect_more(false);
        }
        if event.last_message && !self.retrying {
            self.end = Some(End::Complete);
        }

        let mut raw = Vec::with_capacity(header.prefix.len() + header.len);
        raw.extend_from_slice(&header.prefix);
        raw.extend_from_slice(payload);
        Some(Ok(Frame {
            event,
            raw: Bytes::from(raw),
            prefix_len: header.prefix.len(),
            retry,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::Gate;
    use crate::stream::pipe::Channel;
    use axl_proto::build_event_stream::build_event::Payload;
    use axl_proto::build_event_stream::build_finished::ExitCode;
    use axl_proto::build_event_stream::{BuildFinished, BuildStarted};
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    fn started() -> BuildEvent {
        BuildEvent {
            payload: Some(Payload::Started(BuildStarted::default())),
            ..Default::default()
        }
    }

    fn finished(code: i32, last_message: bool) -> BuildEvent {
        BuildEvent {
            last_message,
            payload: Some(Payload::Finished(BuildFinished {
                exit_code: Some(ExitCode {
                    code,
                    ..Default::default()
                }),
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn plain(last_message: bool) -> BuildEvent {
        BuildEvent {
            last_message,
            ..Default::default()
        }
    }

    fn encoded(event: &BuildEvent) -> Vec<u8> {
        event.encode_length_delimited_to_vec()
    }

    /// A stream under test: the flags stand in for the client and server.
    struct Rig {
        reader: Reader,
        path: std::path::PathBuf,
        ended: Arc<AtomicBool>,
        held: Arc<AtomicBool>,
    }

    impl Rig {
        fn new() -> Self {
            let channel = Channel::at(
                std::env::temp_dir().join(format!("bazel-bes-test-{}.bin", uuid::Uuid::new_v4())),
            )
            .unwrap();
            let path = channel.path().to_path_buf();
            let ended = Arc::new(AtomicBool::new(false));
            let held = Arc::new(AtomicBool::new(true));
            let (e, h) = (ended.clone(), held.clone());
            let mut gate = Gate::new(move || e.load(Ordering::SeqCst))
                .with_held(move || h.load(Ordering::SeqCst));
            gate.quiet = Duration::from_millis(200);
            gate.quiet_empty = Duration::from_millis(100);
            let reader = Reader::new(channel.follow(gate).unwrap());
            Self {
                reader,
                path,
                ended,
                held,
            }
        }

        fn writer(&self) -> File {
            OpenOptions::new().write(true).open(&self.path).unwrap()
        }

        fn client_exits(&self) {
            self.ended.store(true, Ordering::SeqCst);
        }

        fn server_lets_go(&self) {
            self.held.store(false, Ordering::SeqCst);
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
        let mut w = rig.writer();
        for e in [started(), plain(false), finished(0, true)] {
            w.write_all(&encoded(&e)).unwrap();
        }
        let events = rig.frames();
        assert_eq!(events, vec![started(), plain(false), finished(0, true)]);
    }

    #[test]
    fn a_frame_carries_its_raw_bytes_and_its_payload() {
        let mut rig = Rig::new();
        let event = finished(7, true);
        rig.writer().write_all(&encoded(&event)).unwrap();
        let frame = rig.reader.next().unwrap().unwrap();
        assert_eq!(&frame.raw[..], &encoded(&event)[..]);
        assert_eq!(frame.payload(), &event.encode_to_vec()[..]);
        assert_eq!(BuildEvent::decode(frame.payload()).unwrap(), event);
    }

    #[test]
    fn payload_bytes_is_the_payload_without_a_copy() {
        let mut rig = Rig::new();
        rig.writer()
            .write_all(&encoded(&finished(3, true)))
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
            "every writer closed after 5 bytes"
        );
        assert_eq!(
            End::Corrupt("bad".into()).to_string(),
            "the stream was corrupt: bad"
        );
    }

    #[test]
    fn last_message_completes_the_stream_while_the_writer_is_still_attached() {
        let mut rig = Rig::new();
        let mut w = rig.writer();
        w.write_all(&encoded(&started())).unwrap();
        w.write_all(&encoded(&finished(0, true))).unwrap();
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
        let mut w = rig.writer();
        w.write_all(&encoded(&finished(0, true))).unwrap();
        w.write_all(&encoded(&plain(false))).unwrap();
        assert_eq!(rig.frames().len(), 1);
        assert!(rig.reader.next().is_none(), "the iterator is fused");
    }

    #[test]
    fn the_end_is_not_known_until_iteration_finishes() {
        let mut rig = Rig::new();
        rig.writer().write_all(&encoded(&started())).unwrap();
        assert!(rig.reader.end().is_none());
        rig.reader.next().unwrap().unwrap();
        assert!(rig.reader.end().is_none(), "mid-stream");
    }

    #[test]
    fn a_stream_that_stops_without_last_message_is_incomplete_not_failed() {
        let mut rig = Rig::new();
        {
            let mut w = rig.writer();
            w.write_all(&encoded(&started())).unwrap();
            w.write_all(&encoded(&plain(false))).unwrap();
        } // the writer closes without the final event
        rig.server_lets_go();
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
        let mut w = rig.writer();
        w.write_all(&encoded(&started())).unwrap();
        rig.client_exits();
        let tail = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            w.write_all(&encoded(&finished(0, true))).unwrap();
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
        let mut w = rig.writer();
        w.write_all(&encoded(&started())).unwrap();
        rig.client_exits();
        let t = Instant::now();
        assert_eq!(rig.frames().len(), 1);
        assert!(t.elapsed() >= Duration::from_millis(200));
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
            let mut w = rig.writer();
            w.write_all(&encoded(&started())).unwrap();
            let second = encoded(&finished(0, true));
            w.write_all(&second[..second.len() - 1]).unwrap();
        }
        rig.server_lets_go();
        assert_eq!(rig.frames(), vec![started()]);
        assert!(matches!(
            rig.reader.end(),
            Some(End::Incomplete(Done::Drained { .. }))
        ));
    }

    #[test]
    fn a_frame_that_does_not_decode_is_a_corrupt_stream() {
        let mut rig = Rig::new();
        let mut w = rig.writer();
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
        let mut w = rig.writer();
        // A varint for 2^40.
        w.write_all(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x20]).unwrap();
        let err = rig.reader.next().unwrap().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(matches!(rig.reader.end(), Some(End::Corrupt(_))));
    }

    #[test]
    fn a_failed_channel_is_an_incomplete_stream() {
        let mut rig = Rig::new();
        let mut w = rig.writer();
        w.write_all(&encoded(&started())).unwrap();
        assert!(rig.reader.next().unwrap().is_ok());
        std::fs::remove_file(&rig.path).unwrap();
        assert!(rig.reader.next().is_none());
        assert!(matches!(
            rig.reader.end(),
            Some(End::Incomplete(Done::Broken { .. }))
        ));
        drop(w);
    }

    #[test]
    fn the_retry_marker_is_recognised_only_for_remote_cache_evicted() {
        let mut rig = Rig::new();
        let mut w = rig.writer();
        for e in [finished(39, false), finished(0, false), plain(false)] {
            w.write_all(&encoded(&e)).unwrap();
        }
        let marks: Vec<bool> = (0..3)
            .map(|_| rig.reader.next().unwrap().unwrap().is_retry_marker())
            .collect();
        assert_eq!(marks, vec![true, false, false]);
    }

    #[test]
    fn a_build_finished_without_an_exit_code_is_not_a_retry() {
        let mut rig = Rig::new();
        let event = BuildEvent {
            payload: Some(Payload::Finished(BuildFinished::default())),
            ..Default::default()
        };
        rig.writer().write_all(&encoded(&event)).unwrap();
        assert!(!rig.reader.next().unwrap().unwrap().is_retry_marker());
    }

    #[test]
    fn last_message_on_a_retry_marker_does_not_end_the_stream() {
        let mut rig = Rig::new();
        let mut w = rig.writer();
        w.write_all(&encoded(&started())).unwrap();
        w.write_all(&encoded(&finished(39, true))).unwrap();
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
            let mut w = rig.writer();
            w.write_all(&encoded(&started())).unwrap();
            w.write_all(&encoded(&finished(39, false))).unwrap();
        } // attempt one closes the file
        {
            let mut w = rig.writer();
            w.write_all(&encoded(&started())).unwrap();
            w.write_all(&encoded(&finished(0, true))).unwrap();
        }
        let events = rig.frames();
        assert_eq!(events.len(), 4);
        assert!(matches!(rig.reader.end(), Some(End::Complete)));
    }

    #[test]
    fn the_gap_between_attempts_is_a_pause_not_an_end() {
        let mut rig = Rig::new();
        rig.server_lets_go(); // nobody holds the file in the gap
        {
            let mut w = rig.writer();
            w.write_all(&encoded(&started())).unwrap();
            w.write_all(&encoded(&finished(39, false))).unwrap();
        }
        let path = rig.path.clone();
        let second_attempt = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            let mut w = OpenOptions::new().write(true).open(&path).unwrap();
            w.write_all(&encoded(&started())).unwrap();
            w.write_all(&encoded(&finished(0, true))).unwrap();
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
            let mut w = rig.writer();
            w.write_all(&encoded(&started())).unwrap();
            w.write_all(&encoded(&finished(39, false))).unwrap();
        }
        rig.client_exits();
        assert_eq!(rig.frames().len(), 2);
        assert!(matches!(
            rig.reader.end(),
            Some(End::Incomplete(Done::Ended { .. }))
        ));
    }

    #[test]
    fn a_final_event_without_a_new_started_after_a_retry_marker_is_still_pending() {
        // Documented behaviour: only `BuildStarted` clears the retry, as it
        // always has.
        let mut rig = Rig::new();
        rig.server_lets_go();
        {
            let mut w = rig.writer();
            w.write_all(&encoded(&finished(39, false))).unwrap();
            w.write_all(&encoded(&finished(0, true))).unwrap();
        }
        rig.client_exits();
        assert_eq!(rig.frames().len(), 2);
        assert!(matches!(
            rig.reader.end(),
            Some(End::Incomplete(Done::Ended { .. }))
        ));
    }

    #[test]
    fn a_large_stream_arrives_whole_and_in_order() {
        let mut rig = Rig::new();
        let mut w = rig.writer();
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
        assert_eq!(events.len(), 201, "well past one pipe buffer's worth");
        for (i, e) in events[..200].iter().enumerate() {
            match &e.payload {
                Some(Payload::Finished(f)) => {
                    assert_eq!(f.exit_code.as_ref().unwrap().code, 1000 + i as i32)
                }
                other => panic!("unexpected payload {other:?}"),
            }
        }
    }
}
