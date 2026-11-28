//! What a build did, in brief, from its BES stream: the tallies of targets and
//! tests and the failures, for the task history's record of the run (see the
//! runtime's `task.json`).
//!
//! A target counts once per label, though Bazel can report one in several
//! configurations or under two ids. One that failed in loading or analysis
//! arrives as an `Aborted` payload under its own `TargetConfigured` or
//! `TargetCompleted` id (under a `ConfiguredLabel` or `UnconfiguredLabel` id
//! it names a root cause, a dependency, not a target asked for); any other
//! abort reason skips it. An aspect's completion is not a target. A flaky
//! test passed, its failed attempts marked `flaky`. A failure is a failed
//! action (Bazel reports every one), a test attempt that did not pass, or a
//! failed target neither explains; the first [`MAX_FAILURES`] are listed, the
//! rest counted. A retry of the build in one stream starts again at its
//! `Started`, so the summary is the last attempt's.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::RecvError;
use std::thread::{self, JoinHandle};

use axl_proto::build_event_stream::aborted::AbortReason;
use axl_proto::build_event_stream::build_event::Payload;
use axl_proto::build_event_stream::build_event_id::Id;
use axl_proto::build_event_stream::{BuildEvent, TestStatus};
use serde::{Deserialize, Serialize};

use crate::bes::BuildEventEnvelope;
use crate::fanout::{Subscriber, SubscriberFilter};

/// The most failures a summary lists.
pub const MAX_FAILURES: usize = 1000;

/// A build's tallies and failures.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// Bazel's own id for the invocation (`BuildStarted.uuid`).
    pub invocation_id: String,
    pub targets_configured: u64,
    pub targets_built: u64,
    pub targets_failed: u64,
    pub targets_skipped: u64,
    pub tests_passed: u64,
    pub tests_flaky: u64,
    pub tests_failed: u64,
    pub failures: Vec<Failure>,
    /// Failures past [`MAX_FAILURES`], not listed.
    pub failures_not_listed: u64,
}

/// One failure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Failure {
    /// `action`, `test` or `target`.
    pub kind: String,
    pub label: String,
    /// An action's mnemonic; a test attempt's `attempt <n>`, after its shard
    /// and run when the test has more than one; empty for a target.
    pub detail: String,
    /// A test attempt's run, shard and attempt, to find its kept logs by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<(i32, i32, i32)>,
    /// Its logs the history kept, relative to the task's entry; filled in by
    /// the history once its indexes are written.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub logs: Vec<String>,
    /// A failed attempt of a test that passed on a later one (its summary
    /// `FLAKY`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flaky: bool,
}

/// The test statuses that are failures; `no_status`, `incomplete` and the
/// like are not.
const FAILED_TESTS: &[TestStatus] = &[
    TestStatus::Failed,
    TestStatus::Timeout,
    TestStatus::FailedToBuild,
    TestStatus::RemoteFailure,
    TestStatus::ToolHaltedBeforeTesting,
];

/// The events a summary is read from, for a filtered subscription.
pub fn filter() -> SubscriberFilter<Arc<BuildEventEnvelope>> {
    Arc::new(
        |envelope: &Arc<BuildEventEnvelope>| match envelope.event.payload.as_ref() {
            Some(
                Payload::Started(_)
                | Payload::Configured(_)
                | Payload::Completed(_)
                | Payload::Aborted(_)
                | Payload::Action(_)
                | Payload::TestResult(_)
                | Payload::TestSummary(_),
            ) => true,
            _ => false,
        },
    )
}

/// Read a summary from `recv` until the stream ends.
pub fn spawn(recv: Subscriber<Arc<BuildEventEnvelope>>) -> JoinHandle<Summary> {
    thread::spawn(move || {
        let mut tally = Tally::default();
        loop {
            match recv.recv() {
                Ok(envelope) => tally.event(&envelope.event),
                Err(RecvError) => break,
            }
        }
        tally.finish()
    })
}

/// A summary being read.
#[derive(Debug, Default)]
pub struct Tally {
    summary: Summary,
    /// Labels counted as built, failed or skipped.
    counted: HashSet<String>,
    /// Failed targets, in order, and those a failure record already explains.
    failed_targets: Vec<String>,
    explained: HashSet<String>,
}

