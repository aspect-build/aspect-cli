//! `java.time`.

use crate::member::{ClassRef, Method, StaticMethod};
use crate::{Handle, Result, vm};
use jni::JNIEnv;
use jni::objects::JValue;
use std::time::Duration as StdDuration;

crate::handle!(
    /// `java.time.Duration`
    Duration,
    "java/time/Duration"
);

static DURATION: ClassRef = ClassRef::new("java/time/Duration");
static OF_SECONDS: StaticMethod =
    StaticMethod::new(&DURATION, "ofSeconds", "(JJ)Ljava/time/Duration;");
static GET_SECONDS: Method = Method::new(&DURATION, "getSeconds", "()J");
static GET_NANO: Method = Method::new(&DURATION, "getNano", "()I");

impl<'l> Duration<'l> {
    /// The `Duration` of a Rust one.
    pub fn new(env: &mut JNIEnv<'l>, d: StdDuration) -> Result<Self> {
        let o = OF_SECONDS.object(
            env,
            &[
                JValue::Long(d.as_secs() as i64),
                JValue::Long(i64::from(d.subsec_nanos())),
            ],
        )?;
        // SAFETY: `ofSeconds` returns a `Duration`.
        Ok(unsafe { Duration::from_raw(o) })
    }
}

impl<'a> Duration<'a> {
    /// As a Rust duration; an error for a negative one.
    pub fn to_std(&self) -> Result<StdDuration> {
        let mut env = vm::env()?;
        let secs = GET_SECONDS.long(&mut env, &self.obj, &[])?;
        let nanos = GET_NANO.int(&mut env, &self.obj, &[])?;
        match (u64::try_from(secs), u32::try_from(nanos)) {
            (Ok(s), Ok(n)) => Ok(StdDuration::new(s, n)),
            _ => Err(crate::Error::msg(format!("negative duration: {self}"))),
        }
    }
}
