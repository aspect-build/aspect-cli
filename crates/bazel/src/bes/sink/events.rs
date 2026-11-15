//! Field helpers for BES events, shared by the trace and metrics sinks:
//! timestamps and durations with their deprecated `*_millis` fallbacks, and
//! test-attempt caching.

use std::time::{Duration, SystemTime};

use axl_proto::{Timestamp, build_event_stream::TestResult};

pub(super) fn system_time(ts: &Timestamp) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::new(ts.seconds.max(0) as u64, ts.nanos.max(0) as u32)
}

/// Prefers the `Timestamp` field, then the deprecated `*_millis` field, then
/// `now`. Older Bazel versions and cached test summaries populate only the
/// millis field.
pub(super) fn time_or_legacy(timestamp: Option<&Timestamp>, legacy_millis: i64) -> SystemTime {
    match timestamp {
        Some(t) => system_time(t),
        None if legacy_millis > 0 => {
            SystemTime::UNIX_EPOCH + Duration::from_millis(legacy_millis as u64)
        }
        None => SystemTime::now(),
    }
}

/// Prefers the `Duration` field, then the deprecated `*_millis` field.
fn duration_or_legacy(duration: Option<&axl_proto::Duration>, legacy_millis: i64) -> Duration {
    match duration {
        Some(d) => Duration::new(d.seconds.max(0) as u64, d.nanos.max(0) as u32),
        None => Duration::from_millis(legacy_millis.max(0) as u64),
    }
}

/// How long the attempt ran, in whichever run produced it.
pub(super) fn attempt_duration(result: &TestResult) -> Duration {
    #[allow(deprecated)]
    duration_or_legacy(
        result.test_attempt_duration.as_ref(),
        result.test_attempt_duration_millis,
    )
}

/// Whether the attempt came from a local or remote cache.
pub(super) fn attempt_cached(result: &TestResult) -> bool {
    result.cached_locally
        || result
            .execution_info
            .as_ref()
            .is_some_and(|info| info.cached_remotely)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axl_proto::build_event_stream::test_result::ExecutionInfo;

    fn at(seconds: u64, nanos: u32) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::new(seconds, nanos)
    }

    #[test]
    fn timestamp_keeps_nanos() {
        let ts = Timestamp {
            seconds: 42,
            nanos: 123_456_789,
        };
        assert_eq!(time_or_legacy(Some(&ts), 999_000), at(42, 123_456_789));
    }

    #[test]
    fn timestamp_falls_back_to_legacy_millis() {
        assert_eq!(
            time_or_legacy(None, 1_723_000_000_250),
            at(1_723_000_000, 250_000_000)
        );
    }

    #[test]
    fn timestamp_uses_now_when_nothing_populated() {
        let before = SystemTime::now();
        assert!(time_or_legacy(None, 0) >= before);
    }

    #[test]
    #[allow(deprecated)]
    fn attempt_duration_falls_back_to_legacy_millis() {
        let legacy = TestResult {
            test_attempt_duration_millis: 1_500,
            ..Default::default()
        };
        assert_eq!(attempt_duration(&legacy), Duration::from_millis(1_500));
        let modern = TestResult {
            test_attempt_duration: Some(axl_proto::Duration {
                seconds: 1,
                nanos: 5,
            }),
            test_attempt_duration_millis: 9_999,
            ..Default::default()
        };
        assert_eq!(attempt_duration(&modern), Duration::new(1, 5));
    }

    #[test]
    fn attempt_cached_covers_local_and_remote() {
        let remote = TestResult {
            execution_info: Some(ExecutionInfo {
                cached_remotely: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let local = TestResult {
            cached_locally: true,
            ..Default::default()
        };
        assert!(attempt_cached(&remote));
        assert!(attempt_cached(&local));
        assert!(!attempt_cached(&TestResult::default()));
    }
}
