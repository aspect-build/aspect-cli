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
    fn get_type_starlark_repr() -> Ty {
        Ty::iter(Ty::int())
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
            "__builtins__ is only available within standard-library modules (@std, @bazel, @aspect)"
        ))
    }
}

/// Public reexport of [`check_std_context`] for sibling submodules (`grpc`)
/// that need to gate `__builtins__.<name>()` accessors.
pub fn check_std_context_pub(eval: &Evaluator) -> anyhow::Result<()> {
    check_std_context(eval)
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

/// Returned by `__builtins__.testing()`. Exposes the `*_test.axl` runner.
#[derive(Debug, Clone, Copy, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BuiltinsTesting;

impl fmt::Display for BuiltinsTesting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<BuiltinsTesting>")
    }
}

starlark_simple_value!(BuiltinsTesting);

#[starlark_value(type = "BuiltinsTesting")]
impl<'v> StarlarkValue<'v> for BuiltinsTesting {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(builtins_testing_methods)
    }
}

/// Marshal a test run into the AXL-facing summary dict:
/// `{"error": None|str, "passed": int, "failed": int, "outcomes": [...]}`,
/// where each outcome is `{"name": str, "passed": bool, "message": None|str}`.
///
/// A module-level failure (parse error, or a `load(...)` the loader-free
/// runner can't resolve) is surfaced as the top-level `error` string with no
/// outcomes, rather than raising — so one bad file never aborts a whole
/// `aspect axl test` run.
fn marshal_test_result<'v>(
    result: anyhow::Result<super::testing::TestSummary>,
    heap: Heap<'v>,
) -> Value<'v> {
    use starlark::values::dict::AllocDict;
    use starlark::values::list::AllocList;

    let (error, outcomes, passed, failed): (Value<'v>, Vec<Value<'v>>, i32, i32) = match result {
        Ok(summary) => {
            let outcomes = summary
                .outcomes
                .iter()
                .map(|o| {
                    let message = match &o.message {
                        Some(m) => heap.alloc(m.as_str()).to_value(),
                        None => Value::new_none(),
                    };
                    let entries: Vec<(Value<'v>, Value<'v>)> = vec![
                        (
                            heap.alloc("name").to_value(),
                            heap.alloc(o.name.as_str()).to_value(),
                        ),
                        (heap.alloc("passed").to_value(), Value::new_bool(o.passed)),
                        (heap.alloc("message").to_value(), message),
                    ];
                    heap.alloc(AllocDict(entries))
                })
                .collect();
            (
                Value::new_none(),
                outcomes,
                summary.passed() as i32,
                summary.failed() as i32,
            )
        }
        Err(e) => (
            heap.alloc(format!("{e:#}").as_str()).to_value(),
            Vec::new(),
            0,
            0,
        ),
    };

    let entries: Vec<(Value<'v>, Value<'v>)> = vec![
        (heap.alloc("error").to_value(), error),
        (
            heap.alloc("passed").to_value(),
            heap.alloc(passed).to_value(),
        ),
        (
            heap.alloc("failed").to_value(),
            heap.alloc(failed).to_value(),
        ),
        (
            heap.alloc("outcomes").to_value(),
            heap.alloc(AllocList(outcomes)).to_value(),
        ),
    ];
    heap.alloc(AllocDict(entries))
}

#[starlark_module]
fn builtins_testing_methods(registry: &mut MethodsBuilder) {
    /// Runs every top-level `def test_*(t)` function defined in `source` (the
    /// contents of a `*_test.axl` file), each with an isolated in-memory
    /// environment overlay, fanned out across worker threads. Returns the
    /// summary dict described on [`marshal_test_result`].
    ///
    /// `source` is evaluated with no module loader, so a file that
    /// `load(...)`s other modules comes back as a top-level `error` rather
    /// than running.
    fn run<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] source: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let _ = this;
        // No `check_std_context` here: the gate lives on the `testing()`
        // accessor (same as `hash()`/`time()`). A task captures the namespace
        // at module-eval time (where the marker is present) and calls `run`
        // later from the shared execution module, which carries no marker —
        // re-checking here would reject every legitimate call.
        let result = {
            let env = super::store::Env::from_eval(eval)?;
            super::testing::run_test_source(source, env)
        };
        Ok(marshal_test_result(result, eval.heap()))
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

    /// Returns the gRPC namespace, exposing `Server` and `Status`
    /// constructors. Only callable from within `@std`/`@bazel` modules;
    /// public modules should `load("@bazel//grpc.axl", "grpc")`.
    fn grpc<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<super::grpc::BuiltinsGrpc> {
        let _ = this;
        super::grpc::make_builtins_grpc(eval)
    }

    /// Returns the testing namespace, exposing `run(source)` — the built-in
    /// parallel `*_test.axl` runner that backs `aspect axl test`.
    ///
    /// Only callable from within standard-library modules (`@std`, `@bazel`,
    /// `@aspect`); there is no public `@std//…` wrapper because the runner is
    /// an internal capability of the `@aspect` standard library.
    fn testing<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<BuiltinsTesting> {
        let _ = this;
        check_std_context(eval)?;
        Ok(BuiltinsTesting)
    }
}

