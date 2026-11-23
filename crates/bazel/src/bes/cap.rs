//! A size limit on the build events the CLI passes on.
//!
//! BES backends refuse a message past their receive limit: 4 MiB is gRPC's
//! default, and silo's is 16 MiB. A refused event ends or exhausts the upload,
//! so the backend misses the rest of the build. Bazel writes the whole command
//! line of a failed action into its `ActionExecuted` event, params file
//! expanded, which for an action with a few hundred thousand arguments is tens
//! of MB.
//!
//! [`cap_event`] drops arguments from the end of such a command line until the
//! event fits in the limit. An event over the limit for any other reason is
//! passed on as it is. Either way it says so, for the caller to warn.
//!
//! The limit is set as the largest message a backend accepts
//! ([`DEFAULT_MAX_MESSAGE_BYTES`] unless configured), and held on the encoded
//! `BuildEvent`, without a length prefix. A gRPC sink sends the event packed in
//! an `Any` inside a `PublishBuildToolEventStreamRequest` (see
//! [`super::client::build_tool::bazel_event_encoded`]), so [`event_limit`]
//! leaves [`REQUEST_HEADROOM`] of the message for that wrapper.

use axl_proto::build_event_stream::BuildEvent;
use axl_proto::build_event_stream::build_event::Payload;
use axl_proto::build_event_stream::build_event_id::Id;
use prost::encoding::{encoded_len_varint, key_len};

use super::kind::payload_name;

/// gRPC's default receive limit, the message size held to unless configured.
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 4 << 20;

/// The smallest message size that can be configured.
pub const MIN_MAX_MESSAGE_BYTES: usize = 64 << 10;

/// Room left in a message for the request wrapping an event: at most about
/// 160 bytes with UUID build and invocation ids.
pub const REQUEST_HEADROOM: usize = 4 << 10;

/// The largest event passed on whole under [`DEFAULT_MAX_MESSAGE_BYTES`].
pub const MAX_EVENT_BYTES: usize = DEFAULT_MAX_MESSAGE_BYTES - REQUEST_HEADROOM;

/// The largest event passed on whole when backends accept messages of up to
/// `max_message_bytes`, which is at least [`MIN_MAX_MESSAGE_BYTES`].
pub fn event_limit(max_message_bytes: usize) -> usize {
    max_message_bytes.max(MIN_MAX_MESSAGE_BYTES) - REQUEST_HEADROOM
}

/// `ActionExecuted.command_line`'s field number.
const COMMAND_LINE_FIELD: u32 = 9;

/// What [`cap_event`] did to an event over the `limit` it was held to.
#[derive(Debug, PartialEq)]
pub enum Capped {
    /// The action's command line was cut to its first `kept` of `total`
    /// arguments; the event went from `before` to `after` bytes.
    CommandLine {
        action: String,
        kept: usize,
        total: usize,
        before: usize,
        after: usize,
        limit: usize,
    },
    /// Over the limit with nothing to cut; passed on unchanged.
    Oversize {
        kind: &'static str,
        bytes: usize,
        limit: usize,
    },
}

impl Capped {
    /// Whether the event was changed, and must be re-encoded.
    pub fn changed(&self) -> bool {
        matches!(self, Self::CommandLine { .. })
    }

    /// What to tell the user.
    pub fn warning(&self) -> String {
        match self {
            Self::CommandLine {
                action,
                kept,
                total,
                before,
                after,
                limit,
            } => {
                let mut msg = format!(
                    "the build event for {action} was {}, over the {} limit for build \
                     events; its command line was cut to the first {kept} of {total} arguments",
                    size(*before),
                    size(*limit),
                );
                if after > limit {
                    msg.push_str(&format!(
                        ", and the event is still {}, which BES backends may refuse",
                        size(*after)
                    ));
                }
                msg
            }
            Self::Oversize { kind, bytes, limit } => format!(
                "a build event of kind {kind} is {}, over the {} limit for build events; \
                 BES backends may refuse it",
                size(*bytes),
                size(*limit),
            ),
        }
    }
}

/// Hold `event`, `len` bytes encoded, to `limit` bytes by cutting its command
/// line; `None` when it fits already.
///
/// Arguments go from the end, each counted at its key, length and bytes. The
/// enclosing length prefixes only shrink as they go, so the event that results
/// fits, with at most one argument more removed than strictly needed. An
/// action too large without its command line loses all of it, and `after`
/// says it is still over.
pub fn cap_event(event: &mut BuildEvent, len: usize, limit: usize) -> Option<Capped> {
    if len <= limit {
        return None;
    }
    let action = match event.payload.as_mut() {
        Some(Payload::Action(action)) if !action.command_line.is_empty() => action,
        other => {
            let kind = other.map_or("payload-less", |p| payload_name(p));
            return Some(Capped::Oversize {
                kind,
                bytes: len,
                limit,
            });
        }
    };
    let total = action.command_line.len();
    let mut over = len - limit;
    while over > 0
        && let Some(arg) = action.command_line.pop()
    {
        let cost = key_len(COMMAND_LINE_FIELD) + encoded_len_varint(arg.len() as u64) + arg.len();
        over = over.saturating_sub(cost);
    }
    let kept = action.command_line.len();
    Some(Capped::CommandLine {
        action: describe_action(event),
        kept,
        total,
        before: len,
        after: prost::Message::encoded_len(event),
        limit,
    })
}

