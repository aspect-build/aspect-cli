//! `basil-core` — the reusable guts of the `basil` fake-`bazel` binary.
//!
//! Two consumers share this crate:
//!   - the standalone `basil` binary (`crates/basil`), spawned by the
//!     `BazelBackend::Fake` path the axl-runtime tests drive; and
//!   - a shipped self-exec subcommand of `aspect` (roadmap item 6) so the AXL
//!     test runner can fork+exec a fake bazel without embedding a second
//!     binary (see `docs/testing.md`, decision 7).
//!
//! Replay path — [`replay_expectation`]: reads a typed, declared
//! [`BazelExpectation`] (length-delimited protobuf) off a control channel and
//! **synthesizes** a consistent `BuildStarted` → `TargetComplete` (one per
//! target) → `BuildFinished` BES stream onto `--build_event_binary_file`,
//! then exits with the fixture's code. This is the contract the AXL
//! `BazelExpectation` record serializes into.
//!
//! Wire format (control channel): the [`BazelExpectation`] message encoded
//! **length-delimited** via `prost::Message::encode_length_delimited` — the
//! exact framing `basil` already uses for `BuildEvent`s, so producer and
//! consumer share one framing convention. The raw `events` escape hatch
//! carries pre-encoded length-delimited `BuildEvent`s as opaque `bytes`, so
//! this crate does not have to re-derive the full BES schema to pass them
//! through untouched.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::time::Duration;

use axl_proto::build_event_stream::{
    BuildEvent, BuildEventId, BuildFinished, BuildStarted, TargetComplete,
    build_event::Payload,
    build_event_id::{BuildFinishedId, BuildStartedId, Id, TargetCompletedId},
    build_finished::ExitCode,
};
use prost::Message;

// ─── Wire format: BazelExpectation ───────────────────────────────────────────

/// The terminal outcome a fixture declares. Mirrors the AXL-facing
/// `BuildResult` enum (`passed | failed | cache_evicted`). Synthesized into a
/// `BuildFinished.exit_code` so the AXL read path observes a real,
/// self-consistent BES terminal event.
///
/// Encoded on the wire as the proto enum's `i32` tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BuildResult {
    /// Clean build: every target completes successfully, exit 0.
    #[default]
    Passed = 0,
    /// At least one target fails; default exit code 1.
    Failed = 1,
    /// `REMOTE_CACHE_EVICTED` (Bazel exit 39) — the bug-1060 shape.
    CacheEvicted = 2,
}

impl BuildResult {
    fn from_i32(v: i32) -> Self {
        match v {
            1 => BuildResult::Failed,
            2 => BuildResult::CacheEvicted,
            _ => BuildResult::Passed,
        }
    }

    /// The default Bazel exit code for this result when the fixture does not
    /// override `exit_code` explicitly.
    fn default_exit_code(self) -> i32 {
        match self {
            BuildResult::Passed => 0,
            BuildResult::Failed => 1,
            BuildResult::CacheEvicted => 39,
        }
    }

    /// Whether per-target `TargetComplete` events should report success.
    fn targets_succeed(self) -> bool {
        matches!(self, BuildResult::Passed)
    }
}

/// A declared, typed fixture describing how the fake bazel should behave for
/// one invocation. Serialized over the control channel as length-delimited
/// protobuf (see crate docs).
///
/// This is the Rust mirror of the AXL `BazelExpectation` record minted by
/// `t.bazel.expect_build(...)`.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct BazelExpectation {
    /// Target patterns the fake "built". One `TargetComplete` is synthesized
    /// per entry.
    #[prost(string, repeated, tag = "1")]
    pub targets: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,

    /// Declared terminal result, encoded as [`BuildResult`]'s `i32` tag.
    #[prost(int32, tag = "2")]
    pub result: i32,

    /// Process exit code. `None` (not present) means "derive from `result`".
    #[prost(int32, optional, tag = "3")]
    pub exit_code: ::core::option::Option<i32>,

    /// Raw escape hatch: pre-encoded length-delimited `BuildEvent`s. When
    /// non-empty, these are written verbatim INSTEAD of the synthesized
    /// stream, so a test can express an event sequence the typed surface
    /// can't yet model. Each entry is one already-length-delimited
    /// `BuildEvent` (so basil-core passes them through without re-deriving
    /// the framing).
    #[prost(bytes = "vec", repeated, tag = "4")]
    pub events: ::prost::alloc::vec::Vec<::prost::alloc::vec::Vec<u8>>,
}

