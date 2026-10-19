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

crate::handle!(
    /// `java.time.Instant`
    Instant,
    "java/time/Instant"
);

static INSTANT: ClassRef = ClassRef::new("java/time/Instant");
static NOW: StaticMethod = StaticMethod::new(&INSTANT, "now", "()Ljava/time/Instant;");
static OF_EPOCH_MILLI: StaticMethod =
    StaticMethod::new(&INSTANT, "ofEpochMilli", "(J)Ljava/time/Instant;");
static OF_EPOCH_SECOND: StaticMethod =
    StaticMethod::new(&INSTANT, "ofEpochSecond", "(J)Ljava/time/Instant;");
static TO_EPOCH_MILLI: Method = Method::new(&INSTANT, "toEpochMilli", "()J");

impl<'l> Instant<'l> {
    /// `Instant.now()`
    pub fn now(env: &mut JNIEnv<'l>) -> Result<Self> {
        let o = NOW.object(env, &[])?;
        // SAFETY: `now` returns an `Instant`.
        Ok(unsafe { Instant::from_raw(o) })
    }

    /// `Instant.ofEpochMilli(ms)`
    pub fn of_epoch_milli(env: &mut JNIEnv<'l>, ms: i64) -> Result<Self> {
        let o = OF_EPOCH_MILLI.object(env, &[JValue::Long(ms)])?;
        // SAFETY: `ofEpochMilli` returns an `Instant`.
        Ok(unsafe { Instant::from_raw(o) })
    }

    /// `Instant.ofEpochSecond(s)`
    pub fn of_epoch_second(env: &mut JNIEnv<'l>, s: i64) -> Result<Self> {
        let o = OF_EPOCH_SECOND.object(env, &[JValue::Long(s)])?;
        // SAFETY: `ofEpochSecond` returns an `Instant`.
        Ok(unsafe { Instant::from_raw(o) })
    }
}

impl<'a> Instant<'a> {
    /// Milliseconds since the epoch.
    pub fn to_epoch_milli(&self) -> Result<i64> {
        let mut env = vm::env()?;
        TO_EPOCH_MILLI.long(&mut env, &self.obj, &[])
    }
}

impl<'a, 'b> PartialOrd<Instant<'b>> for Instant<'a> {
    fn partial_cmp(&self, other: &Instant<'b>) -> Option<std::cmp::Ordering> {
        Some(crate::java::lang::compare_to(&self.obj, &other.obj))
    }
}

impl<'a> Ord for Instant<'a> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        crate::java::lang::compare_to(&self.obj, &other.obj)
    }
}
