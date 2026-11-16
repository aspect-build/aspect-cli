//! In-memory buffering OTLP exporters with late attachment.
//!
//! At process startup the SDK is wired with `BufferingSpanExporter` and
//! `BufferingLogExporter`. They accumulate everything in memory until
//! `install_late(...)` is called (after phase 3 completes), at which point
//! the buffer is replayed to the newly-attached real OTLP exporters and
//! subsequent exports fan out live.
//!
//! Bound: each buffer holds at most `BUFFER_CAP` entries; the oldest are
//! evicted on overflow. Phases 1-3 typically produce well under that
//! (a few hundred records).
//!
//! Spans can be lost on the way to a collector in three places: the batch
//! processor's queue, the start-up buffer, and a failed export. A
//! [`SpanTally`] counts each, shared by the exporter and the
//! [`CountingSpanProcessor`] in front of the batch processor, so the CLI can
//! say how many never arrived.

use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use opentelemetry::{Context, InstrumentationScope};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{LogBatch, LogExporter, SdkLogRecord};
use opentelemetry_sdk::trace::{Span, SpanData, SpanExporter, SpanProcessor};

const BUFFER_CAP: usize = 50_000;

/// Span counts from which [`SpanTally::loss`] works out what never reached a
/// collector.
#[derive(Debug, Default)]
pub struct SpanTally {
    /// Sampled spans that ended, counted before the batch processor's queue.
    ended: AtomicU64,
    /// Spans the batch processor handed to the exporter.
    received: AtomicU64,
    /// Spans evicted from the full start-up buffer.
    evicted: AtomicU64,
    /// Spans in batches an exporter failed to send, summed over exporters.
    failed: AtomicU64,
}

/// Spans that never reached a collector, by where they were lost.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SpanLoss {
    /// Ended but never handed to the exporter: the batch queue was full, or
    /// shutdown timed out before the queue drained.
    pub queued: u64,
    /// Evicted from the start-up buffer before the exporters were installed.
    pub evicted: u64,
    /// Sent in a batch an exporter failed to deliver.
    pub failed: u64,
}

impl SpanTally {
    pub fn loss(&self) -> SpanLoss {
        let received = self.received.load(Ordering::Relaxed);
        SpanLoss {
            queued: self.ended.load(Ordering::Relaxed).saturating_sub(received),
            evicted: self.evicted.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
        }
    }

    fn add(counter: &AtomicU64, n: usize) {
        counter.fetch_add(n as u64, Ordering::Relaxed);
    }
}

/// Counts every span that ends into a [`SpanTally`], then passes it to
/// `inner`. Placed in front of the batch processor, whose queue drops
/// spans when it is full.
#[derive(Debug)]
pub struct CountingSpanProcessor<P> {
    inner: P,
    tally: Arc<SpanTally>,
}

impl<P: SpanProcessor> CountingSpanProcessor<P> {
    pub fn new(inner: P, tally: Arc<SpanTally>) -> Self {
        Self { inner, tally }
    }
}

impl<P: SpanProcessor> SpanProcessor for CountingSpanProcessor<P> {
    fn on_start(&self, span: &mut Span, cx: &Context) {
        self.inner.on_start(span, cx);
    }

    fn on_end(&self, span: SpanData) {
        SpanTally::add(&self.tally.ended, 1);
        self.inner.on_end(span);
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

/// Span exporter with an in-memory buffer that flips to fan-out mode after
/// `install_late`. Cheap to clone — internally an `Arc`. Generic over the
/// late exporter so tests can attach one that fails.
pub struct BufferingSpanExporter<E = opentelemetry_otlp::SpanExporter> {
    inner: Arc<Mutex<SpanState<E>>>,
    tally: Arc<SpanTally>,
}

impl<E> Clone for BufferingSpanExporter<E> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            tally: Arc::clone(&self.tally),
        }
    }
}

impl<E> fmt::Debug for BufferingSpanExporter<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufferingSpanExporter").finish()
    }
}

struct SpanState<E> {
    buffer: Vec<SpanData>,
    /// Late exporters wrapped in `Arc` so `export()` can release the mutex
    /// before awaiting (`MutexGuard<std::sync::Mutex<...>>` is `!Send`).
    /// Resources are applied by the caller of `install_late` before wrapping.
    late: Vec<Arc<E>>,
    replayed: bool,
    /// Most spans `buffer` holds; [`BUFFER_CAP`] outside tests.
    cap: usize,
}

