//! The process's one `JavaVM`. Code that has no env to hand reaches JNI
//! through `env()`: the calling thread is attached the first time it asks and
//! stays attached.

use crate::{Error, Result};
use jni::objects::JObject;
use jni::{JNIEnv, JavaVM, sys};
use std::sync::OnceLock;

struct Registered(*mut sys::JavaVM);

// SAFETY: a `JavaVM` pointer is valid on every thread for the life of the process.
unsafe impl Send for Registered {}
unsafe impl Sync for Registered {}

static VM: OnceLock<Registered> = OnceLock::new();

/// Registers the process's JVM. The first call wins; a process has at most one.
/// `Jvm::start` does this itself.
pub fn set(vm: &JavaVM) {
    let _ = VM.set(Registered(vm.get_java_vm_pointer()));
}

/// The registered JVM.
pub fn get() -> Result<JavaVM> {
    let r = VM
        .get()
        .ok_or_else(|| Error::msg("no JVM is registered; start one with blazon_rt::Jvm::start"))?;
    // SAFETY: the pointer was taken from a live `JavaVM` by `set`.
    Ok(unsafe { JavaVM::from_raw(r.0) }?)
}

/// A `JNIEnv` for the calling thread, attaching it (permanently) if needed.
pub fn env() -> Result<JNIEnv<'static>> {
    let vm = get()?;
    let raw = match vm.get_env() {
        Ok(env) => env.get_raw(),
        Err(_) => vm.attach_current_thread_permanently()?.get_raw(),
    };
    // SAFETY: `raw` is the current thread's env.
    Ok(unsafe { JNIEnv::from_raw(raw) }?)
}

/// Runs `f` inside a local reference frame that has room for `capacity`
/// references; everything `f` makes is released when it returns, so handles
/// made inside must not escape. Handles release their own reference when
/// dropped, so a frame is only needed to bound what `f` holds at once.
pub fn scope<R>(capacity: i32, f: impl FnOnce() -> Result<R>) -> Result<R> {
    let mut env = env()?;
    let mut out = None;
    env.with_local_frame(capacity, |_| {
        out = Some(f());
        Ok::<_, jni::errors::Error>(JObject::null())
    })?;
    out.unwrap_or_else(|| Err(Error::msg("local frame did not run")))
}
