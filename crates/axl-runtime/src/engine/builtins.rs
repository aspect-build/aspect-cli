use std::fmt;
use std::fmt::Debug;
use std::sync::OnceLock;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use allocative::Allocative;
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use derive_more::Display;
use digest::Digest;
use starlark::any::ProvidesStaticType;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::typing::Ty;
use starlark::values::starlark_value_as_type::StarlarkValueAsType;
use starlark::values::{
    self, Heap, NoSerialize, StarlarkValue, Trace, Value, ValueLike, starlark_value,
};

use super::hash::{HashObject, HashState};

#[derive(ProvidesStaticType, Display, Trace, NoSerialize, Allocative, Debug)]
#[display("<sleep_iter>")]
pub struct SleepIter {
    pub rate: u64,
    pub counter: AtomicU64,
}

starlark_simple_value!(SleepIter);

#[starlark_value(type = "sleep_iter")]
impl<'v> values::StarlarkValue<'v> for SleepIter {
    fn eval_type(&self) -> Option<Ty> {
        Some(Ty::iter(
            axl_proto::tools::protos::ExecLogEntry::get_type_starlark_repr(),
        ))
    }

    fn get_type_starlark_repr() -> Ty {
        Ty::iter(axl_proto::tools::protos::ExecLogEntry::get_type_starlark_repr())
    }

    unsafe fn iterate(
        &self,
        me: values::Value<'v>,
        _heap: Heap<'v>,
    ) -> starlark::Result<values::Value<'v>> {
        Ok(me)
    }
    unsafe fn iter_next(&self, _index: usize, heap: Heap<'v>) -> Option<values::Value<'v>> {
        std::thread::sleep(Duration::from_millis(self.rate));
        Some(heap.alloc(self.counter.fetch_add(1, Ordering::Relaxed)))
    }
    unsafe fn iter_stop(&self) {}
}

static MONOTONIC_EPOCH: OnceLock<Instant> = OnceLock::new();

const STD_MARKER: &str = "#_is_std#";

fn check_std_context(eval: &Evaluator) -> anyhow::Result<()> {
    if eval.module().get(STD_MARKER).is_some() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "__builtins__ is only available within @std modules"
        ))
    }
}

/// Returned by `__builtins__.hash()`. Each method returns a fresh HashObject.
#[derive(Debug, Clone, Copy, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BuiltinsHash;

impl fmt::Display for BuiltinsHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<BuiltinsHash>")
    }
}

starlark_simple_value!(BuiltinsHash);

#[starlark_value(type = "BuiltinsHash")]
impl<'v> StarlarkValue<'v> for BuiltinsHash {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(builtins_hash_methods)
    }
}

#[starlark_module]
fn builtins_hash_methods(registry: &mut MethodsBuilder) {
    fn md5<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        Ok(heap.alloc(HashObject::new(HashState::Md5(md5::Md5::new()))))
    }

    fn sha1<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        Ok(heap.alloc(HashObject::new(HashState::Sha1(sha1::Sha1::new()))))
    }

    fn sha224<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        Ok(heap.alloc(HashObject::new(HashState::Sha224(sha2::Sha224::new()))))
    }

    fn sha256<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        Ok(heap.alloc(HashObject::new(HashState::Sha256(sha2::Sha256::new()))))
    }

    fn sha384<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        Ok(heap.alloc(HashObject::new(HashState::Sha384(sha2::Sha384::new()))))
    }

    fn sha512<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        Ok(heap.alloc(HashObject::new(HashState::Sha512(sha2::Sha512::new()))))
    }

    fn blake2b<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        Ok(heap.alloc(HashObject::new(HashState::Blake2b(
            blake2::Blake2b512::new(),
        ))))
    }

    fn blake2s<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        Ok(heap.alloc(HashObject::new(HashState::Blake2s(
            blake2::Blake2s256::new(),
        ))))
    }
}

/// Returned by `__builtins__.base64()`. Exposes encode/decode as methods.
#[derive(Debug, Clone, Copy, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BuiltinsBase64;

impl fmt::Display for BuiltinsBase64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<BuiltinsBase64>")
    }
}

starlark_simple_value!(BuiltinsBase64);

#[starlark_value(type = "BuiltinsBase64")]
impl<'v> StarlarkValue<'v> for BuiltinsBase64 {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(builtins_base64_methods)
    }
}