impl<E: SpanExporter> BufferingSpanExporter<E> {
    pub fn new(tally: Arc<SpanTally>) -> Self {
        Self::with_cap(tally, BUFFER_CAP)
    }

    fn with_cap(tally: Arc<SpanTally>, cap: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SpanState {
                buffer: Vec::new(),
                late: Vec::new(),
                replayed: false,
                cap,
            })),
            tally,
        }
    }

    pub fn tally(&self) -> Arc<SpanTally> {
        Arc::clone(&self.tally)
    }

    /// What never reached a collector, or `None` when no exporter was
    /// installed and nothing was meant to.
    pub fn loss(&self) -> Option<SpanLoss> {
        let st = self.inner.lock().expect("span buffer mutex poisoned");
        (!st.late.is_empty()).then(|| self.tally.loss())
    }

    /// Drain the buffer to the new exporters and switch to fan-out mode for
    /// subsequent exports; with no exporters, the buffer and every later
    /// batch are dropped. Must run within a tokio runtime context (the gRPC
    /// export futures need its reactor). The caller is responsible for
    /// setting each exporter's resource before passing it in.
    pub async fn install_late(&self, exporters: Vec<E>) {
        let arc_exporters: Vec<Arc<E>> = exporters.into_iter().map(Arc::new).collect();

        // Atomically: install late exporters, flip replayed, take buffer.
        // Concurrent `export()` calls after this point fan out via the
        // late exporters; concurrent calls before this point have already
        // pushed into `buffer` (which we drain).
        let buffer = {
            let mut st = self.inner.lock().expect("span buffer mutex poisoned");
            for ex in &arc_exporters {
                st.late.push(Arc::clone(ex));
            }
            st.replayed = true;
            std::mem::take(&mut st.buffer)
        };

        if buffer.is_empty() {
            return;
        }

        // Replay outside the lock. Total ordering at the collector is
        // determined by each record's timestamp, so interleaving with any
        // live batches that arrive during replay is harmless.
        self.fan_out(&arc_exporters, buffer).await;
    }

    async fn fan_out(&self, exporters: &[Arc<E>], batch: Vec<SpanData>) {
        for ex in exporters {
            if ex.export(batch.clone()).await.is_err() {
                SpanTally::add(&self.tally.failed, batch.len());
            }
        }
    }
}

impl<E: SpanExporter> SpanExporter for BufferingSpanExporter<E> {
    async fn export(&self, mut batch: Vec<SpanData>) -> OTelSdkResult {
        SpanTally::add(&self.tally.received, batch.len());
        // Decide and act under one lock acquisition. If buffering, push
        // under the same lock as the `replayed` check and early-return
        // (so `install_late` either drains this batch as part of the
        // buffer, or we already saw `replayed=true` and fan out). If we
        // released the lock before pushing, an interleaved `install_late`
        // could flip `replayed`, drain the buffer, and our subsequent
        // push would land in a buffer that's never replayed — silently
        // dropped. We hold a `MutexGuard<std::sync::Mutex<...>>` only
        // until the early return; the await path drops the guard at the
        // end of this block before any `.await`, keeping the future
        // `Send`.
        let exporters: Vec<Arc<E>> = {
            let mut st = self.inner.lock().expect("span buffer mutex poisoned");
            if !st.replayed {
                let overflow = (st.buffer.len() + batch.len()).saturating_sub(st.cap);
                let from_buffer = overflow.min(st.buffer.len());
                st.buffer.drain(..from_buffer);
                batch.drain(..overflow - from_buffer);
                SpanTally::add(&self.tally.evicted, overflow);
                st.buffer.extend(batch);
                return Ok(());
            }
            st.late.iter().map(Arc::clone).collect()
        };
        self.fan_out(&exporters, batch).await;
        Ok(())
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        let st = self.inner.lock().expect("span buffer mutex poisoned");
        for ex in st.late.iter() {
            let _ = ex.shutdown_with_timeout(timeout);
        }
        Ok(())
    }

    fn set_resource(&mut self, _resource: &Resource) {}
}

/// Log exporter, same buffer-then-fan-out shape as `BufferingSpanExporter`.
#[derive(Clone)]
pub struct BufferingLogExporter {
    inner: Arc<Mutex<LogState>>,
}

