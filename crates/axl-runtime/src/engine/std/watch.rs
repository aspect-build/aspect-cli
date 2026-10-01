use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Duration;

use allocative::Allocative;
use derive_more::Display;

use starlark::StarlarkResultExt;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::ValueLike;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;

use fswatch::WatchConfig;

pub(crate) fn duration_ms(value: i32, what: &str) -> anyhow::Result<u64> {
    value
        .try_into()
        .map_err(|_| anyhow::anyhow!("fs.watch {what} must not be negative: {value}"))
}

/// A live recursive file watch. Poll with `try_pop()`; drop pending noise
/// (e.g. changes caused by our own build) with `drain()`.
#[derive(Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<fs.Watch>")]
pub struct Watch {
    #[allocative(skip)]
    inner: RefCell<Box<dyn fswatch::Subscription>>,
    /// The watched root; event paths are reported relative to it.
    #[allocative(skip)]
    root: std::path::PathBuf,
}

impl Watch {
    /// Start watching `root`. `root` and every `ignore` prefix must be
    /// absolute: the backends match events (which carry absolute paths)
    /// against them, so a relative one would never match — a silent no-op for
    /// an ignore prefix, a broken watch for the root.
    pub(crate) fn new(
        root: String,
        ignore: Vec<String>,
        debounce_ms: u64,
        max_delay_ms: u64,
    ) -> anyhow::Result<Self> {
        let root = PathBuf::from(root);
        if !root.is_absolute() {
            return Err(anyhow::anyhow!(
                "fs.watch root must be absolute: {}",
                root.display()
            ));
        }
        let mut ignore_prefixes = Vec::with_capacity(ignore.len());
        for prefix in ignore {
            let prefix = PathBuf::from(prefix);
            if !prefix.is_absolute() {
                return Err(anyhow::anyhow!(
                    "fs.watch ignore prefix must be absolute: {}",
                    prefix.display()
                ));
            }
            ignore_prefixes.push(prefix);
        }
        let mut config = WatchConfig::new(root.clone());
        config.ignore_prefixes = ignore_prefixes;
        config.debounce = Duration::from_millis(debounce_ms);
        config.max_delay = Duration::from_millis(max_delay_ms);
        Ok(Watch {
            inner: RefCell::new(fswatch::watch(fswatch::Backend::default(), config)?),
            root,
        })
    }
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<fs.Watch>")
    }
}

impl<'v> values::AllocValue<'v> for Watch {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "fs.Watch")]
impl<'v> values::StarlarkValue<'v> for Watch {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("watch_methods", watch_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
pub(crate) fn watch_methods(registry: &mut MethodsBuilder) {
    /// Non-blocking poll. Returns a batch once pending changes have settled
    /// for the debounce window (or the max delay elapsed), otherwise None.
    ///
    /// A batch has `events` (a list) and `rescan` (a bool). Each event is one
    /// of three types — `fs.watch.CreatedEvent`, `fs.watch.ModifiedEvent`,
    /// `fs.watch.RemovedEvent` — carrying `path` (relative to the watched
    /// root). Discriminate them with the global `std.fs.watch` type constants
    /// and `isinstance`. The three kinds are the shared vocabulary both
    /// backends map their native events into.
    ///
    /// `rescan` True means the watcher dropped events and lost track (the OS
    /// event queue overflowed, or watchman restarted / handed us a fresh
    /// instance): `events` is then not a complete list, so the caller must
    /// assume anything under the root may have changed and re-scan from
    /// scratch rather than trusting the batch.
    fn try_pop<'v>(
        this: values::Value<'v>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let watch = this.downcast_ref_err::<Watch>().into_anyhow_result()?;
        let batch = watch.inner.borrow_mut().try_recv()?;
        Ok(match batch {
            None => values::Value::new_none(),
            Some(batch) => {
                let events = heap.alloc(values::list::AllocList(batch.changes.into_iter().map(
                    |change| {
                        // Report paths relative to the watched root; the
                        // backends emit absolute paths under it.
                        let path = change
                            .path
                            .strip_prefix(&watch.root)
                            .unwrap_or(&change.path)
                            .to_string_lossy()
                            .into_owned();
                        match change.kind {
                            fswatch::ChangeKind::Created => heap.alloc(WatchCreated { path }),
                            fswatch::ChangeKind::Modified => heap.alloc(WatchModified { path }),
                            fswatch::ChangeKind::Removed => heap.alloc(WatchRemoved { path }),
                        }
                    },
                )));
                heap.alloc(WatchBatch {
                    events,
                    rescan: values::Value::new_bool(batch.rescan),
                })
            }
        })
    }

    /// Discard every pending change accumulated so far.
    fn drain<'v>(this: values::Value<'v>) -> anyhow::Result<NoneType> {
        let watch = this.downcast_ref_err::<Watch>().into_anyhow_result()?;
        watch.inner.borrow_mut().drain();
        Ok(NoneType)
    }
}

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<fs.WatchBatch rescan:{rescan}>")]
pub struct WatchBatch<'v> {
    events: values::Value<'v>,
    rescan: values::Value<'v>,
}

