//! Which files a build produced for the targets it was asked for, learned from
//! the BES stream as it arrives.
//!
//! A target's outputs are named by file sets that may arrive before or after
//! the `target_completed` that references them, so readiness is a question
//! asked of every event: [`Outputs::handle`] says which targets that event
//! made ready. When Bazel retries a build in the same invocation, the tracker
//! starts over at each attempt's `BuildStarted`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use axl_proto::build_event_stream::build_event::Payload;
use axl_proto::build_event_stream::build_event_id::Id;
use axl_proto::build_event_stream::{BuildEvent, NamedSetOfFiles};

use crate::label::normalize_label;

/// Bound on fileset nesting when walking BES `named_set_of_files` references.
const FILESET_DEPTH: usize = 32;

/// Bazel's name for a target's default outputs.
const DEFAULT_OUTPUT_GROUP: &str = "default";

#[derive(Default)]
struct TargetState {
    /// Output group name → fileset ids, from every `target_completed` for
    /// the label in the current attempt (the target's own and the aspect's).
    groups: HashMap<String, Vec<String>>,
    /// A `target_completed` said the target failed, or an `aborted` arrived.
    failed: bool,
}

/// A target whose outputs are all known.
#[derive(Debug, PartialEq, Eq)]
pub struct Ready {
    /// The target's normalized label.
    pub key: String,
    /// The files of its default output group.
    pub default_files: Vec<PathBuf>,
    /// The files of the extra output group the tracker was asked to follow.
    pub extra_files: Vec<PathBuf>,
}

/// A target the stream ended on without it becoming ready.
#[derive(Debug, PartialEq, Eq)]
pub struct Unsettled {
    pub key: String,
    /// Its own completion in the last attempt reported failure, or it was
    /// aborted.
    pub failed: bool,
    /// It completed with its default outputs.
    pub has_default: bool,
}

pub struct Outputs {
    extra_group: &'static str,
    filesets: HashMap<String, NamedSetOfFiles>,
    targets: HashMap<String, TargetState>,
    pending: HashSet<String>,
    exec_root: Option<PathBuf>,
    workspace_name: String,
    exit_code: Option<i32>,
}

impl Outputs {
    /// Follow `pending` (normalized labels). A target is ready once its default
    /// group and `extra_group` are both complete; `workspace_name` stands
    /// until the stream reports one.
    pub fn new(
        pending: HashSet<String>,
        extra_group: &'static str,
        workspace_name: String,
    ) -> Self {
        Self {
            extra_group,
            filesets: HashMap::new(),
            targets: HashMap::new(),
            pending,
            exec_root: None,
            workspace_name,
            exit_code: None,
        }
    }

    pub fn exec_root(&self) -> Option<&Path> {
        self.exec_root.as_deref()
    }

    pub fn workspace_name(&self) -> &str {
        &self.workspace_name
    }

    /// Take in one event; the targets it made ready, in no particular order.
    pub fn handle(&mut self, event: &BuildEvent) -> Vec<Ready> {
        let id = event.id.as_ref().and_then(|id| id.id.as_ref());
        match (&event.payload, id) {
            (Some(Payload::NamedSetOfFiles(set)), Some(Id::NamedSet(set_id))) => {
                self.filesets.insert(set_id.id.clone(), set.clone());
                let pending: Vec<String> = self.pending.iter().cloned().collect();
                pending
                    .iter()
                    .filter_map(|key| self.check_ready(key))
                    .collect()
            }
            (Some(Payload::Completed(tc)), Some(Id::TargetCompleted(tid))) => {
                let key = normalize_label(&tid.label);
                if !self.pending.contains(&key) {
                    return vec![];
                }
                let state = self.targets.entry(key.clone()).or_default();
                for og in &tc.output_group {
                    state.groups.insert(
                        og.name.clone(),
                        og.file_sets.iter().map(|f| f.id.clone()).collect(),
                    );
                }
                if !tc.success && tid.aspect.is_empty() {
                    state.failed = true;
                }
                self.check_ready(&key).into_iter().collect()
            }
            (Some(Payload::Aborted(_)), Some(Id::TargetCompleted(tid))) => {
                let key = normalize_label(&tid.label);
                if self.pending.contains(&key) {
                    self.targets.entry(key).or_default().failed = true;
                }
                vec![]
            }
            (Some(Payload::WorkspaceInfo(ws)), _) => {
                let root = ws.local_exec_root.trim_end_matches('/');
                if !root.is_empty() {
                    if let Some(name) = root.rsplit('/').next() {
                        self.workspace_name = name.to_owned();
                    }
                    self.exec_root = Some(PathBuf::from(root));
                }
                vec![]
            }
            (Some(Payload::Started(_)), _) => {
                self.restart();
                vec![]
            }
            (Some(Payload::Finished(finished)), _) => {
                self.exit_code = finished.exit_code.as_ref().map(|c| c.code);
                vec![]
            }
            _ => vec![],
        }
    }