impl Tally {
    /// Take one event.
    pub fn event(&mut self, event: &BuildEvent) {
        let id = event.id.as_ref().and_then(|id| id.id.as_ref());
        match (event.payload.as_ref(), id) {
            (Some(Payload::Started(started)), _) => {
                *self = Tally::default();
                self.summary.invocation_id = started.uuid.clone();
            }
            (Some(Payload::Aborted(aborted)), Some(Id::TargetConfigured(id)))
                if id.aspect.is_empty() =>
            {
                self.aborted(&id.label, aborted.reason);
            }
            (Some(Payload::Aborted(aborted)), Some(Id::TargetCompleted(id)))
                if id.aspect.is_empty() =>
            {
                self.aborted(&id.label, aborted.reason);
            }
            (Some(Payload::Configured(_)), Some(Id::TargetConfigured(id)))
                if id.aspect.is_empty() =>
            {
                self.summary.targets_configured += 1;
            }
            (Some(Payload::Completed(target)), Some(Id::TargetCompleted(id)))
                if id.aspect.is_empty() =>
            {
                if !self.counted.insert(id.label.clone()) {
                    return;
                }
                if target.success {
                    self.summary.targets_built += 1;
                } else {
                    self.summary.targets_failed += 1;
                    self.failed_targets.push(id.label.clone());
                }
            }
            (Some(Payload::TestSummary(summary)), Some(Id::TestSummary(_))) => {
                match TestStatus::try_from(summary.overall_status) {
                    Ok(TestStatus::Passed) => self.summary.tests_passed += 1,
                    Ok(TestStatus::Flaky) => {
                        self.summary.tests_passed += 1;
                        self.summary.tests_flaky += 1;
                        let label = event.id.as_ref().and_then(|id| match &id.id {
                            Some(Id::TestSummary(id)) => Some(id.label.as_str()),
                            _ => None,
                        });
                        for f in &mut self.summary.failures {
                            if f.kind == "test" && Some(f.label.as_str()) == label {
                                f.flaky = true;
                            }
                        }
                    }
                    Ok(status) if FAILED_TESTS.contains(&status) => {
                        self.summary.tests_failed += 1;
                    }
                    _ => {}
                }
            }
            (Some(Payload::Action(action)), Some(Id::ActionCompleted(id))) if !action.success => {
                let detail = if action.r#type.is_empty() {
                    "Action".to_string()
                } else {
                    action.r#type.clone()
                };
                self.fail("action", &id.label, detail, None);
            }
            (Some(Payload::TestResult(result)), Some(Id::TestResult(id))) => {
                let failed = TestStatus::try_from(result.status)
                    .is_ok_and(|status| FAILED_TESTS.contains(&status));
                if failed {
                    let detail = format!("attempt {}", id.attempt);
                    self.fail(
                        "test",
                        &id.label,
                        detail,
                        Some((id.run, id.shard, id.attempt)),
                    );
                }
            }
            _ => {}
        }
    }

    /// A target that did not build: failed when its loading or analysis did,
    /// else skipped; once per label.
    fn aborted(&mut self, label: &str, reason: i32) {
        if label.is_empty() || !self.counted.insert(label.to_owned()) {
            return;
        }
        match AbortReason::try_from(reason) {
            Ok(AbortReason::AnalysisFailure | AbortReason::LoadingFailure) => {
                self.summary.targets_failed += 1;
                self.failed_targets.push(label.to_owned());
            }
            _ => self.summary.targets_skipped += 1,
        }
    }

    fn fail(&mut self, kind: &str, label: &str, detail: String, attempt: Option<(i32, i32, i32)>) {
        self.explained.insert(label.to_owned());
        self.push(Failure {
            kind: kind.to_owned(),
            label: label.to_owned(),
            detail,
            attempt,
            logs: vec![],
            flaky: false,
        });
    }

    fn push(&mut self, failure: Failure) {
        if self.summary.failures.len() < MAX_FAILURES {
            self.summary.failures.push(failure);
        } else {
            self.summary.failures_not_listed += 1;
        }
    }

    /// The summary: the failed targets no action or test failure explains
    /// listed after the others, and a test attempt's detail naming its shard
    /// and run where the test has more than one.
    pub fn finish(mut self) -> Summary {
        for label in std::mem::take(&mut self.failed_targets) {
            if !self.explained.contains(&label) {
                self.push(Failure {
                    kind: "target".to_owned(),
                    label,
                    detail: String::new(),
                    attempt: None,
                    logs: vec![],
                    flaky: false,
                });
            }
        }
        let mut spread: std::collections::HashMap<&str, (i32, i32)> = Default::default();
        for f in &self.summary.failures {
            if let Some((run, shard, _)) = f.attempt {
                let s = spread.entry(f.label.as_str()).or_insert((1, 1));
                *s = (s.0.max(run), s.1.max(shard));
            }
        }
        let spread: std::collections::HashMap<String, (i32, i32)> =
            spread.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        for f in &mut self.summary.failures {
            if let Some((run, shard, attempt)) = f.attempt {
                let (runs, shards) = spread[&f.label];
                let mut parts = vec![];
                if shards > 1 {
                    parts.push(format!("shard {shard}"));
                }
                if runs > 1 {
                    parts.push(format!("run {run}"));
                }
                parts.push(format!("attempt {attempt}"));
                f.detail = parts.join(" · ");
            }
        }
        self.summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axl_proto::build_event_stream::build_event_id::{
        ActionCompletedId, BuildStartedId, TargetCompletedId, TargetConfiguredId, TestResultId,
        TestSummaryId,
    };
    use axl_proto::build_event_stream::{
        Aborted, ActionExecuted, BuildEventId, BuildStarted, TargetComplete, TargetConfigured,
        TestResult, TestSummary,
    };

    fn ev(id: Id, payload: Payload) -> BuildEvent {
        BuildEvent {
            id: Some(BuildEventId { id: Some(id) }),
            payload: Some(payload),
            ..Default::default()
        }
    }

    fn configured(label: &str, aspect: &str) -> BuildEvent {
        ev(
            Id::TargetConfigured(TargetConfiguredId {
                label: label.into(),
                aspect: aspect.into(),
            }),
            Payload::Configured(TargetConfigured::default()),
        )
    }

    fn completed(label: &str, aspect: &str, success: bool) -> BuildEvent {
        ev(
            Id::TargetCompleted(TargetCompletedId {
                label: label.into(),
                aspect: aspect.into(),
                ..Default::default()
            }),
            Payload::Completed(TargetComplete {
                success,
                ..Default::default()
            }),
        )
    }

    fn aborted(label: &str, reason: AbortReason) -> BuildEvent {
        ev(
            Id::TargetConfigured(TargetConfiguredId {
                label: label.into(),
                aspect: String::new(),
            }),
            Payload::Aborted(Aborted {
                reason: reason as i32,
                ..Default::default()
            }),
        )
    }

    fn action(label: &str, mnemonic: &str, success: bool) -> BuildEvent {
        ev(
            Id::ActionCompleted(ActionCompletedId {
                label: label.into(),
                ..Default::default()
            }),
            Payload::Action(ActionExecuted {
                success,
                r#type: mnemonic.into(),
                ..Default::default()
            }),
        )
    }

    fn test_result(
        label: &str,
        run: i32,
        shard: i32,
        attempt: i32,
        status: TestStatus,
    ) -> BuildEvent {
        ev(
            Id::TestResult(TestResultId {
                label: label.into(),
                run,
                shard,
                attempt,
                ..Default::default()
            }),
            Payload::TestResult(TestResult {
                status: status as i32,
                ..Default::default()
            }),
        )
    }

    fn test_summary(label: &str, status: TestStatus) -> BuildEvent {
        ev(
            Id::TestSummary(TestSummaryId {
                label: label.into(),
                ..Default::default()
            }),
            Payload::TestSummary(TestSummary {
                overall_status: status as i32,
                ..Default::default()
            }),
        )
    }

    fn summarize(events: &[BuildEvent]) -> Summary {
        let mut tally = Tally::default();
        for e in events {
            tally.event(e);
        }
        tally.finish()
    }

    #[test]
    fn targets_and_tests_are_counted_once_each() {
        let s = summarize(&[
            ev(
                Id::Started(BuildStartedId {}),
                Payload::Started(BuildStarted {
                    uuid: "u-1".into(),
                    ..Default::default()
                }),
            ),
            configured("//a:a", ""),
            configured("//a:a", "//x.bzl%y"),
            completed("//a:a", "", true),
            completed("//a:a", "", true),
            completed("//a:a", "//x.bzl%y", false),
            completed("//b:b", "", false),
            aborted("//c:c", AbortReason::Skipped),
            aborted("//d:d", AbortReason::AnalysisFailure),
            aborted("//d:d", AbortReason::AnalysisFailure),
            test_summary("//t:pass", TestStatus::Passed),
            test_summary("//t:flaky", TestStatus::Flaky),
            test_summary("//t:fail", TestStatus::Failed),
            test_summary("//t:none", TestStatus::NoStatus),
        ]);
        assert_eq!(s.invocation_id, "u-1");
        assert_eq!(
            (
                s.targets_configured,
                s.targets_built,
                s.targets_failed,
                s.targets_skipped
            ),
            (1, 1, 2, 1)
        );
        assert_eq!((s.tests_passed, s.tests_flaky, s.tests_failed), (2, 1, 1));
        let failed: Vec<(&str, &str)> = s
            .failures
            .iter()
            .map(|f| (f.kind.as_str(), f.label.as_str()))
            .collect();
        assert_eq!(failed, [("target", "//b:b"), ("target", "//d:d")]);
    }

    #[test]
    fn failures_are_actions_and_attempts_then_unexplained_targets() {
        let s = summarize(&[
            action("//a:a", "CppCompile", false),
            action("//a:a", "CppCompile", true),
            action("//g:g", "", false),
            completed("//a:a", "", false),
            completed("//b:b", "", false),
            test_result("//t:t", 1, 1, 1, TestStatus::Failed),
            test_result("//t:t", 1, 2, 1, TestStatus::Passed),
            test_result("//t:t", 1, 2, 2, TestStatus::Timeout),
            test_result("//u:u", 1, 1, 1, TestStatus::Flaky),
        ]);
        let got: Vec<(&str, &str, &str)> = s
            .failures
            .iter()
            .map(|f| (f.kind.as_str(), f.label.as_str(), f.detail.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("action", "//a:a", "CppCompile"),
                ("action", "//g:g", "Action"),
                ("test", "//t:t", "shard 1 · attempt 1"),
                ("test", "//t:t", "shard 2 · attempt 2"),
                ("target", "//b:b", ""),
            ]
        );
        assert_eq!(s.failures[2].attempt, Some((1, 1, 1)));
    }

    /// A test whose summary is flaky marks its failed attempts so, and no
    /// other test's.
    #[test]
    fn a_flaky_tests_failed_attempts_are_marked() {
        let s = summarize(&[
            test_result("//t:flaky", 1, 1, 1, TestStatus::Failed),
            test_result("//t:fail", 1, 1, 1, TestStatus::Failed),
            test_result("//t:flaky", 1, 1, 2, TestStatus::Passed),
            test_summary("//t:flaky", TestStatus::Flaky),
            test_summary("//t:fail", TestStatus::Failed),
        ]);
        let got: Vec<(&str, bool)> = s
            .failures
            .iter()
            .map(|f| (f.label.as_str(), f.flaky))
            .collect();
        assert_eq!(got, [("//t:flaky", true), ("//t:fail", false)]);
    }

    #[test]
    fn a_retry_in_the_stream_starts_again() {
        let started = ev(
            Id::Started(BuildStartedId {}),
            Payload::Started(BuildStarted {
                uuid: "second".into(),
                ..Default::default()
            }),
        );
        let s = summarize(&[
            completed("//b:b", "", false),
            started,
            completed("//a:a", "", true),
        ]);
        assert_eq!(
            (s.invocation_id.as_str(), s.targets_built, s.targets_failed),
            ("second", 1, 0)
        );
        assert!(s.failures.is_empty());
    }

    #[test]
    fn failures_past_the_cap_are_counted() {
        let events: Vec<_> = (0..MAX_FAILURES + 5)
            .map(|i| action(&format!("//p:{i}"), "Genrule", false))
            .collect();
        let s = summarize(&events);
        assert_eq!((s.failures.len(), s.failures_not_listed), (MAX_FAILURES, 5));
    }
}
