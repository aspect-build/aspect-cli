//! A build event stream kept on disk, read back whole: the length-delimited
//! `BuildEvent` messages of a `--build_event_binary_file`, such as the one the
//! task history keeps in each build's folder (`build_events.binpb`).
//!
//! Unlike [`super::binary_file::Reader`], which follows a file Bazel is still
//! writing, this reads one that is finished: it ends at the end of the file.
//! A stream that ends cleanly at a message boundary with Bazel's final event
//! (`last_message`) last is [`Stored::complete`]. One cut short (a build killed
//! mid-write), or holding a message that does not decode, stops at the last
//! whole event, with [`Stored::error`] saying why.
//!
//! [`Follower`] reads one that is still being written, as far as it has got,
//! without waiting: each [`Follower::poll`] hands out the whole events written
//! since the last, keeping a message not yet whole for the next.

use std::fs::File;
use std::io::{self, BufReader, ErrorKind, Read};
use std::path::Path;

use axl_proto::build_event_stream::BuildEvent;
use prost::Message;

use crate::delimited;

/// The largest message read: a length past it is taken for a damaged file
/// rather than allocated.
const MAX_MESSAGE: usize = 256 << 20;

/// The events of a stream kept on disk, in order.
pub struct Stored<R = BufReader<File>> {
    reader: R,
    done: bool,
    last_message: bool,
    error: Option<String>,
}

impl Stored {
    /// Read the stream at `path`.
    pub fn open(path: &Path) -> io::Result<Self> {
        Ok(Self::new(BufReader::new(File::open(path)?)))
    }
}

impl<R: Read> Stored<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            done: false,
            last_message: false,
            error: None,
        }
    }

    /// Whether the stream was read to its end and the last event in it was
    /// Bazel's final one: false until it has been read, for a stream cut
    /// short, and for one whose retried attempt stopped before its own end.
    pub fn complete(&self) -> bool {
        self.done && self.error.is_none() && self.last_message
    }

    /// Why reading stopped before the end of the file, if it did.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn stop(&mut self, why: String) -> Option<BuildEvent> {
        self.done = true;
        self.error = Some(why);
        None
    }
}

impl<R: Read> Iterator for Stored<R> {
    type Item = BuildEvent;

    fn next(&mut self) -> Option<BuildEvent> {
        if self.done {
            return None;
        }
        // The end of the file at a message boundary is the end; one partway
        // through a length is a stream cut short.
        let mut first = [0];
        match self.reader.read(&mut first) {
            Ok(0) => {
                self.done = true;
                return None;
            }
            Ok(_) => {}
            Err(e) => return self.stop(format!("reading a message's length: {e}")),
        }
        let len = match delimited::read_varint(&mut (&first[..]).chain(&mut self.reader)) {
            Ok((len, _)) => len,
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                return self.stop(
                    "the stream ends partway through a message: it was cut short".to_owned(),
                );
            }
            Err(e) => return self.stop(format!("reading a message's length: {e}")),
        };
        if len > MAX_MESSAGE {
            return self.stop(format!("a message of {len} bytes: the file is damaged"));
        }
        let mut payload = vec![0; len];
        if let Err(e) = self.reader.read_exact(&mut payload) {
            return self.stop(if e.kind() == ErrorKind::UnexpectedEof {
                "the stream ends partway through a message: it was cut short".to_owned()
            } else {
                format!("reading a message: {e}")
            });
        }
        match BuildEvent::decode(payload.as_slice()) {
            Ok(event) => {
                // The last event read decides: a retry in the stream starts
                // over after an attempt's own final event.
                self.last_message = event.last_message;
                Some(event)
            }
            Err(e) => self.stop(format!("a message that does not decode: {e}")),
        }
    }
}

/// A stream on disk read as it is written: each [`Follower::poll`] gives the
/// whole events added since the last, the start of one not yet whole kept
/// for the next, never waiting. A file not there yet is no events. A file
/// written over from its start (a retried attempt's, truncated in place) is
/// read from its start again, [`Follower::restarts`] counting it: told by
/// its first message changing (the attempt's start, with its own build id
/// and start time), or its being shorter than what was read, as the new
/// attempt may already have outgrown the old by the next poll.
pub struct Follower {
    path: std::path::PathBuf,
    offset: u64,
    pending: Vec<u8>,
    /// The file's first message as read, its length too, up to [`HEAD_MAX`].
    head: Vec<u8>,
    last_message: bool,
    error: Option<String>,
    restarts: u32,
}

/// The most of a file's first message kept to tell one attempt's from the
/// next: that event, with the build's id and start time, differs, however
/// far into it its target patterns push them.
const HEAD_MAX: usize = 64 << 10;

/// The most of a file one [`Follower::poll`] reads, so a big backlog comes
/// in pieces.
const POLL_MAX: u64 = 16 << 20;

