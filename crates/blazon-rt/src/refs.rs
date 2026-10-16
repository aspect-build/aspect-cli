//! Handles, subtyping, global references and exception conversion.

use crate::error::JavaException;
use crate::{Error, Result, vm};
use jni::JNIEnv;
use jni::objects::{GlobalRef, JObject, JString, JThrowable, JValue};
use std::marker::PhantomData;

/// A typed owner of one local reference to a Java object, of the class named by `NAME`.
///
/// Dropping the handle releases the reference. Handles are `!Send`: a local
/// reference belongs to the thread that made it. Store a [`Global`] to keep an
/// object across threads or frames.
pub trait Handle<'a>: Sized + AsRef<JObject<'a>> {
    /// The class, in internal form: `java/util/List`.
    const NAME: &'static str;

    /// Wraps a local reference, taking ownership of it.
    ///
    /// # Safety
    /// `obj` must be an instance of `NAME` (or null) and a local reference of
    /// this thread that nothing else will delete.
    #[doc(hidden)]
    unsafe fn from_raw(obj: JObject<'a>) -> Self;

    /// Gives the local reference up without releasing it.
    #[doc(hidden)]
    fn into_raw(self) -> JObject<'a>;
}

/// `Self` is a `T`: the same class, a subclass, or an implementation of the
/// interface. Generated for every handle over its transitive supertypes;
/// parameters that take a bound class accept `&impl IsA<Class>`.
pub trait IsA<T>: Borrowed {}

/// The object behind a handle, for passing to a JNI call.
pub trait Borrowed {
    #[doc(hidden)]
    fn borrowed(&self) -> JObject<'static>;
}

impl<'a, H: Handle<'a>> Borrowed for H {
    fn borrowed(&self) -> JObject<'static> {
        // SAFETY: a non-owning copy of the pointer, used within the call that
        // borrowed `self`.
        unsafe { JObject::from_raw(self.as_ref().as_raw()) }
    }
}

/// `TryFrom` for handles: the checked downcast of an owned handle.
#[doc(hidden)]
pub fn checked_cast<'a, From: Handle<'a>, To: Handle<'a>>(from: From) -> Result<To> {
    let mut env = vm::env()?;
    let obj = from.as_ref();
    if !obj.is_null() && !env.is_instance_of(obj, To::NAME)? {
        return Err(Error::msg(format!(
            "cannot downcast {} to {}: the object is a {}",
            From::NAME.replace('/', "."),
            To::NAME.replace('/', "."),
            class_name(obj)?
        )));
    }
    // SAFETY: checked above; the one reference moves.
    Ok(unsafe { To::from_raw(from.into_raw()) })
}

/// Explicit conversions between handles.
pub trait HandleExt<'a>: Handle<'a> {
    /// This object as one of its supertypes.
    #[must_use]
    fn upcast<T: Handle<'a> + Static>(&self) -> T
    where
        Self: IsA<<T as Static>::Static>,
    {
        let r = vm::env().and_then(|env| Ok(env.new_local_ref(self.as_ref())?));
        match r {
            // SAFETY: `Self: IsA<T>`, and the new reference is ours.
            Ok(obj) => unsafe { T::from_raw(JObject::from_raw(obj.as_raw())) },
            Err(e) => panic!("NewLocalRef failed in upcast: {e}"),
        }
    }

    /// A null handle.
    fn null() -> Self {
        // SAFETY: null is an instance of every class.
        unsafe { Self::from_raw(JObject::null()) }
    }

    fn is_null(&self) -> bool {
        self.as_ref().is_null()
    }

    /// A global reference to this object.
    fn to_global(&self) -> Result<Global<<Self as Static>::Static>>
    where
        Self: Static,
    {
        let env = vm::env()?;
        Ok(Global::new(env.new_global_ref(self.as_ref())?))
    }

    /// This object as a subtype, when it is one.
    fn downcast<T: Handle<'a>>(&self) -> Result<T> {
        let mut env = vm::env()?;
        let obj = self.as_ref();
        if !obj.is_null() && !env.is_instance_of(obj, T::NAME)? {
            return Err(Error::msg(format!(
                "cannot downcast {} to {}: the object is a {}",
                Self::NAME.replace('/', "."),
                T::NAME.replace('/', "."),
                class_name(obj)?
            )));
        }
        let new = env.new_local_ref(obj)?;
        // SAFETY: checked above; the new reference is ours.
        Ok(unsafe { T::from_raw(JObject::from_raw(new.as_raw())) })
    }

    /// Whether this object is an instance of `T`.
    fn is_a<T: Handle<'a>>(&self) -> Result<bool> {
        let mut env = vm::env()?;
        Ok(env.is_instance_of(self.as_ref(), T::NAME)?)
    }
}

impl<'a, H: Handle<'a>> HandleExt<'a> for H {}

