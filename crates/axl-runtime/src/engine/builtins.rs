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
    /// Creates a new MD5 hash object.
    ///
    /// Call `update(data)` to feed bytes or strings into the hash, then
    /// `digest()` for raw bytes or `hexdigest()` for the hex-encoded string.
    ///
    /// # Examples
    ///
    /// ```python
    /// load("@std//hash.axl", "md5")
    /// h = md5()
    /// h.update("hello world")
    /// h.hexdigest()  # "5eb63bbbe01eeed093cb22bb8f5acdc3"
    /// ```
    fn md5<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        Ok(heap.alloc(HashObject::new(HashState::Md5(md5::Md5::new()))))
    }

    /// Creates a new SHA-1 hash object.
    ///
    /// Call `update(data)` to feed bytes or strings into the hash, then
    /// `digest()` for raw bytes or `hexdigest()` for the hex-encoded string.
    ///
    /// # Examples
    ///
    /// ```python
    /// load("@std//hash.axl", "sha1")
    /// h = sha1()
    /// h.update("hello world")
    /// h.hexdigest()  # "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed"
    /// ```
    fn sha1<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        Ok(heap.alloc(HashObject::new(HashState::Sha1(sha1::Sha1::new()))))
    }

    /// Creates a new SHA-224 hash object.
    ///
    /// Call `update(data)` to feed bytes or strings into the hash, then
    /// `digest()` for raw bytes or `hexdigest()` for the hex-encoded string.
    ///
    /// # Examples
    ///
    /// ```python
    /// load("@std//hash.axl", "sha224")
    /// h = sha224()
    /// h.update("hello world")
    /// h.hexdigest()  # "2f05477fc24bb4faefd86517156dafdecec45b8a..."
    /// ```
    fn sha224<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        Ok(heap.alloc(HashObject::new(HashState::Sha224(sha2::Sha224::new()))))
    }

    /// Creates a new SHA-256 hash object.
    ///
    /// Call `update(data)` to feed bytes or strings into the hash, then
    /// `digest()` for raw bytes or `hexdigest()` for the hex-encoded string.
    ///
    /// # Examples
    ///
    /// ```python
    /// load("@std//hash.axl", "sha256")
    /// h = sha256()
    /// h.update("hello world")
    /// h.hexdigest()
    /// # "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
    /// ```
    fn sha256<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        Ok(heap.alloc(HashObject::new(HashState::Sha256(sha2::Sha256::new()))))
    }

    /// Creates a new SHA-384 hash object.
    ///
    /// Call `update(data)` to feed bytes or strings into the hash, then
    /// `digest()` for raw bytes or `hexdigest()` for the hex-encoded string.
    ///
    /// # Examples
    ///
    /// ```python
    /// load("@std//hash.axl", "sha384")
    /// h = sha384()
    /// h.update("hello world")
    /// h.hexdigest()  # 96-char hex string
    /// ```
    fn sha384<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        Ok(heap.alloc(HashObject::new(HashState::Sha384(sha2::Sha384::new()))))
    }

    /// Creates a new SHA-512 hash object.
    ///
    /// Call `update(data)` to feed bytes or strings into the hash, then
    /// `digest()` for raw bytes or `hexdigest()` for the hex-encoded string.
    ///
    /// # Examples
    ///
    /// ```python
    /// load("@std//hash.axl", "sha512")
    /// h = sha512()
    /// h.update("hello ")
    /// h.update("world")
    /// h.hexdigest()  # 128-char hex string
    /// ```
    fn sha512<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        Ok(heap.alloc(HashObject::new(HashState::Sha512(sha2::Sha512::new()))))
    }

    /// Creates a new BLAKE2b-512 hash object.
    ///
    /// Call `update(data)` to feed bytes or strings into the hash, then
    /// `digest()` for raw bytes or `hexdigest()` for the hex-encoded string.
    ///
    /// # Examples
    ///
    /// ```python
    /// load("@std//hash.axl", "blake2b")
    /// h = blake2b()
    /// h.update("hello world")
    /// h.digest()  # raw 64-byte digest as bytes
    /// ```
    fn blake2b<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        Ok(heap.alloc(HashObject::new(HashState::Blake2b(
            blake2::Blake2b512::new(),
        ))))
    }

    /// Creates a new BLAKE2s-256 hash object.
    ///
    /// Call `update(data)` to feed bytes or strings into the hash, then
    /// `digest()` for raw bytes or `hexdigest()` for the hex-encoded string.
    ///
    /// # Examples
    ///
    /// ```python
    /// load("@std//hash.axl", "blake2s")
    /// h = blake2s()
    /// h.update("hello world")
    /// h.digest()  # raw 32-byte digest as bytes
    /// ```
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
    /// Encodes a string or bytes value as a standard Base64 string (RFC 4648).
    ///
    /// Accepts either `str` (encoded as UTF-8) or `bytes`. Output uses the
    /// standard alphabet with `=` padding.
    ///
    /// # Examples
    ///
    /// ```python
    /// load("@std//base64.axl", "base64")
    /// base64.encode("hello")     # "aGVsbG8="
    /// base64.encode(b"\x00\xff")  # "AP8="
    /// ```
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

    /// Decodes a standard Base64-encoded string into `bytes` (RFC 4648).
    ///
    /// Expects the standard alphabet with `=` padding. Raises if the input
    /// is not valid Base64.
    ///
    /// # Examples
    ///
    /// ```python
    /// load("@std//base64.axl", "base64")
    /// base64.decode("aGVsbG8=")  # b"hello"
    /// ```
    fn decode<'v>(this: Value<'v>, value: &str, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        let data = STANDARD
            .decode(value)
            .map_err(|e| anyhow::anyhow!("base64.decode: {}", e))?;
        Ok(heap.alloc(starlark::values::bytes::StarlarkBytes::new(&data)))
    }

    /// Encodes a string or bytes value as a URL-safe Base64 string without padding.
    ///
    /// Uses the URL-safe alphabet (`-` and `_` instead of `+` and `/`) and
    /// omits trailing `=` padding. Accepts either `str` (encoded as UTF-8)
    /// or `bytes`.
    ///
    /// # Examples
    ///
    /// ```python
    /// load("@std//base64.axl", "base64")
    /// base64.encode_url("hello?")  # "aGVsbG8_"  (no '=' padding)
    /// ```
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

    /// Decodes a URL-safe Base64 string (no padding) into `bytes`.
    ///
    /// Expects the URL-safe alphabet (`-` and `_`) with no trailing `=`
    /// padding. Raises if the input is not valid Base64.
    ///
    /// # Examples
    ///
    /// ```python
    /// load("@std//base64.axl", "base64")
    /// base64.decode_url("aGVsbG8_")  # b"hello?"
    /// ```
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
    /// Blocks the current thread for `ms` milliseconds.
    ///
    /// Returns `None`. The sleep is synchronous; the calling task's thread
    /// is parked for the full duration.
    ///
    /// # Examples
    ///
    /// ```python
    /// load("@std//time.axl", "sleep")
    /// sleep(250)  # pause for 250 ms
    /// ```
    fn sleep(this: Value<'_>, ms: u32) -> anyhow::Result<starlark::values::none::NoneType> {
        let _ = this;
        std::thread::sleep(Duration::from_millis(ms as u64));
        Ok(starlark::values::none::NoneType)
    }

    /// Returns an infinite iterator that yields a monotonically increasing
    /// integer every `ms` milliseconds.
    ///
    /// Each call to `next` sleeps for `ms` milliseconds, then returns the
    /// next tick (starting at `0`). Use `break` to stop iteration.
    ///
    /// # Examples
    ///
    /// ```python
    /// load("@std//time.axl", "sleep_iter")
    /// for tick in sleep_iter(1000):  # poll once per second
    ///     if check_done():
    ///         break
    /// ```
    fn sleep_iter(this: Value<'_>, ms: u32) -> anyhow::Result<SleepIter> {
        let _ = this;
        Ok(SleepIter {
            rate: ms as u64,
            counter: AtomicU64::new(0),
        })
    }

    /// Returns a monotonically non-decreasing time in seconds as a float.
    ///
    /// The reference epoch is fixed at process start, so values are only
    /// meaningful relative to other `monotonic()` readings within the same
    /// process. Suitable for measuring elapsed time.
    ///
    /// # Examples
    ///
    /// ```python
    /// load("@std//time.axl", "monotonic")
    /// start = monotonic()
    /// do_work()
    /// elapsed = monotonic() - start  # seconds, as float
    /// ```
    fn monotonic(this: Value<'_>) -> anyhow::Result<f64> {
        let _ = this;
        Ok(MONOTONIC_EPOCH
            .get_or_init(Instant::now)
            .elapsed()
            .as_secs_f64())
    }

    /// Returns a monotonically non-decreasing time in nanoseconds as an int.
    ///
    /// The reference epoch is fixed at process start, so values are only
    /// meaningful relative to other `monotonic_ns()` readings within the
    /// same process. Higher precision than `monotonic()`.
    ///
    /// # Examples
    ///
    /// ```python
    /// load("@std//time.axl", "monotonic_ns")
    /// start = monotonic_ns()
    /// do_work()
    /// elapsed_ns = monotonic_ns() - start  # nanoseconds, as int
    /// ```
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
    /// Returns the hash namespace, exposing constructors for supported
    /// digest algorithms (`md5`, `sha1`, `sha224`, `sha256`, `sha384`,
    /// `sha512`, `blake2b`, `blake2s`).
    ///
    /// Only callable from within `@std` modules; other modules should
    /// `load("@std//hash.axl", ...)` to access hashing.
    fn hash<'v>(this: Value<'v>, eval: &mut Evaluator<'v, '_, '_>) -> anyhow::Result<BuiltinsHash> {
        let _ = this;
        check_std_context(eval)?;
        Ok(BuiltinsHash)
    }

    /// Returns the Base64 namespace, exposing `encode`, `decode`,
    /// `encode_url`, and `decode_url`.
    ///
    /// Only callable from within `@std` modules; other modules should
    /// `load("@std//base64.axl", ...)` to access Base64.
    fn base64<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<BuiltinsBase64> {
        let _ = this;
        check_std_context(eval)?;
        Ok(BuiltinsBase64)
    }

    /// Returns the time namespace, exposing `sleep`, `sleep_iter`,
    /// `monotonic`, and `monotonic_ns`.
    ///
    /// Only callable from within `@std` modules; other modules should
    /// `load("@std//time.axl", ...)` to access time utilities.
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

    /// Returns an infinite iterator that yields a monotonically increasing
    /// integer every `rate` milliseconds.
    ///
    /// Useful for driving polling loops. Each iteration sleeps for `rate`
    /// milliseconds before yielding the next tick (starting at `0`); use
    /// `break` to terminate the loop.
    ///
    /// # Examples
    ///
    /// ```python
    /// for tick in forever(500):  # poll every 500 ms
    ///     status = check()
    ///     if status == "done":
    ///         break
    /// ```
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
