//! `java.util`.

use crate::java::lang::{self, IterOrError, Object};
use crate::member::{ClassRef, Method, StaticMethod};
use crate::{Handle, IsA, Rebind, Result, checked, is_a, vm};
use jni::JNIEnv;
use jni::objects::{JObject, JValue};

static ITERATOR: ClassRef = ClassRef::new("java/util/Iterator");
static HAS_NEXT: Method = Method::new(&ITERATOR, "hasNext", "()Z");
static NEXT: Method = Method::new(&ITERATOR, "next", "()Ljava/lang/Object;");

crate::handle!(
    /// `java.util.Iterator<T>`
    Iterator,
    "java/util/Iterator",
    T
);

impl<'a, T> Iterator<'a, T> {
    pub fn has_next(&self) -> Result<bool> {
        let mut env = vm::env()?;
        HAS_NEXT.boolean(&mut env, &self.obj, &[])
    }
}

impl<'a, T: Rebind<'a>> Iterator<'a, T> {
    /// `next()`; throws `NoSuchElementException` past the end.
    pub fn next_element(&self) -> Result<T::Out> {
        let mut env = vm::env()?;
        let o = NEXT.object(&mut env, &self.obj, &[])?;
        // SAFETY: the promise of the type parameter.
        Ok(unsafe { <T::Out as Handle<'a>>::from_raw(o) })
    }
}

impl<'a, T: Rebind<'a>> std::iter::Iterator for Iterator<'a, T> {
    type Item = Result<T::Out>;
    fn next(&mut self) -> Option<Result<T::Out>> {
        match self.has_next() {
            Ok(true) => Some(self.next_element()),
            Ok(false) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

static COLLECTION: ClassRef = ClassRef::new("java/util/Collection");
static SIZE: Method = Method::new(&COLLECTION, "size", "()I");
static IS_EMPTY: Method = Method::new(&COLLECTION, "isEmpty", "()Z");
static CONTAINS: Method = Method::new(&COLLECTION, "contains", "(Ljava/lang/Object;)Z");
static ADD: Method = Method::new(&COLLECTION, "add", "(Ljava/lang/Object;)Z");
static COLLECTION_ITERATOR: Method = Method::new(&COLLECTION, "iterator", "()Ljava/util/Iterator;");

/// `size`, `isEmpty`, `contains`, `iterator` and the iterator traits for a
/// `java.util.Collection` subtype.
macro_rules! collection {
    ($name:ident) => {
        impl<'a, T> $name<'a, T> {
            pub fn len(&self) -> Result<usize> {
                let mut env = vm::env()?;
                Ok(SIZE.int(&mut env, &self.obj, &[])? as usize)
            }

            pub fn is_empty(&self) -> Result<bool> {
                let mut env = vm::env()?;
                IS_EMPTY.boolean(&mut env, &self.obj, &[])
            }

            pub fn contains(&self, item: &impl IsA<Object<'static>>) -> Result<bool> {
                let mut env = vm::env()?;
                let o = item.borrowed();
                CONTAINS.boolean(&mut env, &self.obj, &[JValue::Object(&o)])
            }

            pub fn iterator(&self) -> Result<Iterator<'a, T>> {
                let mut env = vm::env()?;
                let it = COLLECTION_ITERATOR.object(&mut env, &self.obj, &[])?;
                // SAFETY: `iterator` returns an `Iterator`.
                Ok(unsafe { Iterator::from_raw(it) })
            }

            /// `add(item)` on a mutable collection; false when it was already there.
            pub fn add(&self, item: &impl IsA<T>) -> Result<bool> {
                let mut env = vm::env()?;
                let o = item.borrowed();
                ADD.boolean(&mut env, &self.obj, &[JValue::Object(&o)])
            }
        }

        impl<'a, T: Rebind<'a>> $name<'a, T> {
            /// Iterates without consuming the handle.
            pub fn iter(&self) -> Result<Iterator<'a, T>> {
                self.iterator()
            }
        }

        impl<'a, T: Rebind<'a>> IntoIterator for $name<'a, T> {
            type Item = Result<T::Out>;
            type IntoIter = IterOrError<'a, T>;
            fn into_iter(self) -> Self::IntoIter {
                IterOrError::new(self.iterator())
            }
        }

        impl<'a, T: Rebind<'a>> IntoIterator for &$name<'a, T> {
            type Item = Result<T::Out>;
            type IntoIter = IterOrError<'a, T>;
            fn into_iter(self) -> Self::IntoIter {
                IterOrError::new(self.iterator())
            }
        }
    };
}

