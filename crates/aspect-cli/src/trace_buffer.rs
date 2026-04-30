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

use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use opentelemetry::InstrumentationScope;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{LogBatch, LogExporter, SdkLogRecord};
use opentelemetry_sdk::trace::{SpanData, SpanExporter};

const BUFFER_CAP: usize = 50_000;

/// Span exporter with an in-memory buffer that flips to fan-out mode after
/// `install_late`. Cheap to clone — internally an `Arc`.
#[derive(Clone)]
pub struct BufferingSpanExporter {
    inner: Arc<Mutex<SpanState>>,
}

impl fmt::Debug for BufferingSpanExporter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufferingSpanExporter").finish()
    }
}

struct SpanState {
    buffer: Vec<SpanData>,
    /// Late exporters wrapped in `Arc` so `export()` can release the mutex
    /// before awaiting (`MutexGuard<std::sync::Mutex<...>>` is `!Send`).
    /// Resources are applied by the caller of `install_late` before wrapping.
    late: Vec<Arc<opentelemetry_otlp::SpanExporter>>,
    replayed: bool,
}

impl BufferingSpanExporter {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SpanState {
                buffer: Vec::new(),
                late: Vec::new(),
                replayed: false,
            })),
        }
    }

    /// Drain the buffer to the new exporters and switch to fan-out mode for
    /// subsequent exports. Async — must run within a tokio runtime context
    /// (the OTLP export futures are tokio-based). The caller is responsible
    /// for setting each exporter's resource before passing it in.
    pub async fn install_late(&self, exporters: Vec<opentelemetry_otlp::SpanExporter>) {
        let arc_exporters: Vec<Arc<opentelemetry_otlp::SpanExporter>> =
            exporters.into_iter().map(Arc::new).collect();

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
        for ex in arc_exporters {
            let _ = ex.export(buffer.clone()).await;
        }
    }
}

impl SpanExporter for BufferingSpanExporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
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
        let exporters: Vec<Arc<opentelemetry_otlp::SpanExporter>> = {
            let mut st = self.inner.lock().expect("span buffer mutex poisoned");
            if !st.replayed {
                let avail = BUFFER_CAP.saturating_sub(st.buffer.len());
                if batch.len() > avail {
                    let drop_n = batch.len() - avail;
                    let drain_n = drop_n.min(st.buffer.len());
                    st.buffer.drain(..drain_n);
                }
                st.buffer.extend(batch);
                return Ok(());
            }
            st.late.iter().map(Arc::clone).collect()
        };
        for ex in exporters {
            let _ = ex.export(batch.clone()).await;
        }
        Ok(())
    }

    fn shutdown_with_timeout(&mut self, timeout: Duration) -> OTelSdkResult {
        let mut st = self.inner.lock().expect("span buffer mutex poisoned");
        for ex in st.late.iter_mut() {
            if let Some(ex) = Arc::get_mut(ex) {
                let _ = ex.shutdown_with_timeout(timeout);
            }
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
