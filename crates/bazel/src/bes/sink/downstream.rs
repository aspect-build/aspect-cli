//! What a build's downstream consumers get when the CLI turned on
//! `--build_event_publish_all_actions` for its own readers: the stream as it
//! would have been without the flag. A successful action's `ActionExecuted`
//! event is dropped, and so is its id from the `children` of whichever event
//! announced it, so nothing downstream is told of an event that never comes.
//! Everything else goes through, in Bazel's order. A failed action is kept, as
//! Bazel reports one without the flag.
//!
//! Bazel announces an action (in a `Progress` event's `children`) just before
//! it posts it, but whether it succeeded is only known once it is posted, and
//! a burst of announcements can precede the actions they name. So an event
//! announcing an action not yet posted is held, with everything after it, until
//! each action it announced has been posted; then the held events go out in
//! order, rewritten. Holds last a few events. One that reaches [`MAX_HELD`]
//! events or [`MAX_HOLD`] is let go with its open children in it, and those
//! actions are then passed on whatever their outcome, so what downstream is
//! told of always arrives.
//!
//! The fate of every action posted is kept, as a 64-bit hash of its id (a few
//! tens of bytes an action, per consumer), so an announcement arriving after
//! the action, as a failed target's `TargetComplete` does, is read right. A
//! new attempt in the same stream, starting with `Started`, lets go of what is
//! held and starts over, keeping only what was promised.
//!
//! A gRPC sink numbers its own events, so a dropped event leaves no gap there.
//!
//! Who asked for every action is read here too: a feature, through
//! [`CLI_REQUEST`] on the task's flags ([`take_cli_request`]), or the caller,
//! whose own ask means nothing is trimmed ([`asks_for_all_actions`]).

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axl_proto::build_event_stream::BuildEventId;
use axl_proto::build_event_stream::build_event::Payload;
use axl_proto::build_event_stream::build_event_id::{ActionCompletedId, Id};
use prost::Message;
use prost::bytes::Bytes;

use crate::bes::BuildEventEnvelope;

/// The flag that has Bazel report every action, not only the failed ones.
pub const PUBLISH_ALL_ACTIONS: &str = "--build_event_publish_all_actions";

/// The most events held at once before the oldest is let go.
pub const MAX_HELD: usize = 1024;

/// The longest an event is held before it is let go, so a gRPC sink, which
/// [`Trim::tick`]s while the stream is quiet, never keeps its backend waiting
/// on an action that is slow to be posted. A file sink is let go of on its
/// next event instead.
pub const MAX_HOLD: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fate {
    /// Announced, not yet posted.
    Open,
    /// Announced downstream before it was posted, so passed on whatever its
    /// outcome.
    Promised,
    /// Posted and dropped: it succeeded.
    Dropped,
    /// Posted and passed on.
    Kept,
}

/// The downstream stage of one consumer. Feed it every event with
/// [`push`](Self::push), [`tick`](Self::tick) it while the stream is quiet,
/// and [`flush`](Self::flush) it once the stream ends.
#[derive(Default)]
pub struct Trim {
    /// Each event held, with when it was.
    held: VecDeque<(Instant, Arc<BuildEventEnvelope>)>,
    actions: HashMap<u64, Fate>,
}

fn action_key(id: &ActionCompletedId) -> u64 {
    let mut h = DefaultHasher::new();
    id.primary_output.hash(&mut h);
    id.label.hash(&mut h);
    id.configuration
        .as_ref()
        .map(|c| c.id.as_str())
        .hash(&mut h);
    h.finish()
}

fn action_of(id: &BuildEventId) -> Option<&ActionCompletedId> {
    match id.id.as_ref() {
        Some(Id::ActionCompleted(a)) => Some(a),
        _ => None,
    }
}

/// The `--build_event_publish_all_actions` value a feature injects to ask for
/// every action for the CLI's own readers. It rides the task's flags, so the
/// user's flag filters apply to it, and [`take_cli_request`] takes it off the
/// command line, so it never counts as the caller's ask.
pub const CLI_REQUEST: &str = "%aspect_cli%";

/// `flags` without any [`CLI_REQUEST`], and whether there was one.
pub fn take_cli_request(flags: Vec<String>) -> (Vec<String>, bool) {
    crate::rc::take_flag(flags, PUBLISH_ALL_ACTIONS, CLI_REQUEST)
}

/// Whether `flags`, a command line rendered for `verb` (the caller's flags
/// and its rc, `--config`s expanded), ask Bazel to report every action: the
/// last word on [`PUBLISH_ALL_ACTIONS`] is a yes. A literal beats an rc entry,
/// and the last of each wins, so `--no…` or `=false` turns an earlier one off.
pub fn asks_for_all_actions(flags: &[String], verb: &str) -> bool {
    crate::rc::last_word(flags, verb, |token| {
        crate::rc::bool_token(token, PUBLISH_ALL_ACTIONS).map(|word| word == Ok(true))
    }) == Some(true)
}

