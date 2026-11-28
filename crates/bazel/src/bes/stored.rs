//! A build event stream kept on disk, read back whole: the length-delimited
//! `BuildEvent` messages of a `--build_event_binary_file`, such as the one the
//! task history keeps in each build's folder (`build_events.binpb`), or the
//! lines of a `--build_event_json_file`, each a `BuildEvent` in proto3's JSON
//! form ([`Format`]), decoded by the descriptors the CLI was built with, so
//! a field newer than them is left out as the binary form's would be.
//!
//! Unlike [`super::binary_file::Reader`], which follows a file Bazel is still
//! writing, this reads one that is finished: it ends at the end of the file.
//! A stream that ends cleanly at a message boundary with Bazel's final event
//! (`last_message`) last is [`Stored::complete`]. One cut short (a build killed
//! mid-write), or holding a message that does not decode, stops at the last
//! whole event, with [`Stored::error`] saying why.

use std::fs::File;
use std::io::{self, BufRead, BufReader, ErrorKind, Read};
use std::path::Path;
use std::sync::LazyLock;

use axl_proto::build_event_stream::BuildEvent;
use prost::Message;
use prost_reflect::{DescriptorPool, DeserializeOptions, DynamicMessage, MessageDescriptor};

use crate::delimited;

/// The largest message read: a length past it is taken for a damaged file
/// rather than allocated.
const MAX_MESSAGE: usize = 256 << 20;

/// How a stream kept on disk is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// `--build_event_binary_file`'s: length-delimited messages.
    Binary,
    /// `--build_event_json_file`'s: a message a line, in proto3's JSON form.
    Json,
}

impl Format {
    /// The format named `name`: `binary` or `json`.
    pub fn parse(name: &str) -> Option<Format> {
        match name {
            "binary" => Some(Format::Binary),
            "json" => Some(Format::Json),
            _ => None,
        }
    }
}

/// The events of a stream kept on disk, in order.
pub struct Stored<R = BufReader<File>> {
    reader: R,
    format: Format,
    /// For JSON, the lines read so far, for an error to name.
    line: usize,
    done: bool,
    last_message: bool,
    error: Option<String>,
}

impl Stored {
    /// Read the stream at `path`, written in `format`.
    pub fn open(path: &Path, format: Format) -> io::Result<Self> {
        Ok(Self::with_format(BufReader::new(File::open(path)?), format))
    }
}

impl<R: BufRead> Stored<R> {
    /// A binary stream.
    pub fn new(reader: R) -> Self {
        Self::with_format(reader, Format::Binary)
    }

