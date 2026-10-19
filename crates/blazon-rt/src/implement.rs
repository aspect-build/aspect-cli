//! Implementing Java interfaces and abstract classes from Rust.
//!
//! `blazon` writes, per implemented type, a small class file whose methods
//! are all `native`, and a Rust trait with one method per abstract method.
//! This module defines that class in the JVM once per process, creates
//! instances that own a Rust value, and is what the generated native
//! methods call: [`Implementation::dispatch`] finds the Rust value behind the
//! Java object, runs the call, and turns an `Err` into a thrown
//! `RuntimeException` and a panic into a thrown `IllegalStateException`, so
//! that neither unwinds into the JVM.
//!
//! The Rust value lives in a `Box` whose address is the object's `long handle`
//! field. The class registers each instance with a `Cleaner`
//! (`blazon/impl/Free`); when the Java object is unreachable the cleaner thread
//! drops the box. A native method running on a thread has the object in its
//! frame, so the value outlives every call in flight. The value is dropped on
//! the cleaner thread, hence `Send + Sync`, and calls arrive on whatever thread
//! Bazel uses.

use crate::{Borrowed, Error, Handle, IsA, Result, checked, conv, vm};
use jni::objects::{GlobalRef, JFieldID, JMethodID, JObject, JValue};
use jni::signature::{Primitive, ReturnType};
use jni::{JNIEnv, NativeMethod, sys};
use std::any::{Any, TypeId};
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, OnceLock};

/// The helper class every implementation registers its instances with.
pub const FREE_CLASS: &str = "blazon/impl/Free";

/// A Rust value of any `Send + Sync` type, behind a thin pointer, so that the
/// one `long` of a Java object can own it.
pub struct Owned {
    ptr: *mut (),
    id: TypeId,
    drop: unsafe fn(*mut ()),
}

// SAFETY: `new` only takes `Send + Sync` values.
unsafe impl Send for Owned {}
unsafe impl Sync for Owned {}

unsafe fn drop_box<T>(ptr: *mut ()) {
    // SAFETY: `ptr` came from `Box::into_raw(Box<T>)` in `Owned::new`.
    drop(unsafe { Box::from_raw(ptr.cast::<T>()) });
}

impl Owned {
    pub fn new<T: Send + Sync + 'static>(value: T) -> Owned {
        Owned {
            ptr: Box::into_raw(Box::new(value)).cast(),
            id: TypeId::of::<T>(),
            drop: drop_box::<T>,
        }
    }

    /// The value, when it is a `T`.
    pub fn get<T: 'static>(&self) -> Option<&T> {
        // SAFETY: the type id says `ptr` is a `Box<T>` that lives as long as `self`.
        (self.id == TypeId::of::<T>()).then(|| unsafe { &*self.ptr.cast::<T>() })
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: `ptr` is the `Box<T>` `drop` was made for, dropped once.
        unsafe { (self.drop)(self.ptr) }
    }
}

/// One native method of a generated class.
pub struct Native {
    name: &'static str,
    descriptor: &'static str,
    ptr: *const c_void,
}

// SAFETY: a function pointer.
unsafe impl Sync for Native {}

impl Native {
    pub const fn new(name: &'static str, descriptor: &'static str, ptr: *const c_void) -> Native {
        Native {
            name,
            descriptor,
            ptr,
        }
    }
}

/// What `blazon` generated for one implemented type.
pub struct Def {
    /// The generated class, in internal form.
    pub class: &'static str,
    /// Its class file.
    pub bytes: &'static [u8],
    /// The class file of [`FREE_CLASS`].
    pub free: &'static [u8],
    /// A class of the jar the type is in, whose class loader defines the new class,
    /// so that it sees the interface (internal form).
    pub anchor: &'static str,
    pub natives: &'static [Native],
}

struct Defined {
    class: GlobalRef,
    ctor: JMethodID,
    field: JFieldID,
}

/// A generated class, defined in the JVM on first use.
pub struct Implementation {
    def: &'static Def,
    defined: OnceLock<Defined>,
    lock: Mutex<()>,
}

/// The class loaders `FREE_CLASS` has been defined in.
static FREED: Mutex<Vec<GlobalRef>> = Mutex::new(Vec::new());

fn loader_of<'l>(env: &mut JNIEnv<'l>, anchor: &str) -> Result<JObject<'l>> {
    let class = checked(env.find_class(anchor))?;
    let r = env.call_method(&class, "getClassLoader", "()Ljava/lang/ClassLoader;", &[]);
    let loader = checked(r)?.l()?;
    if !loader.is_null() {
        return Ok(loader);
    }
    let r = env.call_static_method(
        "java/lang/ClassLoader",
        "getSystemClassLoader",
        "()Ljava/lang/ClassLoader;",
        &[],
    );
    Ok(checked(r)?.l()?)
}

