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
use starlark::values;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::StringValue;
use starlark::values::Trace;
use starlark::values::ValueLike;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;

use fswatch::WatchConfig;

pub(crate) fn start(
    roots: Vec<String>,
    ignore: Vec<String>,
    ignore_root_symlinks: bool,
    debounce_ms: u64,
    max_delay_ms: u64,
) -> anyhow::Result<Watch> {
    if roots.is_empty() {
        return Err(anyhow::anyhow!("fs.watch requires at least one root"));
    }
    let mut config = WatchConfig::new(roots.into_iter().map(PathBuf::from).collect());
    config.ignore_prefixes = ignore.into_iter().map(PathBuf::from).collect();
    config.ignore_root_symlinks = ignore_root_symlinks;
    config.debounce = Duration::from_millis(debounce_ms);
    config.max_delay = Duration::from_millis(max_delay_ms);
    let subscription = fswatch::watch(fswatch::Backend::default(), config)?;
    Ok(Watch {
        inner: RefCell::new(subscription),
    })
}

/// A live recursive file watch. Poll with `try_pop()`; drop pending noise
/// (e.g. changes caused by our own build) with `drain()`.
#[derive(Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<fs.Watch>")]
pub struct Watch {
    #[allocative(skip)]
    inner: RefCell<Box<dyn fswatch::Subscription>>,
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
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(watch_methods)
    }
}

#[starlark_module]
pub(crate) fn watch_methods(registry: &mut MethodsBuilder) {
    /// Non-blocking poll. Returns a batch once pending changes have settled
    /// for the debounce window (or the max delay elapsed), otherwise None.
    ///
    /// A batch has `changes` (list of `struct(path, kind)` where kind is
    /// `"created"`, `"modified"` or `"removed"`) and `rescan` (True when the
    /// watcher lost track of state and anything may have changed).
    fn try_pop<'v>(
        this: values::Value<'v>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let watch = this.downcast_ref_err::<Watch>().into_anyhow_result()?;
        let batch = watch.inner.borrow_mut().try_recv()?;
        Ok(match batch {
            None => values::Value::new_none(),
            Some(batch) => {
                let changes = heap.alloc(values::list::AllocList(batch.changes.into_iter().map(
                    |change| {
                        heap.alloc(WatchChange {
                            path: heap.alloc_str(&change.path.to_string_lossy()),
                            kind: heap.alloc_str(match change.kind {
                                fswatch::ChangeKind::Created => "created",
                                fswatch::ChangeKind::Modified => "modified",
                                fswatch::ChangeKind::Removed => "removed",
                            }),
                        })
                    },
                )));
                heap.alloc(WatchBatch {
                    changes,
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
    changes: values::Value<'v>,
    rescan: values::Value<'v>,
}

#[starlark_value(type = "fs.WatchBatch")]
impl<'v> values::StarlarkValue<'v> for WatchBatch<'v> {
    fn get_attr(&self, attr: &str, _: Heap<'v>) -> Option<values::Value<'v>> {
        match attr {
            "changes" => Some(self.changes),
            "rescan" => Some(self.rescan),
            _ => None,
        }
    }
    fn dir_attr(&self) -> Vec<String> {
        vec!["changes".into(), "rescan".into()]
    }
}

impl<'v> values::AllocValue<'v> for WatchBatch<'v> {
    fn alloc_value(self, heap: values::Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<fs.WatchChange path:{path} kind:{kind}>")]
pub struct WatchChange<'v> {
    path: StringValue<'v>,
    kind: StringValue<'v>,
}

#[starlark_value(type = "fs.WatchChange")]
impl<'v> values::StarlarkValue<'v> for WatchChange<'v> {
    fn get_attr(&self, attr: &str, _: Heap<'v>) -> Option<values::Value<'v>> {
        match attr {
            "path" => Some(self.path.to_value()),
            "kind" => Some(self.kind.to_value()),
            _ => None,
        }
    }
    fn dir_attr(&self) -> Vec<String> {
        vec!["path".into(), "kind".into()]
    }
}

impl<'v> values::AllocValue<'v> for WatchChange<'v> {
    fn alloc_value(self, heap: values::Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

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
    w = ctx.std.fs.watch(["{root}"], ignore = ["{root}/skip"], debounce_ms = 50)
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
        paths = [c.path for c in b.changes]
        if "{root}/a.txt" not in paths:
            fail("expected a.txt in %s" % paths)
        for c in b.changes:
            if c.kind not in ["created", "modified", "removed"]:
                fail("bad kind %s" % c.kind)
            if c.path.startswith("{root}/skip"):
                fail("ignored prefix leaked: %s" % c.path)
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
    fn watch_requires_roots() {
        let result = crate::test::eval(
            r#"
def _impl(ctx):
    ctx.std.fs.watch([])
    return 0

Test = task(implementation = _impl)
"#,
        )
        .run_task(0);
        assert!(result.is_err(), "empty roots must error");
    }
}
