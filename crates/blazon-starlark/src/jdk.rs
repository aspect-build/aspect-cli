//! Starlark views of the JDK collections the engine hands back. Elements are
//! wrapped on the way out and unwrapped on the way in, so a `Set<Label>`
//! iterates as Starlark `Label` values.

use crate::{JavaValue, ToJava, error, wrap};
use allocative::Allocative;
use blazon_rt::java::lang::{Iterable, Object};
use blazon_rt::java::util::{Collection, List, Map};
use blazon_rt::member::ClassRef;
use blazon_rt::{Global, Result, vm};
use jni::objects::JObject;
use starlark::collections::StarlarkHasher;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic, Module};
use starlark::values::none::NoneType;
use starlark::values::{
    Heap, NoSerialize, ProvidesStaticType, StarlarkValue, Value, ValueLike, starlark_value,
};
use starlark::{starlark_module, starlark_simple_value};
use std::fmt;
use std::hash::Hasher;

/// How many local references an element loop holds at once.
const FRAME: i32 = 64;

static MAP: ClassRef = ClassRef::new("java/util/Map");
static LIST: ClassRef = ClassRef::new("java/util/List");
static SET: ClassRef = ClassRef::new("java/util/Set");
static COLLECTION: ClassRef = ClassRef::new("java/util/Collection");
static ITERABLE: ClassRef = ClassRef::new("java/lang/Iterable");

fn is(obj: &JObject, class: &ClassRef) -> Result<bool> {
    let mut env = vm::env()?;
    class.is_instance(&mut env, obj)
}

fn global(obj: &JObject) -> Result<Global<Object<'static>>> {
    let env = vm::env()?;
    Ok(Global::new(env.new_global_ref(obj)?))
}

/// The first of the JDK views that fits `obj`, `None` when it is none of them.
pub fn view<'v>(obj: &JObject, heap: Heap<'v>) -> Result<Option<Value<'v>>> {
    Ok(if is(obj, &MAP)? {
        Some(heap.alloc(JavaMap(global(obj)?)))
    } else if is(obj, &LIST)? {
        Some(heap.alloc(JavaList(global(obj)?)))
    } else if is(obj, &SET)? {
        Some(heap.alloc(JavaSet(global(obj)?)))
    } else if is(obj, &COLLECTION)? {
        Some(heap.alloc(JavaCollection(global(obj)?)))
    } else if is(obj, &ITERABLE)? {
        Some(heap.alloc(JavaIterable(global(obj)?)))
    } else {
        None
    })
}

/// The object behind one of the JDK views.
pub fn unview(v: Value) -> Result<Option<JObject<'static>>> {
    macro_rules! try_view {
        ($($t:ty),*) => {$(
            if let Some(x) = v.downcast_ref::<$t>() {
                return Ok(Some(x.java_object()?));
            }
        )*};
    }
    try_view!(JavaMap, JavaList, JavaSet, JavaCollection, JavaIterable);
    Ok(None)
}

