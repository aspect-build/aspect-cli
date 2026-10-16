//! `java.math`.

use crate::member::{ClassRef, Method};
use crate::{Handle, Result, checked, vm};
use jni::JNIEnv;
use jni::objects::JValue;

crate::handle!(
    /// `java.math.BigInteger`
    BigInteger,
    "java/math/BigInteger"
);

static BIG_INTEGER: ClassRef = ClassRef::new("java/math/BigInteger");
static LONG_VALUE_EXACT: Method = Method::new(&BIG_INTEGER, "longValueExact", "()J");

impl<'l> BigInteger<'l> {
    /// Parses a decimal integer, `new BigInteger(text)`.
    pub fn parse(env: &mut JNIEnv<'l>, text: &str) -> Result<Self> {
        let s = env.new_string(text)?;
        let r = env.new_object(
            "java/math/BigInteger",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&s)],
        );
        let o = checked(r)?;
        // SAFETY: the class just constructed.
        Ok(unsafe { BigInteger::from_raw(o) })
    }

    pub fn of(env: &mut JNIEnv<'l>, v: i64) -> Result<Self> {
        let r = env.call_static_method(
            "java/math/BigInteger",
            "valueOf",
            "(J)Ljava/math/BigInteger;",
            &[JValue::Long(v)],
        );
        let o = checked(r)?.l()?;
        // SAFETY: `valueOf` returns a `BigInteger`.
        Ok(unsafe { BigInteger::from_raw(o) })
    }
}

impl<'a> BigInteger<'a> {
    /// `longValueExact()`: an error when the value does not fit.
    pub fn to_i64(&self) -> Result<i64> {
        let mut env = vm::env()?;
        LONG_VALUE_EXACT.long(&mut env, &self.obj, &[])
    }
}