    /// Start over for a new attempt. Bazel retries an evicted build
    /// (`REMOTE_CACHE_EVICTED`) in the same invocation as a fresh stream that
    /// reports every target and file set again and reuses file-set ids, so only
    /// the last attempt's events count. Targets already ready stay settled.
    fn restart(&mut self) {
        self.targets.clear();
        self.filesets.clear();
        self.exit_code = None;
    }

    /// The stream closed: Bazel's exit code (`1` when it never said), and
    /// everything still pending, which did not become ready on its own.
    pub fn finish(&mut self) -> (i32, Vec<Unsettled>) {
        let code = self.exit_code.unwrap_or(1);
        let pending: Vec<String> = self.pending.drain().collect();
        let unsettled = pending
            .into_iter()
            .map(|key| {
                let state = self.targets.get(&key);
                Unsettled {
                    failed: state.is_some_and(|s| s.failed),
                    has_default: state.is_some_and(|s| s.groups.contains_key(DEFAULT_OUTPUT_GROUP)),
                    key,
                }
            })
            .collect();
        (code, unsettled)
    }

    /// Absolute paths of every file under the filesets `ids`, or `None`
    /// while any referenced fileset has not arrived yet.
    fn collect(&self, ids: &[String]) -> Option<Vec<PathBuf>> {
        let mut out = Vec::new();
        let mut visited = HashSet::new();
        let mut frontier: Vec<String> = ids.to_vec();
        for _ in 0..FILESET_DEPTH {
            if frontier.is_empty() {
                break;
            }
            let mut next = Vec::new();
            for id in frontier {
                if !visited.insert(id.clone()) {
                    continue;
                }
                let set = self.filesets.get(&id)?;
                for file in &set.files {
                    if let Some(path) = self.file_path(file) {
                        out.push(path);
                    }
                }
                for child in &set.file_sets {
                    if !visited.contains(&child.id) {
                        next.push(child.id.clone());
                    }
                }
            }
            frontier = next;
        }
        Some(out)
    }

    /// A BES file's local path: the execution root plus `path_prefix` and
    /// `name`, which BES fills whatever the upload strategy; a `file://` URI
    /// when there is no prefix to go by.
    fn file_path(&self, file: &axl_proto::build_event_stream::File) -> Option<PathBuf> {
        use axl_proto::build_event_stream::file::File as Loc;
        if !file.path_prefix.is_empty() {
            if let Some(root) = &self.exec_root {
                let mut path = root.clone();
                for seg in &file.path_prefix {
                    path.push(seg);
                }
                path.push(&file.name);
                return Some(path);
            }
        }
        match &file.file {
            Some(Loc::Uri(uri)) if uri.starts_with("file://") => {
                Some(PathBuf::from(uri.trim_start_matches("file://")))
            }
            _ => None,
        }
    }