impl BazelExpectation {
    /// Construct an expectation from typed inputs. `exit_code = None` defers to
    /// the result's default code.
    pub fn new(targets: Vec<String>, result: BuildResult, exit_code: Option<i32>) -> Self {
        Self {
            targets,
            result: result as i32,
            exit_code,
            events: Vec::new(),
        }
    }

    /// Attach raw pre-encoded length-delimited `BuildEvent`s (escape hatch).
    pub fn with_raw_events(mut self, events: Vec<Vec<u8>>) -> Self {
        self.events = events;
        self
    }

    fn build_result(&self) -> BuildResult {
        BuildResult::from_i32(self.result)
    }

    /// The effective process exit code: explicit override, else the result
    /// default.
    pub fn effective_exit_code(&self) -> i32 {
        self.exit_code
            .unwrap_or_else(|| self.build_result().default_exit_code())
    }

    /// Serialize to a length-delimited protobuf frame (the control-channel
    /// wire format).
    pub fn encode_frame(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode_length_delimited(&mut buf)
            .expect("encode BazelExpectation");
        buf
    }

    /// Read one length-delimited [`BazelExpectation`] frame from `r`.
    pub fn read_frame<R: Read>(mut r: R) -> std::io::Result<Self> {
        let mut bytes = Vec::new();
        r.read_to_end(&mut bytes)?;
        Self::decode_length_delimited(bytes.as_slice())
            .map_err(|e| std::io::Error::other(format!("decode BazelExpectation: {e}")))
    }
}

// ─── Generic synthesis ───────────────────────────────────────────────────────

/// Synthesize the consistent BES event sequence implied by `exp`:
/// `BuildStarted` → one `TargetComplete` per target → `BuildFinished`.
///
/// When `exp.events` is non-empty it is treated as a raw, pre-framed override
/// and returned as-is (already length-delimited), bypassing synthesis.
pub fn synthesize_frames(exp: &BazelExpectation) -> Vec<Vec<u8>> {
    if !exp.events.is_empty() {
        return exp.events.clone();
    }

    let result = exp.build_result();
    let mut events: Vec<BuildEvent> = Vec::with_capacity(exp.targets.len() + 2);
    events.push(build_started());
    for target in &exp.targets {
        events.push(target_complete(target, result.targets_succeed()));
    }
    events.push(build_finished(exp.effective_exit_code(), true));

    events
        .iter()
        .map(|ev| {
            let mut buf = Vec::new();
            ev.encode_length_delimited(&mut buf)
                .expect("encode BuildEvent");
            buf
        })
        .collect()
}

/// Generic replay entrypoint. Synthesizes (or replays the raw escape-hatch)
/// BES stream for `exp` into `bes_path` (the `--build_event_binary_file` the
/// parent wired), then returns the process exit code the caller should exit
/// with.
///
/// `open_delay` widens the window for a late AXL subscriber to land its
/// `.subscribe()` before events fan out (see basil's `Scenario::open_delay`
/// doc); pass `Duration::ZERO` when the test only checks `build.wait()`.
///
/// TODO(increment-2): execlog (`--execution_log_compact_file`) and
/// stdout/stderr synthesis are not yet emitted from the typed fixture. Only
/// the BES surface + exit code are synthesized in this slice.
pub fn replay_expectation(
    exp: &BazelExpectation,
    bes_path: Option<&str>,
    open_delay: Duration,
) -> i32 {
    if let Some(path) = bes_path {
        let frames = synthesize_frames(exp);
        write_frames(path, &frames, open_delay);
    }
    exp.effective_exit_code()
}

