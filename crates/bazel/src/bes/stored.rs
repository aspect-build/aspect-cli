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
}