impl Follower {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            offset: 0,
            pending: vec![],
            head: vec![],
            last_message: false,
            error: None,
            restarts: 0,
        }
    }

    /// Whether the last event read was Bazel's final one.
    pub fn complete(&self) -> bool {
        self.error.is_none() && self.last_message
    }

    /// Why following stopped, if it did: a message that does not decode.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// How often the file started over.
    pub fn restarts(&self) -> u32 {
        self.restarts
    }

    /// The whole events written since the last poll, at most `limit`, in
    /// order; the rest wait for the next.
    pub fn poll(&mut self, limit: usize) -> Vec<BuildEvent> {
        use std::io::{Seek, SeekFrom};
        let mut out = vec![];
        let Ok(mut file) = File::open(&self.path) else {
            return out;
        };
        let Ok(len) = file.metadata().map(|m| m.len()) else {
            return out;
        };
        if self.started_over(&mut file, len) {
            self.offset = 0;
            self.pending.clear();
            self.head.clear();
            self.last_message = false;
            self.error = None;
            self.restarts += 1;
        }
        if self.error.is_some() {
            return out;
        }
        // Whole messages already read come first.
        self.decode(limit, &mut out);
        if out.len() >= limit {
            return out;
        }
        if len > self.offset && file.seek(SeekFrom::Start(self.offset)).is_ok() {
            let mut more = vec![];
            if (&mut file).take(POLL_MAX).read_to_end(&mut more).is_ok() {
                if self.head.len() as u64 == self.offset {
                    self.extend_head(&more);
                }
                self.offset += more.len() as u64;
                self.pending.extend(more);
            }
        }
        self.decode(limit, &mut out);
        out
    }

    /// Take what of `more`, read right after the head, belongs to the first
    /// message: its length, then as much of it as [`HEAD_MAX`] allows.
    fn extend_head(&mut self, mut more: &[u8]) {
        loop {
            let target = match varint(&self.head) {
                Some((len, used)) => used.saturating_add(len).min(HEAD_MAX),
                None => HEAD_MAX,
            };
            let take = target.saturating_sub(self.head.len()).min(more.len());
            if take == 0 {
                return;
            }
            // A byte at a time until the length is whole, so the target
            // is the message's as soon as it is known.
            let take = if varint(&self.head).is_none() {
                1
            } else {
                take
            };
            self.head.extend_from_slice(&more[..take]);
            more = &more[take..];
        }
    }

    /// Whether the file at `len` bytes was written over since it was read:
    /// shorter than what was read, or its first message no longer that.
    fn started_over(&self, file: &mut File, len: u64) -> bool {
        use std::io::{Seek, SeekFrom};
        if len < self.offset {
            return true;
        }
        if self.head.is_empty() {
            return false;
        }
        let mut now = vec![0; self.head.len()];
        let read = file
            .seek(SeekFrom::Start(0))
            .and_then(|_| file.read_exact(&mut now));
        read.is_ok() && now != self.head
    }

    /// Take whole messages off the front of what was read, up to `limit`
    /// in `out`.
    fn decode(&mut self, limit: usize, out: &mut Vec<BuildEvent>) {
        let mut at = 0;
        while out.len() < limit {
            let Some((len, used)) = varint(&self.pending[at..]) else {
                break;
            };
            if len > MAX_MESSAGE {
                self.error = Some(format!("a message of {len} bytes: the file is damaged"));
                break;
            }
            let start = at + used;
            if self.pending.len() < start + len {
                break;
            }
            match BuildEvent::decode(&self.pending[start..start + len]) {
                Ok(event) => {
                    self.last_message = event.last_message;
                    out.push(event);
                }
                Err(e) => {
                    self.error = Some(format!("a message that does not decode: {e}"));
                    break;
                }
            }
            at = start + len;
        }
        self.pending.drain(..at);
    }
}