/// One open/write/close cycle on `path` writing every frame in `frames`
/// (each already length-delimited). The read side observes a writer appear,
/// drain bytes, and disappear — same shape as Bazel writing the BEP file.
pub fn write_frames(path: &str, frames: &[Vec<u8>], open_delay: Duration) {
    let mut f = OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap_or_else(|e| panic!("basil-core: opening BES path {path:?} for write: {e}"));
    if !open_delay.is_zero() {
        std::thread::sleep(open_delay);
    }
    for frame in frames {
        f.write_all(frame)
            .unwrap_or_else(|e| panic!("basil-core: writing to BES path: {e}"));
    }
}

// ─── BES event constructors ──────────────────────────────────────────────────

pub fn build_started() -> BuildEvent {
    BuildEvent {
        id: Some(BuildEventId {
            id: Some(Id::Started(BuildStartedId {})),
        }),
        last_message: false,
        payload: Some(Payload::Started(BuildStarted::default())),
        ..Default::default()
    }
}

pub fn build_finished(code: i32, last: bool) -> BuildEvent {
    BuildEvent {
        id: Some(BuildEventId {
            id: Some(Id::BuildFinished(BuildFinishedId {})),
        }),
        last_message: last,
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

/// A `TargetComplete` event for `label`, reporting `success`.
pub fn target_complete(label: &str, success: bool) -> BuildEvent {
    BuildEvent {
        id: Some(BuildEventId {
            id: Some(Id::TargetCompleted(TargetCompletedId {
                label: label.to_string(),
                ..Default::default()
            })),
        }),
        last_message: false,
        payload: Some(Payload::Completed(TargetComplete {
            success,
            ..Default::default()
        })),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expectation_round_trips_through_the_wire_frame() {
        let exp = BazelExpectation::new(
            vec!["//a:b".into(), "//c:d".into()],
            BuildResult::Failed,
            Some(7),
        );
        let frame = exp.encode_frame();
        let decoded = BazelExpectation::read_frame(frame.as_slice()).unwrap();
        assert_eq!(decoded.targets, vec!["//a:b", "//c:d"]);
        assert_eq!(decoded.build_result(), BuildResult::Failed);
        assert_eq!(decoded.effective_exit_code(), 7);
    }

    #[test]
    fn default_exit_codes_follow_the_result() {
        assert_eq!(
            BazelExpectation::new(vec![], BuildResult::Passed, None).effective_exit_code(),
            0
        );
        assert_eq!(
            BazelExpectation::new(vec![], BuildResult::Failed, None).effective_exit_code(),
            1
        );
        assert_eq!(
            BazelExpectation::new(vec![], BuildResult::CacheEvicted, None).effective_exit_code(),
            39
        );
    }

    #[test]
    fn synthesizes_started_then_target_then_finished() {
        let exp = BazelExpectation::new(vec!["//x:y".into()], BuildResult::Passed, None);
        let frames = synthesize_frames(&exp);
        // BuildStarted + one TargetComplete + BuildFinished.
        assert_eq!(frames.len(), 3);
        // Decode the middle frame and confirm it's a successful TargetComplete.
        let ev = BuildEvent::decode_length_delimited(frames[1].as_slice()).unwrap();
        match ev.payload {
            Some(Payload::Completed(tc)) => assert!(tc.success),
            other => panic!("expected TargetComplete, got {other:?}"),
        }
    }

    #[test]
    fn raw_events_escape_hatch_bypasses_synthesis() {
        let raw = {
            let mut buf = Vec::new();
            build_started().encode_length_delimited(&mut buf).unwrap();
            buf
        };
        let exp = BazelExpectation::new(vec!["//x:y".into()], BuildResult::Passed, None)
            .with_raw_events(vec![raw.clone()]);
        let frames = synthesize_frames(&exp);
        assert_eq!(frames, vec![raw]);
    }
}
