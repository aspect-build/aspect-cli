//! `bazel.build_events.read(path)`: a build event stream kept on disk, such
//! as a build's `build_events.binpb` in the task history, iterated as the
//! same `BuildEvent` values a live build's `bazel.build_events.iterator()`
//! yields, so code written for a build's events reads an earlier build's.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use allocative::Allocative;
use bazel::bes::stored::Stored;
use derive_more::Display;
use starlark::values::{
    AllocValue, Heap, NoSerialize, ProvidesStaticType, StarlarkValue, Trace, Value, starlark_value,
};

/// The events of a stream kept on disk, read as they are iterated: one pass,
/// every loop over it going on from where the last stopped.
#[derive(Clone, Display, ProvidesStaticType, Trace, NoSerialize, Allocative)]
#[display("<bazel.build.StoredBuildEvents>")]
pub struct StoredBuildEvents {
    #[allocative(skip)]
    stored: Arc<Mutex<Stored>>,
    #[allocative(skip)]
    kinds: Option<Arc<HashSet<i32>>>,
}

impl std::fmt::Debug for StoredBuildEvents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredBuildEvents").finish_non_exhaustive()
    }
}

impl StoredBuildEvents {
    pub fn open(path: &Path, kinds: Option<HashSet<i32>>) -> anyhow::Result<Self> {
        let stored = Stored::open(path).map_err(|e| {
            anyhow::anyhow!("cannot read the build events at {}: {e}", path.display())
        })?;
        Ok(Self {
            stored: Arc::new(Mutex::new(stored)),
            kinds: kinds.map(Arc::new),
        })
    }
}

impl<'v> AllocValue<'v> for StoredBuildEvents {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "bazel.build.StoredBuildEvents")]
impl<'v> StarlarkValue<'v> for StoredBuildEvents {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        let stored = self.stored.lock().unwrap();
        match attribute {
            "complete" => Some(heap.alloc(stored.complete())),
            "error" => Some(match stored.error() {
                Some(e) => heap.alloc(e),
                None => Value::new_none(),
            }),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "complete" | "error")
    }

    unsafe fn iterate(&self, me: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        Ok(me)
    }

    unsafe fn iter_next(&self, _i: usize, heap: Heap<'v>) -> Option<Value<'v>> {
        let mut stored = self.stored.lock().unwrap();
        for event in stored.by_ref() {
            let wanted = self
                .kinds
                .as_ref()
                .is_none_or(|kinds| bazel::bes::kind::event_kind_in(&event, kinds));
            if wanted {
                return Some(event.alloc_value(heap));
            }
        }
        None
    }

    unsafe fn iter_stop(&self) {}
}
