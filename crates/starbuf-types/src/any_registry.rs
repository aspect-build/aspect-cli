use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use starlark::values::{AllocValue, Heap, Value};

/// Trait for type-erased allocation on the Starlark heap.
pub trait AnyAllocatable: Send + Sync + 'static {
    fn alloc_on<'v>(self: Box<Self>, heap: Heap<'v>) -> Value<'v>;
}

/// Blanket impl for all types that can be allocated on the Starlark heap.
impl<T> AnyAllocatable for T
where
    T: for<'v> AllocValue<'v> + Send + Sync + 'static,
{
    fn alloc_on<'v>(self: Box<Self>, heap: Heap<'v>) -> Value<'v> {
        heap.alloc(*self)
    }
}

type DeserializerFn = Box<dyn Fn(&[u8]) -> anyhow::Result<Box<dyn AnyAllocatable>> + Send + Sync>;

static REGISTRY: OnceLock<Mutex<HashMap<String, DeserializerFn>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, DeserializerFn>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register a protobuf message type so it can be unpacked from `Any`.
pub fn register(
    full_name: String,
    deserializer: Box<dyn Fn(&[u8]) -> anyhow::Result<Box<dyn AnyAllocatable>> + Send + Sync>,
) {
    registry().lock().unwrap().insert(full_name, deserializer);
}

/// Strip the `type.googleapis.com/` prefix from a type URL.
fn normalize_type_url(url: &str) -> &str {
    url.strip_prefix("type.googleapis.com/").unwrap_or(url)
}

/// Decode an `Any` value and allocate the result on the Starlark heap.
pub fn unpack<'v>(type_url: &str, bytes: &[u8], heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
    let name = normalize_type_url(type_url);
    let reg = registry().lock().unwrap();
    let deserializer = reg
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("unknown Any type: {}", name))?;
    let value = deserializer(bytes)?;
    Ok(value.alloc_on(heap))
}
