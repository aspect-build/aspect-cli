//! The CLI's own metrics, on the `aspect-cli` meter.
//!
//! | Instrument | Unit | Attributes |
//! |---|---|---|
//! | `aspect.cli.startup.duration` | `s` | `task` |
//! | `aspect.cli.task.duration` | `s` | `task`, `exit_code` |
//!
//! `task` is the task's kind (`build`, `test`, …), not its name, so the
//! number of series stays bounded. Both are histograms in seconds with
//! boundaries for the durations they measure; the SDK's defaults are sized
//! for milliseconds.

use std::time::Duration;

use opentelemetry::{KeyValue, global};

const METER: &str = "aspect-cli";

/// The OTel semantic-convention boundaries for durations, in seconds.
const STARTUP_BOUNDARIES: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

/// From a quick `aspect version`-sized task to a multi-hour CI build.
const TASK_BOUNDARIES: &[f64] = &[
    0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1200.0, 1800.0, 3600.0, 7200.0,
];

/// Time from the start of `main` until the selected task begins executing.
pub fn record_startup(task: &str, elapsed: Duration) {
    histogram(
        "aspect.cli.startup.duration",
        "Time from the start of aspect-cli's main until the selected task begins executing",
        STARTUP_BOUNDARIES,
    )
    .record(
        elapsed.as_secs_f64(),
        &[KeyValue::new("task", task.to_owned())],
    );
}

/// One task run, from the start of its pre-task hooks to the end of its
/// defers, and the code it exited with.
pub fn record_task(task: &str, exit_code: u8, elapsed: Duration) {
    histogram(
        "aspect.cli.task.duration",
        "Duration of a task run, by task kind and exit code",
        TASK_BOUNDARIES,
    )
    .record(
        elapsed.as_secs_f64(),
        &[
            KeyValue::new("task", task.to_owned()),
            KeyValue::new("exit_code", i64::from(exit_code)),
        ],
    );
}

fn histogram(
    name: &'static str,
    description: &'static str,
    boundaries: &[f64],
) -> opentelemetry::metrics::Histogram<f64> {
    global::meter(METER)
        .f64_histogram(name)
        .with_unit("s")
        .with_description(description)
        .with_boundaries(boundaries.to_vec())
        .build()
}