/// A handle's `'static` spelling, the type a `Global` remembers.
pub trait Static {
    type Static;
}

/// Relates a handle's `'static` spelling, as stored in a `Global`, to the same
/// handle for a shorter lifetime. Generated for every handle.
pub trait Rebind<'a> {
    type Out: Handle<'a>;
}

/// A global reference remembered with the handle type it was taken from.
pub struct Global<T>(GlobalRef, PhantomData<fn() -> T>);

impl<T> Global<T> {
    pub fn new(r: GlobalRef) -> Self {
        Global(r, PhantomData)
    }

    /// A handle on the object for this thread, with its own local reference.
    pub fn get<'a>(&self, env: &mut JNIEnv<'a>) -> Result<T::Out>
    where
        T: Rebind<'a>,
    {
        let local = env.new_local_ref(self.0.as_obj())?;
        // SAFETY: the global holds an instance of `T`'s class; the reference is new.
        Ok(unsafe { T::Out::from_raw(local) })
    }

    /// The raw object; valid for as long as this global lives, never deleted here.
    pub fn raw(&self) -> jni::sys::jobject {
        self.0.as_obj().as_raw()
    }

    pub fn as_global_ref(&self) -> &GlobalRef {
        &self.0
    }
}

impl<T> Clone for Global<T> {
    fn clone(&self) -> Self {
        Global(self.0.clone(), PhantomData)
    }
}

impl<T> std::fmt::Debug for Global<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Global<{}>", std::any::type_name::<T>())
    }
}

/// Turns a JNI result into an error if a Java exception is pending, clearing
/// it, so that handles never see each other's exceptions.
pub fn checked<T, E: Into<Error>>(r: std::result::Result<T, E>) -> Result<T> {
    match vm::env() {
        Ok(mut env) => checked_in(&mut env, r),
        Err(e) => Err(e),
    }
}

/// [`checked`] for a caller that has its env, which saves attaching again.
pub(crate) fn checked_in<T, E: Into<Error>>(
    env: &mut JNIEnv,
    r: std::result::Result<T, E>,
) -> Result<T> {
    if !env.exception_check().unwrap_or(true) {
        return r.map_err(Into::into);
    }
    match pending(env)? {
        Some(java) => Err(java),
        None => r.map_err(Into::into),
    }
}

/// The pending exception as an error, cleared.
pub(crate) fn pending(env: &mut JNIEnv) -> Result<Option<Error>> {
    if !env.exception_check()? {
        return Ok(None);
    }
    let throwable = env.exception_occurred()?;
    env.exception_clear()?;
    Ok(Some(Error::java(describe(env, &throwable))))
}

fn describe(env: &mut JNIEnv, t: &JThrowable) -> JavaException {
    let class = class_name(t).unwrap_or_else(|_| "java.lang.Throwable".into());
    let message = (|| -> Result<Option<String>> {
        let m = call_obj(env, t, "getMessage", "()Ljava/lang/String;")?;
        if m.is_null() {
            return Ok(None);
        }
        Ok(Some(env.get_string(&JString::from(m))?.into()))
    })()
    .unwrap_or(None);
    let trace = stack_trace(env, t).unwrap_or_else(|_| format!("{class}: <unprintable>"));
    JavaException {
        class,
        message,
        trace,
    }
}

fn call_obj<'l>(env: &mut JNIEnv<'l>, obj: &JObject, name: &str, sig: &str) -> Result<JObject<'l>> {
    let r = env.call_method(obj, name, sig, &[]).and_then(|v| v.l());
    if env.exception_check()? {
        env.exception_clear()?;
        return Err(Error::msg(format!("{name} threw")));
    }
    Ok(r?)
}

fn stack_trace(env: &mut JNIEnv, t: &JThrowable) -> Result<String> {
    let sw = env.new_object("java/io/StringWriter", "()V", &[])?;
    let pw = env.new_object(
        "java/io/PrintWriter",
        "(Ljava/io/Writer;)V",
        &[JValue::Object(&sw)],
    )?;
    let printed = env.call_method(
        t,
        "printStackTrace",
        "(Ljava/io/PrintWriter;)V",
        &[JValue::Object(&pw)],
    );
    if env.exception_check()? {
        env.exception_clear()?;
        return Err(Error::msg("printStackTrace threw"));
    }
    printed?;
    let s = call_obj(env, &sw, "toString", "()Ljava/lang/String;")?;
    Ok(env.get_string(&JString::from(s))?.into())
}

/// `obj.getClass().getName()`, or `null`.
pub fn class_name(obj: &JObject) -> Result<String> {
    if obj.is_null() {
        return Ok("null".into());
    }
    let mut env = vm::env()?;
    let cls = env.get_object_class(obj)?;
    let name = call_obj(&mut env, &cls, "getName", "()Ljava/lang/String;")?;
    Ok(env.get_string(&JString::from(name))?.into())
}