crate::handle!(
    /// `java.util.Collection<T>`
    Collection,
    "java/util/Collection",
    T
);
collection!(Collection);

crate::handle!(
    /// `java.util.List<T>`
    List,
    "java/util/List",
    T
);
collection!(List);

crate::handle!(
    /// `java.util.Set<T>`
    Set,
    "java/util/Set",
    T
);
collection!(Set);

is_a!(Collection[T] => Iterable[T]);
is_a!(List[T] => Collection[T]);
is_a!(List[T] => Iterable[T]);
is_a!(Set[T] => Collection[T]);
is_a!(Set[T] => Iterable[T]);

use crate::java::lang::Iterable;

static LIST: ClassRef = ClassRef::new("java/util/List");
static GET: Method = Method::new(&LIST, "get", "(I)Ljava/lang/Object;");
static ARRAYS: ClassRef = ClassRef::new("java/util/Arrays");
static AS_LIST: StaticMethod =
    StaticMethod::new(&ARRAYS, "asList", "([Ljava/lang/Object;)Ljava/util/List;");

impl<'a, T: Rebind<'a>> List<'a, T> {
    /// `get(index)`
    pub fn get(&self, index: usize) -> Result<T::Out> {
        let mut env = vm::env()?;
        let o = GET.object(&mut env, &self.obj, &[JValue::Int(index as i32)])?;
        // SAFETY: the promise of the type parameter.
        Ok(unsafe { <T::Out as Handle<'a>>::from_raw(o) })
    }
}

impl<'l, T> List<'l, T> {
    /// A new empty, mutable `ArrayList`.
    pub fn new(env: &mut JNIEnv<'l>) -> Result<Self> {
        let r = env.new_object(ARRAY_LIST_NAME, "()V", &[]);
        let o = checked(r)?;
        // SAFETY: an `ArrayList` is a `List`.
        Ok(unsafe { List::from_raw(o) })
    }

    /// A fixed-size list of these elements, `Arrays.asList(items)`.
    pub fn of<I: IsA<T>>(env: &mut JNIEnv<'l>, items: &[I]) -> Result<Self> {
        let null = JObject::null();
        let arr = env.new_object_array(items.len() as i32, "java/lang/Object", &null)?;
        for (i, item) in items.iter().enumerate() {
            env.set_object_array_element(&arr, i as i32, item.borrowed())?;
        }
        let o = AS_LIST.object(env, &[JValue::Object(&arr)])?;
        // SAFETY: `asList` returns a `List`.
        Ok(unsafe { List::from_raw(o) })
    }
}

const ARRAY_LIST_NAME: &str = "java/util/ArrayList";
const LINKED_HASH_SET_NAME: &str = "java/util/LinkedHashSet";

impl<'l> List<'l, lang::String<'static>> {
    /// A fixed-size list of Java strings.
    pub fn of_strs<S: AsRef<str>>(env: &mut JNIEnv<'l>, items: &[S]) -> Result<Self> {
        let strings = items
            .iter()
            .map(|s| lang::String::new(env, s.as_ref()))
            .collect::<Result<Vec<_>>>()?;
        List::of(env, &strings)
    }
}

impl<'a, T, H: IsA<T>> FromIterator<H> for List<'a, T> {
    /// Collects into a new `ArrayList`.
    ///
    /// # Panics
    /// When the JVM is not running or cannot make the list.
    fn from_iter<I: IntoIterator<Item = H>>(iter: I) -> Self {
        let mut env = match vm::env() {
            Ok(e) => e,
            Err(e) => panic!("List::from_iter: {e}"),
        };
        let mut list = match List::new(&mut env) {
            Ok(l) => l,
            Err(e) => panic!("List::from_iter: {e}"),
        };
        list.extend(iter);
        list
    }
}

