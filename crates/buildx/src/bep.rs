//! Reading a binary Build Event Protocol file, and checking that its event
//! graph is consistent: tests and examples use it to see what a build emitted.

use axl_proto::build_event_stream::{BuildEvent, build_event, build_event_id};
use prost::Message;
use std::collections::{BTreeMap, HashSet};

/// The events of a binary BEP file (length-delimited `BuildEvent`s).
pub fn read_events(bytes: &[u8]) -> Result<Vec<BuildEvent>, prost::DecodeError> {
    let mut buf = bytes;
    let mut events = Vec::new();
    while !buf.is_empty() {
        events.push(BuildEvent::decode_length_delimited(&mut buf)?);
    }
    Ok(events)
}

/// The kind of an event, as its id's oneof names it (`started`, `named_set`, ...).
pub fn kind(e: &BuildEvent) -> &'static str {
    use build_event_id::Id;
    match e.id.as_ref().and_then(|i| i.id.as_ref()) {
        None => "none",
        Some(Id::Unknown(_)) => "unknown",
        Some(Id::Progress(_)) => "progress",
        Some(Id::Started(_)) => "started",
        Some(Id::UnstructuredCommandLine(_)) => "unstructured_command_line",
        Some(Id::StructuredCommandLine(_)) => "structured_command_line",
        Some(Id::WorkspaceStatus(_)) => "workspace_status",
        Some(Id::OptionsParsed(_)) => "options_parsed",
        Some(Id::Fetch(_)) => "fetch",
        Some(Id::Configuration(_)) => "configuration",
        Some(Id::TargetConfigured(_)) => "target_configured",
        Some(Id::Pattern(_)) => "pattern",
        Some(Id::PatternSkipped(_)) => "pattern_skipped",
        Some(Id::NamedSet(_)) => "named_set",
        Some(Id::TargetCompleted(_)) => "target_completed",
        Some(Id::ActionCompleted(_)) => "action_completed",
        Some(Id::UnconfiguredLabel(_)) => "unconfigured_label",
        Some(Id::ConfiguredLabel(_)) => "configured_label",
        Some(Id::TestResult(_)) => "test_result",
        Some(Id::TestProgress(_)) => "test_progress",
        Some(Id::TestSummary(_)) => "test_summary",
        Some(Id::TargetSummary(_)) => "target_summary",
        Some(Id::BuildFinished(_)) => "build_finished",
        Some(Id::BuildToolLogs(_)) => "build_tool_logs",
        Some(Id::BuildMetrics(_)) => "build_metrics",
        Some(Id::Workspace(_)) => "workspace",
        Some(Id::BuildMetadata(_)) => "build_metadata",
        Some(Id::ConvenienceSymlinksIdentified(_)) => "convenience_symlinks_identified",
        Some(Id::ExecRequest(_)) => "exec_request",
    }
}

/// How many events of each kind.
pub fn histogram(events: &[BuildEvent]) -> BTreeMap<&'static str, usize> {
    let mut h = BTreeMap::new();
    for e in events {
        *h.entry(kind(e)).or_insert(0) += 1;
    }
    h
}

/// Checks the graph the way a consumer that tolerates reordering reads it: `started`
/// first, every event announced by exactly one other event (anywhere in the stream)
/// and arriving exactly once, every announced event arrived, `last_message` only
/// on the final event, and `build_finished` present. Bazel itself posts events
/// from several threads, so under load a child can arrive before its parent;
/// `check_graph` is the strict order check.
pub fn check_complete(events: &[BuildEvent]) -> Result<(), String> {
    let Some(first) = events.first() else {
        return Err("no events".into());
    };
    if kind(first) != "started" {
        return Err(format!("the first event is {}, not started", kind(first)));
    }
    let mut announced: HashSet<Vec<u8>> = HashSet::new();
    let mut seen: HashSet<Vec<u8>> = HashSet::new();
    for (i, e) in events.iter().enumerate() {
        let id = e.id.as_ref().ok_or("an event without an id")?;
        if !seen.insert(id.encode_to_vec()) {
            return Err(format!("event {i} ({}) appears twice", kind(e)));
        }
        for c in &e.children {
            announced.insert(c.encode_to_vec());
        }
        let last = i + 1 == events.len();
        if e.last_message != last {
            return Err(format!(
                "event {i} ({}) has last_message = {} but is {}the last",
                kind(e),
                e.last_message,
                if last { "" } else { "not " }
            ));
        }
    }
    announced.insert(first.id.as_ref().expect("id").encode_to_vec());
    if let Some(i) = events
        .iter()
        .position(|e| !announced.contains(&e.id.as_ref().expect("id").encode_to_vec()))
    {
        return Err(format!(
            "event {i} ({}) was never announced by any event",
            kind(&events[i])
        ));
    }
    let missing = announced.iter().filter(|a| !seen.contains(*a)).count();
    if missing > 0 {
        return Err(format!("{missing} announced events never arrived"));
    }
    if !events.iter().any(|e| kind(e) == "build_finished") {
        return Err("no build_finished event".into());
    }
    Ok(())
}

/// Checks the graph in order: `started` first, every announced child present exactly once
/// and announced by an earlier event, `build_finished` present, and the last event
/// the only one with `last_message`.
pub fn check_graph(events: &[BuildEvent]) -> Result<(), String> {
    let Some(first) = events.first() else {
        return Err("no events".into());
    };
    if kind(first) != "started" {
        return Err(format!("the first event is {}, not started", kind(first)));
    }
    let mut announced: HashSet<Vec<u8>> = HashSet::new();
    let mut seen: HashSet<Vec<u8>> = HashSet::new();
    let id_bytes = |id: &axl_proto::build_event_stream::BuildEventId| id.encode_to_vec();
    for (i, e) in events.iter().enumerate() {
        let id = e.id.as_ref().ok_or("an event without an id")?;
        let key = id_bytes(id);
        if i > 0 && !announced.contains(&key) {
            return Err(format!(
                "event {i} ({}) was never announced by an earlier event",
                kind(e)
            ));
        }
        if !seen.insert(key) {
            return Err(format!("event {i} ({}) appears twice", kind(e)));
        }
        for c in &e.children {
            announced.insert(id_bytes(c));
        }
        let last = i + 1 == events.len();
        if e.last_message != last {
            return Err(format!(
                "event {i} ({}) has last_message = {} but is {}the last",
                kind(e),
                e.last_message,
                if last { "" } else { "not " }
            ));
        }
    }
    let missing: Vec<&Vec<u8>> = announced.iter().filter(|a| !seen.contains(*a)).collect();
    if !missing.is_empty() {
        return Err(format!("{} announced events never arrived", missing.len()));
    }
    if !events.iter().any(|e| kind(e) == "build_finished") {
        return Err("no build_finished event".into());
    }
    Ok(())
}

/// The exit code `build_finished` carries: (name, code).
pub fn exit_code(events: &[BuildEvent]) -> Option<(String, i32)> {
    events.iter().find_map(|e| match &e.payload {
        Some(build_event::Payload::Finished(f)) => {
            f.exit_code.as_ref().map(|c| (c.name.clone(), c.code))
        }
        _ => None,
    })
}
