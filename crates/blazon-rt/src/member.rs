//! Cached class references and member IDs, which is what generated calls go
//! through: a class is looked up once, a method or field is resolved once, and
//! each call after that is the JNI `*Unchecked` call plus an exception check.
//! Members that are not public are reached through `java.lang.reflect`, with
//! the reflected `Method` or `Field` cached the same way.

use crate::refs::checked_in;
use crate::{Result, checked};
use jni::JNIEnv;
use jni::objects::{
    GlobalRef, JClass, JFieldID, JMethodID, JObject, JStaticFieldID, JStaticMethodID, JValue,
    JValueOwned,
};
use jni::signature::{Primitive, ReturnType};
use jni::sys::jvalue;
use std::sync::OnceLock;

/// A class looked up once, in internal form (`java/util/List`).
pub struct ClassRef {
    name: &'static str,
    cell: OnceLock<GlobalRef>,
}

impl ClassRef {
    pub const fn new(name: &'static str) -> Self {
        ClassRef {
            name,
            cell: OnceLock::new(),
        }
    }

    /// The class object, valid for the life of the process.
    pub fn get(&self, env: &mut JNIEnv) -> Result<JClass<'static>> {
        if let Some(g) = self.cell.get() {
            // SAFETY: `g` holds a `Class` and lives in a static.
            return Ok(unsafe { JClass::from_raw(g.as_obj().as_raw()) });
        }
        let found = env.find_class(self.name);
        let class = checked(found)?;
        let global = env.new_global_ref(&class)?;
        let global = self.cell.get_or_init(|| global);
        // SAFETY: as above.
        Ok(unsafe { JClass::from_raw(global.as_obj().as_raw()) })
    }
}

impl ClassRef {
    /// Whether `obj` is an instance of the class.
    pub fn is_instance(&self, env: &mut JNIEnv, obj: &JObject) -> Result<bool> {
        let class = self.get(env)?;
        Ok(env.is_instance_of(obj, &class)?)
    }
}

fn raw_args<R>(args: &[JValue], f: impl FnOnce(&[jvalue]) -> R) -> R {
    const INLINE: usize = 8;
    if args.len() <= INLINE {
        let mut buf = [jvalue {
            l: std::ptr::null_mut(),
        }; INLINE];
        for (slot, a) in buf.iter_mut().zip(args) {
            *slot = a.as_jni();
        }
        f(&buf[..args.len()])
    } else {
        let v: Vec<jvalue> = args.iter().map(JValue::as_jni).collect();
        f(&v)
    }
}

/// An instance method.
pub struct Method {
    class: &'static ClassRef,
    name: &'static str,
    sig: &'static str,
    id: OnceLock<JMethodID>,
}

impl Method {
    pub const fn new(class: &'static ClassRef, name: &'static str, sig: &'static str) -> Self {
        Method {
            class,
            name,
            sig,
            id: OnceLock::new(),
        }
    }

    fn id(&self, env: &mut JNIEnv) -> Result<JMethodID> {
        if let Some(id) = self.id.get() {
            return Ok(*id);
        }
        let class = self.class.get(env)?;
        let found = env.get_method_id(&class, self.name, self.sig);
        let id = checked(found)?;
        Ok(*self.id.get_or_init(|| id))
    }