#[starlark_module]
fn builtins_base64_methods(registry: &mut MethodsBuilder) {
    fn encode<'v>(this: Value<'v>, value: Value<'v>) -> anyhow::Result<String> {
        let _ = this;
        if let Some(s) = value.unpack_str() {
            Ok(STANDARD.encode(s.as_bytes()))
        } else if let Some(b) = value.downcast_ref::<starlark::values::bytes::StarlarkBytes>() {
            Ok(STANDARD.encode(b.as_bytes()))
        } else {
            Err(anyhow::anyhow!(
                "base64.encode: expected str or bytes, got {}",
                value.get_type()
            ))
        }
    }

    fn decode<'v>(this: Value<'v>, value: &str, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        let data = STANDARD
            .decode(value)
            .map_err(|e| anyhow::anyhow!("base64.decode: {}", e))?;
        Ok(heap.alloc(starlark::values::bytes::StarlarkBytes::new(&data)))
    }

    fn encode_url<'v>(this: Value<'v>, value: Value<'v>) -> anyhow::Result<String> {
        let _ = this;
        if let Some(s) = value.unpack_str() {
            Ok(URL_SAFE_NO_PAD.encode(s.as_bytes()))
        } else if let Some(b) = value.downcast_ref::<starlark::values::bytes::StarlarkBytes>() {
            Ok(URL_SAFE_NO_PAD.encode(b.as_bytes()))
        } else {
            Err(anyhow::anyhow!(
                "base64.encode_url: expected str or bytes, got {}",
                value.get_type()
            ))
        }
    }

    fn decode_url<'v>(this: Value<'v>, value: &str, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        let data = URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|e| anyhow::anyhow!("base64.decode_url: {}", e))?;
        Ok(heap.alloc(starlark::values::bytes::StarlarkBytes::new(&data)))
    }
}

/// The `__builtins__` object. Always present in globals.
#[derive(Debug, Clone, Copy, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Builtins;

impl fmt::Display for Builtins {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<__builtins__>")
    }
}

starlark_simple_value!(Builtins);

#[starlark_value(type = "__builtins__")]
impl<'v> StarlarkValue<'v> for Builtins {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(builtins_methods)
    }
}

/// Returned by `__builtins__.time()`. Exposes sleep and sleep_iter.
#[derive(Debug, Clone, Copy, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BuiltinsTime;

impl fmt::Display for BuiltinsTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<BuiltinsTime>")
    }
}

starlark_simple_value!(BuiltinsTime);

#[starlark_value(type = "BuiltinsTime")]
impl<'v> StarlarkValue<'v> for BuiltinsTime {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(builtins_time_methods)
    }
}

#[starlark_module]
fn builtins_time_methods(registry: &mut MethodsBuilder) {
    fn sleep(this: Value<'_>, ms: u32) -> anyhow::Result<starlark::values::none::NoneType> {
        let _ = this;
        std::thread::sleep(Duration::from_millis(ms as u64));
        Ok(starlark::values::none::NoneType)
    }

    fn sleep_iter(this: Value<'_>, ms: u32) -> anyhow::Result<SleepIter> {
        let _ = this;
        Ok(SleepIter {
            rate: ms as u64,
            counter: AtomicU64::new(0),
        })
    }

    fn monotonic(this: Value<'_>) -> anyhow::Result<f64> {
        let _ = this;
        Ok(MONOTONIC_EPOCH
            .get_or_init(Instant::now)
            .elapsed()
            .as_secs_f64())
    }

    fn monotonic_ns(this: Value<'_>) -> anyhow::Result<i64> {
        let _ = this;
        Ok(MONOTONIC_EPOCH
            .get_or_init(Instant::now)
            .elapsed()
            .as_nanos() as i64)
    }
}

#[starlark_module]
fn builtins_methods(registry: &mut MethodsBuilder) {
    fn hash<'v>(this: Value<'v>, eval: &mut Evaluator<'v, '_, '_>) -> anyhow::Result<BuiltinsHash> {
        let _ = this;
        check_std_context(eval)?;
        Ok(BuiltinsHash)
    }

    fn base64<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<BuiltinsBase64> {
        let _ = this;
        check_std_context(eval)?;
        Ok(BuiltinsBase64)
    }

    fn time<'v>(this: Value<'v>, eval: &mut Evaluator<'v, '_, '_>) -> anyhow::Result<BuiltinsTime> {
        let _ = this;
        check_std_context(eval)?;
        Ok(BuiltinsTime)
    }
}