unsafe extern "system" fn free_drop(
    _env: *mut sys::JNIEnv,
    _class: sys::jclass,
    handle: sys::jlong,
) {
    if handle != 0 {
        // SAFETY: the handle is the `Box<Owned>` made by `instantiate`, registered once.
        let owned = unsafe { Box::from_raw(handle as *mut Owned) };
        let _ = catch_unwind(AssertUnwindSafe(move || drop(owned)));
    }
}

fn ensure_free(env: &mut JNIEnv, loader: &JObject, bytes: &[u8]) -> Result<()> {
    let mut freed = FREED.lock().unwrap_or_else(|e| e.into_inner());
    for known in freed.iter() {
        if env.is_same_object(known.as_obj(), loader)? {
            return Ok(());
        }
    }
    let class = checked(env.define_class(FREE_CLASS, loader, bytes))?;
    let natives = [NativeMethod {
        name: "drop".into(),
        sig: "(J)V".into(),
        fn_ptr: free_drop as *mut c_void,
    }];
    checked(env.register_native_methods(&class, &natives))?;
    freed.push(env.new_global_ref(loader)?);
    Ok(())
}

impl Implementation {
    pub const fn new(def: &'static Def) -> Implementation {
        Implementation {
            def,
            defined: OnceLock::new(),
            lock: Mutex::new(()),
        }
    }

    fn defined(&self, env: &mut JNIEnv) -> Result<&Defined> {
        if let Some(d) = self.defined.get() {
            return Ok(d);
        }
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(d) = self.defined.get() {
            return Ok(d);
        }
        let def = self.def;
        let describe = |e: Error| e.context(format!("define {}", def.class.replace('/', ".")));
        let loader = loader_of(env, def.anchor).map_err(describe)?;
        ensure_free(env, &loader, def.free).map_err(describe)?;
        let class = checked(env.define_class(def.class, &loader, def.bytes)).map_err(describe)?;
        let natives: Vec<NativeMethod> = def
            .natives
            .iter()
            .map(|n| NativeMethod {
                name: n.name.into(),
                sig: n.descriptor.into(),
                fn_ptr: n.ptr.cast_mut(),
            })
            .collect();
        checked(env.register_native_methods(&class, &natives)).map_err(describe)?;
        let ctor = checked(env.get_method_id(&class, "<init>", "(J)V")).map_err(describe)?;
        let field = checked(env.get_field_id(&class, "handle", "J")).map_err(describe)?;
        let class = env.new_global_ref(&class)?;
        let _ = self.defined.set(Defined { class, ctor, field });
        self.defined
            .get()
            .ok_or_else(|| Error::msg("the implementation class was not defined"))
    }

