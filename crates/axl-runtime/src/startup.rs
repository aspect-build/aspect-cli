//! Start-up timing: how long the CLI takes from the start of `main` until the
//! selected task begins executing.
//!
//! `main` calls [`mark_process_start`] first and times each start-up step with
//! [`step`], [`step_async`], [`time`] or a [`StepTimer`]. Each step:
//!
//! * runs under an INFO span named `startup.<step>`, so an OTLP exporter
//!   receives the steps as children of the `root` span (spans from before the
//!   exporters are installed are buffered and replayed). A step timed with
//!   [`time`] gets no span of its own: it already has one, or it runs before
//!   tracing exists;
//! * appends its duration to a process-wide list.
//!
//! When the task is about to run, [`report`] closes the measurement and, when
//! an exporter is registered, emits one `startup` event on the `axl.event`
//! target: its `description` is a
//! human-readable breakdown, which `ASPECT_DEBUG=1` prints to stderr, and its
//! `fields` are the same numbers in milliseconds as JSON. [`total`] then
//! returns the measured time for callers that attach it elsewhere.
//!
//! The steps run one after another, so whatever the total does not account
//! for is reported as `other`.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static PROCESS_START: OnceLock<Instant> = OnceLock::new();
static STEPS: Mutex<Vec<(&'static str, Duration)>> = Mutex::new(Vec::new());
static TOTAL: OnceLock<Duration> = OnceLock::new();

/// Record the start of the measurement. The first call wins.
pub fn mark_process_start() {
    PROCESS_START.get_or_init(Instant::now);
}

/// The instant [`mark_process_start`] recorded, or the first time this is
/// asked in a process that never marked it.
pub fn process_start() -> Instant {
    *PROCESS_START.get_or_init(Instant::now)
}

/// Times one start-up step from construction until [`StepTimer::finish`],
/// with its `startup.<name>` span entered on the current thread meanwhile.
/// For a step whose body cannot be a closure; prefer [`step`].
pub struct StepTimer {
    name: &'static str,
    started: Instant,
    _span: tracing::span::EnteredSpan,
}

impl StepTimer {
    pub fn start(name: &'static str) -> Self {
        Self {
            name,
            started: Instant::now(),
            _span: span(name).entered(),
        }
    }

    /// Record the step. A timer dropped without this records nothing, which
    /// is what an early return out of the step wants.
    pub fn finish(self) {
        record(self.name, self.started.elapsed());
    }
}

/// Run `f` as start-up step `name`.
pub fn step<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let timer = StepTimer::start(name);
    let out = f();
    timer.finish();
    out
}

/// Await `f` as start-up step `name`.
pub async fn step_async<T>(name: &'static str, f: impl Future<Output = T>) -> T {
    use tracing::Instrument;
    let started = Instant::now();
    let out = f.instrument(span(name)).await;
    record(name, started.elapsed());
    out
}

fn span(name: &'static str) -> tracing::Span {
    tracing::info_span!("startup", otel.name = %format_args!("startup.{name}"))
}

/// Run `f` as start-up step `name` without a span of its own; see the
/// module docs.
pub fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let out = f();
    record(name, started.elapsed());
    out
}

fn record(name: &'static str, elapsed: Duration) {
    if let Ok(mut steps) = STEPS.lock() {
        steps.push((name, elapsed));
    }
}

/// The steps recorded so far, in the order they finished.
pub fn steps() -> Vec<(&'static str, Duration)> {
    STEPS.lock().map(|s| s.clone()).unwrap_or_default()
}

/// The start-up time, once [`report`] has closed the measurement.
pub fn total() -> Option<Duration> {
    TOTAL.get().copied()
}

/// Close the measurement for `task` and emit the `startup` event, when any
/// exporter is listening. Only the first call reports.
pub fn report(task: &str) {
    let total = process_start().elapsed();
    if TOTAL.set(total).is_err() {
        return;
    }
    emit(task, total, &steps());
}