impl Trim {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take the next event of the stream, and hand `out` whatever can go
    /// downstream now.
    pub fn push(
        &mut self,
        envelope: Arc<BuildEventEnvelope>,
        out: &mut impl FnMut(Arc<BuildEventEnvelope>),
    ) {
        self.push_at(Instant::now(), envelope, out);
    }

    /// Hand `out` whatever has been held for [`MAX_HOLD`].
    pub fn tick(&mut self, out: &mut impl FnMut(Arc<BuildEventEnvelope>)) {
        self.release(out, Release::Due(Instant::now()));
    }

    /// Hand `out` every event still held.
    pub fn flush(&mut self, out: &mut impl FnMut(Arc<BuildEventEnvelope>)) {
        self.release(out, Release::All);
    }

    fn push_at(
        &mut self,
        now: Instant,
        envelope: Arc<BuildEventEnvelope>,
        out: &mut impl FnMut(Arc<BuildEventEnvelope>),
    ) {
        if matches!(envelope.event.payload, Some(Payload::Started(_))) {
            self.flush(out);
            self.actions.retain(|_, fate| *fate == Fate::Promised);
        }
        let mut dropped = false;
        if let Some(action) = envelope.event.id.as_ref().and_then(action_of) {
            let key = action_key(action);
            let succeeded =
                matches!(envelope.event.payload.as_ref(), Some(Payload::Action(a)) if a.success);
            dropped = succeeded && self.actions.get(&key) != Some(&Fate::Promised);
            let fate = if dropped { Fate::Dropped } else { Fate::Kept };
            self.actions.insert(key, fate);
        }
        if !dropped {
            let mut opens = false;
            for child in envelope.event.children.iter().filter_map(action_of) {
                let fate = self.actions.entry(action_key(child)).or_insert(Fate::Open);
                opens |= *fate == Fate::Open;
            }
            if opens || !self.held.is_empty() {
                self.held.push_back((now, envelope));
            } else {
                out(self.trimmed(envelope));
            }
        }
        self.release(out, Release::Due(now));
    }

    /// Hand `out` the held events that can go: those at the head with no open
    /// announcement, and, for `release`, the ones it lets go regardless, whose
    /// open announcements are then [`Fate::Promised`].
    fn release(&mut self, out: &mut impl FnMut(Arc<BuildEventEnvelope>), release: Release) {
        while let Some((at, head)) = self.held.front() {
            let open: Vec<u64> = head
                .event
                .children
                .iter()
                .filter_map(action_of)
                .map(action_key)
                .filter(|key| self.actions.get(key) == Some(&Fate::Open))
                .collect();
            let forced = match release {
                Release::All => true,
                Release::Due(now) => {
                    self.held.len() > MAX_HELD || now.duration_since(*at) >= MAX_HOLD
                }
            };
            if !open.is_empty() && !forced {
                break;
            }
            for key in open {
                self.actions.insert(key, Fate::Promised);
            }
            let (_, head) = self.held.pop_front().expect("a head was just read");
            out(self.trimmed(head));
        }
    }

    /// `envelope` without the children it announced that were dropped,
    /// re-encoded only when there were any.
    fn trimmed(&self, envelope: Arc<BuildEventEnvelope>) -> Arc<BuildEventEnvelope> {
        let gone = |child: &BuildEventId| {
            action_of(child)
                .is_some_and(|a| self.actions.get(&action_key(a)) == Some(&Fate::Dropped))
        };
        if !envelope.event.children.iter().any(gone) {
            return envelope;
        }
        let mut event = BuildEventEnvelope::into_event(envelope);
        event.children.retain(|child| !gone(child));
        let encoded = Bytes::from(event.encode_to_vec());
        Arc::new(BuildEventEnvelope { event, encoded })
    }
}

/// Which held events [`Trim::release`] may let go with an announcement open.
#[derive(Clone, Copy)]
enum Release {
    /// Every one.
    All,
    /// Those over [`MAX_HELD`] or held for [`MAX_HOLD`] by this time.
    Due(Instant),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bes::testing::{action, action_id, envelope, started};
    use axl_proto::build_event_stream::BuildEvent;
    use axl_proto::build_event_stream::build_event_id::{ProgressId, TargetCompletedId};
    use axl_proto::build_event_stream::{Progress, TargetComplete};

