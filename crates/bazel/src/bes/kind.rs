//! The kinds of BES event a filter can name.

use std::collections::HashSet;

use axl_proto::build_event_stream::BuildEvent;
use axl_proto::build_event_stream::build_event::Payload;

/// True when `event`'s payload kind is in `kinds`. A payload-less event never
/// matches a filter (there's no kind to test). Used to build the send-side
/// subscriber filter of an event iterator.
pub fn event_kind_in(event: &BuildEvent, kinds: &HashSet<i32>) -> bool {
    let Some(payload) = event.payload.as_ref() else {
        return false;
    };
    kinds.contains(&payload_discriminant(payload))
}

/// Every BES payload kind a `kinds=` list can name, as
/// `(tag, type_name, legacy_aliases)`.
///
/// `tag` is this runtime's payload discriminant: the integer
/// [`payload_discriminant`] returns and the set `kinds=` is matched against.
/// It is deliberately opaque and is *not* the BEP proto field number (the two
/// numberings diverge for most kinds); only the two sides agreeing matters,
/// which `payload_tags_agree_with_the_kind_table` pins.
///
/// `type_name` is the snake_case name of the payload *message*, which is
/// exactly the string `type(event.payload)` returns in AXL. Every kind is
/// reachable under that name, so a `kinds=` filter can be written with the
/// same literal the `type(event.payload) == "..."` test uses. Closing that
/// gap is why this table exists: `"action_completed"` was accepted while
/// `type()` reported `action_executed`, so the filter read correctly and the
/// comparison never matched.
///
/// `legacy_aliases` are the other spellings `kinds=` has always accepted —
/// the BEP `payload` oneof field name (`action`, `completed`, …), the
/// `event.kind` string where it differs (`action_completed`,
/// `target_completed`, …), and `named_set`, which is neither: it is the
/// `BuildEventId` oneof *field* name, accepted from before the others and kept
/// for that reason alone. They are a public AXL surface that third-party
/// `.aspect/*.axl` passes, so they stay accepted — frozen by
/// `the_spellings_accepted_before_the_payload_names_still_resolve`, which holds
/// its own literal list because a test derived from this table cannot notice a
/// row losing an alias.
pub const EVENT_KINDS: &[(i32, &str, &[&str])] = &[
    (3, "progress", &[]),
    (4, "aborted", &[]),
    (5, "build_started", &["started"]),
    (6, "pattern_expanded", &["expanded"]),
    (7, "target_configured", &["configured"]),
    (8, "action_executed", &["action", "action_completed"]),
    (9, "target_complete", &["completed", "target_completed"]),
    (10, "test_result", &[]),
    (11, "build_finished", &["finished"]),
    (12, "unstructured_command_line", &[]),
    (13, "command_line", &["structured_command_line"]),
    (14, "options_parsed", &[]),
    (15, "named_set_of_files", &["named_set"]),
    (16, "workspace_status", &[]),
    (17, "fetch", &[]),
    (19, "configuration", &[]),
    (20, "test_summary", &[]),
    (21, "build_tool_logs", &[]),
    (22, "build_metrics", &[]),
    (24, "build_metadata", &[]),
    (25, "workspace_config", &["workspace_info"]),
    (26, "target_summary", &[]),
    (27, "convenience_symlinks_identified", &[]),
    (28, "exec_request_constructed", &["exec_request"]),
    (30, "test_progress", &[]),
];

/// The tail of a `kinds=` rejection: every accepted spelling, with the
/// `type(event.payload)` names first because those are the ones that also
/// work as a `type()` comparison literal.
pub fn event_kind_help() -> String {
    let mut types: Vec<&str> = EVENT_KINDS.iter().map(|(_, name, _)| *name).collect();
    types.sort_unstable();
    let mut legacy: Vec<&str> = EVENT_KINDS
        .iter()
        .flat_map(|(_, _, aliases)| aliases.iter().copied())
        .collect();
    legacy.sort_unstable();
    format!(
        "name a payload the way `type(event.payload)` reports it — {} — or use \
         one of the older aliases, also accepted: {}",
        types.join(", "),
        legacy.join(", "),
    )
}

/// Resolve a payload tag given as an integer. A tag no row claims is
/// rejected rather than passed through, because it could only ever filter to
/// the empty set, and silently matching nothing is the failure mode worth
/// refusing.
///
/// This does **not** rescue someone who read an older docstring's claim that
/// these are BEP proto field numbers. They are this runtime's own numbering, and
/// 11 of the 25 real field numbers are themselves claimed tags meaning a
/// different kind — `7` (BEP `ActionExecuted`) selects `target_configured`,
/// `8` (`TargetComplete`) selects `action_executed`. Those stay silent, and no
/// validation can catch them. The integers are documented as opaque and
/// `event_kind_help()` lists only names, so there is no supported way to obtain
/// a correct one; the path survives for compatibility alone.
pub fn tag_from_int(n: i32) -> Result<i32, String> {
    if !EVENT_KINDS.iter().any(|(tag, _, _)| *tag == n) {
        return Err(format!(
            "unknown build_event payload tag {n}; {}",
            event_kind_help()
        ));
    }
    Ok(n)
}

/// Resolve a payload given by name: its `type(event.payload)` name or any
/// legacy alias.
pub fn tag_from_name(s: &str) -> Result<i32, String> {
    for (tag, type_name, aliases) in EVENT_KINDS {
        if s == *type_name || aliases.contains(&s) {
            return Ok(*tag);
        }
    }
    Err(format!(
        "unknown build_event kind '{s}'; {}",
        event_kind_help()
    ))
}

/// Maps a payload variant to this runtime's payload tag — the integers
/// [`EVENT_KINDS`] lists, so `kinds=` matches by integer set lookup.
pub fn payload_discriminant(p: &Payload) -> i32 {
    match p {
        Payload::Progress(_) => 3,
        Payload::Aborted(_) => 4,
        Payload::Started(_) => 5,
        Payload::UnstructuredCommandLine(_) => 12,
        Payload::StructuredCommandLine(_) => 13,
        Payload::OptionsParsed(_) => 14,
        Payload::WorkspaceStatus(_) => 16,
        Payload::Fetch(_) => 17,
        Payload::Configuration(_) => 19,
        Payload::Expanded(_) => 6,
        Payload::Configured(_) => 7,
        Payload::Action(_) => 8,
        Payload::NamedSetOfFiles(_) => 15,
        Payload::Completed(_) => 9,
        Payload::TestResult(_) => 10,
        Payload::TestSummary(_) => 20,
        Payload::TargetSummary(_) => 26,
        Payload::Finished(_) => 11,
        Payload::BuildToolLogs(_) => 21,
        Payload::BuildMetrics(_) => 22,
        Payload::WorkspaceInfo(_) => 25,
        Payload::BuildMetadata(_) => 24,
        Payload::ConvenienceSymlinksIdentified(_) => 27,
        Payload::ExecRequest(_) => 28,
        Payload::TestProgress(_) => 30,
    }
}
