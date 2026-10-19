//! `java.lang`.

use crate::member::{ClassRef, Method};
use crate::{Handle, Rebind, Result, vm};
use jni::JNIEnv;
use jni::objects::{JObject, JString, JValue};
use std::cmp::Ordering;

static OBJECT: ClassRef = ClassRef::new("java/lang/Object");
static TO_STRING: Method = Method::new(&OBJECT, "toString", "()Ljava/lang/String;");
static EQUALS: Method = Method::new(&OBJECT, "equals", "(Ljava/lang/Object;)Z");
static HASH_CODE: Method = Method::new(&OBJECT, "hashCode", "()I");
static GET_CLASS: Method = Method::new(&OBJECT, "getClass", "()Ljava/lang/Class;");

fn read_string(env: &mut JNIEnv, s: JObject) -> Result<std::string::String> {
    let js = JString::from(s);
    let text: std::string::String = env.get_string(&js)?.into();
    let _ = env.delete_local_ref(js);
    Ok(text)
}

/// `obj.toString()`, or `null`.
#[doc(hidden)]
pub fn to_string(obj: &JObject) -> Result<std::string::String> {
    if obj.is_null() {
        return Ok("null".into());
    }
    let mut env = vm::env()?;
    let s = TO_STRING.object(&mut env, obj, &[])?;
    read_string(&mut env, s)
}

/// `a.equals(b)`; two nulls are equal.
#[doc(hidden)]
pub fn equals(a: &JObject, b: &JObject) -> Result<bool> {
    if a.is_null() || b.is_null() {
        return Ok(a.is_null() && b.is_null());
    }
    let mut env = vm::env()?;
    EQUALS.boolean(&mut env, a, &[JValue::Object(b)])
}

/// `obj.hashCode()`
#[doc(hidden)]
pub fn hash_code(obj: &JObject) -> Result<i32> {
    if obj.is_null() {
        return Ok(0);
    }
    let mut env = vm::env()?;
    HASH_CODE.int(&mut env, obj, &[])
}

static COMPARABLE: ClassRef = ClassRef::new("java/lang/Comparable");
static COMPARE_TO: Method = Method::new(&COMPARABLE, "compareTo", "(Ljava/lang/Object;)I");

/// `a.compareTo(b)` for a `Comparable`; incomparable objects order as equal.
#[doc(hidden)]
pub fn compare_to(a: &JObject, b: &JObject) -> Ordering {
    let Ok(mut env) = vm::env() else {
        return Ordering::Equal;
    };
    COMPARE_TO
        .int(&mut env, a, &[JValue::Object(b)])
        .map(|n| n.cmp(&0))
        .unwrap_or(Ordering::Equal)
}

crate::handle!(@root
    /// `java.lang.Object`
    Object,
    "java/lang/Object"
);

impl<'a> Object<'a> {
    /// Every reference is an `Object`, so this needs no check.
    pub fn of(obj: JObject<'a>) -> Self {
        // SAFETY: every object is an `Object`.
        unsafe { Object::from_raw(obj) }
    }

    /// `this.getClass()`
    pub fn class(&self) -> Result<Class<'a>> {
        let mut env = vm::env()?;
        let c = GET_CLASS.object(&mut env, &self.obj, &[])?;
        // SAFETY: `getClass` returns a `Class`.
        Ok(unsafe { Class::from_raw(c) })
    }
}

crate::handle!(
    /// `java.lang.Class<T>`
    Class,
    "java/lang/Class",
    T
);

static CLASS: ClassRef = ClassRef::new("java/lang/Class");
static GET_NAME: Method = Method::new(&CLASS, "getName", "()Ljava/lang/String;");

impl<'a, T> Class<'a, T> {
    /// `Class.getName()`, e.g. `java.util.Map$Entry`.
    pub fn name(&self) -> Result<std::string::String> {
        let mut env = vm::env()?;
        let n = GET_NAME.object(&mut env, &self.obj, &[])?;
        read_string(&mut env, n)
    }
}

impl<'l, T> Class<'l, T> {
    /// The class with this internal name (`java/util/List`), through `FindClass`.
    pub fn find(env: &mut JNIEnv<'l>, internal: &str) -> Result<Self> {
        let r = env.find_class(internal);
        let c = crate::checked(r)?;
        // SAFETY: `FindClass` returns a `Class`.
        Ok(unsafe { Class::from_raw(c.into()) })
    }
}

crate::handle!(
    /// `java.lang.String`
    String,
    "java/lang/String"
);

impl<'l> String<'l> {
    pub fn new(env: &mut JNIEnv<'l>, s: &str) -> Result<Self> {
        let r = env.new_string(s);
        let j = crate::checked(r)?;
        // SAFETY: a `JString` is a `java.lang.String`.
        Ok(unsafe { String::from_raw(j.into()) })
    }
}