    /// The target `key`, if it is now ready; it stops being pending.
    fn check_ready(&mut self, key: &str) -> Option<Ready> {
        let state = self.targets.get(key)?;
        if state.failed {
            return None;
        }
        let (default_ids, extra_ids) = (
            state.groups.get(DEFAULT_OUTPUT_GROUP)?,
            state.groups.get(self.extra_group)?,
        );
        let (default_files, extra_files) = (self.collect(default_ids)?, self.collect(extra_ids)?);
        self.pending.remove(key);
        Some(Ready {
            key: key.to_owned(),
            default_files,
            extra_files,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axl_proto::build_event_stream::{
        BuildEventId, BuildFinished, BuildStarted, File, OutputGroup, TargetComplete,
        WorkspaceConfig, build_event_id, build_finished::ExitCode,
    };

    fn named_set(id: &str, files: &[(&[&str], &str)]) -> BuildEvent {
        BuildEvent {
            id: Some(BuildEventId {
                id: Some(Id::NamedSet(build_event_id::NamedSetOfFilesId {
                    id: id.to_owned(),
                })),
            }),
            payload: Some(Payload::NamedSetOfFiles(NamedSetOfFiles {
                files: files
                    .iter()
                    .map(|(prefix, name)| File {
                        name: (*name).to_owned(),
                        path_prefix: prefix.iter().map(|s| (*s).to_owned()).collect(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn completed(label: &str, groups: &[(&str, &str)]) -> BuildEvent {
        BuildEvent {
            id: Some(BuildEventId {
                id: Some(Id::TargetCompleted(build_event_id::TargetCompletedId {
                    label: label.to_owned(),
                    ..Default::default()
                })),
            }),
            payload: Some(Payload::Completed(TargetComplete {
                success: true,
                output_group: groups
                    .iter()
                    .map(|(name, set)| OutputGroup {
                        name: (*name).to_owned(),
                        file_sets: vec![build_event_id::NamedSetOfFilesId {
                            id: (*set).to_owned(),
                        }],
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn workspace(exec_root: &str) -> BuildEvent {
        BuildEvent {
            payload: Some(Payload::WorkspaceInfo(WorkspaceConfig {
                local_exec_root: exec_root.to_owned(),
            })),
            ..Default::default()
        }
    }

    fn started() -> BuildEvent {
        BuildEvent {
            payload: Some(Payload::Started(BuildStarted::default())),
            ..Default::default()
        }
    }

    fn finished(code: i32) -> BuildEvent {
        BuildEvent {
            payload: Some(Payload::Finished(BuildFinished {
                exit_code: Some(ExitCode {
                    code,
                    ..Default::default()
                }),
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn failed(label: &str, groups: &[(&str, &str)]) -> BuildEvent {
        let mut event = completed(label, groups);
        if let Some(Payload::Completed(tc)) = &mut event.payload {
            tc.success = false;
        }
        event
    }

    const EVICTED: i32 = crate::bes::binary_file::REMOTE_CACHE_EVICTED;

    fn aborted(label: &str) -> BuildEvent {
        BuildEvent {
            id: Some(BuildEventId {
                id: Some(Id::TargetCompleted(build_event_id::TargetCompletedId {
                    label: label.to_owned(),
                    ..Default::default()
                })),
            }),
            payload: Some(Payload::Aborted(Default::default())),
            ..Default::default()
        }
    }

    fn outputs() -> Outputs {
        Outputs::new(
            HashSet::from(["//:t".to_owned()]),
            "extra",
            "_main".to_owned(),
        )
    }

    #[test]
    fn a_target_is_ready_once_both_groups_and_their_filesets_are_known() {
        let mut o = outputs();
        assert!(o.handle(&workspace("/x/_main/")).is_empty());
        assert_eq!(o.workspace_name(), "_main");
        assert_eq!(o.exec_root(), Some(Path::new("/x/_main")));
        assert!(o.handle(&named_set("1", &[(&["bin"], "tool")])).is_empty());
        // The groups name filesets; `2` has not arrived, so nothing is ready.
        assert!(
            o.handle(&completed("//:t", &[("default", "1"), ("extra", "2")]))
                .is_empty()
        );
        let ready = o.handle(&named_set("2", &[(&["bin"], "tool.json")]));
        assert_eq!(
            ready,
            vec![Ready {
                key: "//:t".to_owned(),
                default_files: vec![PathBuf::from("/x/_main/bin/tool")],
                extra_files: vec![PathBuf::from("/x/_main/bin/tool.json")],
            }]
        );
        assert!(
            o.finish().1.is_empty(),
            "a ready target is no longer pending"
        );
    }

    #[test]
    fn a_failed_target_never_becomes_ready_and_is_reported_unsettled() {
        let mut o = outputs();
        o.handle(&workspace("/x/_main"));
        o.handle(&named_set("1", &[(&["bin"], "tool")]));
        assert!(
            o.handle(&failed("//:t", &[("default", "1"), ("extra", "1")]))
                .is_empty()
        );
        let (code, unsettled) = o.finish();
        assert_eq!(code, 1, "Bazel never said, so assume failure");
        assert_eq!(
            unsettled,
            vec![Unsettled {
                key: "//:t".to_owned(),
                failed: true,
                has_default: true,
            }]
        );
    }

    /// Bazel retries an evicted build in the same invocation, and only the last
    /// attempt says whether the target built.
    #[test]
    fn a_target_the_retried_attempt_builds_is_ready() {
        let mut o = outputs();
        o.handle(&started());
        o.handle(&workspace("/x/_main"));
        o.handle(&named_set("1", &[(&["bin"], "tool")]));
        assert!(
            o.handle(&failed("//:t", &[("default", "1"), ("extra", "1")]))
                .is_empty()
        );
        o.handle(&finished(EVICTED));

        o.handle(&started());
        o.handle(&named_set("1", &[(&["bin"], "tool")]));
        let ready = o.handle(&completed("//:t", &[("default", "1"), ("extra", "1")]));
        o.handle(&finished(0));

        assert_eq!(ready.len(), 1, "the retry built the target");
        assert_eq!(o.finish(), (0, vec![]));
    }

    #[test]
    fn a_target_every_attempt_fails_is_reported_failed() {
        let mut o = outputs();
        for code in [EVICTED, 1] {
            o.handle(&started());
            o.handle(&named_set("1", &[(&["bin"], "tool")]));
            o.handle(&failed("//:t", &[("default", "1"), ("extra", "1")]));
            o.handle(&finished(code));
        }
        let (code, unsettled) = o.finish();
        assert_eq!(code, 1, "the last attempt's exit code");
        assert_eq!(unsettled.len(), 1);
        assert!(unsettled[0].failed);
    }

    /// A target aborted by the evicted attempt is judged by the retry too.
    #[test]
    fn a_target_aborted_before_the_retry_builds_is_ready() {
        let mut o = outputs();
        o.handle(&started());
        o.handle(&aborted("//:t"));
        o.handle(&finished(EVICTED));
        o.handle(&started());
        o.handle(&named_set("1", &[(&["bin"], "tool")]));
        let ready = o.handle(&completed("//:t", &[("default", "1"), ("extra", "1")]));
        assert_eq!(ready.len(), 1);
    }

    /// File-set ids restart with each attempt, so an id the retry reuses names
    /// the retry's files, and a group naming one the retry has not sent yet is
    /// not ready on the evicted attempt's set.
    #[test]
    fn a_reused_file_set_id_names_the_retried_attempts_files() {
        let mut o = outputs();
        o.handle(&workspace("/x/_main"));
        o.handle(&started());
        o.handle(&named_set("1", &[(&["bin"], "old")]));
        o.handle(&failed("//:t", &[("default", "1"), ("extra", "1")]));
        o.handle(&finished(EVICTED));

        o.handle(&started());
        assert!(
            o.handle(&completed("//:t", &[("default", "1"), ("extra", "1")]))
                .is_empty(),
            "set 1 has not arrived in this attempt",
        );
        let ready = o.handle(&named_set("1", &[(&["bin"], "new")]));
        assert_eq!(ready.len(), 1);
        assert_eq!(
            ready[0].default_files,
            vec![PathBuf::from("/x/_main/bin/new")]
        );
    }

    /// A retried attempt that stops before its `BuildFinished` reports Bazel's
    /// silence, not the evicted attempt's code.
    #[test]
    fn a_retry_that_never_finishes_reports_no_exit_code() {
        let mut o = outputs();
        o.handle(&started());
        o.handle(&finished(EVICTED));
        o.handle(&started());
        assert_eq!(o.finish().0, 1);
    }

    /// A target an earlier attempt already handed over stays settled.
    #[test]
    fn a_target_ready_before_a_retry_stays_settled() {
        let mut o = outputs();
        o.handle(&started());
        o.handle(&named_set("1", &[(&["bin"], "tool")]));
        assert_eq!(
            o.handle(&completed("//:t", &[("default", "1"), ("extra", "1")]))
                .len(),
            1
        );
        o.handle(&finished(EVICTED));
        o.handle(&started());
        o.handle(&finished(0));
        assert_eq!(o.finish(), (0, vec![]));
    }

    #[test]
    fn targets_nobody_asked_for_are_ignored() {
        let mut o = outputs();
        assert!(
            o.handle(&completed("//:other", &[("default", "1"), ("extra", "1")]))
                .is_empty()
        );
        assert_eq!(o.finish().1.len(), 1);
    }
}
