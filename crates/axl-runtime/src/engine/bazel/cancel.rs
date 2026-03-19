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
    startup_flags: Vec<String>,
}

impl Cancellation {
    pub fn new(server_pid: u32, startup_flags: Vec<String>) -> Self {
        Self {
            server_pid,
            startup_flags,
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
        Ok(info::is_server_busy(&cancellation.startup_flags))
    }

    /// Block until the cancelled invocation finishes.
    ///
    /// Polls until the server is no longer busy.
    fn wait<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = 200)] poll_ms: i32,
    ) -> anyhow::Result<bool> {
        let cancellation = this.downcast_ref::<Cancellation>().unwrap();
        let poll_ms = poll_ms.max(0) as u64;
        while info::is_server_busy(&cancellation.startup_flags) {
            std::thread::sleep(std::time::Duration::from_millis(poll_ms));
        }
        Ok(true)
    }

    /// Force-kill the invocation by sending SIGKILL to the server daemon.
    fn force<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let cancellation = this.downcast_ref::<Cancellation>().unwrap();
        process::sigkill(cancellation.server_pid);
        Ok(true)
    }
}