#[starlark_value(type = "fs.WatchBatch")]
impl<'v> values::StarlarkValue<'v> for WatchBatch<'v> {
    fn get_attr(&self, attr: &str, _: Heap<'v>) -> Option<values::Value<'v>> {
        match attr {
            "events" => Some(self.events),
            "rescan" => Some(self.rescan),
            _ => None,
        }
    }
    fn dir_attr(&self) -> Vec<String> {
        vec!["events".into(), "rescan".into()]
    }
}

impl<'v> values::AllocValue<'v> for WatchBatch<'v> {
    fn alloc_value(self, heap: values::Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

// One watch-event type per change kind, so the kind is the value's type rather
// than a stringly-typed field. Registered as global type constants under
// `std.fs.watch` (see `std::register_globals`). Written out rather than
// macro-generated because the kinds are expected to diverge with
// kind-specific fields.

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<fs.watch.CreatedEvent path:{path}>")]
pub(crate) struct WatchCreated {
    path: String,
}

#[starlark_value(type = "fs.watch.CreatedEvent")]
impl<'v> values::StarlarkValue<'v> for WatchCreated {
    fn get_attr(&self, attr: &str, heap: Heap<'v>) -> Option<values::Value<'v>> {
        (attr == "path").then(|| heap.alloc_str(&self.path).to_value())
    }
    fn dir_attr(&self) -> Vec<String> {
        vec!["path".into()]
    }
}

starlark_simple_value!(WatchCreated);

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<fs.watch.ModifiedEvent path:{path}>")]
pub(crate) struct WatchModified {
    path: String,
}

#[starlark_value(type = "fs.watch.ModifiedEvent")]
impl<'v> values::StarlarkValue<'v> for WatchModified {
    fn get_attr(&self, attr: &str, heap: Heap<'v>) -> Option<values::Value<'v>> {
        (attr == "path").then(|| heap.alloc_str(&self.path).to_value())
    }
    fn dir_attr(&self) -> Vec<String> {
        vec!["path".into()]
    }
}

starlark_simple_value!(WatchModified);

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<fs.watch.RemovedEvent path:{path}>")]
pub(crate) struct WatchRemoved {
    path: String,
}

#[starlark_value(type = "fs.watch.RemovedEvent")]
impl<'v> values::StarlarkValue<'v> for WatchRemoved {
    fn get_attr(&self, attr: &str, heap: Heap<'v>) -> Option<values::Value<'v>> {
        (attr == "path").then(|| heap.alloc_str(&self.path).to_value())
    }
    fn dir_attr(&self) -> Vec<String> {
        vec!["path".into()]
    }
}

starlark_simple_value!(WatchRemoved);

#[cfg(test)]
mod tests {
    #[test]
    fn watch_from_starlark_task() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("skip")).unwrap();
        let root_s = root.to_string_lossy();
        let exit = crate::test::eval(&format!(
            r#"
load("@std//time.axl", "sleep_iter")

def _impl(ctx):
    w = ctx.std.fs.watch("{root}", ignore = ["{root}/skip"], debounce_ms = 50)
    for i in sleep_iter(20):
        if i > 600:
            fail("no batch within deadline")
        # Repeat the writes each tick: events emitted before the OS watch
        # stream is fully up may be dropped, and a batch drained with them.
        ctx.std.fs.write("{root}/skip/noise.txt", str(i))
        ctx.std.fs.write("{root}/a.txt", str(i))
        b = w.try_pop()
        if b == None:
            continue
        if type(b.rescan) != type(True):
            fail("rescan must be a bool")
        # Paths are relative to the watched root.
        paths = [e.path for e in b.events]
        if "a.txt" not in paths:
            fail("expected a.txt in %s" % paths)
        for e in b.events:
            if not (
                isinstance(e, std.fs.watch.CreatedEvent) or
                isinstance(e, std.fs.watch.ModifiedEvent) or
                isinstance(e, std.fs.watch.RemovedEvent)
            ):
                fail("bad event type %s" % type(e))
            if e.path.startswith("/"):
                fail("path not relative: %s" % e.path)
            if e.path.startswith("skip"):
                fail("ignored prefix leaked: %s" % e.path)
        break
    w.drain()
    return 0

Test = task(implementation = _impl)
"#,
            root = root_s,
        ))
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    #[test]
    fn watch_rejects_invalid_arguments() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let calls = [
            "\"relative/dir\"".to_string(),
            format!("\"{}\", ignore = [\"rel/skip\"]", root.display()),
            format!("\"{}\", debounce_ms = -1", root.display()),
            format!("\"{}\", max_delay_ms = -1", root.display()),
        ];
        for call in calls {
            let result = crate::test::eval(&format!(
                r#"
def _impl(ctx):
    ctx.std.fs.watch({call})
    return 0

Test = task(implementation = _impl)
"#,
            ))
            .run_task(0);
            assert!(result.is_err(), "invalid argument must error: {call}");
        }
    }
}