impl<'a> String<'a> {
    /// The contents as a Rust string; a null string is an error.
    pub fn to_rust(&self) -> Result<std::string::String> {
        if self.obj.is_null() {
            return Err(crate::Error::msg("null java.lang.String"));
        }
        let mut env = vm::env()?;
        // SAFETY: a non-owning view of this handle's reference.
        let js = unsafe { JString::from_raw(self.obj.as_raw()) };
        Ok(env.get_string(&js)?.into())
    }
}

impl<'a> TryFrom<&String<'a>> for std::string::String {
    type Error = crate::Error;
    fn try_from(s: &String<'a>) -> Result<Self> {
        s.to_rust()
    }
}

impl<'a, 'b> PartialOrd<String<'b>> for String<'a> {
    fn partial_cmp(&self, other: &String<'b>) -> Option<Ordering> {
        Some(compare_to(&self.obj, &other.obj))
    }
}

impl<'a> Ord for String<'a> {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_to(&self.obj, &other.obj)
    }
}

macro_rules! boxed {
    ($(#[$meta:meta])* $name:ident, $class:literal, $prim:ty, $sig:literal, $jv:ident, $get:literal, $getsig:literal, $unwrap:ident) => {
        crate::handle!($(#[$meta])* $name, $class);

        impl<'l> $name<'l> {
            /// `valueOf`
            pub fn of(env: &mut JNIEnv<'l>, v: $prim) -> Result<Self> {
                let r = env.call_static_method($class, "valueOf", $sig, &[JValue::$jv(v as _)]);
                let o = crate::checked(r)?.l()?;
                // SAFETY: `valueOf` returns the boxed type.
                Ok(unsafe { $name::from_raw(o) })
            }
        }

        impl<'a> $name<'a> {
            pub fn value(&self) -> Result<$prim> {
                let mut env = vm::env()?;
                let r = env.call_method(&self.obj, $get, $getsig, &[]);
                Ok(crate::checked(r)?.$unwrap()?)
            }
        }

        impl<'a, 'b> PartialOrd<$name<'b>> for $name<'a> {
            fn partial_cmp(&self, other: &$name<'b>) -> Option<Ordering> {
                Some(compare_to(&self.obj, &other.obj))
            }
        }

        impl<'a> Ord for $name<'a> {
            fn cmp(&self, other: &Self) -> Ordering {
                compare_to(&self.obj, &other.obj)
            }
        }
    };
}

boxed!(
    /// `java.lang.Boolean`
    Boolean,
    "java/lang/Boolean",
    bool,
    "(Z)Ljava/lang/Boolean;",
    Bool,
    "booleanValue",
    "()Z",
    z
);
boxed!(
    /// `java.lang.Integer`
    Integer,
    "java/lang/Integer",
    i32,
    "(I)Ljava/lang/Integer;",
    Int,
    "intValue",
    "()I",
    i
);
boxed!(
    /// `java.lang.Long`
    Long,
    "java/lang/Long",
    i64,
    "(J)Ljava/lang/Long;",
    Long,
    "longValue",
    "()J",
    j
);

crate::handle!(
    /// `java.lang.Iterable<T>`
    Iterable,
    "java/lang/Iterable",
    T
);

static ITERABLE: ClassRef = ClassRef::new("java/lang/Iterable");
static ITERATOR: Method = Method::new(&ITERABLE, "iterator", "()Ljava/util/Iterator;");

impl<'a, T> Iterable<'a, T> {
    /// `iterator()`
    pub fn iterator(&self) -> Result<super::util::Iterator<'a, T>> {
        let mut env = vm::env()?;
        let it = ITERATOR.object(&mut env, &self.obj, &[])?;
        // SAFETY: `iterator` returns an `Iterator`.
        Ok(unsafe { super::util::Iterator::from_raw(it) })
    }
}

impl<'a, T: Rebind<'a>> Iterable<'a, T> {
    /// Iterates without consuming the handle.
    pub fn iter(&self) -> Result<super::util::Iterator<'a, T>> {
        self.iterator()
    }
}

impl<'a, T: Rebind<'a>> IntoIterator for Iterable<'a, T> {
    type Item = Result<T::Out>;
    type IntoIter = IterOrError<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        IterOrError::new(self.iterator())
    }
}

impl<'a, T: Rebind<'a>> IntoIterator for &Iterable<'a, T> {
    type Item = Result<T::Out>;
    type IntoIter = IterOrError<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        IterOrError::new(self.iterator())
    }
}

/// An iterator that reports a failure to start as its only item.
pub struct IterOrError<'a, T>(
    std::result::Result<super::util::Iterator<'a, T>, Option<crate::Error>>,
);

