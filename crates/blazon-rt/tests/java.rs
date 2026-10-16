//! Runs against the JDK embedded in the extracted Bazel 9.0.1 install, and is
//! skipped when that is not present.

use blazon_rt::java::io::ByteArrayOutputStream;
use blazon_rt::java::lang::{Boolean, Integer, Long, Object, String as JString};
use blazon_rt::java::math::BigInteger;
use blazon_rt::java::util::{List, Map, Optional, Set};
use blazon_rt::{Jvm, Result};
use std::path::PathBuf;

fn libjvm() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let p = PathBuf::from(home)
        .join("Library/Caches/buildx/bazel/9.0.1/embedded_tools/jdk/lib/server/libjvm.dylib");
    p.exists().then_some(p)
}

/// Upcasts keep the handle's lifetime; no `JNIEnv<'static>` is needed.
fn upcast_in<'l>(env: &mut jni::JNIEnv<'l>) -> Result<String> {
    let s = JString::new(env, "up")?;
    let o: Object<'l> = s.upcast();
    Ok(o.to_string())
}

#[test]
fn jdk_handles() -> Result<()> {
    let Some(libjvm) = libjvm() else {
        eprintln!("skipped: no embedded JDK");
        return Ok(());
    };
    let _jvm = Jvm::start(&libjvm, &[])?;
    let mut env = blazon_rt::vm::env()?;
    assert_eq!(upcast_in(&mut env)?, "up");

    let s = JString::new(&mut env, "héllo")?;
    assert_eq!(s.to_rust()?, "héllo");
    assert_eq!(s.to_string(), "héllo");
    assert_eq!(s, s.clone());
    assert!(JString::new(&mut env, "a")? < JString::new(&mut env, "b")?);

    assert!(Boolean::of(&mut env, true)?.value()?);
    assert_eq!(Integer::of(&mut env, -7)?.value()?, -7);
    assert_eq!(Long::of(&mut env, 1 << 40)?.value()?, 1 << 40);
    assert!(Integer::of(&mut env, 1)? < Integer::of(&mut env, 2)?);
    let big = BigInteger::parse(&mut env, "12345678901234567890")?;
    assert_eq!(big.to_string(), "12345678901234567890");
    assert!(big.to_i64().is_err());
    assert_eq!(BigInteger::of(&mut env, 42)?.to_i64()?, 42);

    let list: List<JString<'static>> = List::of_strs(&mut env, &["a", "b", "c"])?;
    assert_eq!(list.len()?, 3);
    assert_eq!(list.get(1)?.to_rust()?, "b");
    let items: Vec<String> = list.iter()?.map(|s| s?.to_rust()).collect::<Result<_>>()?;
    assert_eq!(items, ["a", "b", "c"]);
    let via_iterable: Vec<String> = list
        .upcast::<blazon_rt::java::lang::Iterable<JString<'static>>>()
        .into_iter()
        .map(|s| s?.to_rust())
        .collect::<Result<_>>()?;
    assert_eq!(via_iterable, items);
    assert!(list.contains(&JString::new(&mut env, "c")?)?);

    let collected: List<JString<'static>> = ["x", "y"]
        .iter()
        .map(|s| JString::new(&mut env, s))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .collect();
    assert_eq!(collected.len()?, 2);

    let map: Map<JString<'static>, JString<'static>> = Map::new(&mut env)?;
    let key = JString::new(&mut env, "k")?;
    map.put(&key, &JString::new(&mut env, "v")?)?;
    assert_eq!(map.len()?, 1);
    assert!(map.contains_key(&key)?);
    assert_eq!(map.get(&key)?.map(|v| v.to_string()), Some("v".to_string()));
    assert!(map.get(&JString::new(&mut env, "x")?)?.is_none());
    let keys: Set<JString<'static>> = map.keys()?;
    assert_eq!(keys.len()?, 1);
    for e in map.entries()? {
        let e = e?;
        assert_eq!(e.key()?.to_rust()?, "k");
        assert_eq!(e.value()?.to_rust()?, "v");
    }
    assert_eq!(map.values()?.into_iter().count(), 1);

    let x = JString::new(&mut env, "x")?;
    let opt: Optional<JString<'static>> = Optional::of(&mut env, &x)?;
    assert_eq!(opt.value()?.map(|v| v.to_string()), Some("x".to_string()));
    assert!(Optional::<JString>::empty(&mut env)?.value()?.is_none());

    let out = ByteArrayOutputStream::new(&mut env)?;
    assert!(out.to_vec()?.is_empty());

    let z = JString::new(&mut env, "z")?;
    let obj: Object = z.upcast();
    assert_eq!(obj.class()?.name()?, "java.lang.String");
    assert_eq!(obj.to_string(), "z");
    let back: JString = obj.downcast()?;
    assert_eq!(back, z);
    let one = Integer::of(&mut env, 1)?;
    let err = one.upcast::<Object>().downcast::<JString>().unwrap_err();
    assert!(err.to_string().contains("java.lang.Integer"), "{err}");

    let err = list.get(99).unwrap_err();
    let ex = err.exception().expect("a Java exception");
    assert!(ex.class.contains("IndexOutOfBounds"), "{ex:?}");
    assert!(std::error::Error::source(&err).is_some());
    Ok(())
}
