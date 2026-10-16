//! Call overhead of the runtime: `cargo run -p blazon-selftest --example bench`
//! (add `--release` for optimised numbers). Needs a `libjvm`.

use blazon_rt::java::lang::String as JString;
use blazon_rt::java::util::List;
use blazon_rt::member::{ClassRef, Method, StaticMethod};
use blazon_rt::reflect::ReflectMethod;
use blazon_rt::{Jvm, vm};
use jni::objects::JValue;
use std::time::Instant;

const N: usize = 100_000;

fn time<R>(name: &str, n: usize, mut f: impl FnMut() -> R) {
    for _ in 0..2 {
        f();
    }
    let t = Instant::now();
    let reps = 5;
    for _ in 0..reps {
        f();
    }
    let d = t.elapsed() / reps;
    println!(
        "{name:55} {:>9.2} ms/{n}  {:>7.0} ns/op",
        d.as_secs_f64() * 1e3,
        d.as_nanos() as f64 / n as f64
    );
}

static MATH: ClassRef = ClassRef::new("java/lang/Math");
static ABS: StaticMethod = StaticMethod::new(&MATH, "abs", "(I)I");
static STRING: ClassRef = ClassRef::new("java/lang/String");
static IS_LATIN1: ReflectMethod = ReflectMethod::new(&STRING, "isLatin1", &[]);
static LIST: ClassRef = ClassRef::new("java/util/List");
static SIZE: Method = Method::new(&LIST, "size", "()I");

fn main() -> blazon_rt::Result<()> {
    let Some(libjvm) = blazon_selftest::libjvm() else {
        eprintln!("no libjvm");
        return Ok(());
    };
    let _jvm = Jvm::start(&libjvm, &["-Xmx1g".to_string()])?;
    let mut env = vm::env()?;

    time("static call through a cached member (Math.abs)", N, || {
        let mut env = vm::env().expect("env");
        let mut s = 0i64;
        for i in 0..N as i32 {
            s += i64::from(ABS.int(&mut env, &[JValue::Int(-i)]).expect("abs"));
        }
        s
    });

    time("vm::env() alone", N, || {
        for _ in 0..N {
            let _ = vm::env().expect("env");
        }
    });
    time("raw Math.abs, by name, no checks", N, || {
        let mut env = vm::env().expect("env");
        let mut s = 0i64;
        for i in 0..N as i32 {
            s += i64::from(
                env.call_static_method("java/lang/Math", "abs", "(I)I", &[JValue::Int(-i)])
                    .and_then(|v| v.i())
                    .expect("abs"),
            );
        }
        s
    });
    let list: List<JString<'static>> = List::new(&mut env)?;
    for i in 0..N {
        let s = JString::new(&mut env, &i.to_string())?;
        list.add(&s)?;
    }
    time("List::len (attach, cached ID, exception check)", N, || {
        let mut s = 0usize;
        for _ in 0..N {
            s += list.len().expect("len");
        }
        s
    });
    time("raw cached Method::int on the list", N, || {
        let mut env = vm::env().expect("env");
        let mut s = 0i64;
        for _ in 0..N {
            s += i64::from(SIZE.int(&mut env, list.as_ref(), &[]).expect("size"));
        }
        s
    });
    time("iterate 100k elements (handles drop their refs)", N, || {
        vm::scope(64, || {
            let mut c = 0;
            for x in list.iter()? {
                let _ = x?;
                c += 1;
            }
            Ok(c)
        })
        .expect("iterate")
    });
    time(
        "iterate 100k elements and read each as a Rust string",
        N,
        || {
            vm::scope(64, || {
                let mut c = 0;
                for x in list.iter()? {
                    c += x?.to_rust()?.len();
                }
                Ok(c)
            })
            .expect("iterate")
        },
    );
    let probe = JString::new(&mut env, "hello")?;
    time("ClassRef::is_instance", N, || {
        let mut env = vm::env().expect("env");
        let mut c = 0;
        for _ in 0..N {
            if STRING
                .is_instance(&mut env, probe.as_ref())
                .expect("instance")
            {
                c += 1;
            }
        }
        c
    });
    time(
        "reflective call, reflected Method cached (x10k)",
        10_000,
        || {
            let mut env = vm::env().expect("env");
            for _ in 0..10_000 {
                let o = IS_LATIN1
                    .invoke(&mut env, Some(probe.as_ref()), &[])
                    .expect("invoke");
                let _ = blazon_rt::conv::boxed_bool(o, "isLatin1").expect("bool");
            }
        },
    );
    Ok(())
}
