//! StateDict — a freeze-immune mutable key-value store for feature closure state.
//!
//! Starlark dicts (`{}`) are frozen when the config heap is frozen, making them
//! immutable before task execution begins. `StateDict` uses a shared `Rc<RefCell<...>>`
//! that survives the freeze: `StateDict<'v>` and its frozen form `FrozenStateDict`
//! share the same backing map, so callbacks registered during config evaluation can
//! read and write state during task execution.
//!
//! Usage in feature impls:
//! ```python
//! state = state_dict()
//!
//! def _build_start(ctx):
//!     state["token"] = compute_token(ctx)
//!
//! def _build_end(ctx, exit_code):
//!     token = state.get("token")  # None if not set
//! ```

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use allocative::Allocative;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{
    AllocValue, Freeze, FreezeError, Freezer, Heap, NoSerialize, ProvidesStaticType, StarlarkValue,
    Trace, Tracer, Value, ValueLike, starlark_value,
    starlark_value_as_type::StarlarkValueAsType,
};
use starlark_map::small_map::SmallMap;

type Inner = SmallMap<String, Value<'static>>;
type SharedInner = Rc<RefCell<Inner>>;

/// Mutable key-value store that survives config heap freeze.
///
/// Create via `state_dict()`. Supports `state[key]`, `state[key] = value`,
/// `state.get(key)`, and `key in state`.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StateDict<'v> {
    #[allocative(skip)]
    inner: SharedInner,
    _ph: std::marker::PhantomData<Value<'v>>,
}

impl<'v> fmt::Display for StateDict<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "state_dict(len={})", self.inner.borrow().len())
    }
}

unsafe impl<'v> Trace<'v> for StateDict<'v> {
    fn trace(&mut self, _tracer: &Tracer<'v>) {
        // Values stored here come from the task heap (as Value<'static> transmutes);
        // the config GC does not need to trace them.
    }
}

impl<'v> AllocValue<'v> for StateDict<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Freeze for StateDict<'v> {
    type Frozen = FrozenStateDict;

    fn freeze(self, _freezer: &Freezer) -> Result<FrozenStateDict, FreezeError> {
        // Share the Rc — FrozenStateDict retains full read/write access.
        Ok(FrozenStateDict { inner: self.inner })
    }
}

#[starlark_value(type = "StateDict")]
impl<'v> StarlarkValue<'v> for StateDict<'v> {
    type Canonical = Self;

    fn at(&self, index: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        at_impl(&self.inner, index)
    }

    fn set_at(&self, index: Value<'v>, new_value: Value<'v>) -> starlark::Result<()> {
        set_at_impl(&self.inner, index, new_value)
    }

    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        is_in_impl(&self.inner, other)
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(live_methods)
    }
}

// --- Frozen form ---

/// Frozen form of `StateDict`. Shares its backing map with the original,
/// remaining fully writable during task execution.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenStateDict {
    #[allocative(skip)]
    inner: SharedInner,
}

// SAFETY: AXL runtime is single-threaded; these values are never accessed concurrently.
unsafe impl Send for FrozenStateDict {}
unsafe impl Sync for FrozenStateDict {}
unsafe impl<'v> Send for StateDict<'v> {}
unsafe impl<'v> Sync for StateDict<'v> {}

impl fmt::Display for FrozenStateDict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "state_dict(len={})", self.inner.borrow().len())
    }
}

starlark_simple_value!(FrozenStateDict);

#[starlark_value(type = "StateDict")]
impl<'v> StarlarkValue<'v> for FrozenStateDict {
    type Canonical = StateDict<'v>;

    fn at(&self, index: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        at_impl(&self.inner, index)
    }

    fn set_at(&self, index: Value<'v>, new_value: Value<'v>) -> starlark::Result<()> {
        set_at_impl(&self.inner, index, new_value)
    }

    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        is_in_impl(&self.inner, other)
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(frozen_methods)
    }
}

// --- Shared logic ---

fn at_impl<'v>(inner: &SharedInner, index: Value<'v>) -> starlark::Result<Value<'v>> {
    let key = require_str_key(index)?;
    match inner.borrow().get(key) {
        // SAFETY: Values live on the task heap which is alive during task execution.
        Some(v) => Ok(unsafe { std::mem::transmute::<Value<'static>, Value<'v>>(*v) }),
        None => Err(starlark::Error::new_other(anyhow::anyhow!(
            "Key {:?} not found in StateDict",
            key
        ))),
    }
}

fn set_at_impl<'v>(
    inner: &SharedInner,
    index: Value<'v>,
    new_value: Value<'v>,
) -> starlark::Result<()> {
    let key = require_str_key(index)?;
    // SAFETY: Values stored here live on the task heap, which is alive for the
    // full duration of task execution (the only time set_at is called post-freeze).
    let raw: Value<'static> = unsafe { std::mem::transmute(new_value) };
    inner.borrow_mut().insert(key.to_string(), raw);
    Ok(())
}

fn is_in_impl<'v>(inner: &SharedInner, other: Value<'v>) -> starlark::Result<bool> {
    let key = require_str_key(other)?;
    Ok(inner.borrow().contains_key(key))
}

fn require_str_key<'v>(v: Value<'v>) -> starlark::Result<&'v str> {
    v.unpack_str().ok_or_else(|| {
        starlark::Error::new_other(anyhow::anyhow!(
            "StateDict key must be a string, got '{}'",
            v.get_type()
        ))
    })
}

fn get_impl<'v>(inner: &SharedInner, key: &str, default: Option<Value<'v>>) -> anyhow::Result<Value<'v>> {
    match inner.borrow().get(key) {
        Some(v) => Ok(unsafe { std::mem::transmute::<Value<'static>, Value<'v>>(*v) }),
        None => Ok(default.unwrap_or_else(Value::new_none)),
    }
}

// --- Methods ---

#[starlark_module]
fn live_methods(registry: &mut MethodsBuilder) {
    /// Returns the value for the given key, or `default` (None by default) if not present.
    fn get<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] key: &str,
        #[starlark(require = pos)] default: Option<Value<'v>>,
    ) -> anyhow::Result<Value<'v>> {
        let sd = this
            .downcast_ref::<StateDict<'v>>()
            .expect("method is registered on StateDict");
        get_impl(&sd.inner, key, default)
    }
}

#[starlark_module]
fn frozen_methods(registry: &mut MethodsBuilder) {
    /// Returns the value for the given key, or `default` (None by default) if not present.
    fn get<'v>(
        this: &FrozenStateDict,
        #[starlark(require = pos)] key: &str,
        #[starlark(require = pos)] default: Option<Value<'v>>,
    ) -> anyhow::Result<Value<'v>> {
        get_impl(&this.inner, key, default)
    }
}

// --- Constructor ---

pub fn new_state_dict<'v>(heap: Heap<'v>) -> Value<'v> {
    heap.alloc(StateDict {
        inner: Rc::new(RefCell::new(SmallMap::new())),
        _ph: std::marker::PhantomData,
    })
}

// --- Global registration ---

#[starlark_module]
pub fn register_globals(globals: &mut GlobalsBuilder) {
    /// Creates a mutable key-value store that survives the config heap freeze.
    ///
    /// Use `state_dict()` instead of `{}` in feature implementations when you need
    /// state that is written during task execution (e.g. in `build_start`/`build_end` hooks).
    fn state_dict<'v>(heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        Ok(new_state_dict(heap))
    }

    const StateDict: StarlarkValueAsType<StateDict<'static>> = StarlarkValueAsType::new();
}