impl<'a, T> IterOrError<'a, T> {
    #[doc(hidden)]
    pub fn new(r: Result<super::util::Iterator<'a, T>>) -> Self {
        IterOrError(r.map_err(Some))
    }
}

impl<'a, T: Rebind<'a>> std::iter::Iterator for IterOrError<'a, T> {
    type Item = Result<T::Out>;
    fn next(&mut self) -> Option<Result<T::Out>> {
        match &mut self.0 {
            Ok(it) => it.next(),
            Err(e) => e.take().map(Err),
        }
    }
}

crate::handle!(
    /// `java.lang.Runnable`
    Runnable,
    "java/lang/Runnable"
);

static RUNNABLE: ClassRef = ClassRef::new("java/lang/Runnable");
static RUN: Method = Method::new(&RUNNABLE, "run", "()V");

impl<'a> Runnable<'a> {
    pub fn run(&self) -> Result<()> {
        let mut env = vm::env()?;
        RUN.void(&mut env, &self.obj, &[])
    }
}

crate::handle!(
    /// `java.lang.Throwable`
    Throwable,
    "java/lang/Throwable"
);

static THROWABLE: ClassRef = ClassRef::new("java/lang/Throwable");
static GET_MESSAGE: Method = Method::new(&THROWABLE, "getMessage", "()Ljava/lang/String;");
static GET_CAUSE: Method = Method::new(&THROWABLE, "getCause", "()Ljava/lang/Throwable;");
static PRINT_STACK_TRACE: Method =
    Method::new(&THROWABLE, "printStackTrace", "(Ljava/io/PrintWriter;)V");

impl<'a> Throwable<'a> {
    /// `getMessage()`, `None` when it is null.
    pub fn message(&self) -> Result<Option<std::string::String>> {
        let mut env = vm::env()?;
        let m = GET_MESSAGE.object(&mut env, &self.obj, &[])?;
        if m.is_null() {
            return Ok(None);
        }
        Ok(Some(read_string(&mut env, m)?))
    }

    /// `getCause()`, `None` when there is none.
    pub fn cause(&self) -> Result<Option<Throwable<'a>>> {
        let mut env = vm::env()?;
        let c = GET_CAUSE.object(&mut env, &self.obj, &[])?;
        if c.is_null() {
            return Ok(None);
        }
        // SAFETY: `getCause` returns a `Throwable`.
        Ok(Some(unsafe { Throwable::from_raw(c) }))
    }

    /// The class name, e.g. `java.lang.IllegalStateException`.
    pub fn class_name(&self) -> Result<std::string::String> {
        let mut env = vm::env()?;
        let c = GET_CLASS.object(&mut env, &self.obj, &[])?;
        // SAFETY: `getClass` returns a `Class`.
        unsafe { Class::<Object<'static>>::from_raw(c) }.name()
    }

    /// What `printStackTrace` prints: the exception, its frames and its causes.
    pub fn stack_trace(&self) -> Result<std::string::String> {
        let mut env = vm::env()?;
        let sw = env.new_object("java/io/StringWriter", "()V", &[])?;
        let pw = env.new_object(
            "java/io/PrintWriter",
            "(Ljava/io/Writer;)V",
            &[JValue::Object(&sw)],
        )?;
        PRINT_STACK_TRACE.void(&mut env, &self.obj, &[JValue::Object(&pw)])?;
        to_string(&sw)
    }
}

impl std::error::Error for Throwable<'_> {}

crate::handle!(
    /// `java.lang.Exception`
    Exception,
    "java/lang/Exception"
);

crate::handle!(
    /// `java.lang.RuntimeException`
    RuntimeException,
    "java/lang/RuntimeException"
);

crate::is_a!(Exception[] => Throwable[]);
crate::is_a!(RuntimeException[] => Exception[]);
crate::is_a!(RuntimeException[] => Throwable[]);

macro_rules! throwable_view {
    ($($t:ident),*) => {$(
        impl<'a> $t<'a> {
            /// `getMessage()`, `None` when it is null.
            pub fn message(&self) -> Result<Option<std::string::String>> {
                Throwable::from(self.clone()).message()
            }

            /// `getCause()`, `None` when there is none.
            pub fn cause(&self) -> Result<Option<Throwable<'a>>> {
                Throwable::from(self.clone()).cause()
            }

            /// What `printStackTrace` prints.
            pub fn stack_trace(&self) -> Result<std::string::String> {
                Throwable::from(self.clone()).stack_trace()
            }

            /// The class name, e.g. `java.lang.IllegalStateException`.
            pub fn class_name(&self) -> Result<std::string::String> {
                Throwable::from(self.clone()).class_name()
            }
        }

        impl std::error::Error for $t<'_> {}
    )*};
}

throwable_view!(Exception, RuntimeException);
