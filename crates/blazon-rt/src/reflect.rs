//! Members that are not public, reached through `java.lang.reflect`. The
//! reflected `Method` or `Field` is looked up and made accessible once. Values
//! cross as objects, so primitives are boxed by the caller.

use crate::member::{ClassRef, Method};
use crate::{Result, checked};
use jni::JNIEnv;
use jni::objects::{GlobalRef, JObject, JObjectArray, JValue};
use std::sync::OnceLock;

static OBJECT: ClassRef = ClassRef::new("java/lang/Object");
static REFLECT_METHOD: ClassRef = ClassRef::new("java/lang/reflect/Method");
static INVOKE: Method = Method::new(
    &REFLECT_METHOD,
    "invoke",
    "(Ljava/lang/Object;[Ljava/lang/Object;)Ljava/lang/Object;",
);
static REFLECT_FIELD: ClassRef = ClassRef::new("java/lang/reflect/Field");
static FIELD_GET: Method = Method::new(
    &REFLECT_FIELD,
    "get",
    "(Ljava/lang/Object;)Ljava/lang/Object;",
);
static FIELD_SET: Method = Method::new(
    &REFLECT_FIELD,
    "set",
    "(Ljava/lang/Object;Ljava/lang/Object;)V",
);
static REFLECT_CONSTRUCTOR: ClassRef = ClassRef::new("java/lang/reflect/Constructor");
static NEW_INSTANCE: Method = Method::new(
    &REFLECT_CONSTRUCTOR,
    "newInstance",
    "([Ljava/lang/Object;)Ljava/lang/Object;",
);

/// An `Object[]` of `args`.
fn object_array<'l>(env: &mut JNIEnv<'l>, args: &[&JObject]) -> Result<JObjectArray<'l>> {
    let class = OBJECT.get(env)?;
    let null = JObject::null();
    let array = env.new_object_array(args.len() as i32, &class, &null)?;
    for (i, a) in args.iter().enumerate() {
        env.set_object_array_element(&array, i as i32, a)?;
    }
    Ok(array)
}

/// The `Class` object for a field descriptor: `I`, `Ljava/lang/String;`, `[B`.
fn class_of<'l>(env: &mut JNIEnv<'l>, descriptor: &str) -> Result<JObject<'l>> {
    let wrapper = match descriptor {
        "Z" => Some("java/lang/Boolean"),
        "B" => Some("java/lang/Byte"),
        "C" => Some("java/lang/Character"),
        "S" => Some("java/lang/Short"),
        "I" => Some("java/lang/Integer"),
        "J" => Some("java/lang/Long"),
        "F" => Some("java/lang/Float"),
        "D" => Some("java/lang/Double"),
        _ => None,
    };
    if let Some(w) = wrapper {
        let r = env.get_static_field(w, "TYPE", "Ljava/lang/Class;");
        return Ok(checked(r)?.l()?);
    }
    let name = descriptor
        .strip_prefix('L')
        .and_then(|d| d.strip_suffix(';'))
        .unwrap_or(descriptor);
    let r = env.find_class(name);
    Ok(checked(r)?.into())
}

fn accessible<'l>(env: &mut JNIEnv<'l>, member: &JObject<'l>) -> Result<()> {
    let r = env.call_method(member, "setAccessible", "(Z)V", &[JValue::Bool(1)]);
    checked(r)?;
    Ok(())
}

/// A method of a class, reflected once. `params` are field descriptors.
pub struct ReflectMethod {
    class: &'static ClassRef,
    name: &'static str,
    params: &'static [&'static str],
    cell: OnceLock<GlobalRef>,
}