#[starlark_module]
pub fn register_globals(globals: &mut GlobalsBuilder) {
    const __builtins__: Builtins = Builtins;
    const Hash: StarlarkValueAsType<HashObject> = StarlarkValueAsType::new();

    fn forever(#[starlark()] rate: u32) -> anyhow::Result<SleepIter> {
        Ok(SleepIter {
            rate: rate as u64,
            counter: AtomicU64::new(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::eval::api::eval_expr;

    #[test]
    fn sleep_returns_none() {
        let result = eval_expr(
            r#"
load("@std//time.axl", "sleep")
sleep(1)
"#,
        )
        .unwrap();
        assert_eq!(result, "None");
    }

    #[test]
    fn sleep_via_time_struct() {
        let result = eval_expr(
            r#"
load("@std//time.axl", "time")
time.sleep(1)
"#,
        )
        .unwrap();
        assert_eq!(result, "None");
    }

    #[test]
    fn sleep_iter_yields_incrementing_ticks() {
        let result = eval_expr(
            r#"
load("@std//time.axl", "sleep_iter")
ticks = []
for t in sleep_iter(1):
    ticks.append(t)
    if t >= 2:
        break
ticks
"#,
        )
        .unwrap();
        assert_eq!(result, "[0, 1, 2]");
    }

    #[test]
    fn sleep_iter_via_time_struct() {
        let result = eval_expr(
            r#"
load("@std//time.axl", "time")
ticks = []
for t in time.sleep_iter(1):
    ticks.append(t)
    if t >= 2:
        break
ticks
"#,
        )
        .unwrap();
        assert_eq!(result, "[0, 1, 2]");
    }

    #[test]
    fn monotonic_returns_float() {
        let result = eval_expr(
            r#"
load("@std//time.axl", "monotonic")
t = monotonic()
type(t) == "float"
"#,
        )
        .unwrap();
        assert_eq!(result, "True");
    }

    #[test]
    fn monotonic_is_non_decreasing() {
        let result = eval_expr(
            r#"
load("@std//time.axl", "monotonic")
t1 = monotonic()
t2 = monotonic()
t2 >= t1
"#,
        )
        .unwrap();
        assert_eq!(result, "True");
    }

    #[test]
    fn monotonic_ns_returns_int() {
        let result = eval_expr(
            r#"
load("@std//time.axl", "monotonic_ns")
t = monotonic_ns()
type(t) == "int"
"#,
        )
        .unwrap();
        assert_eq!(result, "True");
    }

    #[test]
    fn monotonic_ns_is_non_decreasing() {
        let result = eval_expr(
            r#"
load("@std//time.axl", "monotonic_ns")
t1 = monotonic_ns()
t2 = monotonic_ns()
t2 >= t1
"#,
        )
        .unwrap();
        assert_eq!(result, "True");
    }

    #[test]
    fn monotonic_via_time_struct() {
        let result = eval_expr(
            r#"
load("@std//time.axl", "time")
type(time.monotonic()) == "float"
"#,
        )
        .unwrap();
        assert_eq!(result, "True");
    }

    #[test]
    fn monotonic_ns_via_time_struct() {
        let result = eval_expr(
            r#"
load("@std//time.axl", "time")
type(time.monotonic_ns()) == "int"
"#,
        )
        .unwrap();
        assert_eq!(result, "True");
    }

    #[test]
    fn monotonic_is_positive() {
        let result = eval_expr(
            r#"
load("@std//time.axl", "monotonic")
monotonic() > 0.0
"#,
        )
        .unwrap();
        assert_eq!(result, "True");
    }

    #[test]
    fn monotonic_ns_is_positive() {
        let result = eval_expr(
            r#"
load("@std//time.axl", "monotonic_ns")
monotonic_ns() > 0
"#,
        )
        .unwrap();
        assert_eq!(result, "True");
    }

    #[test]
    fn monotonic_advances_after_sleep() {
        let result = eval_expr(
            r#"
load("@std//time.axl", "monotonic", "sleep")
t1 = monotonic()
sleep(10)
t2 = monotonic()
t2 - t1 >= 0.01
"#,
        )
        .unwrap();
        assert_eq!(result, "True");
    }

    #[test]
    fn monotonic_ns_advances_after_sleep() {
        let result = eval_expr(
            r#"
load("@std//time.axl", "monotonic_ns", "sleep")
t1 = monotonic_ns()
sleep(10)
t2 = monotonic_ns()
t2 - t1 >= 10000000
"#,
        )
        .unwrap();
        assert_eq!(result, "True");
    }

    #[test]
    fn monotonic_ns_consistent_with_monotonic() {
        // monotonic_ns should be within 1 second of monotonic() * 1e9
        let result = eval_expr(
            r#"
load("@std//time.axl", "monotonic", "monotonic_ns")
s = monotonic()
ns = monotonic_ns()
diff = ns - int(s * 1000000000)
diff > -1000000000 and diff < 1000000000
"#,
        )
        .unwrap();
        assert_eq!(result, "True");
    }
}