/// Registers the `json` namespace with `encode`, `decode`, and the
/// fallible `try_decode`. Replaces `LibraryExtension::Json` from the
/// Starlark stdlib (whose namespace is otherwise replaced — not merged
/// — when a second `globals.namespace("json", ...)` call lands).
///
/// `try_decode` is the reason this exists: Starlark has no try/except,
/// so a normal `json.decode` failure crashes the entire task. Any I/O
/// boundary that decodes an untrusted body (HTTP response, subprocess
/// output, file content from a flaky source) should use `try_decode`
/// and check the result rather than letting a malformed input bring
/// down the whole evaluation.
pub fn register_json(globals: &mut GlobalsBuilder) {
    #[starlark_module]
    fn json_members(globals: &mut GlobalsBuilder) {
        /// Encode a value to a JSON string. Mirrors the Starlark stdlib's
        /// `json.encode`, with an additional optional `indent` parameter:
        /// pass a non-negative integer to pretty-print the output with
        /// that many spaces of indentation per nesting level (newlines
        /// inserted between elements). Omitting `indent` produces the
        /// stdlib's compact single-line form.
        fn encode(
            #[starlark(require = pos)] x: Value,
            #[starlark(require = named)] indent: Option<u32>,
        ) -> anyhow::Result<String> {
            let Some(width) = indent else {
                return x.to_json();
            };
            let value = x.to_json_value()?;
            let spaces = vec![b' '; width as usize];
            let mut buf = Vec::new();
            let formatter = serde_json::ser::PrettyFormatter::with_indent(&spaces);
            let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
            serde::Serialize::serialize(&value, &mut ser)?;
            Ok(String::from_utf8(buf)?)
        }

        /// Decode a JSON string. Raises on parse failure. Mirrors the
        /// Starlark stdlib's `json.decode`. Use `try_decode` instead at
        /// I/O boundaries where a malformed input would otherwise crash
        /// the caller.
        fn decode<'v>(
            #[starlark(require = pos)] x: &str,
            heap: Heap<'v>,
        ) -> anyhow::Result<Value<'v>> {
            Ok(heap.alloc(serde_json::from_str::<serde_json::Value>(x)?))
        }

        /// Decode a JSON string. Returns `default` (None by default) on
        /// parse failure instead of raising. Distinguishing "parse
        /// failed" from "valid `null` parse": both produce None unless
        /// the caller passes a sentinel `default`.
        ///
        /// # Examples
        ///
        /// ```python
        /// json.try_decode('{"a": 1}')        # {"a": 1}
        /// json.try_decode("not json")        # None
        /// json.try_decode("not json", {})    # {}
        /// json.try_decode("null")            # None  (valid parse)
        /// json.try_decode("null", "MISS")    # None  (still valid; not the sentinel)
        /// ```
        fn try_decode<'v>(
            #[starlark(require = pos)] x: &str,
            #[starlark(require = pos)] default: Option<Value<'v>>,
            heap: Heap<'v>,
        ) -> anyhow::Result<Value<'v>> {
            match serde_json::from_str::<serde_json::Value>(x) {
                Ok(v) => Ok(heap.alloc(v)),
                Err(_) => Ok(default.unwrap_or_else(Value::new_none)),
            }
        }
    }
    globals.namespace("json", json_members);
}

#[starlark_module]
pub fn register_globals(globals: &mut GlobalsBuilder) {
    const __builtins__: Builtins = Builtins;
    const Hash: StarlarkValueAsType<HashObject> = StarlarkValueAsType::new();
}