fn new_local(g: &Global<Object<'static>>) -> Result<JObject<'static>> {
    let env = vm::env()?;
    // SAFETY: a non-owning view of the global, for the call that copies it.
    Ok(env.new_local_ref(unsafe { JObject::from_raw(g.raw()) })?)
}

fn object(g: &Global<Object<'static>>) -> Result<Object<'static>> {
    let mut env = vm::env()?;
    g.get(&mut env)
}

fn items<'v>(g: &Global<Object<'static>>, heap: Heap<'v>) -> Result<Vec<Value<'v>>> {
    vm::scope(FRAME, || {
        let it: Iterable = object(g)?.downcast()?;
        let mut out = Vec::new();
        for item in it {
            out.push(wrap(item?.as_ref(), heap)?);
        }
        Ok(out)
    })
}

fn size(g: &Global<Object<'static>>) -> Result<i32> {
    let c: Collection = object(g)?.downcast()?;
    Ok(c.len()? as i32)
}

fn contains(g: &Global<Object<'static>>, needle: Value) -> Result<bool> {
    let c: Collection = object(g)?.downcast()?;
    let n = Object::of(needle.to_java()?);
    c.contains(&n)
}

fn same_object(a: &Global<Object<'static>>, b: &Global<Object<'static>>) -> Result<bool> {
    Ok(object(a)? == object(b)?)
}

fn hash(g: &Global<Object<'static>>) -> Result<i32> {
    blazon_rt::java::lang::hash_code(object(g)?.as_ref())
}

fn repr_of(open: &str, close: &str, items: Vec<Value>) -> String {
    let parts: Vec<String> = items.iter().map(|v| v.to_repr()).collect();
    format!("{open}{}{close}", parts.join(", "))
}

fn shown(g: &Global<Object<'static>>, open: &str, close: &str, name: &str) -> String {
    let rendered = Module::with_temp_heap(|m| {
        Ok::<_, starlark::Error>(items(g, m.heap()).map(|i| repr_of(open, close, i)))
    });
    match rendered {
        Ok(Ok(s)) => s,
        _ => format!("<{name}>"),
    }
}

macro_rules! view {
    ($(#[$m:meta])* $name:ident, $ty:literal, $open:literal, $close:literal, sized: $sized:tt) => {
        $(#[$m])*
        #[derive(ProvidesStaticType, NoSerialize, Allocative)]
        pub struct $name(#[allocative(skip)] Global<Object<'static>>);

        starlark_simple_value!($name);

        impl JavaValue for $name {
            fn java_object(&self) -> Result<JObject<'static>> {
                new_local(&self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&shown(&self.0, $open, $close, $ty))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str($ty)
            }
        }

        #[starlark_value(type = $ty)]
        impl<'v> StarlarkValue<'v> for $name {
            fn iterate_collect(&self, heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
                items(&self.0, heap).map_err(error)
            }

            fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
                match other.downcast_ref::<$name>() {
                    Some(o) => same_object(&self.0, &o.0).map_err(error),
                    None => Ok(false),
                }
            }

            fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
                hasher.write_i32(hash(&self.0).map_err(error)?);
                Ok(())
            }

            view!(@sized $sized);
        }
    };
    (@sized true) => {
        fn length(&self) -> starlark::Result<i32> {
            size(&self.0).map_err(error)
        }

        fn to_bool(&self) -> bool {
            size(&self.0).map(|n| n > 0).unwrap_or(true)
        }

        fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
            contains(&self.0, other).map_err(error)
        }
    };
    (@sized false) => {};
}

view!(
    /// A `java.lang.Iterable` that is not a collection: iterable once per call.
    JavaIterable, "Iterable", "[", "]", sized: false
);
view!(
    /// A `java.util.Collection` that is neither a list nor a set.
    JavaCollection, "Collection", "[", "]", sized: true
);
view!(
    /// A `java.util.Set`.
    JavaSet, "Set", "set([", "])", sized: true
);

/// A `java.util.List`, indexable.
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct JavaList(#[allocative(skip)] Global<Object<'static>>);

starlark_simple_value!(JavaList);

impl JavaValue for JavaList {
    fn java_object(&self) -> Result<JObject<'static>> {
        new_local(&self.0)
    }
}

impl fmt::Display for JavaList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&shown(&self.0, "[", "]", "List"))
    }
}

impl fmt::Debug for JavaList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("List")
    }
}

#[starlark_value(type = "List")]
impl<'v> StarlarkValue<'v> for JavaList {
    fn iterate_collect(&self, heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
        items(&self.0, heap).map_err(error)
    }

    fn length(&self) -> starlark::Result<i32> {
        size(&self.0).map_err(error)
    }

    fn to_bool(&self) -> bool {
        size(&self.0).map(|n| n > 0).unwrap_or(true)
    }

    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        contains(&self.0, other).map_err(error)
    }

    fn at(&self, index: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let n = size(&self.0).map_err(error)?;
        let i = index.unpack_i32().ok_or_else(|| {
            error(blazon_rt::Error::msg(format!(
                "list index must be an int, got {}",
                index.get_type()
            )))
        })?;
        let i = if i < 0 { i + n } else { i };
        if i < 0 || i >= n {
            return Err(error(blazon_rt::Error::msg(format!(
                "list index {i} out of range for length {n}"
            ))));
        }
        vm::scope(FRAME, || {
            let l: List = object(&self.0)?.downcast()?;
            wrap(l.get(i as usize)?.as_ref(), heap)
        })
        .map_err(error)
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        match other.downcast_ref::<JavaList>() {
            Some(o) => same_object(&self.0, &o.0).map_err(error),
            None => Ok(false),
        }
    }

    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        hasher.write_i32(hash(&self.0).map_err(error)?);
        Ok(())
    }
}

