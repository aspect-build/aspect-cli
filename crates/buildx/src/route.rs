//! Routing from Starlark: `ctx.execution.route(fn)` and the `Spawn` record the
//! function receives.
//!
//! Bazel calls the strategy on its own threads, so the function runs with one
//! evaluator per call, and must be frozen (a top-level function of the script):
//! a closure over mutable state could not be shared between those threads.

use crate::execution::{Executor, Router, SpawnView};
use crate::reapi;
use allocative::Allocative;
use anyhow::{Result, anyhow, bail};
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic, Module};
use starlark::eval::Evaluator;
use starlark::values::dict::AllocDict;
use starlark::values::list::AllocList;
use starlark::values::{
    FrozenHeapRef, FrozenValue, Heap, NoSerialize, ProvidesStaticType, StarlarkValue, Value,
    ValueLike, starlark_value,
};
use starlark::{starlark_module, starlark_simple_value};
use std::fmt;
use std::sync::Arc;

/// A frozen function and the heap that keeps it alive.
pub struct RouteFn {
    value: FrozenValue,
    _heap: FrozenHeapRef,
}

impl RouteFn {
    /// `f` must be frozen: a function defined at the top level of the script.
    pub fn new(f: Value, heap: &FrozenHeapRef) -> Result<RouteFn> {
        let Some(value) = f.unpack_frozen() else {
            bail!(
                "a routing function must be frozen: pass a function defined at the top level of the script, not a closure or lambda created while the script runs"
            );
        };
        Ok(RouteFn {
            value,
            _heap: heap.clone(),
        })
    }
}

pub struct StarlarkRouter {
    pub function: RouteFn,
    /// The executors the function may pick, by name: `spawn.executors`.
    pub executors: Vec<(String, Arc<dyn Executor>)>,
}

impl Router for StarlarkRouter {
    fn sandboxing(&self) -> crate::sandbox::Sandboxing {
        self.executors
            .iter()
            .map(|(_, e)| e.sandboxing())
            .max()
            .unwrap_or(crate::sandbox::Sandboxing::None)
    }

    /// Every executor in the `executors` dict is reachable, because the function
    /// may return any of them, so all of them are described, not only those a
    /// spawn ends up on.
    fn describe(&self, used: &mut crate::events::Used) {
        for (_, executor) in &self.executors {
            executor.describe(used);
        }
    }

    fn route(&self, spawn: &Arc<SpawnView>) -> Result<Arc<dyn Executor>> {
        Module::with_temp_heap(|module| {
            let mut eval = Evaluator::new(&module);
            let record = module.heap().alloc(SpawnValue {
                view: spawn.clone(),
                executors: self.executors.clone(),
            });
            let out = eval
                .eval_function(self.function.value.to_value(), &[record], &[])
                .map_err(|e| anyhow!("routing function failed: {e}"))?;
            let Some(executor) = out.downcast_ref::<crate::values::ExecutorValue>() else {
                bail!(
                    "the routing function returned a {}; return one of the executors given to route",
                    out.get_type()
                );
            };
            Ok(executor.0.clone())
        })
    }
}

/// The `Spawn` a routing function receives.
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct SpawnValue {
    #[allocative(skip)]
    view: Arc<SpawnView>,
    #[allocative(skip)]
    executors: Vec<(String, Arc<dyn Executor>)>,
}

impl fmt::Debug for SpawnValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Spawn({})", self.view.data.mnemonic)
    }
}

impl fmt::Display for SpawnValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Spawn({})", self.view.data.mnemonic)
    }
}

starlark_simple_value!(SpawnValue);

#[starlark_value(type = "Spawn")]
impl<'v> StarlarkValue<'v> for SpawnValue {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("spawn_methods", spawn_methods);
        Some(RES.methods())
    }
}

fn pairs<'v>(heap: Heap<'v>, p: &[(String, String)]) -> Value<'v> {
    heap.alloc(AllocDict(p.iter().map(|(k, v)| (k.as_str(), v.as_str()))))
}

#[starlark_module]
fn spawn_methods(registry: &mut MethodsBuilder) {
    /// The executors given to `route`, by name, to return from the function.
    #[starlark(attribute)]
    fn executors<'v>(this: &SpawnValue, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        Ok(heap.alloc(AllocDict(
            this.executors
                .iter()
                .map(|(k, v)| (k.as_str(), crate::values::ExecutorValue(v.clone()))),
        )))
    }

    /// The action's mnemonic, such as `Genrule`, `CppCompile` or `Javac`.
    #[starlark(attribute)]
    fn mnemonic(this: &SpawnValue) -> anyhow::Result<String> {
        Ok(this.view.data.mnemonic.clone())
    }

    /// The execution requirements tags (`no-sandbox`, `local`, ...) as a dict.
    #[starlark(attribute)]
    fn execution_info<'v>(this: &SpawnValue, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        Ok(pairs(heap, &this.view.data.execution_info))
    }

    /// The execution platform's and the action's exec properties, merged.
    #[starlark(attribute)]
    fn platform_properties<'v>(this: &SpawnValue, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        Ok(pairs(heap, &this.view.data.platform_properties))
    }

    /// The command line.
    #[starlark(attribute)]
    fn arguments<'v>(this: &SpawnValue, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        Ok(heap.alloc(AllocList(
            this.view.data.arguments.iter().map(String::as_str),
        )))
    }

    /// The environment variables.
    #[starlark(attribute)]
    fn environment<'v>(this: &SpawnValue, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        Ok(pairs(heap, &this.view.data.environment))
    }

    /// The directory the command runs in, relative to the execution root; empty is the root.
    #[starlark(attribute)]
    fn working_directory(this: &SpawnValue) -> anyhow::Result<String> {
        Ok(this.view.data.working_directory.clone())
    }

    /// Execution-root relative paths of the declared outputs.
    #[starlark(attribute)]
    fn outputs<'v>(this: &SpawnValue, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        Ok(heap.alloc(AllocList(this.view.data.outputs.iter().map(String::as_str))))
    }

    /// The REAPI `Action` (command and input root digests, timeout, do_not_cache).
    /// Built on first use: it digests the input tree.
    #[starlark(attribute)]
    fn action<'v>(this: &SpawnValue, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        Ok(heap.alloc(this.view.request()?.action.clone()))
    }

    /// The digest of the REAPI `Action`: its cache key. Built on first use.
    #[starlark(attribute)]
    fn action_digest(this: &SpawnValue) -> anyhow::Result<String> {
        let digest = &this.view.request()?.action_digest;
        Ok(format!("{}/{}", digest.hash, digest.size_bytes))
    }

    /// The REAPI `Command`.
    #[starlark(attribute)]
    fn command<'v>(this: &SpawnValue, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        Ok(heap.alloc(this.view.request()?.command.clone()))
    }

    /// The input root as a REAPI `Tree`. Built on first use.
    #[starlark(attribute)]
    fn input_tree<'v>(this: &SpawnValue, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let request = this.view.request()?;
        let tree = &request.input_tree;
        Ok(heap.alloc(reapi::tree_message(tree)))
    }
}

/// An executor value that routes with a function: what `ctx.execution.route(fn)` returns.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct RouteExecutor {
    #[allocative(skip)]
    pub router: RouterRef,
}

pub struct RouterRef(pub Arc<dyn Router>);

impl fmt::Debug for RouterRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Router")
    }
}

impl fmt::Display for RouteExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RouteExecutor")
    }
}

starlark_simple_value!(RouteExecutor);

#[starlark_value(type = "RouteExecutor")]
impl<'v> StarlarkValue<'v> for RouteExecutor {}