#[cfg(test)]
mod marshal_tests {
    use super::marshal_test_result;
    use crate::engine::testing::{TestOutcome, TestSummary};
    use starlark::environment::Module;

    #[test]
    fn ok_summary_marshals_to_documented_dict_shape() {
        Module::with_temp_heap(|m| -> anyhow::Result<()> {
            let summary = TestSummary {
                outcomes: vec![
                    TestOutcome {
                        name: "test_a".to_string(),
                        passed: true,
                        message: None,
                    },
                    TestOutcome {
                        name: "test_b".to_string(),
                        passed: false,
                        message: Some("boom".to_string()),
                    },
                ],
            };
            let repr = marshal_test_result(Ok(summary), m.heap()).to_repr();
            // Top-level summary fields.
            assert!(repr.contains("\"error\""), "{repr}");
            assert!(repr.contains("None"), "{repr}");
            assert!(repr.contains("\"passed\""), "{repr}");
            assert!(repr.contains("\"failed\""), "{repr}");
            assert!(repr.contains("\"outcomes\""), "{repr}");
            // Per-outcome fields, including the failure message.
            assert!(repr.contains("\"name\""), "{repr}");
            assert!(repr.contains("test_a"), "{repr}");
            assert!(repr.contains("test_b"), "{repr}");
            assert!(repr.contains("boom"), "{repr}");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn module_error_marshals_as_top_level_error_with_no_outcomes() {
        Module::with_temp_heap(|m| -> anyhow::Result<()> {
            let repr = marshal_test_result(Err(anyhow::anyhow!("kaboom")), m.heap()).to_repr();
            assert!(repr.contains("kaboom"), "{repr}");
            assert!(repr.contains("\"outcomes\""), "{repr}");
            assert!(repr.contains("[]"), "{repr}");
            Ok(())
        })
        .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use crate::axl_eval;

    #[test]
    fn sleep_returns_none() {
        let result = axl_eval!(
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
        let result = axl_eval!(
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
        let result = axl_eval!(
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
        let result = axl_eval!(
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
        let result = axl_eval!(
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
        let result = axl_eval!(
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
        let result = axl_eval!(
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
        let result = axl_eval!(
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
        let result = axl_eval!(
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
        let result = axl_eval!(
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
        let result = axl_eval!(
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
        let result = axl_eval!(
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
        let result = axl_eval!(
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
        let result = axl_eval!(
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
        let result = axl_eval!(
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

    #[test]
    fn json_encode_compact_by_default() {
        // No indent argument → compact single-line output (stdlib parity).
        let result = axl_eval!(r#"json.encode({"a": 1, "b": [2, 3]})"#,).unwrap();
        assert_eq!(result, "\"{\\\"a\\\":1,\\\"b\\\":[2,3]}\"");
    }

    #[test]
    fn json_encode_indent_pretty_prints() {
        // indent=N pretty-prints with N spaces per nesting level.
        let result = axl_eval!(r#"json.encode({"a": 1, "b": [2, 3]}, indent = 2)"#,).unwrap();
        assert_eq!(
            result,
            "\"{\\n  \\\"a\\\": 1,\\n  \\\"b\\\": [\\n    2,\\n    3\\n  ]\\n}\""
        );
    }

    #[test]
    fn json_encode_preserves_dict_insertion_order() {
        // serde_json's `preserve_order` feature is enabled so the pretty
        // path matches the compact path's insertion-ordered keys; sorted
        // output would silently reorganize customer-facing manifests.
        let result = axl_eval!(r#"json.encode({"z": 1, "a": 2, "m": 3}, indent = 2)"#,).unwrap();
        assert_eq!(
            result,
            "\"{\\n  \\\"z\\\": 1,\\n  \\\"a\\\": 2,\\n  \\\"m\\\": 3\\n}\""
        );
    }

    #[test]
    fn json_encode_indent_zero_no_indentation_but_newlines() {
        // indent=0 still pretty-prints — newlines between elements but no
        // leading spaces. Distinct from omitting indent, which suppresses
        // newlines entirely.
        let result = axl_eval!(r#"json.encode([1, 2, 3], indent = 0)"#,).unwrap();
        assert_eq!(result, "\"[\\n1,\\n2,\\n3\\n]\"");
    }
}