impl<'a, T, H: IsA<T>> Extend<H> for List<'a, T> {
    /// # Panics
    /// When `add` throws, as it does for a fixed-size list.
    fn extend<I: IntoIterator<Item = H>>(&mut self, iter: I) {
        for item in iter {
            if let Err(e) = self.add(&item) {
                panic!("List::extend: {e}");
            }
        }
    }
}

impl<'l, T> Set<'l, T> {
    /// A new empty, mutable `LinkedHashSet`.
    pub fn new(env: &mut JNIEnv<'l>) -> Result<Self> {
        let r = env.new_object(LINKED_HASH_SET_NAME, "()V", &[]);
        let o = checked(r)?;
        // SAFETY: a `LinkedHashSet` is a `Set`.
        Ok(unsafe { Set::from_raw(o) })
    }
}

static MAP: ClassRef = ClassRef::new("java/util/Map");
static MAP_SIZE: Method = Method::new(&MAP, "size", "()I");
static MAP_IS_EMPTY: Method = Method::new(&MAP, "isEmpty", "()Z");
static CONTAINS_KEY: Method = Method::new(&MAP, "containsKey", "(Ljava/lang/Object;)Z");
static MAP_GET: Method = Method::new(&MAP, "get", "(Ljava/lang/Object;)Ljava/lang/Object;");
static PUT: Method = Method::new(
    &MAP,
    "put",
    "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
);
static KEY_SET: Method = Method::new(&MAP, "keySet", "()Ljava/util/Set;");
static VALUES: Method = Method::new(&MAP, "values", "()Ljava/util/Collection;");
static ENTRY_SET: Method = Method::new(&MAP, "entrySet", "()Ljava/util/Set;");

crate::handle!(
    /// `java.util.Map<K, V>`
    Map,
    "java/util/Map",
    K,
    V
);

crate::handle!(
    /// `java.util.Map.Entry<K, V>`
    Entry,
    "java/util/Map$Entry",
    K,
    V
);

static ENTRY: ClassRef = ClassRef::new("java/util/Map$Entry");
static GET_KEY: Method = Method::new(&ENTRY, "getKey", "()Ljava/lang/Object;");
static GET_VALUE: Method = Method::new(&ENTRY, "getValue", "()Ljava/lang/Object;");