    fn progress(n: i32, actions: &[&str]) -> BuildEvent {
        let next = BuildEventId {
            id: Some(Id::Progress(ProgressId {
                opaque_count: n + 1,
            })),
        };
        BuildEvent {
            id: Some(BuildEventId {
                id: Some(Id::Progress(ProgressId { opaque_count: n })),
            }),
            children: std::iter::once(next)
                .chain(actions.iter().map(|a| action_id(a)))
                .collect(),
            payload: Some(Payload::Progress(Progress::default())),
            ..Default::default()
        }
    }

    fn target_complete(label: &str, actions: &[&str]) -> BuildEvent {
        BuildEvent {
            id: Some(BuildEventId {
                id: Some(Id::TargetCompleted(TargetCompletedId {
                    label: label.into(),
                    ..Default::default()
                })),
            }),
            children: actions.iter().map(|a| action_id(a)).collect(),
            payload: Some(Payload::Completed(TargetComplete::default())),
            ..Default::default()
        }
    }

    /// A short name for each event `trim` passes on, with the actions it
    /// still announces.
    fn run(trim: &mut Trim, events: Vec<BuildEvent>) -> Vec<String> {
        let mut seen = vec![];
        let mut out = |e: Arc<BuildEventEnvelope>| {
            assert_eq!(
                e.encoded,
                Bytes::from(e.event.encode_to_vec()),
                "bytes match the event"
            );
            seen.push(name(&e.event));
        };
        for event in events {
            trim.push(envelope(event), &mut out);
        }
        seen
    }

    fn name(event: &BuildEvent) -> String {
        let announced: Vec<&str> = event
            .children
            .iter()
            .filter_map(action_of)
            .map(|a| a.label.as_str())
            .collect();
        let kind = match event.payload.as_ref() {
            Some(Payload::Progress(_)) => "progress".to_string(),
            Some(Payload::Action(a)) => format!("action {}", a_label(event, a.success)),
            Some(Payload::Completed(_)) => "target".to_string(),
            Some(Payload::Started(_)) => "started".to_string(),
            _ => "other".to_string(),
        };
        if announced.is_empty() {
            kind
        } else {
            format!("{kind} [{}]", announced.join(","))
        }
    }

    fn a_label(event: &BuildEvent, success: bool) -> String {
        let label = event.id.as_ref().and_then(action_of).unwrap().label.clone();
        format!("{label}{}", if success { "" } else { "!" })
    }

