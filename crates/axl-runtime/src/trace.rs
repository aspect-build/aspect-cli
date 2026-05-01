use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use allocative::Allocative;
use starlark::any::ProvidesStaticType;
use starlark::environment::GlobalsBuilder;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Arguments;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;
use starlark::values::tuple::UnpackTuple;

/// True while telemetry routing is potentially live: `true` from process
/// start so the trace builtins emit during phases 1-3 (the buffering layer
/// captures them for replay), and either kept `true` after phase 3 if any
/// exporter was registered (OTLP or file/stderr) or flipped to `false` if
/// none were. Drives the cheap-skip fast-path for `trace.log` / `trace.event`
/// after late init.
static ACTIVE: AtomicBool = AtomicBool::new(true);

/// Mark telemetry as active. Called by `ctx.telemetry.exporters.add(...)`.
/// Idempotent.
pub fn enable() {
    ACTIVE.store(true, Ordering::Relaxed);
}

/// Mark telemetry as permanently inactive for the rest of the run. Called
/// by the runtime after phase 3 if no exporter was registered.
pub fn disable() {
    ACTIVE.store(false, Ordering::Relaxed);
}

/// True when telemetry routing is live. The host's subscriber chain
/// decides what to do with emitted events (stderr, OTel pipeline, both,
/// neither — but if neither, this returns `false`).
pub fn enabled() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Rust-side log emission. The host's tracing subscriber chain decides what
/// to do with it (stderr fmt layer when `ASPECT_DEBUG=1`, OTel logs bridge
/// when an OTLP endpoint is registered, both, or neither).
#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => {{
        if $crate::trace::enabled() {
            ::tracing::trace!(target: "axl.log", $($arg)*);
        }
    }};
}

/// `Display` adapter that renders `Some(s)` as `s` and `None` as the empty
/// string. Used to keep optional `tracing` event fields free of the literal
/// `"None"` debug output when not set by the caller.
struct OptStr<'a>(Option<&'a str>);

impl fmt::Display for OptStr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(s) => f.write_str(s),
            None => Ok(()),
        }
    }
}

/// `Display` adapter that renders a Starlark `Value` via its `repr()` form.
/// `None` (when no `fields` arg was passed) renders as the empty string.
struct ValueRepr<'v>(Option<Value<'v>>);

impl fmt::Display for ValueRepr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(v) = self.0 {
            let mut s = String::new();
            v.collect_repr(&mut s);
            f.write_str(&s)?;
        }
        Ok(())
    }
}

fn emit_event(
    name: &str,
    message: Option<&str>,
    category: Option<&str>,
    error: Option<&str>,
    fields: Option<Value<'_>>,
) {
    if !enabled() {
        return;
    }
    let descr = OptStr(message);
    let cat = OptStr(category);
    let err = OptStr(error);
    let flds = ValueRepr(fields);
    // Field-name routing into `tracing-opentelemetry`'s span-event sink:
    // - `message` is special-cased to become the span event NAME, so
    //   we map our user-facing `name` arg there to get a meaningful
    //   identifier in the OTel viewer.
    // - `error` triggers exception-event semantics if seen before any
    //   `message` field; ordering `message` first puts `error` on the
    //   regular-attribute branch instead.
    // - `description` is our human-readable message slot, surfaced as
    //   a regular attribute (no special-casing in tracing-opentelemetry).
    // Span events have no severity in OTel, so we always emit at INFO.
    tracing::info!(
        target: "axl.event",
        message = name,
        description = %descr,
        category = %cat,
        error = %err,
        fields = %flds,
    );
}