    fn invoke<'l>(
        &self,
        env: &mut JNIEnv<'l>,
        obj: &JObject,
        ret: ReturnType,
        args: &[JValue],
    ) -> Result<JValueOwned<'l>> {
        let id = self.id(env)?;
        // SAFETY: `id` was resolved with this method's own signature, which the
        // generator derived from the same class file as `ret` and `args`.
        let r = raw_args(args, |a| unsafe {
            env.call_method_unchecked(obj, id, ret, a)
        });
        checked_in(env, r)
    }

    pub fn void(&self, env: &mut JNIEnv, obj: &JObject, args: &[JValue]) -> Result<()> {
        Ok(self
            .invoke(env, obj, ReturnType::Primitive(Primitive::Void), args)?
            .v()?)
    }

    pub fn object<'l>(
        &self,
        env: &mut JNIEnv<'l>,
        obj: &JObject,
        args: &[JValue],
    ) -> Result<JObject<'l>> {
        Ok(self.invoke(env, obj, ReturnType::Object, args)?.l()?)
    }

    pub fn boolean(&self, env: &mut JNIEnv, obj: &JObject, args: &[JValue]) -> Result<bool> {
        Ok(self
            .invoke(env, obj, ReturnType::Primitive(Primitive::Boolean), args)?
            .z()?)
    }

    pub fn int(&self, env: &mut JNIEnv, obj: &JObject, args: &[JValue]) -> Result<i32> {
        Ok(self
            .invoke(env, obj, ReturnType::Primitive(Primitive::Int), args)?
            .i()?)
    }

    pub fn long(&self, env: &mut JNIEnv, obj: &JObject, args: &[JValue]) -> Result<i64> {
        Ok(self
            .invoke(env, obj, ReturnType::Primitive(Primitive::Long), args)?
            .j()?)
    }

    pub fn float(&self, env: &mut JNIEnv, obj: &JObject, args: &[JValue]) -> Result<f32> {
        Ok(self
            .invoke(env, obj, ReturnType::Primitive(Primitive::Float), args)?
            .f()?)
    }

    pub fn double(&self, env: &mut JNIEnv, obj: &JObject, args: &[JValue]) -> Result<f64> {
        Ok(self
            .invoke(env, obj, ReturnType::Primitive(Primitive::Double), args)?
            .d()?)
    }

    pub fn byte(&self, env: &mut JNIEnv, obj: &JObject, args: &[JValue]) -> Result<i8> {
        Ok(self
            .invoke(env, obj, ReturnType::Primitive(Primitive::Byte), args)?
            .b()?)
    }

    pub fn short(&self, env: &mut JNIEnv, obj: &JObject, args: &[JValue]) -> Result<i16> {
        Ok(self
            .invoke(env, obj, ReturnType::Primitive(Primitive::Short), args)?
            .s()?)
    }

    pub fn char(&self, env: &mut JNIEnv, obj: &JObject, args: &[JValue]) -> Result<u16> {
        Ok(self
            .invoke(env, obj, ReturnType::Primitive(Primitive::Char), args)?
            .c()?)
    }
}

/// A static method.
pub struct StaticMethod {
    class: &'static ClassRef,
    name: &'static str,
    sig: &'static str,
    id: OnceLock<JStaticMethodID>,
}

impl StaticMethod {
    pub const fn new(class: &'static ClassRef, name: &'static str, sig: &'static str) -> Self {
        StaticMethod {
            class,
            name,
            sig,
            id: OnceLock::new(),
        }
    }

    fn id(&self, env: &mut JNIEnv) -> Result<JStaticMethodID> {
        if let Some(id) = self.id.get() {
            return Ok(*id);
        }
        let class = self.class.get(env)?;
        let found = env.get_static_method_id(&class, self.name, self.sig);
        let id = checked(found)?;
        Ok(*self.id.get_or_init(|| id))
    }