    /// Whether a caller passing `caller` under an rc of `rc` asks for every
    /// action, read off the command line the rc renders for `build`.
    fn asks(rc: &str, caller: &[&str]) -> bool {
        use crate::rc::{Bazelrc, RcOption};
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".bazelrc"), rc).unwrap();
        let caller: Vec<RcOption> = caller
            .iter()
            .map(|f| RcOption {
                value: f.to_string(),
                ..RcOption::default()
            })
            .collect();
        let rc = Bazelrc::new(dir.path(), &["--nosystem_rc", "--nohome_rc"], &caller).unwrap();
        let rendered = rc.resolve_for_command("build").unwrap().1;
        asks_for_all_actions(&rendered, "build")
    }

    #[test]
    fn a_feature_request_is_taken_off_and_is_not_the_callers() {
        let flags: Vec<String> = [
            "--default_override=0:build=--build_event_publish_all_actions=%aspect_cli%",
            "--a",
        ]
        .map(String::from)
        .to_vec();
        let (kept, asked) = take_cli_request(flags);
        assert!(asked);
        assert_eq!(kept, ["--a"]);
        assert!(!asks_for_all_actions(&kept, "build"));
    }

    #[test]
    fn the_caller_asks_for_every_action_by_flag_rc_or_config() {
        assert!(!asks("", &[]), "nothing asked");
        assert!(asks("", &["--build_event_publish_all_actions"]));
        assert!(asks("", &["--build_event_publish_all_actions=yes"]));
        assert!(asks("", &["--build_event_publish_all_actions=1"]));
        assert!(
            asks("build --build_event_publish_all_actions\n", &[]),
            "in the rc"
        );
        assert!(asks("common --build_event_publish_all_actions=true\n", &[]));
        assert!(
            asks(
                "build:ci --build_event_publish_all_actions\n",
                &["--config=ci"]
            ),
            "under a config in use",
        );
        assert!(
            !asks("build:ci --build_event_publish_all_actions\n", &[]),
            "a config not in use"
        );
        assert!(!asks("", &["--build_event_publish_all_actions=false"]));
        assert!(
            !asks(
                "",
                &[
                    "--build_event_publish_all_actions",
                    "--nobuild_event_publish_all_actions"
                ]
            ),
            "the last word wins",
        );
        assert!(
            !asks(
                "build --build_event_publish_all_actions\n",
                &["--nobuild_event_publish_all_actions"]
            ),
            "the caller's flag beats the rc",
        );
    }

    #[test]
    fn only_successful_actions_are_dropped() {
        let mut trim = Trim::new();
        let seen = run(
            &mut trim,
            vec![
                started(),
                action("ok", true),
                action("bad", false),
                target_complete("//:t", &[]),
                progress(1, &[]),
            ],
        );
        assert_eq!(seen, ["started", "action bad!", "target", "progress"]);
    }

    #[test]
    fn an_action_announced_before_a_hold_was_let_go_is_passed_on() {
        let mut trim = Trim::new();
        let mut events = vec![progress(0, &["late"])];
        events.extend((1..=MAX_HELD as i32).map(|n| progress(n, &[])));
        events.push(action("late", true));
        let seen = run(&mut trim, events);
        assert_eq!(seen.first().map(String::as_str), Some("progress [late]"));
        assert_eq!(seen.last().map(String::as_str), Some("action late"));
    }

    #[test]
    fn a_hold_is_let_go_after_max_hold() {
        let mut trim = Trim::new();
        let mut seen = vec![];
        let mut out = |e: Arc<BuildEventEnvelope>| seen.push(name(&e.event));
        let start = Instant::now();
        trim.push_at(start, envelope(progress(1, &["slow"])), &mut out);
        trim.push_at(start + MAX_HOLD / 2, envelope(progress(2, &[])), &mut out);
        trim.release(&mut out, Release::Due(start + MAX_HOLD / 2));
        trim.release(&mut out, Release::Due(start + MAX_HOLD));
        trim.push_at(start + MAX_HOLD, envelope(action("slow", true)), &mut out);
        assert_eq!(seen, ["progress [slow]", "progress", "action slow"]);
    }

    #[test]
    fn an_aborted_action_resolves_its_hold() {
        use axl_proto::build_event_stream::Aborted;
        let aborted = BuildEvent {
            id: Some(action_id("cut")),
            payload: Some(Payload::Aborted(Aborted::default())),
            ..Default::default()
        };
        let mut trim = Trim::new();
        let seen = run(&mut trim, vec![progress(1, &["cut"]), aborted]);
        assert_eq!(seen, ["progress [cut]", "other"]);
    }

    #[test]
    fn a_burst_of_announcements_is_held_until_each_action_is_posted() {
        let mut trim = Trim::new();
        let seen = run(
            &mut trim,
            vec![
                progress(1, &["a"]),
                progress(2, &["b"]),
                progress(3, &["c"]),
                action("b", true),
                action("a", false),
                action("c", true),
                target_complete("//:t", &[]),
            ],
        );
        assert_eq!(
            seen,
            [
                "progress [a]",
                "progress",
                "progress",
                "action a!",
                "target"
            ],
            "the held events go out in order, without the dropped actions",
        );
    }

    #[test]
    fn nothing_is_held_without_an_open_announcement() {
        let mut trim = Trim::new();
        let mut seen = vec![];
        trim.push(envelope(target_complete("//:t", &[])), &mut |e| {
            seen.push(name(&e.event))
        });
        assert_eq!(seen, ["target"]);
    }

    #[test]
    fn an_announcement_after_the_action_is_trimmed_without_a_hold() {
        let mut trim = Trim::new();
        let seen = run(
            &mut trim,
            vec![
                progress(1, &["ok", "bad"]),
                action("ok", true),
                action("bad", false),
                target_complete("//:t", &["ok", "bad"]),
            ],
        );
        assert_eq!(seen, ["progress [bad]", "action bad!", "target [bad]"]);
    }

    #[test]
    fn flush_lets_go_of_what_never_resolved() {
        let mut trim = Trim::new();
        let mut seen = run(
            &mut trim,
            vec![progress(1, &["never"]), target_complete("//:t", &[])],
        );
        assert!(seen.is_empty());
        trim.flush(&mut |e| seen.push(name(&e.event)));
        assert_eq!(seen, ["progress [never]", "target"]);
    }

    #[test]
    fn a_hold_is_let_go_at_its_limit() {
        let mut trim = Trim::new();
        let mut events = vec![progress(0, &["never"])];
        events.extend((1..=MAX_HELD as i32).map(|n| progress(n, &[])));
        let seen = run(&mut trim, events);
        assert_eq!(seen.first().map(String::as_str), Some("progress [never]"));
        assert_eq!(seen.len(), MAX_HELD + 1);
    }

    #[test]
    fn a_new_attempt_releases_the_last_and_keeps_its_promises() {
        let mut trim = Trim::new();
        let seen = run(
            &mut trim,
            vec![
                progress(1, &["a"]),
                started(),
                progress(1, &["a"]),
                action("a", true),
            ],
        );
        assert_eq!(
            seen,
            ["progress [a]", "started", "progress [a]", "action a"],
            "what the last attempt promised is still passed on",
        );
    }
}