    /// A stream written in `format`.
    pub fn with_format(reader: R, format: Format) -> Self {
        Self {
            reader,
            format,
            line: 0,
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

impl<R: BufRead> Stored<R> {
    /// The next line's event of a JSON stream; blank lines are none.
    fn next_json(&mut self) -> Option<BuildEvent> {
        loop {
            let mut line = String::new();
            match self.reader.read_line(&mut line) {
                Ok(0) => {
                    self.done = true;
                    return None;
                }
                Ok(_) => self.line += 1,
                Err(e) => return self.stop(format!("reading line {}: {e}", self.line + 1)),
            }
            if line.trim().is_empty() {
                continue;
            }
            return match from_json(&line) {
                Ok(event) => {
                    self.last_message = event.last_message;
                    Some(event)
                }
                Err(_) if !line.ends_with('\n') => self.stop(format!(
                    "the stream ends partway through line {}: it was cut short",
                    self.line
                )),
                Err(e) => self.stop(format!("line {} does not decode: {e}", self.line)),
            };
        }
    }
}

/// The `BuildEvent` message's descriptor, from the set the CLI was built
/// with ([`axl_proto::FILE_DESCRIPTOR_SET`]).
static BUILD_EVENT: LazyLock<Result<MessageDescriptor, String>> = LazyLock::new(|| {
    DescriptorPool::decode(axl_proto::FILE_DESCRIPTOR_SET)
        .map_err(|e| format!("the build event descriptors do not decode: {e}"))?
        .get_message_by_name("build_event_stream.BuildEvent")
        .ok_or_else(|| "no BuildEvent among the descriptors".to_owned())
});

/// A `BuildEvent` from its proto3 JSON form, as `--build_event_json_file`
/// writes one a line. A field the descriptors do not know is left out.
pub fn from_json(text: &str) -> Result<BuildEvent, String> {
    let descriptor = BUILD_EVENT.clone()?;
    let mut de = serde_json::Deserializer::from_str(text);
    let message = DynamicMessage::deserialize_with_options(
        descriptor,
        &mut de,
        &DeserializeOptions::new().deny_unknown_fields(false),
    )
    .map_err(|e| e.to_string())?;
    de.end().map_err(|e| e.to_string())?;
    BuildEvent::decode(message.encode_to_vec().as_slice()).map_err(|e| e.to_string())
}

impl<R: BufRead> Iterator for Stored<R> {
    type Item = BuildEvent;

    fn next(&mut self) -> Option<BuildEvent> {
        if self.done {
            return None;
        }
        if self.format == Format::Json {
            return self.next_json();
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

    /// A JSON stream's lines read as the binary form's messages would, a
    /// field the descriptors do not know left out; one cut short or not JSON
    /// stops with an error.
    #[test]
    fn a_json_stream_reads_as_the_binary_one() {
        let text = concat!(
            r#"{"id":{"started":{}},"children":[{"pattern":{"pattern":["//..."]}}],"started":{"uuid":"u-1","startTimeMillis":"5","buildToolVersion":"9.3.3","newerField":{"x":1}}}"#,
            "\n\n",
            r#"{"id":{"actionCompleted":{"primaryOutput":"bazel-out/a","label":"//a:b"}},"action":{"success":false,"type":"Genrule","exitCode":1,"stderr":{"name":"stderr","uri":"file:///x/a%20b"},"failureDetail":{"message":"failed","spawn":{"code":"NON_ZERO_EXIT","spawnExitCode":3}}}}"#,
            "\n",
            r#"{"id":{"buildFinished":{}},"finished":{"exitCode":{"name":"BUILD_FAILURE","code":1}},"lastMessage":true}"#,
            "\n",
        );
        let mut stored = Stored::with_format(Cursor::new(text), Format::Json);
        let read: Vec<BuildEvent> = stored.by_ref().collect();
        assert!(stored.complete(), "{:?}", stored.error());
        assert_eq!(read.len(), 3);
        use axl_proto::build_event_stream::build_event::Payload;
        let Some(Payload::Started(started)) = &read[0].payload else {
            panic!("{:?}", read[0]);
        };
        assert_eq!(
            (started.uuid.as_str(), started.start_time_millis),
            ("u-1", 5)
        );
        let Some(Payload::Action(action)) = &read[1].payload else {
            panic!("{:?}", read[1]);
        };
        assert_eq!((action.exit_code, action.r#type.as_str()), (1, "Genrule"));
        let detail = action.failure_detail.as_ref().unwrap();
        assert_eq!(detail.message, "failed");
        let Some(Payload::Finished(finished)) = &read[2].payload else {
            panic!("{:?}", read[2]);
        };
        assert_eq!(finished.exit_code.as_ref().unwrap().code, 1);

        let mut cut = Stored::with_format(Cursor::new(&text[..text.len() - 10]), Format::Json);
        assert_eq!(cut.by_ref().count(), 2);
        assert!(
            cut.error().unwrap().contains("cut short"),
            "{:?}",
            cut.error()
        );
        let mut garbage = Stored::with_format(Cursor::new("{]\n"), Format::Json);
        assert_eq!(garbage.next(), None);
        assert!(
            garbage.error().unwrap().contains("line 1"),
            "{:?}",
            garbage.error()
        );
    }

    #[test]
    fn an_empty_file_is_no_events() {
        let mut stored = Stored::new(Cursor::new(Vec::new()));
        assert_eq!(stored.next(), None);
        assert!(!stored.complete() && stored.error().is_none());
    }
}
