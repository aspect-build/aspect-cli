//! The Build Event Protocol.

use std::sync::Arc;

use axl_proto::build_event_stream::BuildEvent;
use prost::bytes::Bytes;

pub mod binary_file;
pub mod cap;
pub mod client;
pub mod feed;
pub mod kind;
pub mod outputs;
pub mod redact;
pub mod sink;
pub mod stored;
pub mod warning;

#[cfg(test)]
mod testing;

/// One build event as a feed hands it to subscribers: decoded, and with it
/// the exact bytes it was decoded from (re-encoded when redaction or the size
/// cap of [`cap`] changed it).
/// A subscriber that only forwards events — the gRPC sink — takes the bytes
/// and never touches the proto, so a backlog behind a slow backend costs what
/// bazel wrote and no more; subscribers that read fields share the one decode
/// behind the `Arc`.
#[derive(Debug)]
pub struct BuildEventEnvelope {
    pub event: BuildEvent,
    /// The serialized `BuildEvent` message, without its length prefix.
    pub encoded: Bytes,
}

impl BuildEventEnvelope {
    /// The decoded event, moved out when this was the last reference and
    /// cloned otherwise.
    pub fn into_event(this: Arc<Self>) -> BuildEvent {
        match Arc::try_unwrap(this) {
            Ok(envelope) => envelope.event,
            Err(shared) => shared.event.clone(),
        }
    }
}

/// The local path a BES `File` names by `file://` URI, `%XX` escapes
/// decoded; None for a remote or inline one. Bazel spells a non-ASCII
/// name's UTF-8 bytes in it each as a Latin-1 character, and names the
/// file on disk that way or as meant, by version: the spelling that is
/// there, the URI's when neither is.
pub fn local_path(file: &axl_proto::build_event_stream::File) -> Option<std::path::PathBuf> {
    use axl_proto::build_event_stream::file;
    match file.file.as_ref()? {
        file::File::Uri(uri) => uri.strip_prefix("file://").map(|p| {
            let as_written = std::path::PathBuf::from(percent_decode(p));
            match repaired(&as_written.to_string_lossy()) {
                Some(meant) if !as_written.exists() && std::path::Path::new(&meant).exists() => {
                    std::path::PathBuf::from(meant)
                }
                _ => as_written,
            }
        }),
        _ => None,
    }
}

/// `text` as Bazel meant it, when Bazel's build events carried its UTF-8
/// bytes each as a Latin-1 character (`ÐµÐ` for `е`): every character such a
/// byte, at least one past ASCII, and the bytes valid UTF-8; else None.
pub fn repaired(text: &str) -> Option<String> {
    if text.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> = text
        .chars()
        .map(|c| u8::try_from(u32::from(c)).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

/// `%XX` escapes decoded, as Bazel writes a file URI's path.
pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push(h << 4 | l);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

#[cfg(test)]
mod tests {
    /// Bazel's Latin-1 spelling of UTF-8 decoded back; other text left be.
    #[test]
    fn repaired_decodes_bazels_spelling_only() {
        let spelled: String = "тест".bytes().map(char::from).collect();
        assert_eq!(super::repaired(&spelled).as_deref(), Some("тест"));
        assert_eq!(super::repaired("plain"), None);
        assert_eq!(super::repaired("тест"), None);
        assert_eq!(super::repaired("caf\u{e9}"), None);
    }
}