/// `trace` namespace.
///
/// Entry points:
///
/// * `trace(*args)` / `trace.log(*args)` — free-form log line. The bare
///   call is an alias for `trace.log`. With `ASPECT_DEBUG=1`, prints to
///   stderr. With OTLP active (any endpoint registered via
///   `ctx.telemetry.exporters.add(...)`), emits as a `Severity::Trace`
///   LogRecord. Keyword arguments are not accepted on the log path; route
///   structured data through `trace.event`.
/// * `trace.event(name, *, message=None, category=None, error=None, fields=None)`
///   — structured event. Top-level slots and `fields` live in separate
///   namespaces. With `ASPECT_DEBUG=1`, prints a compact line to stderr.
///   With OTLP active, attaches as an OTel span event on the active span
///   (`axl.event` target). Span events have no severity slot in the OTel
///   data model, so there's no `level=` parameter.
/// * `trace.enabled` — `True` when either sink is active.
#[derive(Debug, Clone, Copy, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Trace;

impl fmt::Display for Trace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<trace>")
    }
}

starlark_simple_value!(Trace);

#[starlark_value(type = "trace")]
impl<'v> StarlarkValue<'v> for Trace {
    fn invoke(
        &self,
        _me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        // Bare `trace(...)` is an alias for `trace.log(...)`. Reject
        // kwargs so callers don't accidentally produce log records when
        // they meant `trace.event(...)`.
        if !args.names_map()?.is_empty() {
            return Err(starlark::Error::new_kind(starlark::ErrorKind::Function(
                anyhow::anyhow!(
                    "trace(...) takes no keyword arguments; use trace.event(name, ...) for structured events"
                ),
            )));
        }
        if !enabled() {
            return Ok(Value::new_none());
        }
        let heap = eval.heap();
        let mut buf = String::new();
        for (i, v) in args.positions(heap)?.enumerate() {
            if i > 0 {
                buf.push(' ');
            }
            v.collect_str(&mut buf);
        }
        tracing::trace!(target: "axl.log", "{}", buf);
        Ok(Value::new_none())
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(trace_methods)
    }

    fn get_attr(&self, attr: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        match attr {
            "enabled" => Some(Value::new_bool(enabled())),
            _ => None,
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["enabled".to_owned(), "log".to_owned(), "event".to_owned()]
    }
}

#[starlark_module]
fn trace_methods(registry: &mut MethodsBuilder) {
    /// Emit a free-form log line. Positional args are stringified and
    /// joined by spaces. Routed through `tracing` on the `axl.log` target;
    /// the host's subscriber chain decides where it lands (stderr fmt
    /// layer when `ASPECT_DEBUG=1`, OTel logs bridge when an OTLP endpoint
    /// is registered).
    fn log<'v>(
        this: Value<'v>,
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
    ) -> anyhow::Result<starlark::values::none::NoneType> {
        let _ = this;
        if !enabled() {
            return Ok(starlark::values::none::NoneType);
        }
        let mut buf = String::new();
        for (i, v) in args.items.iter().enumerate() {
            if i > 0 {
                buf.push(' ');
            }
            v.collect_str(&mut buf);
        }
        tracing::trace!(target: "axl.log", "{}", buf);
        Ok(starlark::values::none::NoneType)
    }

    /// Emit a structured event. `name` identifies what happened (machine-
    /// readable). `message` is an optional human-readable string. `level`
    /// selects severity (`trace` / `debug` / `info` / `warn` / `error`).
    /// `category` records who emitted it (extension/plugin/hook). `error`
    /// is a typed slot for failure events. `fields` is an open dict for
    /// everything else; values are recorded via their `repr()` form.
    ///
    /// Top-level slots and `fields` are separate namespaces — there is no
    /// merging. A consumer can always tell where a key came from:
    /// `fields.x` is user data, `category` is structural.
    fn event<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] name: &str,
        #[starlark(require = named, default = NoneOr::None)] message: NoneOr<&'v str>,
        #[starlark(require = named, default = NoneOr::None)] category: NoneOr<&'v str>,
        #[starlark(require = named, default = NoneOr::None)] error: NoneOr<&'v str>,
        #[starlark(require = named, default = NoneOr::None)] fields: NoneOr<Value<'v>>,
    ) -> anyhow::Result<starlark::values::none::NoneType> {
        let _ = this;
        emit_event(
            name,
            message.into_option(),
            category.into_option(),
            error.into_option(),
            fields.into_option(),
        );
        Ok(starlark::values::none::NoneType)
    }
}