    fn invoke<'l>(
        &self,
        env: &mut JNIEnv<'l>,
        ret: ReturnType,
        args: &[JValue],
    ) -> Result<JValueOwned<'l>> {
        let id = self.id(env)?;
        let class = self.class.get(env)?;
        // SAFETY: as for `Method::invoke`.
        let r = raw_args(args, |a| unsafe {
            env.call_static_method_unchecked(&class, id, ret, a)
        });
        checked_in(env, r)
    }

    pub fn void(&self, env: &mut JNIEnv, args: &[JValue]) -> Result<()> {
        Ok(self
            .invoke(env, ReturnType::Primitive(Primitive::Void), args)?
            .v()?)
    }

    pub fn object<'l>(&self, env: &mut JNIEnv<'l>, args: &[JValue]) -> Result<JObject<'l>> {
        Ok(self.invoke(env, ReturnType::Object, args)?.l()?)
    }

    pub fn boolean(&self, env: &mut JNIEnv, args: &[JValue]) -> Result<bool> {
        Ok(self
            .invoke(env, ReturnType::Primitive(Primitive::Boolean), args)?
            .z()?)
    }

    pub fn int(&self, env: &mut JNIEnv, args: &[JValue]) -> Result<i32> {
        Ok(self
            .invoke(env, ReturnType::Primitive(Primitive::Int), args)?
            .i()?)
    }

    pub fn long(&self, env: &mut JNIEnv, args: &[JValue]) -> Result<i64> {
        Ok(self
            .invoke(env, ReturnType::Primitive(Primitive::Long), args)?
            .j()?)
    }

    pub fn float(&self, env: &mut JNIEnv, args: &[JValue]) -> Result<f32> {
        Ok(self
            .invoke(env, ReturnType::Primitive(Primitive::Float), args)?
            .f()?)
    }

    pub fn double(&self, env: &mut JNIEnv, args: &[JValue]) -> Result<f64> {
        Ok(self
            .invoke(env, ReturnType::Primitive(Primitive::Double), args)?
            .d()?)
    }

    pub fn byte(&self, env: &mut JNIEnv, args: &[JValue]) -> Result<i8> {
        Ok(self
            .invoke(env, ReturnType::Primitive(Primitive::Byte), args)?
            .b()?)
    }

    pub fn short(&self, env: &mut JNIEnv, args: &[JValue]) -> Result<i16> {
        Ok(self
            .invoke(env, ReturnType::Primitive(Primitive::Short), args)?
            .s()?)
    }

    pub fn char(&self, env: &mut JNIEnv, args: &[JValue]) -> Result<u16> {
        Ok(self
            .invoke(env, ReturnType::Primitive(Primitive::Char), args)?
            .c()?)
    }
}

/// A constructor.
pub struct Constructor {
    class: &'static ClassRef,
    sig: &'static str,
    id: OnceLock<JMethodID>,
}

impl Constructor {
    pub const fn new(class: &'static ClassRef, sig: &'static str) -> Self {
        Constructor {
            class,
            sig,
            id: OnceLock::new(),
        }
    }

    pub fn new_object<'l>(&self, env: &mut JNIEnv<'l>, args: &[JValue]) -> Result<JObject<'l>> {
        let class = self.class.get(env)?;
        let id = match self.id.get() {
            Some(id) => *id,
            None => {
                let found = env.get_method_id(&class, "<init>", self.sig);
                let id = checked(found)?;
                *self.id.get_or_init(|| id)
            }
        };
        // SAFETY: `id` is this class's constructor with this signature.
        let r = raw_args(args, |a| unsafe { env.new_object_unchecked(&class, id, a) });
        checked_in(env, r)
    }
}

/// An instance field.
pub struct Field {
    class: &'static ClassRef,
    name: &'static str,
    sig: &'static str,
    id: OnceLock<JFieldID>,
}

impl Field {
    pub const fn new(class: &'static ClassRef, name: &'static str, sig: &'static str) -> Self {
        Field {
            class,
            name,
            sig,
            id: OnceLock::new(),
        }
    }

    fn id(&self, env: &mut JNIEnv) -> Result<JFieldID> {
        if let Some(id) = self.id.get() {
            return Ok(*id);
        }
        let class = self.class.get(env)?;
        let found = env.get_field_id(&class, self.name, self.sig);
        let id = checked(found)?;
        Ok(*self.id.get_or_init(|| id))
    }