    /// A new instance of the class that owns `value`. The value is dropped when
    /// the object is collected.
    pub fn instantiate<'l, T: Send + Sync + 'static>(
        &self,
        env: &mut JNIEnv<'l>,
        value: T,
    ) -> Result<JObject<'l>> {
        let defined = self.defined(env)?;
        let raw = Box::into_raw(Box::new(Owned::new(value)));
        // SAFETY: `ctor` is `(J)V` of the class; the class holds a `Class`.
        let class = unsafe { jni::objects::JClass::from_raw(defined.class.as_obj().as_raw()) };
        let made = unsafe {
            env.new_object_unchecked(&class, defined.ctor, &[JValue::Long(raw as i64).as_jni()])
        };
        match checked(made) {
            Ok(o) => Ok(o),
            Err(e) => {
                // SAFETY: the constructor failed before registering the box with the cleaner.
                drop(unsafe { Box::from_raw(raw) });
                Err(e)
            }
        }
    }

    /// Runs `f` on the Rust value behind `this` for a native method.
    ///
    /// An `Err` is thrown as a `RuntimeException` naming `what`, with the error
    /// chain in its cause; a panic is thrown as an `IllegalStateException`; the
    /// result is then [`Zero::ZERO`], which the JVM ignores.
    ///
    /// # Safety
    /// `env` and `this` are the arguments of a native method of this class, and
    /// `T` is the type the object was instantiated with.
    pub unsafe fn dispatch<T: 'static, R: Zero>(
        &self,
        env: *mut sys::JNIEnv,
        this: sys::jobject,
        what: &'static str,
        f: impl FnOnce(&mut JNIEnv<'static>, &T) -> Result<R>,
    ) -> R {
        // SAFETY: by the caller.
        let Ok(mut env) = (unsafe { JNIEnv::from_raw(env) }) else {
            return R::ZERO;
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<R> {
            let defined = self
                .defined
                .get()
                .ok_or_else(|| Error::msg("the implementation class is not defined"))?;
            // SAFETY: by the caller.
            let this = unsafe { JObject::from_raw(this) };
            let handle = env
                .get_field_unchecked(&this, defined.field, ReturnType::Primitive(Primitive::Long))
                .and_then(|v| v.j())?;
            if handle == 0 {
                return Err(Error::msg("the object has no Rust value"));
            }
            // SAFETY: the field holds the box made by `instantiate`, alive while `this` is.
            let owned = unsafe { &*(handle as *const Owned) };
            let value = owned.get::<T>().ok_or_else(|| {
                Error::msg("the object's Rust value is not of the type the trait was made for")
            })?;
            f(&mut env, value)
        }));
        match outcome {
            Ok(Ok(v)) => v,
            Ok(Err(e)) if e.throwable().is_some() && rethrow(&mut env, &e) => R::ZERO,
            Ok(Err(e)) => {
                throw(
                    &mut env,
                    "java/lang/RuntimeException",
                    &format!("{what}: {e}"),
                    &causes(&e),
                );
                R::ZERO
            }
            Err(payload) => {
                let text = panic_text(&*payload);
                throw(
                    &mut env,
                    "java/lang/IllegalStateException",
                    &format!("Rust panic in {what}: {text}"),
                    &[],
                );
                R::ZERO
            }
        }
    }
}

/// The error chain below `e`, one entry per source.
pub fn causes(e: &(dyn std::error::Error + 'static)) -> Vec<String> {
    let mut out = Vec::new();
    let mut source = e.source();
    while let Some(s) = source {
        out.push(s.to_string());
        source = s.source();
    }
    out
}

/// What a panic carried, when it was a string.
pub fn panic_text(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "a panic with a payload that is not a string".to_string()
    }
}

/// Throws the exception object `e` came from, unchanged; false when that fails.
fn rethrow(env: &mut JNIEnv, e: &Error) -> bool {
    let Some(g) = e.throwable() else {
        return false;
    };
    if env.exception_check().unwrap_or(true) {
        return true;
    }
    // SAFETY: the global holds a `Throwable`.
    let t = unsafe { jni::objects::JThrowable::from_raw(g.as_obj().as_raw()) };
    env.throw(&t).is_ok()
}

fn throw(env: &mut JNIEnv, class: &str, message: &str, causes: &[String]) {
    if env.exception_check().unwrap_or(true) {
        return;
    }
    let made = (|| -> jni::errors::Result<()> {
        let text = env.new_string(message)?;
        let exception = if causes.is_empty() {
            env.new_object(class, "(Ljava/lang/String;)V", &[JValue::Object(&text)])?
        } else {
            let cause_text =
                env.new_string(format!("caused by: {}", causes.join("\ncaused by: ")))?;
            let cause = env.new_object(
                "java/lang/RuntimeException",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&cause_text)],
            )?;
            env.new_object(
                class,
                "(Ljava/lang/String;Ljava/lang/Throwable;)V",
                &[JValue::Object(&text), JValue::Object(&cause)],
            )?
        };
        env.throw(jni::objects::JThrowable::from(exception))
    })();
    if made.is_err() {
        let _ = env.exception_clear();
        let _ = env.throw_new(class, message);
    }
}

/// The value a native method returns when it threw.
pub trait Zero {
    const ZERO: Self;
}

macro_rules! zero {
    ($($t:ty => $v:expr),*) => {$(impl Zero for $t { const ZERO: Self = $v; })*};
}

zero!(() => (), sys::jboolean => 0, sys::jbyte => 0, sys::jchar => 0, sys::jshort => 0,
    sys::jint => 0, sys::jlong => 0, sys::jfloat => 0.0, sys::jdouble => 0.0,
    sys::jobject => std::ptr::null_mut());

/// An argument of a native method, as a new local reference this thread owns.
///
/// # Safety
/// `raw` is an argument of the native method being run, or null.
pub unsafe fn local(env: &mut JNIEnv<'static>, raw: sys::jobject) -> Result<JObject<'static>> {
    // SAFETY: by the caller.
    Ok(env.new_local_ref(unsafe { JObject::from_raw(raw) })?)
}

/// A handle as the result of a native method, which takes over its reference.
pub fn object<'a>(h: impl Handle<'a>) -> sys::jobject {
    h.into_raw().as_raw()
}

