use allocative::Allocative;
use derive_more::Display;
use starlark::values;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::starlark_value;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinSet;

use super::future::StarlarkFuture;
use super::rt::AsyncRuntime;

#[derive(ProvidesStaticType, Display, NoSerialize, Allocative)]
#[display("<future.iter>")]
pub struct FutureStream {
    #[allocative(skip)]
    pub(super) rt: AsyncRuntime,
    #[allocative(skip)]
    pub(super) stream: Arc<RwLock<JoinSet<Box<dyn super::future::FutureAlloc>>>>,
}

impl std::fmt::Debug for FutureStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FutureStream")
            .field("rt", &self.rt)
            .field("stream", &self.stream)
            .finish()
    }
}

impl FutureStream {
    pub fn new(rt: AsyncRuntime, futures: Vec<StarlarkFuture>) -> Self {
        let mut set = JoinSet::new();
        let guard = rt.enter();
        for future in futures {
            set.spawn(future.as_fut());
        }
        drop(guard);
        Self {
            rt,
            stream: Arc::new(RwLock::new(set)),
        }
    }
}

unsafe impl<'v> Trace<'v> for FutureStream {
    fn trace(&mut self, _tracer: &values::Tracer<'v>) {}
}

impl Clone for FutureStream {
    fn clone(&self) -> Self {
        todo!()
    }
}

impl<'v> AllocValue<'v> for FutureStream {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "future_iter")]
impl<'v> values::StarlarkValue<'v> for FutureStream {
    unsafe fn iterate(
        &self,
        me: values::Value<'v>,
        _heap: &'v Heap,
    ) -> starlark::Result<values::Value<'v>> {
        Ok(me)
    }
    unsafe fn iter_next(&self, _index: usize, heap: &'v Heap) -> Option<values::Value<'v>> {
        let stream: Arc<RwLock<JoinSet<_>>> = Arc::clone(&self.stream);
        let out = self.rt.block_on(async {
            tokio::task::spawn(async move {
                let mut stream = stream.write().await;
                stream.join_next().await
            })
            .await
            .unwrap()
        });
        let value = out?.ok()?;
        Some(value.alloc_value_fut(heap))
    }
    unsafe fn iter_stop(&self) {
        // TODO: destroy the joinset, ensure nothing is left behind.
    }
}