impl ReflectMethod {
    pub const fn new(
        class: &'static ClassRef,
        name: &'static str,
        params: &'static [&'static str],
    ) -> Self {
        ReflectMethod {
            class,
            name,
            params,
            cell: OnceLock::new(),
        }
    }

    fn method(&self, env: &mut JNIEnv) -> Result<GlobalRef> {
        if let Some(g) = self.cell.get() {
            return Ok(g.clone());
        }
        let class = self.class.get(env)?;
        let null = JObject::null();
        let types = env.new_object_array(self.params.len() as i32, "java/lang/Class", &null)?;
        for (i, p) in self.params.iter().enumerate() {
            let c = class_of(env, p)?;
            env.set_object_array_element(&types, i as i32, &c)?;
        }
        let jname = env.new_string(self.name)?;
        let r = env.call_method(
            &class,
            "getDeclaredMethod",
            "(Ljava/lang/String;[Ljava/lang/Class;)Ljava/lang/reflect/Method;",
            &[JValue::Object(&jname), JValue::Object(&types)],
        );
        let method = checked(r)?.l()?;
        accessible(env, &method)?;
        let global = env.new_global_ref(&method)?;
        Ok(self.cell.get_or_init(|| global).clone())
    }

    /// Calls the method on `this` (`None` for a static method); a void method gives null.
    pub fn invoke<'l>(
        &self,
        env: &mut JNIEnv<'l>,
        this: Option<&JObject>,
        args: &[&JObject],
    ) -> Result<JObject<'l>> {
        let method = self.method(env)?;
        let array = object_array(env, args)?;
        let null = JObject::null();
        let receiver = this.unwrap_or(&null);
        INVOKE.object(
            env,
            method.as_obj(),
            &[JValue::Object(receiver), JValue::Object(&array)],
        )
    }
}

/// A field of a class, reflected once.
pub struct ReflectField {
    class: &'static ClassRef,
    name: &'static str,
    cell: OnceLock<GlobalRef>,
}

impl ReflectField {
    pub const fn new(class: &'static ClassRef, name: &'static str) -> Self {
        ReflectField {
            class,
            name,
            cell: OnceLock::new(),
        }
    }

    fn field(&self, env: &mut JNIEnv) -> Result<GlobalRef> {
        if let Some(g) = self.cell.get() {
            return Ok(g.clone());
        }
        let class = self.class.get(env)?;
        let jname = env.new_string(self.name)?;
        let r = env.call_method(
            &class,
            "getDeclaredField",
            "(Ljava/lang/String;)Ljava/lang/reflect/Field;",
            &[JValue::Object(&jname)],
        );
        let field = checked(r)?.l()?;
        accessible(env, &field)?;
        let global = env.new_global_ref(&field)?;
        Ok(self.cell.get_or_init(|| global).clone())
    }

    /// The field's value on `this` (`None` for a static field).
    pub fn get<'l>(&self, env: &mut JNIEnv<'l>, this: Option<&JObject>) -> Result<JObject<'l>> {
        let field = self.field(env)?;
        let null = JObject::null();
        FIELD_GET.object(
            env,
            field.as_obj(),
            &[JValue::Object(this.unwrap_or(&null))],
        )
    }

    /// Sets the field on `this`, even if it is final.
    pub fn set(&self, env: &mut JNIEnv, this: &JObject, value: &JObject) -> Result<()> {
        let field = self.field(env)?;
        FIELD_SET.void(
            env,
            field.as_obj(),
            &[JValue::Object(this), JValue::Object(value)],
        )
    }
}

/// A constructor of a class, reflected once. `params` are field descriptors.
pub struct ReflectConstructor {
    class: &'static ClassRef,
    params: &'static [&'static str],
    cell: OnceLock<GlobalRef>,
}

impl ReflectConstructor {
    pub const fn new(class: &'static ClassRef, params: &'static [&'static str]) -> Self {
        ReflectConstructor {
            class,
            params,
            cell: OnceLock::new(),
        }
    }

    fn constructor(&self, env: &mut JNIEnv) -> Result<GlobalRef> {
        if let Some(g) = self.cell.get() {
            return Ok(g.clone());
        }
        let class = self.class.get(env)?;
        let null = JObject::null();
        let types = env.new_object_array(self.params.len() as i32, "java/lang/Class", &null)?;
        for (i, p) in self.params.iter().enumerate() {
            let c = class_of(env, p)?;
            env.set_object_array_element(&types, i as i32, &c)?;
        }
        let r = env.call_method(
            &class,
            "getDeclaredConstructor",
            "([Ljava/lang/Class;)Ljava/lang/reflect/Constructor;",
            &[JValue::Object(&types)],
        );
        let ctor = checked(r)?.l()?;
        accessible(env, &ctor)?;
        let global = env.new_global_ref(&ctor)?;
        Ok(self.cell.get_or_init(|| global).clone())
    }

    /// `newInstance(args)`
    pub fn new_instance<'l>(&self, env: &mut JNIEnv<'l>, args: &[&JObject]) -> Result<JObject<'l>> {
        let ctor = self.constructor(env)?;
        let array = object_array(env, args)?;
        NEW_INSTANCE.object(env, ctor.as_obj(), &[JValue::Object(&array)])
    }
}
