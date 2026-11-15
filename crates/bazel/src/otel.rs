//! Spans for work Bazel already finished, timed from what Bazel reported.
//!
//! A `tracing` span is timed by when it is entered and dropped, so a span for
//! an action, test or spawn that ended before its record arrived is built as a
//! [`CompletedSpan`] and exported with an OpenTelemetry tracer instead,
//! parented to the current `tracing` span through
//! `OpenTelemetrySpanExt::context`. That parent link needs the tracer and the
//! `tracing-opentelemetry` layer to share one provider, which `aspect-cli`'s
//! `trace::init` arranges by installing it globally.

use std::time::{Duration, SystemTime};

use axl_proto::Timestamp;
use opentelemetry::{
    KeyValue,
    trace::{Span as _, Status, Tracer},
};
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

#[derive(Debug)]
pub(crate) struct CompletedSpan {
    pub name: &'static str,
    pub start: SystemTime,
    pub end: SystemTime,
    pub status: Status,
    pub attributes: Vec<KeyValue>,
}

impl CompletedSpan {
    /// Exports the span as a child of the current `tracing` span. An end
    /// before the start is clamped to the start.
    pub fn export<T: Tracer>(self, tracer: &T) {
        let parent = tracing::Span::current().context();
        let mut span = tracer
            .span_builder(self.name)
            .with_start_time(self.start)
            .with_attributes(self.attributes)
            .start_with_context(tracer, &parent);
        span.set_status(self.status);
        span.end_with_timestamp(self.end.max(self.start));
    }
}

/// A protobuf timestamp as a `SystemTime`; negative fields clamp to zero.
pub(crate) fn system_time(ts: &Timestamp) -> SystemTime {
    SystemTime::UNIX_EPOCH
        + duration(&axl_proto::Duration {
            seconds: ts.seconds,
            nanos: ts.nanos,
        })
}

/// A protobuf duration as a `Duration`; negative fields clamp to zero.
pub(crate) fn duration(d: &axl_proto::Duration) -> Duration {
    Duration::new(d.seconds.max(0) as u64, d.nanos.max(0) as u32)
}