/// A `java.util.Map`: iterates its keys, indexes by key.
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct JavaMap(#[allocative(skip)] Global<Object<'static>>);

starlark_simple_value!(JavaMap);

impl JavaValue for JavaMap {
    fn java_object(&self) -> Result<JObject<'static>> {
        new_local(&self.0)
    }
}

impl JavaMap {
    fn map<R>(&self, f: impl FnOnce(Map) -> Result<R>) -> Result<R> {
        vm::scope(FRAME, || f(object(&self.0)?.downcast()?))
    }

    fn keys<'v>(&self, heap: Heap<'v>) -> Result<Vec<Value<'v>>> {
        self.map(|m| {
            let mut out = Vec::new();
            for k in m.keys()? {
                out.push(wrap(k?.as_ref(), heap)?);
            }
            Ok(out)
        })
    }

    fn get<'v>(&self, key: Value<'v>, heap: Heap<'v>) -> Result<Option<Value<'v>>> {
        let k = Object::of(key.to_java()?);
        self.map(|m| match m.get(&k)? {
            Some(v) => Ok(Some(wrap(v.as_ref(), heap)?)),
            None => Ok(None),
        })
    }
}

impl fmt::Display for JavaMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rendered = Module::with_temp_heap(|module| {
            let heap = module.heap();
            Ok::<_, starlark::Error>(self.map(|m| {
                let mut parts = Vec::new();
                for e in m.entries()? {
                    let e = e?;
                    parts.push(format!(
                        "{}: {}",
                        wrap(e.key()?.as_ref(), heap)?.to_repr(),
                        wrap(e.value()?.as_ref(), heap)?.to_repr()
                    ));
                }
                Ok(format!("{{{}}}", parts.join(", ")))
            }))
        });
        match rendered {
            Ok(Ok(s)) => f.write_str(&s),
            _ => f.write_str("<Map>"),
        }
    }
}

impl fmt::Debug for JavaMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Map")
    }
}

#[starlark_value(type = "Map")]
impl<'v> StarlarkValue<'v> for JavaMap {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("java_map_methods", java_map_methods);
        Some(RES.methods())
    }

    fn iterate_collect(&self, heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
        self.keys(heap).map_err(error)
    }

    fn length(&self) -> starlark::Result<i32> {
        self.map(|m| Ok(m.len()? as i32)).map_err(error)
    }

    fn to_bool(&self) -> bool {
        self.map(|m| Ok(!m.is_empty()?)).unwrap_or(true)
    }

    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        let k = Object::of(other.to_java().map_err(error)?);
        self.map(|m| m.contains_key(&k)).map_err(error)
    }

    fn at(&self, index: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        match self.get(index, heap).map_err(error)? {
            Some(v) => Ok(v),
            None => Err(error(blazon_rt::Error::msg(format!(
                "key {} not found in map",
                index.to_repr()
            )))),
        }
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        match other.downcast_ref::<JavaMap>() {
            Some(o) => same_object(&self.0, &o.0).map_err(error),
            None => Ok(false),
        }
    }

    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        hasher.write_i32(hash(&self.0).map_err(error)?);
        Ok(())
    }
}

#[starlark_module]
fn java_map_methods(b: &mut MethodsBuilder) {
    /// The value for `key`, or `default` when the map has none.
    fn get<'v>(
        this: &JavaMap,
        #[starlark(require = pos)] key: Value<'v>,
        #[starlark(require = pos, default = NoneType)] default: Value<'v>,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        Ok(this.get(key, heap).map_err(error)?.unwrap_or(default))
    }

    /// The keys, as a list.
    fn keys<'v>(this: &JavaMap, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        Ok(heap.alloc(this.keys(heap).map_err(error)?))
    }

    /// The values, as a list.
    fn values<'v>(this: &JavaMap, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let out = this
            .map(|m| {
                let mut out = Vec::new();
                for v in m.values()? {
                    out.push(wrap(v?.as_ref(), heap)?);
                }
                Ok(out)
            })
            .map_err(error)?;
        Ok(heap.alloc(out))
    }

    /// The (key, value) pairs, as a list of tuples.
    fn items<'v>(this: &JavaMap, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let out = this
            .map(|m| {
                let mut out = Vec::new();
                for e in m.entries()? {
                    let e = e?;
                    out.push(heap.alloc((
                        wrap(e.key()?.as_ref(), heap)?,
                        wrap(e.value()?.as_ref(), heap)?,
                    )));
                }
                Ok(out)
            })
            .map_err(error)?;
        Ok(heap.alloc(out))
    }
}
