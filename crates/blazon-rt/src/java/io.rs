//! `java.io`.

use crate::member::{ClassRef, Method};
use crate::{Handle, Result, checked, vm};
use jni::JNIEnv;
use jni::objects::JByteArray;

crate::handle!(
    /// `java.io.OutputStream`
    OutputStream,
    "java/io/OutputStream"
);

crate::handle!(
    /// `java.io.ByteArrayOutputStream`
    ByteArrayOutputStream,
    "java/io/ByteArrayOutputStream"
);

crate::is_a!(ByteArrayOutputStream[] => OutputStream[]);

static BAOS: ClassRef = ClassRef::new("java/io/ByteArrayOutputStream");
static TO_BYTE_ARRAY: Method = Method::new(&BAOS, "toByteArray", "()[B");

impl<'l> ByteArrayOutputStream<'l> {
    pub fn new(env: &mut JNIEnv<'l>) -> Result<Self> {
        let r = env.new_object("java/io/ByteArrayOutputStream", "()V", &[]);
        let o = checked(r)?;
        // SAFETY: the class just constructed.
        Ok(unsafe { ByteArrayOutputStream::from_raw(o) })
    }
}

impl<'a> ByteArrayOutputStream<'a> {
    /// `toByteArray()`
    pub fn to_vec(&self) -> Result<Vec<u8>> {
        let mut env = vm::env()?;
        let arr = TO_BYTE_ARRAY.object(&mut env, &self.obj, &[])?;
        let bytes = env.convert_byte_array(JByteArray::from(arr))?;
        Ok(bytes)
    }
}
