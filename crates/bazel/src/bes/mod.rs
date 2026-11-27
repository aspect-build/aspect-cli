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

/// The URI a BES `File` names it by; None for an inline one.
pub fn uri_of(file: &axl_proto::build_event_stream::File) -> Option<&str> {
    use axl_proto::build_event_stream::file;
    match file.file.as_ref()? {
        file::File::Uri(uri) => Some(uri),
        _ => None,
    }
}

/// The local path a `file://` URI names, `%XX` escapes decoded
/// ([`percent_decode`]); None for any other scheme. Bazel spells a
/// non-ASCII name's UTF-8 bytes in it each as a Latin-1 character, and
/// names the file on disk that way or as meant, by version: the URI's
/// spelling, unless only the one meant ([`repaired`]) exists.
pub fn local_path(uri: &str) -> Option<std::path::PathBuf> {
    let as_written = std::path::PathBuf::from(percent_decode(uri.strip_prefix("file://")?));
    match repaired(&as_written.to_string_lossy()) {
        Some(meant) if !as_written.exists() && std::path::Path::new(&meant).exists() => {
            Some(std::path::PathBuf::from(meant))
        }
        _ => Some(as_written),
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

/// `%XX` escapes decoded, as Bazel writes a file URI's path and the console
/// an editor link's: a `%` not followed by two hex digits is kept, and bytes
/// that decode to no UTF-8 are replaced (U+FFFD).
pub fn percent_decode(s: &str) -> String {
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
    /// A `file://` URI's path, decoded; none for another scheme.
    #[test]
    fn local_path_decodes_a_file_uri_only() {
        use std::path::PathBuf;
        assert_eq!(
            super::local_path("file:///tmp/a%20b/c"),
            Some(PathBuf::from("/tmp/a b/c"))
        );
        assert_eq!(super::local_path("bytestream://cache/blobs/x/1"), None);
        assert_eq!(super::local_path("/tmp/a"), None);
    }

    /// Of the URI's spelling of a non-ASCII name and the one Bazel meant,
    /// the one on disk; the URI's when neither or both are.
    #[test]
    fn local_path_takes_the_spelling_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let meant = "тест";
        let spelled: String = meant.bytes().map(char::from).collect();
        let uri = |name: &str| {
            let encoded: String = name
                .chars()
                .map(|c| {
                    // As Bazel writes it: a C1 character's UTF-8 escaped.
                    if ('\u{80}'..='\u{9f}').contains(&c) {
                        c.to_string().bytes().map(|b| format!("%{b:02X}")).collect()
                    } else {
                        c.to_string()
                    }
                })
                .collect();
            format!("file://{}/{encoded}.log", dir.path().display())
        };
        let at = |name: &str| dir.path().join(format!("{name}.log"));

        assert_eq!(
            super::local_path(&uri(&spelled)),
            Some(at(&spelled)),
            "neither"
        );
        std::fs::write(at(meant), "").unwrap();
        assert_eq!(
            super::local_path(&uri(&spelled)),
            Some(at(meant)),
            "as meant"
        );
        std::fs::write(at(&spelled), "").unwrap();
        assert_eq!(
            super::local_path(&uri(&spelled)),
            Some(at(&spelled)),
            "both"
        );
        std::fs::remove_file(at(meant)).unwrap();
        assert_eq!(
            super::local_path(&uri(&spelled)),
            Some(at(&spelled)),
            "as written"
        );
    }

    /// Escapes decoded; a `%` without two hex digits after it kept.
    #[test]
    fn percent_decode_keeps_a_bare_percent() {
        assert_eq!(super::percent_decode("a%20b"), "a b");
        assert_eq!(super::percent_decode("a%a\u{e9}.rs"), "a%a\u{e9}.rs");
        assert_eq!(super::percent_decode("%\u{e9}x"), "%\u{e9}x");
        assert_eq!(super::percent_decode("100%"), "100%");
        assert_eq!(super::percent_decode("%D1%82"), "т");
    }

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
