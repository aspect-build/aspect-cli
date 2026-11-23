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
//! event fits in [`MAX_EVENT_BYTES`]. An event over the limit for any other
//! reason is passed on as it is. Either way it says so, for the caller to warn.

use axl_proto::build_event_stream::BuildEvent;
use axl_proto::build_event_stream::build_event::Payload;
use axl_proto::build_event_stream::build_event_id::Id;
use prost::encoding::{encoded_len_varint, key_len};

use super::kind::payload_name;

/// The largest event passed on whole, in encoded bytes.
pub const MAX_EVENT_BYTES: usize = 4 << 20;

/// `ActionExecuted.command_line`'s field number.
const COMMAND_LINE_FIELD: u32 = 9;

/// What [`cap_event`] did to an event over the limit.
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
    },
    /// Over the limit with nothing to cut; passed on unchanged.
    Oversize { kind: &'static str, bytes: usize },
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
            } => {
                let mut msg = format!(
                    "the build event for {action} was {}, over the {} limit for build \
                     events; its command line was cut to the first {kept} of {total} arguments",
                    mib(*before),
                    mib(MAX_EVENT_BYTES),
                );
                if *after > MAX_EVENT_BYTES {
                    msg.push_str(&format!(
                        ", and the event is still {}, which BES backends may refuse",
                        mib(*after)
                    ));
                }
                msg
            }
            Self::Oversize { kind, bytes } => format!(
                "a {kind} build event is {}, over the {} limit for build events; \
                 BES backends may refuse it",
                mib(*bytes),
                mib(MAX_EVENT_BYTES),
            ),
        }
    }
}

/// Fit `event`, `len` bytes encoded, in [`MAX_EVENT_BYTES`] where its command
/// line is what can give; `None` when it fits already.
pub fn cap_event(event: &mut BuildEvent, len: usize) -> Option<Capped> {
    if len <= MAX_EVENT_BYTES {
        return None;
    }
    let action_label = match &event.id.as_ref().and_then(|id| id.id.as_ref()) {
        Some(Id::ActionCompleted(id)) => id.label.clone(),
        _ => String::new(),
    };
    let kind = event.payload.as_ref().map_or("payload-less", payload_name);
    let action = match event.payload.as_mut() {
        Some(Payload::Action(action)) if !action.command_line.is_empty() => action,
        _ => return Some(Capped::Oversize { kind, bytes: len }),
    };
    // Each argument costs its key, its length and its bytes. The enclosing
    // length prefixes only shrink as arguments go, so this is an upper bound.
    let total = action.command_line.len();
    let mut over = len - MAX_EVENT_BYTES;
    while over > 0
        && let Some(arg) = action.command_line.pop()
    {
        let cost = key_len(COMMAND_LINE_FIELD) + encoded_len_varint(arg.len() as u64) + arg.len();
        over = over.saturating_sub(cost);
    }
    let kept = action.command_line.len();
    let action = match (action_label.is_empty(), action.r#type.is_empty()) {
        (false, false) => format!("{action_label} ({})", action.r#type),
        (false, true) => action_label,
        (true, false) => format!("a {} action", action.r#type),
        (true, true) => "an action".to_owned(),
    };
    let after = prost::Message::encoded_len(event);
    Some(Capped::CommandLine {
        action,
        kept,
        total,
        before: len,
        after,
    })
}

fn mib(bytes: usize) -> String {
    format!("{:.1} MiB", bytes as f64 / (1 << 20) as f64)
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
                cap_event(&mut event, len)
            },
            None
        );
        assert_eq!(event, before);
    }

    #[test]
    fn a_long_command_line_is_cut_from_the_end_to_just_under_the_limit() {
        let mut event = failed_action(100_000, 100);
        let len = event.encoded_len();
        assert!(len > MAX_EVENT_BYTES);
        let capped = cap_event(&mut event, len).unwrap();
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
            cap_event(&mut event, len)
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
            cap_event(&mut event, len)
        }
        .unwrap();
        assert_eq!(event, before);
        assert!(!capped.changed());
        assert_eq!(
            capped.warning(),
            "a progress build event is 4.0 MiB, over the 4.0 MiB limit for build events; \
             BES backends may refuse it"
        );
    }
}
