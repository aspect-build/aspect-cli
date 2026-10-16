//! The conversions generated code applies at the boundary: Java results to
//! Rust values and Rust arguments to Java values. They live here so that the
//! generated code reads as plain calls, and so that the `unsafe` in them is
//! written and reviewed once.

use crate::java::lang::{self, Object};
use crate::{Borrowed, Error, Handle, IsA, Result, vm};
use jni::JNIEnv;
use jni::objects::{JByteArray, JObject, JObjectArray, JString, JValue};

fn null_result(what: &str) -> Error {
    Error::msg(format!(
        "{what} returned null; if that is expected, mark the member nullable in the spec with a trailing `?`"
    ))
}

/// A result that is an instance of `H`, as a handle; null is an error naming `what`.
pub fn wrap<'a, H: Handle<'a>>(o: JObject<'a>, what: &str) -> Result<H> {
    if o.is_null() {
        return Err(null_result(what));
    }
    // SAFETY: the generated call's descriptor says the result is an `H`.
    Ok(unsafe { H::from_raw(o) })
}

/// Like [`wrap`], with null as `None`.
pub fn wrap_opt<'a, H: Handle<'a>>(o: JObject<'a>) -> Result<Option<H>> {
    if o.is_null() {
        return Ok(None);
    }
    // SAFETY: as for `wrap`.
    Ok(Some(unsafe { H::from_raw(o) }))
}

/// A subtype's handle as its supertype's.
///
/// # Safety
/// `From` must be a subtype of `To`.
pub unsafe fn upcast_owned<'a, From: Handle<'a>, To: Handle<'a>>(from: From) -> To {
    // SAFETY: by the caller; the one reference moves.
    unsafe { To::from_raw(from.into_raw()) }
}

/// A `java.lang.String` result as a Rust string; null is an error naming `what`.
pub fn string(o: JObject, what: &str) -> Result<String> {
    string_opt(o)?.ok_or_else(|| null_result(what))
}

/// A `java.lang.String` result, null as `None`.
pub fn string_opt(o: JObject) -> Result<Option<String>> {
    if o.is_null() {
        return Ok(None);
    }
    let mut env = vm::env()?;
    let js = JString::from(o);
    let text: String = env.get_string(&js)?.into();
    let _ = env.delete_local_ref(js);
    Ok(Some(text))
}

macro_rules! boxed_result {
    ($name:ident, $opt:ident, $ty:ty, $class:literal, $method:literal, $sig:literal, $get:ident) => {
        /// A boxed result as a scalar; null is an error naming `what`.
        pub fn $name(o: JObject, what: &str) -> Result<$ty> {
            $opt(o)?.ok_or_else(|| null_result(what))
        }

        /// A boxed result, null as `None`.
        pub fn $opt(o: JObject) -> Result<Option<$ty>> {
            if o.is_null() {
                return Ok(None);
            }
            let mut env = vm::env()?;
            let r = env.call_method(&o, $method, $sig, &[]);
            let v = crate::checked(r)?.$get()?;
            let _ = env.delete_local_ref(o);
            Ok(Some(v))
        }
    };
}

boxed_result!(
    boxed_bool,
    boxed_bool_opt,
    bool,
    "java/lang/Boolean",
    "booleanValue",
    "()Z",
    z
);
boxed_result!(
    boxed_int,
    boxed_int_opt,
    i32,
    "java/lang/Integer",
    "intValue",
    "()I",
    i
);
boxed_result!(
    boxed_long,
    boxed_long_opt,
    i64,
    "java/lang/Long",
    "longValue",
    "()J",
    j
);

/// An `Integer` or `Long` result, borrowed, as an `i64`.
pub fn boxed_long_or_int(o: &JObject) -> Result<i64> {
    let mut env = vm::env()?;
    let r = env.call_method(o, "longValue", "()J", &[]);
    Ok(crate::checked(r)?.j()?)
}

/// A `java.util.Optional` result as `Option`.
pub fn optional<'a, T: crate::Rebind<'a>>(o: JObject<'a>, what: &str) -> Result<Option<T::Out>> {
    let opt: crate::java::util::Optional<'a, T> = wrap(o, what)?;
    opt.value()
}

/// A `byte[]` result; null is an error naming `what`.
pub fn bytes(o: JObject, what: &str) -> Result<Vec<u8>> {
    if o.is_null() {
        return Err(null_result(what));
    }
    let env = vm::env()?;
    Ok(env.convert_byte_array(JByteArray::from(o))?)
}