impl<'a, K: Rebind<'a>, V: Rebind<'a>> Entry<'a, K, V> {
    pub fn key(&self) -> Result<K::Out> {
        let mut env = vm::env()?;
        let o = GET_KEY.object(&mut env, &self.obj, &[])?;
        // SAFETY: the promise of the type parameter.
        Ok(unsafe { <K::Out as Handle<'a>>::from_raw(o) })
    }

    pub fn value(&self) -> Result<V::Out> {
        let mut env = vm::env()?;
        let o = GET_VALUE.object(&mut env, &self.obj, &[])?;
        // SAFETY: the promise of the type parameter.
        Ok(unsafe { <V::Out as Handle<'a>>::from_raw(o) })
    }
}

impl<'l, K, V> Map<'l, K, V> {
    /// A new empty, mutable `LinkedHashMap`.
    pub fn new(env: &mut JNIEnv<'l>) -> Result<Self> {
        let r = env.new_object("java/util/LinkedHashMap", "()V", &[]);
        let o = checked(r)?;
        // SAFETY: a `LinkedHashMap` is a `Map`.
        Ok(unsafe { Map::from_raw(o) })
    }
}

impl<'a, K, V> Map<'a, K, V> {
    pub fn len(&self) -> Result<usize> {
        let mut env = vm::env()?;
        Ok(MAP_SIZE.int(&mut env, &self.obj, &[])? as usize)
    }

    pub fn is_empty(&self) -> Result<bool> {
        let mut env = vm::env()?;
        MAP_IS_EMPTY.boolean(&mut env, &self.obj, &[])
    }

    pub fn contains_key(&self, key: &impl IsA<Object<'static>>) -> Result<bool> {
        let mut env = vm::env()?;
        let k = key.borrowed();
        CONTAINS_KEY.boolean(&mut env, &self.obj, &[JValue::Object(&k)])
    }

    /// `put(key, value)` on a mutable map.
    pub fn put(&self, key: &impl IsA<K>, value: &impl IsA<V>) -> Result<()> {
        let mut env = vm::env()?;
        let (k, v) = (key.borrowed(), value.borrowed());
        let old = PUT.object(
            &mut env,
            &self.obj,
            &[JValue::Object(&k), JValue::Object(&v)],
        )?;
        let _ = env.delete_local_ref(old);
        Ok(())
    }
}

impl<'a, K: Rebind<'a>, V: Rebind<'a>> Map<'a, K, V> {
    /// `keySet()`
    pub fn keys(&self) -> Result<Set<'a, K>> {
        let mut env = vm::env()?;
        let o = KEY_SET.object(&mut env, &self.obj, &[])?;
        // SAFETY: `keySet` returns a `Set`.
        Ok(unsafe { Set::from_raw(o) })
    }

    /// `values()`
    pub fn values(&self) -> Result<Collection<'a, V>> {
        let mut env = vm::env()?;
        let o = VALUES.object(&mut env, &self.obj, &[])?;
        // SAFETY: `values` returns a `Collection`.
        Ok(unsafe { Collection::from_raw(o) })
    }

    /// `entrySet()`
    pub fn entries(&self) -> Result<Set<'a, Entry<'static, K, V>>> {
        let mut env = vm::env()?;
        let o = ENTRY_SET.object(&mut env, &self.obj, &[])?;
        // SAFETY: `entrySet` returns a `Set`.
        Ok(unsafe { Set::from_raw(o) })
    }

    /// `get(key)`, `None` when the map has no mapping (or maps to null).
    pub fn get(&self, key: &impl IsA<Object<'static>>) -> Result<Option<V::Out>> {
        let mut env = vm::env()?;
        let k = key.borrowed();
        let o = MAP_GET.object(&mut env, &self.obj, &[JValue::Object(&k)])?;
        if o.is_null() {
            return Ok(None);
        }
        // SAFETY: the promise of the type parameter.
        Ok(Some(unsafe { <V::Out as Handle<'a>>::from_raw(o) }))
    }
}

crate::handle!(
    /// `java.util.SortedMap<K, V>`: a [`Map`] that iterates in key order.
    SortedMap,
    "java/util/SortedMap",
    K,
    V
);

crate::is_a!(SortedMap[K, V] => Map[K, V]);

static SORTED_MAP: ClassRef = ClassRef::new("java/util/SortedMap");
static FIRST_KEY: Method = Method::new(&SORTED_MAP, "firstKey", "()Ljava/lang/Object;");
static LAST_KEY: Method = Method::new(&SORTED_MAP, "lastKey", "()Ljava/lang/Object;");

impl<'l, K, V> SortedMap<'l, K, V> {
    /// A new empty, mutable `TreeMap` in the keys' natural order.
    pub fn new(env: &mut JNIEnv<'l>) -> Result<Self> {
        let r = env.new_object("java/util/TreeMap", "()V", &[]);
        let o = checked(r)?;
        // SAFETY: a `TreeMap` is a `SortedMap`.
        Ok(unsafe { SortedMap::from_raw(o) })
    }
}

impl<'a, K, V> SortedMap<'a, K, V> {
    fn as_map(&self) -> Map<'a, K, V> {
        Map::from(self.clone())
    }

    pub fn len(&self) -> Result<usize> {
        self.as_map().len()
    }

    pub fn is_empty(&self) -> Result<bool> {
        self.as_map().is_empty()
    }

    pub fn contains_key(&self, key: &impl IsA<Object<'static>>) -> Result<bool> {
        self.as_map().contains_key(key)
    }

    /// `put(key, value)` on a mutable map.
    pub fn put(&self, key: &impl IsA<K>, value: &impl IsA<V>) -> Result<()> {
        self.as_map().put(key, value)
    }
}

impl<'a, K: Rebind<'a>, V: Rebind<'a>> SortedMap<'a, K, V> {
    /// `keySet()`, in key order.
    pub fn keys(&self) -> Result<Set<'a, K>> {
        self.as_map().keys()
    }

    /// `values()`, in key order.
    pub fn values(&self) -> Result<Collection<'a, V>> {
        self.as_map().values()
    }

    /// `entrySet()`, in key order.
    pub fn entries(&self) -> Result<Set<'a, Entry<'static, K, V>>> {
        self.as_map().entries()
    }

    /// `get(key)`, `None` when the map has no mapping (or maps to null).
    pub fn get(&self, key: &impl IsA<Object<'static>>) -> Result<Option<V::Out>> {
        self.as_map().get(key)
    }

    /// `firstKey()`, `None` when the map is empty.
    pub fn first_key(&self) -> Result<Option<K::Out>> {
        self.end(&FIRST_KEY)
    }

    /// `lastKey()`, `None` when the map is empty.
    pub fn last_key(&self) -> Result<Option<K::Out>> {
        self.end(&LAST_KEY)
    }

    fn end(&self, method: &Method) -> Result<Option<K::Out>> {
        if self.is_empty()? {
            return Ok(None);
        }
        let mut env = vm::env()?;
        let o = method.object(&mut env, &self.obj, &[])?;
        // SAFETY: the promise of the type parameter.
        Ok(Some(unsafe { <K::Out as Handle<'a>>::from_raw(o) }))
    }
}

crate::handle!(
    /// `java.util.Optional<T>`
    Optional,
    "java/util/Optional",
    T
);

static OPTIONAL: ClassRef = ClassRef::new("java/util/Optional");
static EMPTY: StaticMethod = StaticMethod::new(&OPTIONAL, "empty", "()Ljava/util/Optional;");
static OF: StaticMethod =
    StaticMethod::new(&OPTIONAL, "of", "(Ljava/lang/Object;)Ljava/util/Optional;");
static IS_PRESENT: Method = Method::new(&OPTIONAL, "isPresent", "()Z");
static OPTIONAL_GET: Method = Method::new(&OPTIONAL, "get", "()Ljava/lang/Object;");

impl<'l, T> Optional<'l, T> {
    pub fn empty(env: &mut JNIEnv<'l>) -> Result<Self> {
        let o = EMPTY.object(env, &[])?;
        // SAFETY: `empty` returns an `Optional`.
        Ok(unsafe { Optional::from_raw(o) })
    }

    pub fn of(env: &mut JNIEnv<'l>, value: &impl IsA<T>) -> Result<Self> {
        let v = value.borrowed();
        let o = OF.object(env, &[JValue::Object(&v)])?;
        // SAFETY: `of` returns an `Optional`.
        Ok(unsafe { Optional::from_raw(o) })
    }
}

impl<'a, T: Rebind<'a>> Optional<'a, T> {
    pub fn is_present(&self) -> Result<bool> {
        let mut env = vm::env()?;
        IS_PRESENT.boolean(&mut env, &self.obj, &[])
    }

    /// The value, `None` when empty.
    pub fn value(&self) -> Result<Option<T::Out>> {
        if !self.is_present()? {
            return Ok(None);
        }
        let mut env = vm::env()?;
        let o = OPTIONAL_GET.object(&mut env, &self.obj, &[])?;
        // SAFETY: the promise of the type parameter.
        Ok(Some(unsafe { <T::Out as Handle<'a>>::from_raw(o) }))
    }
}

crate::handle!(
    /// `java.util.UUID`
    UUID,
    "java/util/UUID"
);

static UUID_CLASS: ClassRef = ClassRef::new("java/util/UUID");
static RANDOM_UUID: StaticMethod =
    StaticMethod::new(&UUID_CLASS, "randomUUID", "()Ljava/util/UUID;");

impl<'l> UUID<'l> {
    /// `UUID.randomUUID()`
    pub fn random(env: &mut JNIEnv<'l>) -> Result<Self> {
        let o = RANDOM_UUID.object(env, &[])?;
        // SAFETY: `randomUUID` returns a `UUID`.
        Ok(unsafe { UUID::from_raw(o) })
    }
}
