//! Runtime for the Starlark types `blazon` generates.
//!
//! Every generated Java class gets a Starlark value type holding a global
//! reference to the object. This crate is what those types share: the
//! conversion between Java and Starlark values, a per-process registry the
//! generated code fills in, and Starlark views of the JDK collections.
//!
//! Java to Starlark copies only `String`, `Boolean`, `Integer`, `Long`,
//! `StarlarkInt` and null/`NoneType`; every other object is wrapped by the
//! registry as the most specific generated type, or by one of the JDK views
//! in [`jdk`], and is an error naming the class when nothing matches. Starlark
//! to Java is the reverse; a generated value hands its object through.

pub mod jdk;

use blazon_rt::member::ClassRef;
use blazon_rt::{Error, Handle, Result, vm};
use jni::objects::JObject;
use starlark::values::none::NoneType;
use starlark::values::{Heap, Value};
use std::sync::OnceLock;

pub use either::Either;
pub use starlark;

/// What a generated version module provides. Registered once per process.
pub trait Registry: Send + Sync {
    /// The object as the most specific generated type, `None` when no
    /// generated type matches.
    fn wrap<'v>(&self, obj: &JObject, heap: Heap<'v>) -> Result<Option<Value<'v>>>;

    /// The object behind a generated value, `None` when `v` is not one.
    fn unwrap(&self, v: Value) -> Result<Option<JObject<'static>>>;

    /// `Starlark.NONE`, as a new local reference.
    fn none(&self) -> Result<JObject<'static>>;

    /// `StarlarkInt.of(v)`, as a new local reference.
    fn int(&self, v: i64) -> Result<JObject<'static>>;

    /// The value of a `StarlarkInt`, `None` when it does not fit in 64 bits.
    fn int_value(&self, obj: &JObject) -> Result<Option<i64>>;
}

static REGISTRY: OnceLock<&'static dyn Registry> = OnceLock::new();

/// Registers the process's generated types. A process binds one Bazel
/// version, so the first call wins.
pub fn register(r: &'static dyn Registry) {
    let _ = REGISTRY.set(r);
}

fn registry() -> Result<&'static dyn Registry> {
    REGISTRY.get().copied().ok_or_else(|| {
        Error::msg(
            "no generated Starlark types are registered; call the version module's `register()`",
        )
    })
}

/// Converts a runtime error for the Starlark evaluator.
pub fn error(e: Error) -> starlark::Error {
    starlark::Error::new_other(e)
}

/// A generated Starlark value: the Java object it stands for.
pub trait JavaValue {
    /// A new local reference to the object.
    fn java_object(&self) -> Result<JObject<'static>>;
}

/// What can become a Java argument.
pub trait ToJava {
    /// The Java object, as a new local reference the caller owns.
    fn to_java(&self) -> Result<JObject<'static>>;
}

impl<T: JavaValue> ToJava for &T {
    fn to_java(&self) -> Result<JObject<'static>> {
        (**self).java_object()
    }
}

impl ToJava for &str {
    fn to_java(&self) -> Result<JObject<'static>> {
        let mut env = vm::env()?;
        Ok(blazon_rt::java::lang::String::new(&mut env, self)?.into_raw())
    }
}

impl ToJava for bool {
    fn to_java(&self) -> Result<JObject<'static>> {
        let mut env = vm::env()?;
        Ok(blazon_rt::java::lang::Boolean::of(&mut env, *self)?.into_raw())
    }
}

impl ToJava for i64 {
    fn to_java(&self) -> Result<JObject<'static>> {
        registry()?.int(*self)
    }
}

impl ToJava for i32 {
    fn to_java(&self) -> Result<JObject<'static>> {
        registry()?.int(i64::from(*self))
    }
}

impl ToJava for NoneType {
    fn to_java(&self) -> Result<JObject<'static>> {
        registry()?.none()
    }
}

impl ToJava for Value<'_> {
    fn to_java(&self) -> Result<JObject<'static>> {
        unwrap(*self)
    }
}

impl<L: ToJava, R: ToJava> ToJava for Either<L, R> {
    fn to_java(&self) -> Result<JObject<'static>> {
        match self {
            Either::Left(l) => l.to_java(),
            Either::Right(r) => r.to_java(),
        }
    }
}

/// The classes `wrap` tests for before the registry, each looked up once.
struct Scalars {
    string: ClassRef,
    boolean: ClassRef,
    integer: ClassRef,
    long: ClassRef,
    starlark_int: ClassRef,
    none: ClassRef,
}

static SCALARS: Scalars = Scalars {
    string: ClassRef::new("java/lang/String"),
    boolean: ClassRef::new("java/lang/Boolean"),
    integer: ClassRef::new("java/lang/Integer"),
    long: ClassRef::new("java/lang/Long"),
    starlark_int: ClassRef::new("net/starlark/java/eval/StarlarkInt"),
    none: ClassRef::new("net/starlark/java/eval/NoneType"),
};

fn is(env: &mut jni::JNIEnv, obj: &JObject, class: &ClassRef) -> Result<bool> {
    class.is_instance(env, obj)
}

/// A Java object as a Starlark value.
pub fn wrap<'v>(obj: &JObject, heap: Heap<'v>) -> Result<Value<'v>> {
    if obj.is_null() {
        return Ok(Value::new_none());
    }
    let mut env = vm::env()?;
    if is(&mut env, obj, &SCALARS.string)? {
        // SAFETY: a non-owning view of `obj`, which is a `String`.
        let js = unsafe { jni::objects::JString::from_raw(obj.as_raw()) };
        let s: String = env.get_string(&js)?.into();
        return Ok(heap.alloc_str(&s).to_value());
    }
    if is(&mut env, obj, &SCALARS.boolean)? {
        let r = env.call_method(obj, "booleanValue", "()Z", &[]);
        return Ok(Value::new_bool(blazon_rt::checked(r)?.z()?));
    }
    if is(&mut env, obj, &SCALARS.integer)? || is(&mut env, obj, &SCALARS.long)? {
        let n = blazon_rt::conv::boxed_long_or_int(obj)?;
        return Ok(heap.alloc(n));
    }
    if is(&mut env, obj, &SCALARS.starlark_int)? {
        let n = registry()?
            .int_value(obj)?
            .ok_or_else(|| Error::msg("integer does not fit in 64 bits"))?;
        return Ok(heap.alloc(n));
    }
    if is(&mut env, obj, &SCALARS.none)? {
        return Ok(Value::new_none());
    }
    if let Some(v) = registry()?.wrap(obj, heap)? {
        return Ok(v);
    }
    if let Some(v) = jdk::view(obj, heap)? {
        return Ok(v);
    }
    Err(Error::msg(format!(
        "no generated Starlark type for {}; bind it in the blazon spec",
        blazon_rt::class_name(obj)?
    )))
}

/// A Starlark value as a Java object, as a new local reference the caller owns.
pub fn unwrap(v: Value) -> Result<JObject<'static>> {
    if v.is_none() {
        return registry()?.none();
    }
    if let Some(b) = v.unpack_bool() {
        return b.to_java();
    }
    if let Some(s) = v.unpack_str() {
        return s.to_java();
    }
    if let Some(i) = v.unpack_i32() {
        return i.to_java();
    }
    if let Some(o) = registry()?.unwrap(v)? {
        return Ok(o);
    }
    if let Some(o) = jdk::unview(v)? {
        return Ok(o);
    }
    Err(Error::msg(format!(
        "cannot pass a Starlark {} to Java; only str, bool, int, None and values of bound Java types convert",
        v.get_type()
    )))
}