impl fmt::Debug for BufferingLogExporter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufferingLogExporter").finish()
    }
}

struct LogState {
    buffer: Vec<(SdkLogRecord, InstrumentationScope)>,
    late: Vec<Arc<opentelemetry_otlp::LogExporter>>,
    replayed: bool,
}

impl BufferingLogExporter {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(LogState {
                buffer: Vec::new(),
                late: Vec::new(),
                replayed: false,
            })),
        }
    }

    /// Same contract as [`BufferingSpanExporter::install_late`].
    pub async fn install_late(&self, exporters: Vec<opentelemetry_otlp::LogExporter>) {
        let arc_exporters: Vec<Arc<opentelemetry_otlp::LogExporter>> =
            exporters.into_iter().map(Arc::new).collect();

        let buffer = {
            let mut st = self.inner.lock().expect("log buffer mutex poisoned");
            for ex in &arc_exporters {
                st.late.push(Arc::clone(ex));
            }
            st.replayed = true;
            std::mem::take(&mut st.buffer)
        };

        if buffer.is_empty() {
            return;
        }

        // LogBatch::new_with_owned_data is crate-private; build a borrowed
        // LogBatch from references into the still-owned buffer.
        let pairs: Vec<(&SdkLogRecord, &InstrumentationScope)> =
            buffer.iter().map(|(r, s)| (r, s)).collect();
        for ex in arc_exporters {
            let _ = ex.export(LogBatch::new(&pairs)).await;
        }
    }
}

