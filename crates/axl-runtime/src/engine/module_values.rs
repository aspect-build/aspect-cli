//! What a module keeps live on its heap for the builtins it calls, in its one
//! traced `extra_value` slot: the error values it raised, and the context of
//! the task it runs.
//!
//! A raised value cannot ride inside an `anyhow::Error`, which must be
//! `'static`, and freezing it would forward the live value (and every value
//! its fields reach) out from under the code still holding it. So `fail(e)`
//! parks `e` here, where the garbage collector sees it, and the error carries
//! only its slot ([`raise`], [`raised`]). The task context is recorded when a
//! task's body starts ([`set_task_context`]), for builtins that call back into
//! the task's hooks with it ([`task_context`]).

use std::cell::RefCell;
use std::fmt::{self, Display};
use std::sync::atomic::{AtomicU64, Ordering};

use allocative::Allocative;
use starlark::eval::Evaluator;
use starlark::starlark_simple_value;
use starlark::values::{
    Freeze, FreezeResult, Freezer, NoSerialize, ProvidesStaticType, StarlarkValue, Trace, Value,
    ValueLike, starlark_value,
};

#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
struct ModuleValues<'v> {
    #[trace(static)]
    id: u64,
    #[allocative(skip)]
    raised: RefCell<Vec<Value<'v>>>,
    #[allocative(skip)]
    task_context: RefCell<Option<Value<'v>>>,
}

static MODULE_VALUES_ID: AtomicU64 = AtomicU64::new(0);

impl Display for ModuleValues<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("module_values")
    }
}

#[starlark_value(type = "module_values")]
impl<'v> StarlarkValue<'v> for ModuleValues<'v> {}

/// A module that finishes loading keeps none of them.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, derive_more::Display)]
#[display("module_values")]
struct FrozenModuleValues;

starlark_simple_value!(FrozenModuleValues);

#[starlark_value(type = "module_values")]
impl<'v> StarlarkValue<'v> for FrozenModuleValues {
    type Canonical = ModuleValues<'v>;
}

impl Freeze for ModuleValues<'_> {
    type Frozen = FrozenModuleValues;

    fn freeze(self, _freezer: &Freezer) -> FreezeResult<Self::Frozen> {
        Ok(FrozenModuleValues)
    }
}

impl<'v> ModuleValues<'v> {
    /// The current module's store, created on first use. `None` if the slot
    /// already holds something else.
    fn of(eval: &Evaluator<'v, '_, '_>) -> Option<&'v ModuleValues<'v>> {
        let module = eval.module();
        let slot = match module.extra_value() {
            Some(v) => v,
            None => {
                let v = eval.heap().alloc_complex(ModuleValues {
                    id: MODULE_VALUES_ID.fetch_add(1, Ordering::SeqCst),
                    raised: RefCell::new(Vec::new()),
                    task_context: RefCell::new(None),
                });
                module.set_extra_value(v);
                v
            }
        };
        slot.downcast_ref::<ModuleValues<'v>>()
    }
}

/// Park `value`, a raised error value, in the module `eval` runs; its slot,
/// for [`raised`], or None if the module's slot holds something else.
pub(crate) fn raise<'v>(eval: &Evaluator<'v, '_, '_>, value: Value<'v>) -> Option<(u64, usize)> {
    ModuleValues::of(eval).map(|store| {
        let mut raised = store.raised.borrow_mut();
        raised.push(value);
        (store.id, raised.len() - 1)
    })
}

/// The value parked at `slot`, if it was raised in the module `eval` runs.
pub(crate) fn raised<'v>(
    eval: &Evaluator<'v, '_, '_>,
    (id, index): (u64, usize),
) -> Option<Value<'v>> {
    let store = eval
        .module()
        .extra_value()?
        .downcast_ref::<ModuleValues<'v>>()?;
    if store.id != id {
        return None;
    }
    store.raised.borrow().get(index).copied()
}

/// Record `ctx` as the context of the task `eval` runs.
pub(crate) fn set_task_context<'v>(eval: &Evaluator<'v, '_, '_>, ctx: Value<'v>) {
    if let Some(store) = ModuleValues::of(eval) {
        *store.task_context.borrow_mut() = Some(ctx);
    }
}

/// The context of the task `eval` runs, once [`set_task_context`] recorded
/// it: None outside a task body.
pub(crate) fn task_context<'v>(eval: &Evaluator<'v, '_, '_>) -> Option<Value<'v>> {
    ModuleValues::of(eval).and_then(|store| *store.task_context.borrow())
}
