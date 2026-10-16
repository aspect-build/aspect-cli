use crate::{Error, Result};
use jni::{JavaVM, sys};
use libloading::{Library, Symbol};
use std::ffi::{CString, c_char, c_void};
use std::path::Path;
use std::ptr;

type CreateJavaVm =
    unsafe extern "system" fn(*mut *mut sys::JavaVM, *mut *mut c_void, *mut c_void) -> sys::jint;

/// A JVM created inside this process from a `libjvm` chosen at runtime.
pub struct Jvm {
    vm: JavaVM,
}

impl Jvm {
    pub fn start(libjvm: &Path, options: &[String]) -> Result<Jvm> {
        let lib = unsafe { Library::new(libjvm) }
            .map_err(|e| Error::msg(format!("load {}: {e}", libjvm.display())))?;
        let create: Symbol<CreateJavaVm> = unsafe { lib.get(b"JNI_CreateJavaVM\0") }
            .map_err(|e| Error::msg(format!("{}: {e}", libjvm.display())))?;
        let strings = options
            .iter()
            .map(|o| CString::new(o.as_str()))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::msg(format!("JVM option: {e}")))?;
        let mut jopts: Vec<sys::JavaVMOption> = strings
            .iter()
            .map(|s| sys::JavaVMOption {
                optionString: s.as_ptr() as *mut c_char,
                extraInfo: ptr::null_mut(),
            })
            .collect();
        let mut args = sys::JavaVMInitArgs {
            version: sys::JNI_VERSION_1_8,
            nOptions: jopts.len() as sys::jint,
            options: jopts.as_mut_ptr(),
            ignoreUnrecognized: sys::JNI_FALSE,
        };
        let mut vm: *mut sys::JavaVM = ptr::null_mut();
        let mut env: *mut c_void = ptr::null_mut();
        let rc = unsafe { create(&mut vm, &mut env, &mut args as *mut _ as *mut c_void) };
        if rc != sys::JNI_OK {
            return Err(Error::msg(format!("JNI_CreateJavaVM returned {rc}")));
        }
        let vm = unsafe { JavaVM::from_raw(vm) }?;
        crate::vm::set(&vm);
        // The registered `JavaVM` outlives this value, so the library must too.
        std::mem::forget(lib);
        Ok(Jvm { vm })
    }

    pub fn vm(&self) -> &JavaVM {
        &self.vm
    }
}