/// "//label (Mnemonic)" for an action event, with what of the two it has.
fn describe_action(event: &BuildEvent) -> String {
    let label = match event.id.as_ref().and_then(|id| id.id.as_ref()) {
        Some(Id::ActionCompleted(id)) => id.label.as_str(),
        _ => "",
    };
    let mnemonic = match &event.payload {
        Some(Payload::Action(action)) => action.r#type.as_str(),
        _ => "",
    };
    match (label.is_empty(), mnemonic.is_empty()) {
        (false, false) => format!("{label} ({mnemonic})"),
        (false, true) => label.to_owned(),
        (true, false) => format!("a {mnemonic} action"),
        (true, true) => "an action".to_owned(),
    }
}

/// `bytes` in MiB, or in KiB under 1 MiB, so a small configured limit does
/// not read as `0.1 MiB`.
fn size(bytes: usize) -> String {
    if bytes < 1 << 20 {
        format!("{:.1} KiB", bytes as f64 / (1 << 10) as f64)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1 << 20) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axl_proto::build_event_stream::{ActionExecuted, BuildEventId, Progress, build_event_id};
    use prost::Message;

    fn failed_action(args: usize, arg_len: usize) -> BuildEvent {
        BuildEvent {
            id: Some(BuildEventId {
                id: Some(Id::ActionCompleted(build_event_id::ActionCompletedId {
                    label: "//big:link".into(),
                    ..Default::default()
                })),
            }),
            payload: Some(Payload::Action(ActionExecuted {
                r#type: "CppLink".into(),
                exit_code: 1,
                command_line: (0..args)
                    .map(|i| format!("{i:0>width$}", width = arg_len))
                    .collect(),
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    #[test]
    fn an_event_within_the_limit_is_left_alone() {
        let mut event = failed_action(1000, 100);
        let before = event.clone();
        assert_eq!(
            {
                let len = event.encoded_len();
                cap_event(&mut event, len, MAX_EVENT_BYTES)
            },
            None
        );
        assert_eq!(event, before);
    }

    /// A failed action `bytes` long encoded, its last argument the padding.
    fn action_of(bytes: usize) -> BuildEvent {
        let mut event = failed_action(10, 10);
        let Some(Payload::Action(action)) = &mut event.payload else {
            unreachable!()
        };
        action.command_line.push(String::new());
        let mut pad = bytes - event.encoded_len();
        loop {
            let Some(Payload::Action(action)) = &mut event.payload else {
                unreachable!()
            };
            *action.command_line.last_mut().unwrap() = "p".repeat(pad);
            match event.encoded_len() {
                n if n == bytes => return event,
                n => pad = pad + bytes - n,
            }
        }
    }

    #[test]
    fn an_event_exactly_at_the_limit_is_left_alone() {
        let mut event = action_of(MAX_EVENT_BYTES);
        let before = event.clone();
        assert_eq!(
            cap_event(&mut event, MAX_EVENT_BYTES, MAX_EVENT_BYTES),
            None
        );
        assert_eq!(event, before);
    }

    #[test]
    fn an_event_a_byte_over_loses_only_its_last_argument() {
        let mut event = action_of(MAX_EVENT_BYTES + 1);
        let capped = cap_event(&mut event, MAX_EVENT_BYTES + 1, MAX_EVENT_BYTES).unwrap();
        assert!(
            matches!(
                capped,
                Capped::CommandLine {
                    kept: 10,
                    total: 11,
                    ..
                }
            ),
            "{capped:?}"
        );
        assert!(event.encoded_len() <= MAX_EVENT_BYTES);
    }

    /// The publish request a gRPC sink wraps an event at the limit in fits in
    /// [`DEFAULT_MAX_MESSAGE_BYTES`], even with ids far longer than UUIDs.
    #[test]
    fn an_event_at_the_limit_fits_in_a_publish_request() {
        let encoded = action_of(MAX_EVENT_BYTES).encode_to_vec();
        for id_len in [36, 1024] {
            let req = crate::bes::client::build_tool::bazel_event_encoded(
                "b".repeat(id_len),
                "i".repeat(id_len),
                i64::MAX,
                encoded.clone().into(),
                prost_types::Timestamp {
                    seconds: i64::MAX,
                    nanos: i32::MAX,
                },
            );
            assert!(req.encoded_len() <= DEFAULT_MAX_MESSAGE_BYTES, "{id_len}");
        }
    }

    #[test]
    fn a_long_command_line_is_cut_from_the_end_to_just_under_the_limit() {
        let mut event = failed_action(100_000, 100);
        let len = event.encoded_len();
        assert!(len > MAX_EVENT_BYTES);
        let capped = cap_event(&mut event, len, MAX_EVENT_BYTES).unwrap();
        let after = event.encoded_len();
        let Capped::CommandLine {
            kept,
            total,
            before,
            after: reported,
            ..
        } = &capped
        else {
            panic!("{capped:?}");
        };
        assert!(capped.changed());
        assert_eq!((*total, *before, *reported), (100_000, len, after));
        assert!(after <= MAX_EVENT_BYTES, "{after}");
        // One more argument would not have fit.
        assert!(after + 103 > MAX_EVENT_BYTES, "{after}");
        let Some(Payload::Action(action)) = &event.payload else {
            unreachable!()
        };
        assert_eq!(action.command_line.len(), *kept);
        assert_eq!(action.command_line[0], format!("{:0>100}", 0));
        assert_eq!(action.command_line[kept - 1], format!("{:0>100}", kept - 1));
        assert!(
            capped
                .warning()
                .starts_with("the build event for //big:link (CppLink) was 9.7 MiB"),
            "{}",
            capped.warning()
        );
        assert!(
            capped
                .warning()
                .ends_with(&format!("cut to the first {kept} of 100000 arguments")),
            "{}",
            capped.warning()
        );
    }

    #[test]
    fn an_action_too_large_without_its_command_line_says_so() {
        let mut event = failed_action(10, 10);
        let Some(Payload::Action(action)) = &mut event.payload else {
            unreachable!()
        };
        action.failure_detail = Some(axl_proto::failure_details::FailureDetail {
            message: "x".repeat(MAX_EVENT_BYTES),
            ..Default::default()
        });
        let capped = {
            let len = event.encoded_len();
            cap_event(&mut event, len, MAX_EVENT_BYTES)
        }
        .unwrap();
        assert!(
            matches!(
                capped,
                Capped::CommandLine {
                    kept: 0,
                    total: 10,
                    ..
                }
            ),
            "{capped:?}"
        );
        assert!(
            capped.warning().contains("which BES backends may refuse"),
            "{}",
            capped.warning()
        );
    }

    #[test]
    fn an_action_is_named_by_what_its_event_has() {
        let mut event = failed_action(0, 0);
        assert_eq!(describe_action(&event), "//big:link (CppLink)");
        let Some(Payload::Action(action)) = &mut event.payload else {
            unreachable!()
        };
        action.r#type.clear();
        assert_eq!(describe_action(&event), "//big:link");
        event.id = None;
        assert_eq!(describe_action(&event), "an action");
        let Some(Payload::Action(action)) = &mut event.payload else {
            unreachable!()
        };
        action.r#type = "Javac".into();
        assert_eq!(describe_action(&event), "a Javac action");
    }

    #[test]
    fn any_other_large_event_is_passed_on_unchanged() {
        let mut event = BuildEvent {
            payload: Some(Payload::Progress(Progress {
                stderr: "x".repeat(MAX_EVENT_BYTES + 1),
                ..Default::default()
            })),
            ..Default::default()
        };
        let before = event.clone();
        let capped = {
            let len = event.encoded_len();
            cap_event(&mut event, len, MAX_EVENT_BYTES)
        }
        .unwrap();
        assert_eq!(event, before);
        assert!(!capped.changed());
        assert_eq!(
            capped.warning(),
            "a build event of kind progress is 4.0 MiB, over the 4.0 MiB limit for build events; \
             BES backends may refuse it"
        );
    }

    #[test]
    fn a_configured_message_size_sets_the_limit() {
        assert_eq!(event_limit(DEFAULT_MAX_MESSAGE_BYTES), MAX_EVENT_BYTES);
        assert_eq!(event_limit(16 << 20), (16 << 20) - REQUEST_HEADROOM);
        assert_eq!(event_limit(1), MIN_MAX_MESSAGE_BYTES - REQUEST_HEADROOM);

        let limit = event_limit(16 << 20);
        let mut within = action_of(10 << 20);
        assert_eq!(cap_event(&mut within, 10 << 20, limit), None);
        let mut over = action_of(limit + 1);
        let capped = cap_event(&mut over, limit + 1, limit).unwrap();
        assert!(over.encoded_len() <= limit);
        assert!(
            capped.warning().contains("over the 16.0 MiB limit"),
            "{}",
            capped.warning()
        );
    }

    #[test]
    fn a_limit_under_a_mib_is_reported_in_kib() {
        let limit = event_limit(MIN_MAX_MESSAGE_BYTES);
        let mut over = action_of(100 << 10);
        let capped = cap_event(&mut over, 100 << 10, limit).unwrap();
        assert!(
            capped
                .warning()
                .contains("was 100.0 KiB, over the 60.0 KiB limit"),
            "{}",
            capped.warning()
        );
    }
}