    fn get<'l>(
        &self,
        env: &mut JNIEnv<'l>,
        obj: &JObject,
        ty: ReturnType,
    ) -> Result<JValueOwned<'l>> {
        let id = self.id(env)?;
        let r = env.get_field_unchecked(obj, id, ty);
        checked_in(env, r)
    }

    pub fn object<'l>(&self, env: &mut JNIEnv<'l>, obj: &JObject) -> Result<JObject<'l>> {
        Ok(self.get(env, obj, ReturnType::Object)?.l()?)
    }

    pub fn boolean(&self, env: &mut JNIEnv, obj: &JObject) -> Result<bool> {
        Ok(self
            .get(env, obj, ReturnType::Primitive(Primitive::Boolean))?
            .z()?)
    }

    pub fn int(&self, env: &mut JNIEnv, obj: &JObject) -> Result<i32> {
        Ok(self
            .get(env, obj, ReturnType::Primitive(Primitive::Int))?
            .i()?)
    }

    pub fn long(&self, env: &mut JNIEnv, obj: &JObject) -> Result<i64> {
        Ok(self
            .get(env, obj, ReturnType::Primitive(Primitive::Long))?
            .j()?)
    }

    pub fn float(&self, env: &mut JNIEnv, obj: &JObject) -> Result<f32> {
        Ok(self
            .get(env, obj, ReturnType::Primitive(Primitive::Float))?
            .f()?)
    }

    pub fn double(&self, env: &mut JNIEnv, obj: &JObject) -> Result<f64> {
        Ok(self
            .get(env, obj, ReturnType::Primitive(Primitive::Double))?
            .d()?)
    }

    pub fn byte(&self, env: &mut JNIEnv, obj: &JObject) -> Result<i8> {
        Ok(self
            .get(env, obj, ReturnType::Primitive(Primitive::Byte))?
            .b()?)
    }

    pub fn short(&self, env: &mut JNIEnv, obj: &JObject) -> Result<i16> {
        Ok(self
            .get(env, obj, ReturnType::Primitive(Primitive::Short))?
            .s()?)
    }

    pub fn char(&self, env: &mut JNIEnv, obj: &JObject) -> Result<u16> {
        Ok(self
            .get(env, obj, ReturnType::Primitive(Primitive::Char))?
            .c()?)
    }
}

/// A static field.
pub struct StaticField {
    class: &'static ClassRef,
    name: &'static str,
    sig: &'static str,
    id: OnceLock<JStaticFieldID>,
}

impl StaticField {
    pub const fn new(class: &'static ClassRef, name: &'static str, sig: &'static str) -> Self {
        StaticField {
            class,
            name,
            sig,
            id: OnceLock::new(),
        }
    }

    fn get<'l>(
        &self,
        env: &mut JNIEnv<'l>,
        ty: jni::signature::JavaType,
    ) -> Result<JValueOwned<'l>> {
        let class = self.class.get(env)?;
        let id = match self.id.get() {
            Some(id) => *id,
            None => {
                let found = env.get_static_field_id(&class, self.name, self.sig);
                let id = checked(found)?;
                *self.id.get_or_init(|| id)
            }
        };
        let r = env.get_static_field_unchecked(&class, id, ty);
        checked_in(env, r)
    }

    pub fn object<'l>(&self, env: &mut JNIEnv<'l>) -> Result<JObject<'l>> {
        let ty = jni::signature::JavaType::Object(String::new());
        Ok(self.get(env, ty)?.l()?)
    }

    pub fn boolean(&self, env: &mut JNIEnv) -> Result<bool> {
        Ok(self
            .get(env, jni::signature::JavaType::Primitive(Primitive::Boolean))?
            .z()?)
    }

    pub fn int(&self, env: &mut JNIEnv) -> Result<i32> {
        Ok(self
            .get(env, jni::signature::JavaType::Primitive(Primitive::Int))?
            .i()?)
    }

    pub fn long(&self, env: &mut JNIEnv) -> Result<i64> {
        Ok(self
            .get(env, jni::signature::JavaType::Primitive(Primitive::Long))?
            .j()?)
    }

    pub fn double(&self, env: &mut JNIEnv) -> Result<f64> {
        Ok(self
            .get(env, jni::signature::JavaType::Primitive(Primitive::Double))?
            .d()?)
    }
}
