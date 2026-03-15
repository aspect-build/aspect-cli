use allocative::Allocative;
use derive_more::Display;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::values;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

use super::info;
use super::process;

#[derive(Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.build.Cancellation>")]
pub struct Cancellation {
    #[allocative(skip)]
    server_pid: u32,
    #[allocative(skip)]
    child_pid: Option<u32>,
    #[allocative(skip)]
    output_base: Option<String>,
}

impl Cancellation {
    pub fn new(server_pid: u32, child_pid: Option<u32>, output_base: Option<String>) -> Self {
        Self {
            server_pid,
            child_pid,
            output_base,
        }
    }
}

impl<'v> AllocValue<'v> for Cancellation {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "bazel.build.Cancellation")]
impl<'v> values::StarlarkValue<'v> for Cancellation {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(cancellation_methods)
    }
}

#[starlark_module]
pub(crate) fn cancellation_methods(registry: &mut MethodsBuilder) {
    /// Whether the bazel server is currently busy (lock held by another client).
    /// Queries in real time via `bazel --noblock_for_lock info`.
    #[starlark(attribute)]
    fn busy<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let cancellation = this.downcast_ref::<Cancellation>().unwrap();
        Ok(info::is_server_busy(cancellation.output_base.as_deref()))
    }

    /// Block until the cancelled invocation finishes.
    ///
    /// For per-build cancellations, returns immediately — call `build.wait()`
    /// to reap the child process and join stream threads.
    ///
    /// For server-wide cancellations, polls until the server is no longer busy.
    fn wait<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = 200)] poll_ms: i32,
    ) -> anyhow::Result<bool> {
        let cancellation = this.downcast_ref::<Cancellation>().unwrap();
        // Per-build: build.wait() handles child reap + stream joins atomically.
        // We can't reap the child here without leaving dangling stream threads.
        if cancellation.child_pid.is_some() {
            return Ok(true);
        }
        // Server-wide: poll until the server is no longer busy.
        while info::is_server_busy(cancellation.output_base.as_deref()) {
            std::thread::sleep(std::time::Duration::from_millis(poll_ms as u64));
        }
        Ok(true)
    }

    /// Force-kill the invocation: SIGTERM to the client child process (if any)
    /// and SIGKILL to the server daemon.
    fn force<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let cancellation = this.downcast_ref::<Cancellation>().unwrap();
        if let Some(child_pid) = cancellation.child_pid {
            process::sigterm(child_pid);
        }
        process::sigkill(cancellation.server_pid);
        Ok(true)
    }
}