#[starlark_module]
pub fn register_globals(globals: &mut GlobalsBuilder) {
    const trace: Trace = Trace;
}

/// Process-wide lock serializing all trace-touching tests. The
/// `tracing` crate's per-callsite interest cache is global and not
/// per-thread; concurrent tests that fire trace.event/trace.log under
/// different subscribers can poison each other's interest cache and
/// produce flaky failures (see `telemetry_tests::capture` for the
/// rebuild-cache machinery on the recorder side).
#[cfg(test)]
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::TEST_LOCK;
    use crate::eval::api::eval_expr;

    fn lock_test() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn trace_log_returns_none() {
        let _g = lock_test();
        let result = eval_expr(r#"trace.log("hello", 1)"#).unwrap();
        assert_eq!(result, "None");
    }

    #[test]
    fn trace_event_minimal_returns_none() {
        let _g = lock_test();
        let result = eval_expr(r#"trace.event("evt.name")"#).unwrap();
        assert_eq!(result, "None");
    }

    #[test]
    fn trace_event_full_returns_none() {
        let _g = lock_test();
        let result = eval_expr(
            r#"trace.event(
                "evt.name",
                message = "human readable",
                category = "axl.test",
                error = None,
                fields = {"k": 1, "v": [1, 2]},
            )"#,
        )
        .unwrap();
        assert_eq!(result, "None");
    }

    #[test]
    fn trace_event_requires_string_name() {
        let _g = lock_test();
        let err = eval_expr(r#"trace.event(42)"#);
        assert!(err.is_err());
    }

    #[test]
    fn trace_event_rejects_unknown_kwarg() {
        // `level=` was removed when `trace.event` pivoted to span events.
        let _g = lock_test();
        let err = eval_expr(r#"trace.event("foo", level = "warn")"#);
        assert!(err.is_err());
    }

    #[test]
    fn trace_event_does_not_accept_bare_kwargs() {
        // Old API: `trace.event("name", k = v)` no longer works — kwargs
        // must go through the `fields = {...}` slot under the new shape.
        let _g = lock_test();
        let err = eval_expr(r#"trace.event("name", random_key = 1)"#);
        assert!(err.is_err());
    }

    #[test]
    fn trace_bare_call_aliases_log() {
        let _g = lock_test();
        let result = eval_expr(r#"trace("hello", 1)"#).unwrap();
        assert_eq!(result, "None");
    }

    #[test]
    fn trace_bare_call_rejects_kwargs() {
        let _g = lock_test();
        let err = eval_expr(r#"trace("name", k = 1)"#);
        assert!(err.is_err());
    }
}

/// Telemetry-emission tests: install a minimal recording `tracing::Subscriber`
/// for the duration of each test and assert on what `trace.event` / `trace.log`
/// actually push into the tracing pipeline (target, level, fields). This
/// exercises the same path the OTel logs bridge consumes downstream.
#[cfg(test)]
mod telemetry_tests {
    use std::collections::BTreeMap;
    use std::fmt;
    use std::sync::{Arc, Mutex};

    use tracing::dispatcher::Dispatch;
    use tracing::field::{Field, Visit};
    use tracing::span;
    use tracing::{Event, Metadata, Subscriber};

    use crate::eval::api::eval_expr;

    #[derive(Debug, Clone)]
    struct Recorded {
        target: String,
        level: String,
        fields: BTreeMap<String, String>,
    }

    /// Captures every `Event` into a shared `Vec` so the test can assert
    /// after the eval. Spans are no-ops; we don't emit them from `trace.*`.
    struct Recorder {
        events: Arc<Mutex<Vec<Recorded>>>,
    }

    impl Subscriber for Recorder {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }
        fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
            // Default `unwrap_or(LevelFilter::OFF)` would cause the
            // `level_enabled!` short-circuit in tracing's emit macros to
            // drop all our events before they reach this subscriber.
            Some(tracing::level_filters::LevelFilter::TRACE)
        }
        fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
            span::Id::from_u64(1)
        }
        fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
        fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
        fn event(&self, event: &Event<'_>) {
            let mut visitor = Collector::default();
            event.record(&mut visitor);
            self.events.lock().unwrap().push(Recorded {
                target: event.metadata().target().to_owned(),
                level: event.metadata().level().to_string(),
                fields: visitor.fields,
            });
        }
        fn enter(&self, _: &span::Id) {}
        fn exit(&self, _: &span::Id) {}
    }

    #[derive(Default)]
    struct Collector {
        fields: BTreeMap<String, String>,
    }

    impl Visit for Collector {
        fn record_str(&mut self, f: &Field, v: &str) {
            self.fields.insert(f.name().to_owned(), v.to_owned());
        }
        fn record_debug(&mut self, f: &Field, v: &dyn fmt::Debug) {
            self.fields.insert(f.name().to_owned(), format!("{:?}", v));
        }
        fn record_i64(&mut self, f: &Field, v: i64) {
            self.fields.insert(f.name().to_owned(), v.to_string());
        }
        fn record_u64(&mut self, f: &Field, v: u64) {
            self.fields.insert(f.name().to_owned(), v.to_string());
        }
        fn record_f64(&mut self, f: &Field, v: f64) {
            self.fields.insert(f.name().to_owned(), v.to_string());
        }
        fn record_bool(&mut self, f: &Field, v: bool) {
            self.fields.insert(f.name().to_owned(), v.to_string());
        }
    }

    use super::TEST_LOCK;

    /// Run `expr` with a recording subscriber installed. Returns whatever
    /// `tracing` events fired during the eval.
    ///
    /// Rebuilds the callsite-interest cache before running — without that,
    /// any callsite hit by a previous test under the no-op default
    /// subscriber stays cached as `Interest::never()` for the rest of the
    /// process and our recorder never gets asked.
    fn capture(expr: &str) -> Vec<Recorded> {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        tracing::callsite::rebuild_interest_cache();
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorder = Recorder {
            events: events.clone(),
        };
        let dispatch = Dispatch::new(recorder);
        tracing::dispatcher::with_default(&dispatch, || {
            eval_expr(expr).unwrap();
        });
        let captured = events.lock().unwrap().clone();
        captured
    }

    #[test]
    fn event_minimal_lands_on_axl_event_at_info() {
        let evs = capture(r#"trace.event("evt.name")"#);
        assert_eq!(evs.len(), 1);
        let e = &evs[0];
        assert_eq!(e.target, "axl.event");
        // Span events always emit at INFO at the tracing layer; OTel's
        // span.add_event has no severity slot, so the level is fixed.
        assert_eq!(e.level, "INFO");
        // Our `name` arg is mapped to tracing's `message` field so that
        // `tracing-opentelemetry`'s span-event visitor uses it as the
        // span event NAME in OTel.
        assert_eq!(
            e.fields.get("message").map(|s| s.as_str()),
            Some("evt.name")
        );
        // Optional slots default to empty when not provided.
        assert_eq!(e.fields.get("description").map(|s| s.as_str()), Some(""));
        assert_eq!(e.fields.get("category").map(|s| s.as_str()), Some(""));
        assert_eq!(e.fields.get("error").map(|s| s.as_str()), Some(""));
        assert_eq!(e.fields.get("fields").map(|s| s.as_str()), Some(""));
    }

    #[test]
    fn event_carries_all_top_level_slots() {
        let evs = capture(
            r#"trace.event(
                "build.cancel",
                message = "user requested cancel",
                category = "bazel.runner",
                error = "ctrl-c",
                fields = {"cancelling": True, "n": 3},
            )"#,
        );
        assert_eq!(evs.len(), 1);
        let e = &evs[0];
        assert_eq!(e.target, "axl.event");
        assert_eq!(e.level, "INFO");
        // `name` lands on tracing's `message` field (becomes OTel span
        // event name); the user-facing `message` slot lands on
        // `description` (becomes a regular span-event attribute).
        assert_eq!(e.fields.get("message").unwrap(), "build.cancel");
        assert_eq!(
            e.fields.get("description").unwrap(),
            "user requested cancel"
        );
        assert_eq!(e.fields.get("category").unwrap(), "bazel.runner");
        assert_eq!(e.fields.get("error").unwrap(), "ctrl-c");
        // `fields` is a Display-formatted Starlark `repr()` of the dict.
        // Starlark dict repr uses double quotes and Python-style booleans.
        let f = e.fields.get("fields").unwrap();
        assert!(f.contains("\"cancelling\": True"), "got: {}", f);
        assert!(f.contains("\"n\": 3"), "got: {}", f);
    }

    #[test]
    fn log_lands_on_axl_log_at_trace_level() {
        let evs = capture(r#"trace.log("hello", "world", 42)"#);
        assert_eq!(evs.len(), 1);
        let e = &evs[0];
        assert_eq!(e.target, "axl.log");
        assert_eq!(e.level, "TRACE");
        // The log path uses the `tracing::trace!("{}", buf)` shape; the
        // formatted body lands on the `message` field.
        assert_eq!(
            e.fields.get("message").map(|s| s.as_str()),
            Some("hello world 42")
        );
    }

    #[test]
    fn bare_trace_call_emits_to_axl_log() {
        let evs = capture(r#"trace("alias", "for", "log")"#);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].target, "axl.log");
        assert_eq!(evs[0].level, "TRACE");
        assert_eq!(
            evs[0].fields.get("message").map(|s| s.as_str()),
            Some("alias for log")
        );
    }

    #[test]
    fn sanity_recorder_captures_direct_tracing_emit() {
        // Sanity: install the recorder and emit via `tracing::info!`
        // directly. If this fails, the issue is the subscriber wiring,
        // not eval_expr.
        tracing::callsite::rebuild_interest_cache();
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorder = Recorder {
            events: events.clone(),
        };
        let dispatch = Dispatch::new(recorder);
        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!(target: "test.direct", name = "hi");
        });
        let captured = events.lock().unwrap().clone();
        assert_eq!(captured.len(), 1, "got: {:?}", captured);
        assert_eq!(captured[0].target, "test.direct");
    }

    #[test]
    fn sanity_recorder_captures_emit_through_eval_runtime() {
        // Same sanity as above but the closure first creates a Tokio
        // runtime (matching `eval_expr`) before emitting. Isolates whether
        // the tokio runtime creation displaces our dispatcher.
        tracing::callsite::rebuild_interest_cache();
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorder = Recorder {
            events: events.clone(),
        };
        let dispatch = Dispatch::new(recorder);
        tracing::dispatcher::with_default(&dispatch, || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let _g = rt.enter();
            tracing::info!(target: "test.direct", name = "hi");
        });
        let captured = events.lock().unwrap().clone();
        assert_eq!(captured.len(), 1, "got: {:?}", captured);
    }

    #[test]
    fn no_event_when_otel_inactive() {
        // Take the lock manually so the disable/restore window can't race
        // with other capture-based tests touching the same global flag.
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        super::disable();
        // Inline the capture body to avoid re-entering TEST_LOCK.
        tracing::callsite::rebuild_interest_cache();
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorder = Recorder {
            events: events.clone(),
        };
        let dispatch = Dispatch::new(recorder);
        tracing::dispatcher::with_default(&dispatch, || {
            eval_expr(r#"trace.event("should.not.emit")"#).unwrap();
        });
        let captured = events.lock().unwrap().clone();
        super::enable();
        assert!(
            captured.is_empty(),
            "expected no events, got {:?}",
            captured
        );
    }
}