/// Emit the `startup` event for `steps` within `total`, unless no exporter
/// is listening.
fn emit(task: &str, total: Duration, steps: &[(&'static str, Duration)]) {
    if !crate::trace::enabled() {
        return;
    }
    let (description, fields) = summarize(task, total, steps);
    tracing::info!(
        target: "axl.event",
        message = "startup",
        description = %description,
        category = "aspect",
        error = "",
        fields = %fields,
    );
}

/// The event's `description` and `fields` for `steps` within `total`. A
/// name recorded more than once is reported once, with its durations summed.
fn summarize(
    task: &str,
    total: Duration,
    steps: &[(&'static str, Duration)],
) -> (String, serde_json::Value) {
    let mut merged: Vec<(&'static str, Duration)> = Vec::new();
    for (name, d) in steps {
        match merged.iter_mut().find(|(n, _)| n == name) {
            Some((_, sum)) => *sum += *d,
            None => merged.push((name, *d)),
        }
    }
    let steps = &merged;
    let other = total.saturating_sub(steps.iter().map(|(_, d)| *d).sum());
    let parts: Vec<String> = steps
        .iter()
        .chain([&("other", other)])
        .map(|(name, d)| format!("{name} {}ms", d.as_millis()))
        .collect();
    let description = format!(
        "{task} started in {}ms ({})",
        total.as_millis(),
        parts.join(", ")
    );

    let mut fields = serde_json::Map::new();
    fields.insert("total_ms".into(), ms(total).into());
    for (name, d) in steps.iter().chain([&("other", other)]) {
        fields.insert(format!("{name}_ms"), ms(*d).into());
    }
    (description, fields.into())
}

/// Milliseconds, to the microsecond.
fn ms(d: Duration) -> f64 {
    d.as_micros() as f64 / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_returns_its_value_and_is_recorded() {
        assert_eq!(step("test_step", || 7), 7);
        assert!(steps().iter().any(|(name, _)| *name == "test_step"));
    }

    #[tokio::test]
    async fn step_async_returns_its_value_and_is_recorded() {
        assert_eq!(step_async("test_async_step", async { 7 }).await, 7);
        assert!(steps().iter().any(|(name, _)| *name == "test_async_step"));
    }

    #[test]
    fn time_returns_its_value_and_is_recorded() {
        assert_eq!(time("test_untraced_step", || 7), 7);
        assert!(
            steps()
                .iter()
                .any(|(name, _)| *name == "test_untraced_step")
        );
    }

    #[test]
    fn only_the_first_report_counts() {
        report("first");
        let first = total().expect("reported");
        std::thread::sleep(Duration::from_millis(2));
        report("second");
        assert_eq!(total(), Some(first));
    }

    #[test]
    fn a_timer_dropped_without_finish_records_nothing() {
        drop(StepTimer::start("test_abandoned"));
        assert!(!steps().iter().any(|(name, _)| *name == "test_abandoned"));
    }

    #[test]
    fn summarize_accounts_for_every_millisecond() {
        let steps = [
            ("a", Duration::from_millis(10)),
            ("b", Duration::from_millis(25)),
        ];
        let (description, fields) = summarize("build", Duration::from_millis(40), &steps);
        assert_eq!(
            description,
            "build started in 40ms (a 10ms, b 25ms, other 5ms)"
        );
        assert_eq!(
            fields,
            serde_json::json!({"total_ms": 40.0, "a_ms": 10.0, "b_ms": 25.0, "other_ms": 5.0})
        );
    }

    /// The event carries the slots the file sinks render: `message` as
    /// its name, the breakdown in `description`, JSON in `fields`.
    #[test]
    fn emit_writes_the_startup_event() {
        use crate::trace::telemetry_tests::capture_events;
        let steps = [("discover", Duration::from_millis(30))];
        let events = capture_events(|| emit("build", Duration::from_millis(40), &steps));
        let [event] = events.as_slice() else {
            panic!("expected one event, got {events:?}");
        };
        assert_eq!(event.target, "axl.event");
        let field = |name: &str| event.fields.get(name).map(String::as_str);
        assert_eq!(field("message"), Some("startup"));
        assert_eq!(field("category"), Some("aspect"));
        assert_eq!(
            field("description"),
            Some("build started in 40ms (discover 30ms, other 10ms)")
        );
        let fields: serde_json::Value = serde_json::from_str(field("fields").unwrap()).unwrap();
        assert_eq!(fields["discover_ms"], 30.0);
    }

    /// With no exporter registered, nothing is emitted.
    #[test]
    fn emit_is_silent_without_an_exporter() {
        use crate::trace::telemetry_tests::capture_events;
        let events = capture_events(|| {
            crate::trace::disable();
            emit("build", Duration::from_millis(1), &[]);
            crate::trace::enable();
        });
        assert!(events.is_empty(), "{events:?}");
    }

    #[test]
    fn summarize_merges_a_repeated_step() {
        let steps = [
            ("a", Duration::from_millis(10)),
            ("b", Duration::from_millis(5)),
            ("a", Duration::from_millis(15)),
        ];
        let (description, fields) = summarize("build", Duration::from_millis(30), &steps);
        assert_eq!(
            description,
            "build started in 30ms (a 25ms, b 5ms, other 0ms)"
        );
        assert_eq!(fields["a_ms"], 25.0);
    }

    /// Steps that overrun the total (a clock read after the last step) report
    /// `other` as zero rather than wrapping.
    #[test]
    fn summarize_never_reports_negative_other() {
        let steps = [("a", Duration::from_millis(50))];
        let (description, _) = summarize("build", Duration::from_millis(40), &steps);
        assert!(description.ends_with("other 0ms)"), "{description}");
    }
}