fn elements<'a, T>(
    o: JObject<'a>,
    what: &str,
    mut each: impl FnMut(JObject<'a>) -> Result<T>,
) -> Result<Vec<T>> {
    if o.is_null() {
        return Err(null_result(what));
    }
    let mut env = vm::env()?;
    let arr = JObjectArray::from(o);
    let n = env.get_array_length(&arr)?;
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let e = env.get_object_array_element(&arr, i)?;
        out.push(each(e)?);
    }
    Ok(out)
}

/// A `String[]` result.
pub fn strings(o: JObject, what: &str) -> Result<Vec<String>> {
    elements(o, what, |e| string(e, what))
}

/// A reference array result, as handles.
pub fn objects<'a, T: crate::Rebind<'a>>(o: JObject<'a>, what: &str) -> Result<Vec<T::Out>> {
    elements(o, what, |e| wrap(e, what))
}

/// A `String` argument.
pub fn jstring<'l>(env: &mut JNIEnv<'l>, s: &str) -> Result<lang::String<'l>> {
    lang::String::new(env, s)
}

/// A handle's object, or null when there is none: for an argument that may be null.
pub fn or_null<'a, H: Handle<'a>>(h: &Option<H>) -> JObject<'static> {
    match h {
        // SAFETY: a non-owning copy, used within the call that borrowed `h`.
        Some(h) => unsafe { JObject::from_raw(h.as_ref().as_raw()) },
        None => JObject::null(),
    }
}

/// An object argument that may be null.
pub fn borrowed_opt(o: Option<&impl Borrowed>) -> JObject<'static> {
    o.map(Borrowed::borrowed).unwrap_or_else(JObject::null)
}

/// Boxes a primitive argument as the wrapper class reflection expects.
pub fn box_bool<'l>(env: &mut JNIEnv<'l>, v: bool) -> Result<Object<'l>> {
    Ok(Object::of(lang::Boolean::of(env, v)?.into_raw()))
}

pub fn box_int<'l>(env: &mut JNIEnv<'l>, v: i32) -> Result<Object<'l>> {
    Ok(Object::of(lang::Integer::of(env, v)?.into_raw()))
}

pub fn box_long<'l>(env: &mut JNIEnv<'l>, v: i64) -> Result<Object<'l>> {
    Ok(Object::of(lang::Long::of(env, v)?.into_raw()))
}

/// An `Optional` argument.
pub fn optional_arg<'l>(env: &mut JNIEnv<'l>, v: Option<&impl Borrowed>) -> Result<Object<'l>> {
    let r = match v {
        Some(v) => {
            let o = v.borrowed();
            env.call_static_method(
                "java/util/Optional",
                "of",
                "(Ljava/lang/Object;)Ljava/util/Optional;",
                &[JValue::Object(&o)],
            )
        }
        None => {
            env.call_static_method("java/util/Optional", "empty", "()Ljava/util/Optional;", &[])
        }
    };
    Ok(Object::of(crate::checked(r)?.l()?))
}

/// A `byte[]` argument.
pub fn byte_array<'l>(env: &mut JNIEnv<'l>, v: &[u8]) -> Result<Object<'l>> {
    Ok(Object::of(env.byte_array_from_slice(v)?.into()))
}

/// A `String[]` argument.
pub fn string_array<'l, S: AsRef<str>>(env: &mut JNIEnv<'l>, v: &[S]) -> Result<Object<'l>> {
    let null = JObject::null();
    let arr = env.new_object_array(v.len() as i32, "java/lang/String", &null)?;
    for (i, s) in v.iter().enumerate() {
        let js = env.new_string(s.as_ref())?;
        env.set_object_array_element(&arr, i as i32, &js)?;
        let _ = env.delete_local_ref(js);
    }
    Ok(Object::of(arr.into()))
}

/// A reference array argument whose element class is `class` (internal form).
pub fn object_array<'l, T>(
    env: &mut JNIEnv<'l>,
    class: &str,
    v: &[impl IsA<T>],
) -> Result<Object<'l>> {
    let null = JObject::null();
    let arr = env.new_object_array(v.len() as i32, class, &null)?;
    for (i, e) in v.iter().enumerate() {
        env.set_object_array_element(&arr, i as i32, e.borrowed())?;
    }
    Ok(Object::of(arr.into()))
}

/// The `Object` view of a result, for members whose Java type is `Object`.
pub fn object<'a>(o: JObject<'a>, what: &str) -> Result<Object<'a>> {
    wrap(o, what)
}