/// A length's varint at the start of `bytes` and how many bytes it took;
/// None while it is not whole.
fn varint(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut value: u64 = 0;
    for (i, &b) in bytes.iter().enumerate().take(10) {
        value |= u64::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            return Some((usize::try_from(value).unwrap_or(usize::MAX), i + 1));
        }
    }
    if bytes.len() >= 10 {
        return Some((usize::MAX, 10));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn event(n: i64, last: bool) -> BuildEvent {
        BuildEvent {
            last_message: last,
            payload: Some(
                axl_proto::build_event_stream::build_event::Payload::Progress(
                    axl_proto::build_event_stream::Progress {
                        stdout: format!("line {n}"),
                        ..Default::default()
                    },
                ),
            ),
            ..Default::default()
        }
    }

    fn file(events: &[BuildEvent]) -> Vec<u8> {
        events
            .iter()
            .flat_map(|e| e.encode_length_delimited_to_vec())
            .collect()
    }

    #[test]
    fn a_whole_stream_reads_back_in_order_and_is_complete() {
        let events = [event(1, false), event(2, false), event(3, true)];
        let mut stored = Stored::new(Cursor::new(file(&events)));
        let read: Vec<BuildEvent> = stored.by_ref().collect();
        assert_eq!(read, events);
        assert!(stored.complete() && stored.error().is_none());
    }

    /// A retry in the stream starts over after an attempt's final event: an
    /// attempt that then stopped at a message boundary leaves it incomplete.
    #[test]
    fn only_a_final_last_event_completes_it() {
        let mut stored = Stored::new(Cursor::new(file(&[event(1, true), event(2, false)])));
        assert_eq!(stored.by_ref().count(), 2);
        assert!(!stored.complete() && stored.error().is_none());
    }

    #[test]
    fn a_stream_without_its_last_message_is_not_complete() {
        let mut stored = Stored::new(Cursor::new(file(&[event(1, false)])));
        assert_eq!(stored.by_ref().count(), 1);
        assert!(!stored.complete() && stored.error().is_none());
    }

    #[test]
    fn a_stream_cut_short_stops_at_the_last_whole_event() {
        let mut bytes = file(&[event(1, false), event(2, true)]);
        bytes.truncate(bytes.len() - 2);
        let mut stored = Stored::new(Cursor::new(bytes));
        assert_eq!(stored.by_ref().count(), 1);
        assert!(!stored.complete());
        assert!(
            stored.error().unwrap().contains("cut short"),
            "{:?}",
            stored.error()
        );
    }

    #[test]
    fn a_stream_cut_partway_through_a_length_is_cut_short() {
        let mut bytes = file(&[event(1, false)]);
        bytes.push(0x80);
        let mut stored = Stored::new(Cursor::new(bytes));
        assert_eq!(stored.by_ref().count(), 1);
        assert!(stored.error().unwrap().contains("cut short"));
    }

    #[test]
    fn garbage_stops_with_an_error() {
        let mut bytes = file(&[event(1, false)]);
        bytes.extend([0x03, 0xff, 0xff, 0xff]);
        let mut stored = Stored::new(Cursor::new(bytes));
        assert_eq!(stored.by_ref().count(), 1);
        assert!(stored.error().unwrap().contains("does not decode"));
        let mut huge = Stored::new(Cursor::new(vec![0xff, 0xff, 0xff, 0xff, 0x7f]));
        assert_eq!(huge.next(), None);
        assert!(huge.error().unwrap().contains("damaged"));
    }

    #[test]
    fn an_empty_file_is_no_events() {
        let mut stored = Stored::new(Cursor::new(Vec::new()));
        assert_eq!(stored.next(), None);
        assert!(!stored.complete() && stored.error().is_none());
    }

    /// Written a piece at a time, it hands out each event once whole, then
    /// follows the file starting over, as a retried attempt does.
    #[test]
    fn a_follower_reads_what_is_written_as_it_comes() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.binpb");
        let mut follower = Follower::new(&path);
        assert!(follower.poll(10).is_empty(), "no file yet, no events");
        let bytes = file(&[event(1, false), event(2, false), event(3, true)]);
        let mut f = std::fs::File::create(&path).unwrap();
        let first = bytes.len() / 3 + 2;
        f.write_all(&bytes[..first]).unwrap();
        assert_eq!(follower.poll(10), [event(1, false)]);
        assert!(follower.poll(10).is_empty());
        f.write_all(&bytes[first..]).unwrap();
        assert_eq!(follower.poll(1), [event(2, false)], "at most the limit");
        assert_eq!(follower.poll(10), [event(3, true)]);
        assert!(follower.complete() && follower.error().is_none());
        drop(f);
        std::fs::write(&path, file(&[event(4, false)])).unwrap();
        assert_eq!(follower.poll(10), [event(4, false)]);
        assert_eq!(follower.restarts(), 1);
        assert!(!follower.complete());

        // Written over again, the new attempt already longer than the old
        // by the next poll, its first message the same for its first 300
        // bytes (many target patterns ahead of its build id).
        let started = |n: i64| BuildEvent {
            payload: Some(
                axl_proto::build_event_stream::build_event::Payload::Progress(
                    axl_proto::build_event_stream::Progress {
                        stderr: "p".repeat(300) + &n.to_string(),
                        ..Default::default()
                    },
                ),
            ),
            ..Default::default()
        };
        std::fs::write(&path, file(&[started(1)])).unwrap();
        let mut same_start = Follower::new(&path);
        assert_eq!(same_start.poll(10), [started(1)]);
        let after: Vec<_> = std::iter::once(started(2))
            .chain((20..30).map(|n| event(n, false)))
            .collect();
        std::fs::write(&path, file(&after)).unwrap();
        assert_eq!(same_start.poll(100), after);
        assert_eq!(same_start.restarts(), 1);
        std::fs::write(&path, file(&[event(4, false)])).unwrap();
        follower.poll(10);
        let longer: Vec<_> = (5..12).map(|n| event(n, false)).collect();
        std::fs::write(&path, file(&longer)).unwrap();
        assert_eq!(follower.poll(100), longer);
        assert_eq!(follower.restarts(), 2);
    }

    #[test]
    fn a_follower_stops_at_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.binpb");
        let mut bytes = file(&[event(1, false)]);
        bytes.extend([0x03, 0xff, 0xff, 0xff]);
        std::fs::write(&path, bytes).unwrap();
        let mut follower = Follower::new(&path);
        assert_eq!(follower.poll(10), [event(1, false)]);
        assert!(follower.error().unwrap().contains("does not decode"));
        assert!(follower.poll(10).is_empty());
    }
}