impl LogExporter for BufferingLogExporter {
    async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
        // Same atomic-decide-and-act as `BufferingSpanExporter::export`:
        // do the buffer push under the same lock as the `replayed` check
        // so an interleaved `install_late` cannot drain the buffer
        // between our check and our push. The await path drops the guard
        // at the end of this block before `.await`, keeping the future
        // `Send`.
        let exporters: Vec<Arc<opentelemetry_otlp::LogExporter>> = {
            let mut st = self.inner.lock().expect("log buffer mutex poisoned");
            if !st.replayed {
                for (r, s) in batch.iter() {
                    if st.buffer.len() >= BUFFER_CAP {
                        st.buffer.remove(0);
                    }
                    st.buffer.push((r.clone(), s.clone()));
                }
                return Ok(());
            }
            st.late.iter().map(Arc::clone).collect()
        };
        let pairs: Vec<(&SdkLogRecord, &InstrumentationScope)> = batch.iter().collect();
        for ex in exporters {
            let _ = ex.export(LogBatch::new(&pairs)).await;
        }
        Ok(())
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        let st = self.inner.lock().expect("log buffer mutex poisoned");
        for ex in st.late.iter() {
            let _ = ex.shutdown_with_timeout(timeout);
        }
        Ok(())
    }

    fn set_resource(&mut self, _resource: &Resource) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::logs::{LogRecord as _, Logger as _, LoggerProvider as _};
    use opentelemetry::trace::{Tracer as _, TracerProvider as _};
    use opentelemetry_sdk::error::OTelSdkError;
    use opentelemetry_sdk::logs::SdkLoggerProvider;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SimpleSpanProcessor};

    type TestBuffer = BufferingSpanExporter<InMemorySpanExporter>;

    fn test_buffer() -> TestBuffer {
        BufferingSpanExporter::new(Arc::new(SpanTally::default()))
    }

    fn span_provider<E: SpanExporter + 'static>(
        buffer: &BufferingSpanExporter<E>,
    ) -> SdkTracerProvider {
        SdkTracerProvider::builder()
            .with_span_processor(CountingSpanProcessor::new(
                SimpleSpanProcessor::new(buffer.clone()),
                buffer.tally(),
            ))
            .build()
    }

    fn emit_span(provider: &SdkTracerProvider) {
        provider.tracer("t").in_span("s", |_| {});
    }

    fn span_state<E>(buffer: &BufferingSpanExporter<E>) -> (usize, bool) {
        let st = buffer.inner.lock().unwrap();
        (st.buffer.len(), st.replayed)
    }

    #[test]
    fn spans_buffer_until_installed() {
        let buffer = test_buffer();
        let provider = span_provider(&buffer);
        emit_span(&provider);
        emit_span(&provider);
        assert_eq!(span_state(&buffer), (2, false));
    }

    #[test]
    fn installing_no_span_exporters_ends_buffering() {
        let buffer = test_buffer();
        let provider = span_provider(&buffer);
        emit_span(&provider);
        futures::executor::block_on(buffer.install_late(Vec::new()));
        assert_eq!(span_state(&buffer), (0, true));
        emit_span(&provider);
        assert_eq!(span_state(&buffer), (0, true));
        assert_eq!(buffer.loss(), None, "nothing was meant to be exported");
    }

    #[test]
    fn buffered_and_live_spans_reach_the_installed_exporter() {
        let buffer = test_buffer();
        let provider = span_provider(&buffer);
        emit_span(&provider);
        let exporter = InMemorySpanExporter::default();
        futures::executor::block_on(buffer.install_late(vec![exporter.clone()]));
        emit_span(&provider);
        assert_eq!(exporter.get_finished_spans().unwrap().len(), 2);
        assert_eq!(buffer.loss(), Some(SpanLoss::default()));
    }

    #[test]
    fn a_full_start_up_buffer_evicts_the_oldest_spans() {
        let buffer: TestBuffer = BufferingSpanExporter::with_cap(Arc::new(SpanTally::default()), 2);
        let provider = span_provider(&buffer);
        for name in ["a", "b", "c"] {
            provider.tracer("t").in_span(name, |_| {});
        }
        let exporter = InMemorySpanExporter::default();
        futures::executor::block_on(buffer.install_late(vec![exporter.clone()]));
        let names: Vec<_> = exporter
            .get_finished_spans()
            .unwrap()
            .into_iter()
            .map(|s| s.name.into_owned())
            .collect();
        assert_eq!(names, ["b", "c"]);
        assert_eq!(buffer.loss().unwrap().evicted, 1);
    }

    #[derive(Debug)]
    struct Failing;

    impl SpanExporter for Failing {
        async fn export(&self, _batch: Vec<SpanData>) -> OTelSdkResult {
            Err(OTelSdkError::InternalFailure(
                "unreachable collector".into(),
            ))
        }
    }

    #[test]
    fn spans_an_exporter_rejects_are_counted_as_failed() {
        let buffer: BufferingSpanExporter<Failing> =
            BufferingSpanExporter::new(Arc::new(SpanTally::default()));
        let provider = span_provider(&buffer);
        emit_span(&provider);
        futures::executor::block_on(buffer.install_late(vec![Failing]));
        emit_span(&provider);
        assert_eq!(
            buffer.loss(),
            Some(SpanLoss {
                failed: 2,
                ..SpanLoss::default()
            })
        );
    }

    /// Stands in for a batch processor whose queue is full.
    #[derive(Debug)]
    struct Dropping;

    impl SpanProcessor for Dropping {
        fn on_start(&self, _span: &mut Span, _cx: &Context) {}
        fn on_end(&self, _span: SpanData) {}
        fn force_flush(&self) -> OTelSdkResult {
            Ok(())
        }
        fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
            Ok(())
        }
    }

    #[test]
    fn spans_that_end_but_never_reach_the_exporter_are_counted_as_queued() {
        let buffer = test_buffer();
        let provider = SdkTracerProvider::builder()
            .with_span_processor(CountingSpanProcessor::new(Dropping, buffer.tally()))
            .build();
        emit_span(&provider);
        emit_span(&provider);
        futures::executor::block_on(buffer.install_late(vec![InMemorySpanExporter::default()]));
        assert_eq!(buffer.loss().unwrap().queued, 2);
    }

    #[test]
    fn installing_no_log_exporters_ends_buffering() {
        let buffer = BufferingLogExporter::new();
        let provider = SdkLoggerProvider::builder()
            .with_simple_exporter(buffer.clone())
            .build();
        let logger = provider.logger("t");
        let emit = || {
            let mut record = logger.create_log_record();
            record.set_body("hello".into());
            logger.emit(record);
        };
        let state = || {
            let st = buffer.inner.lock().unwrap();
            (st.buffer.len(), st.replayed)
        };

        emit();
        assert_eq!(state(), (1, false));
        futures::executor::block_on(buffer.install_late(Vec::new()));
        assert_eq!(state(), (0, true));
        emit();
        assert_eq!(state(), (0, true));
    }
}