pub fn object_opt<'a, H: Handle<'a>>(h: Option<H>) -> sys::jobject {
    h.map_or(std::ptr::null_mut(), object)
}

pub fn string(env: &mut JNIEnv<'static>, s: &str) -> Result<sys::jobject> {
    Ok(conv::jstring(env, s)?.into_raw().as_raw())
}

pub fn string_opt(env: &mut JNIEnv<'static>, s: Option<&str>) -> Result<sys::jobject> {
    s.map_or(Ok(std::ptr::null_mut()), |s| string(env, s))
}

macro_rules! boxed {
    ($name:ident, $opt:ident, $t:ty, $f:ident) => {
        pub fn $name(env: &mut JNIEnv<'static>, v: $t) -> Result<sys::jobject> {
            Ok(conv::$f(env, v)?.into_raw().as_raw())
        }

        pub fn $opt(env: &mut JNIEnv<'static>, v: Option<$t>) -> Result<sys::jobject> {
            v.map_or(Ok(std::ptr::null_mut()), |v| $name(env, v))
        }
    };
}

boxed!(boxed_bool, boxed_bool_opt, bool, box_bool);
boxed!(boxed_int, boxed_int_opt, i32, box_int);
boxed!(boxed_long, boxed_long_opt, i64, box_long);

pub fn optional(env: &mut JNIEnv<'static>, v: Option<&impl Borrowed>) -> Result<sys::jobject> {
    Ok(conv::optional_arg(env, v)?.into_raw().as_raw())
}

pub fn bytes(env: &mut JNIEnv<'static>, v: &[u8]) -> Result<sys::jobject> {
    Ok(conv::byte_array(env, v)?.into_raw().as_raw())
}

pub fn bytes_opt(env: &mut JNIEnv<'static>, v: Option<&[u8]>) -> Result<sys::jobject> {
    v.map_or(Ok(std::ptr::null_mut()), |v| bytes(env, v))
}

pub fn strings_opt<S: AsRef<str>>(
    env: &mut JNIEnv<'static>,
    v: Option<&[S]>,
) -> Result<sys::jobject> {
    v.map_or(Ok(std::ptr::null_mut()), |v| strings(env, v))
}

pub fn objects_opt<T, H: IsA<T>>(
    env: &mut JNIEnv<'static>,
    class: &str,
    v: Option<&[H]>,
) -> Result<sys::jobject> {
    v.map_or(Ok(std::ptr::null_mut()), |v| objects::<T, H>(env, class, v))
}

pub fn strings<S: AsRef<str>>(env: &mut JNIEnv<'static>, v: &[S]) -> Result<sys::jobject> {
    Ok(conv::string_array(env, v)?.into_raw().as_raw())
}

pub fn objects<T, H: IsA<T>>(
    env: &mut JNIEnv<'static>,
    class: &str,
    v: &[H],
) -> Result<sys::jobject> {
    Ok(conv::object_array::<T>(env, class, v)?.into_raw().as_raw())
}

/// The calling thread's JNI environment, for code that is handed none.
pub fn env() -> Result<JNIEnv<'static>> {
    vm::env()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Flag(Arc<AtomicBool>);

    impl Drop for Flag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn owned_gives_back_its_type_only_and_drops_once() {
        let dropped = Arc::new(AtomicBool::new(false));
        let owned = Owned::new(Box::new(Flag(dropped.clone())) as Box<dyn Send + Sync>);
        assert!(owned.get::<String>().is_none());
        assert!(owned.get::<Box<dyn Send + Sync>>().is_some());
        assert!(!dropped.load(Ordering::SeqCst));
        drop(owned);
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn error_chains_and_panics_are_text() {
        let e = Error::msg("plain");
        assert!(causes(&e).is_empty());
        let boxed: Box<dyn Any + Send> = Box::new("boom");
        assert_eq!(panic_text(&*boxed), "boom");
        let boxed: Box<dyn Any + Send> = Box::new(String::from("owned"));
        assert_eq!(panic_text(&*boxed), "owned");
        let boxed: Box<dyn Any + Send> = Box::new(7u8);
        assert!(panic_text(&*boxed).contains("not a string"));
    }

    #[test]
    fn zero_is_zero() {
        assert_eq!(<sys::jint as Zero>::ZERO, 0);
        assert!(<sys::jobject as Zero>::ZERO.is_null());
    }
}
