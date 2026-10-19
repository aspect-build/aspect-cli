//! Rust implementing Java types: the fixture's `Callback` interface and `Task`
//! abstract class, called from Java on several threads. One test: a process
//! has one JVM.

use blazon_rt::java::lang::Object;
use blazon_rt::java::util::List;
use blazon_rt::{Jvm, Result};
use blazon_selftest::fixture::vfixture as fx;
use jni::JNIEnv;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

struct Impl {
    dropped: Arc<AtomicBool>,
    notified: Arc<AtomicUsize>,
}

impl Drop for Impl {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl fx::CallbackImpl for Impl {
    fn add<'a>(&self, _env: &mut JNIEnv<'a>, arg0: i32, arg1: i32) -> Result<i32> {
        Ok(arg0 + arg1)
    }

    fn mix<'a>(
        &self,
        _env: &mut JNIEnv<'a>,
        arg0: i64,
        arg1: f64,
        arg2: bool,
        arg3: i8,
        arg4: u16,
        arg5: i16,
        arg6: f32,
    ) -> Result<i64> {
        let flag = if arg2 { 1000 } else { 0 };
        Ok(arg0
            + arg1 as i64
            + flag
            + i64::from(arg3)
            + i64::from(arg4)
            + i64::from(arg5)
            + arg6 as i64)
    }

    fn greet<'a>(&self, _env: &mut JNIEnv<'a>, arg0: String) -> Result<String> {
        Ok(format!("hello {arg0}"))
    }

    fn maybe<'a>(&self, _env: &mut JNIEnv<'a>, arg0: Option<String>) -> Result<Option<String>> {
        Ok(arg0.map(|s| s.to_uppercase()))
    }

    fn parent<'a>(&self, _env: &mut JNIEnv<'a>, arg0: fx::Target<'a>) -> Result<fx::Target<'a>> {
        Ok(arg0)
    }

    fn maybe_parent<'a>(
        &self,
        _env: &mut JNIEnv<'a>,
        arg0: Option<fx::Target<'a>>,
    ) -> Result<Option<fx::Target<'a>>> {
        Ok(arg0)
    }

    fn names<'a>(
        &self,
        env: &mut JNIEnv<'a>,
        arg0: List<'a, fx::JavaString<'static>>,
    ) -> Result<List<'a, fx::JavaString<'static>>> {
        let out: List<'a, fx::JavaString<'static>> = List::new(env)?;
        for s in &arg0 {
            let s = s?.to_rust()?;
            out.add(&fx::JavaString::new(env, &format!("{s}!"))?)?;
        }
        Ok(out)
    }

    fn identity<'a>(&self, _env: &mut JNIEnv<'a>, arg0: Object<'a>) -> Result<Object<'a>> {
        Ok(arg0)
    }

    fn boxed<'a>(&self, _env: &mut JNIEnv<'a>, arg0: bool, arg1: i64) -> Result<i32> {
        Ok(if arg0 { arg1 as i32 } else { -(arg1 as i32) })
    }

    fn notify<'a>(&self, _env: &mut JNIEnv<'a>, arg0: String) -> Result<()> {
        self.notified.fetch_add(arg0.len(), Ordering::SeqCst);
        Ok(())
    }

    fn bytes<'a>(&self, _env: &mut JNIEnv<'a>, arg0: Vec<u8>) -> Result<Vec<u8>> {
        Ok(arg0.into_iter().rev().collect())
    }

    fn strings<'a>(&self, _env: &mut JNIEnv<'a>, arg0: Vec<String>) -> Result<Vec<String>> {
        Ok(arg0.into_iter().map(|s| s.repeat(2)).collect())
    }

    fn maybe_bytes<'a>(
        &self,
        _env: &mut JNIEnv<'a>,
        arg0: Option<Vec<u8>>,
    ) -> Result<Option<Vec<u8>>> {
        Ok(arg0.map(|b| b.into_iter().rev().collect()))
    }

    fn opt<'a>(
        &self,
        _env: &mut JNIEnv<'a>,
        arg0: Option<fx::JavaString<'a>>,
    ) -> Result<Option<fx::JavaString<'a>>> {
        Ok(arg0)
    }

    fn fail<'a>(&self, _env: &mut JNIEnv<'a>, arg0: String) -> Result<i32> {
        Err(blazon_rt::Error::msg(format!("failed because {arg0}")).context("fixture"))
    }

    fn explode<'a>(&self, _env: &mut JNIEnv<'a>) -> Result<i32> {
        panic!("the Rust side panicked");
    }

    fn slow<'a>(&self, _env: &mut JNIEnv<'a>, arg0: i32) -> Result<String> {
        std::thread::sleep(Duration::from_millis(arg0 as u64));
        Ok("slow:done".to_string())
    }
}

struct Job;

impl fx::TaskImpl for Job {
    fn run<'a>(&self, _env: &mut JNIEnv<'a>, arg0: String) -> Result<String> {
        Ok(format!("ran {arg0}"))
    }

    fn priority<'a>(&self, _env: &mut JNIEnv<'a>) -> Result<i32> {
        Ok(7)
    }
}

struct Risk;

impl fx::RiskyImpl for Risk {
    fn run<'a>(&self, env: &mut JNIEnv<'a>, arg0: String) -> Result<String> {
        match arg0.as_str() {
            "make" => Err(fx::BoomException::new(env, "made in Rust")?.into()),
            "delegate" => Ok(fx::Host::boom(env, &arg0)?),
            "plain" => Err(blazon_rt::Error::msg("not a Java error")),
            "panic" => panic!("risky panic"),
            _ => Ok(format!("fine {arg0}")),
        }
    }
}

fn risky(env: &mut JNIEnv<'static>) -> Result<()> {
    let r = fx::Risky::from_impl(env, Box::new(Risk))?;
    assert_eq!(fx::Host::call_risky(env, &r, "x")?, "ok:fine x");
    assert_eq!(fx::Host::call_risky(env, &r, "make")?, "boom:made in Rust");
    assert!(
        !fx::Host::caught_is_original(env)?,
        "the exception Rust made is not the original"
    );
    assert_eq!(fx::Host::call_risky(env, &r, "delegate")?, "boom:original");
    assert!(
        fx::Host::caught_is_original(env)?,
        "the exception Java threw came back out as the same object"
    );
    let plain = fx::Host::call_risky(env, &r, "plain")?;
    assert!(
        plain.starts_with("java.lang.RuntimeException:") && plain.contains("not a Java error"),
        "{plain}"
    );
    let panicked = fx::Host::call_risky(env, &r, "panic")?;
    assert!(
        panicked.starts_with("java.lang.IllegalStateException:")
            && panicked.contains("risky panic"),
        "{panicked}"
    );
    Ok(())
}

fn sorted_map(env: &mut JNIEnv<'static>) -> Result<()> {
    use blazon_rt::java::util::{Map, SortedMap};
    let map: SortedMap<fx::JavaString<'static>, fx::JavaString<'static>> = SortedMap::new(env)?;
    assert_eq!(map.first_key()?, None);
    for (k, v) in [("pear", "3"), ("apple", "1"), ("fig", "2")] {
        map.put(&fx::JavaString::new(env, k)?, &fx::JavaString::new(env, v)?)?;
    }
    assert_eq!(map.len()?, 3);
    let keys: Vec<String> = map
        .keys()?
        .into_iter()
        .map(|k| k?.to_rust())
        .collect::<Result<_>>()?;
    assert_eq!(keys, ["apple", "fig", "pear"], "keys come in order");
    let values: Vec<String> = map
        .values()?
        .into_iter()
        .map(|v| v?.to_rust())
        .collect::<Result<_>>()?;
    assert_eq!(values, ["1", "2", "3"]);
    let fig = fx::JavaString::new(env, "fig")?;
    assert!(map.contains_key(&fig)?);
    assert_eq!(map.get(&fig)?.expect("fig").to_rust()?, "2");
    assert_eq!(map.first_key()?.expect("first").to_rust()?, "apple");
    assert_eq!(map.last_key()?.expect("last").to_rust()?, "pear");
    let as_map: Map<fx::JavaString<'static>, fx::JavaString<'static>> = map.upcast();
    assert_eq!(as_map.len()?, 3);
    Ok(())
}

fn jdk_types(env: &mut JNIEnv<'static>) -> Result<()> {
    use blazon_rt::java::io::{BufferedOutputStream, ByteArrayOutputStream, OutputStream};
    use blazon_rt::java::time::Instant;
    let bytes = ByteArrayOutputStream::new(env)?;
    let out = BufferedOutputStream::new(env, &bytes)?;
    out.flush()?;
    let stream: OutputStream = out.upcast();
    stream.flush()?;
    stream.close()?;

    let early = Instant::of_epoch_milli(env, 1500)?;
    let late = Instant::of_epoch_second(env, 2)?;
    assert_eq!(early.to_epoch_milli()?, 1500);
    assert_eq!(late.to_epoch_milli()?, 2000);
    assert!(early < late);
    assert_eq!(early.to_string(), "1970-01-01T00:00:01.500Z");
    assert_eq!(early, Instant::of_epoch_milli(env, 1500)?);
    assert!(Instant::now(env)?.to_epoch_milli()? > 1_000_000_000_000);
    Ok(())
}

fn new_impl(
    env: &mut JNIEnv<'static>,
) -> Result<(fx::Callback<'static>, Arc<AtomicBool>, Arc<AtomicUsize>)> {
    let dropped = Arc::new(AtomicBool::new(false));
    let notified = Arc::new(AtomicUsize::new(0));
    let callback = fx::Callback::from_impl(
        env,
        Box::new(Impl {
            dropped: dropped.clone(),
            notified: notified.clone(),
        }),
    )?;
    Ok((callback, dropped, notified))
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(20);
    while Instant::now() < until {
        if done() {
            return;
        }
        fx::Host::gc(&mut blazon_rt::vm::env().expect("env")).expect("gc");
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn rust_implements_java_types() -> Result<()> {
    let Some(libjvm) = blazon_selftest::libjvm() else {
        eprintln!("skipped: no libjvm");
        return Ok(());
    };
    let jar = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../blazon/tests/fixture/fixture.jar");
    let _jvm = Jvm::start(&libjvm, &[format!("-Djava.class.path={}", jar.display())])?;
    fx::register();
    let mut env = blazon_rt::vm::env()?;

    let (cb, dropped, notified) = new_impl(&mut env)?;
    assert_eq!(fx::Host::add(&mut env, &cb, 40, 2)?, 42);
    assert_eq!(fx::Host::describe(&mut env, &cb)?, "default:hello d");

    assert_eq!(fx::Host::digest(&mut env, None)?, None);
    assert_eq!(
        fx::Host::digest(&mut env, Some(&[1, 2, 3]))?,
        Some(vec![2, 3, 4])
    );
    let rule = fx::Rule::new(&mut env, "t1", "cc_test")?;
    let target: fx::Target = rule.upcast();
    assert_eq!(
        fx::Host::exercise(&mut env, &cb, &target)?,
        "1114;hello w;X;null;t1;null;true;[a!, b!];id;5;-5;[3, 2, 1];[aa, bb];null;[8, 9];Optional[o];Optional.empty"
    );
    assert_eq!(notified.load(Ordering::SeqCst), 4);

    let sum = fx::Host::parallel(&mut env, &cb, 8, 500)?;
    let expected: i64 = (0..8)
        .map(|t| (0..500).map(|i| i64::from(t + i)).sum::<i64>())
        .sum();
    assert_eq!(sum, expected, "eight Java threads called the Rust value");

    let ok = fx::Host::attempt(&mut env, &cb, "ok")?;
    assert_eq!(ok, "returned 3");
    let failed = fx::Host::attempt(&mut env, &cb, "fail")?;
    let parts: Vec<&str> = failed.split('|').collect();
    assert_eq!(parts[0], "java.lang.RuntimeException", "{failed}");
    assert!(
        parts[1].contains("Callback.fail") && parts[1].contains("fixture: failed because why"),
        "{failed}"
    );
    let panicked = fx::Host::attempt(&mut env, &cb, "explode")?;
    let parts: Vec<&str> = panicked.split('|').collect();
    assert_eq!(parts[0], "java.lang.IllegalStateException", "{panicked}");
    assert!(
        parts[1].contains("Rust panic in Callback.explode: the Rust side panicked"),
        "{panicked}"
    );
    assert_eq!(
        fx::Host::attempt(&mut env, &cb, "ok")?,
        "returned 3",
        "the object is usable after an error and a panic"
    );

    let task = fx::Task::from_impl(&mut env, Box::new(Job))?;
    let t = fx::Host::run_task(&mut env, &task)?;
    assert_eq!(t, "task:ran n:7");

    risky(&mut env)?;
    sorted_map(&mut env)?;
    jdk_types(&mut env)?;

    drop(cb);
    wait_for("the Rust value to be dropped once Java lets go", || {
        dropped.load(Ordering::SeqCst)
    });

    let (cb, dropped, _) = new_impl(&mut env)?;
    fx::Host::hold(&mut env, &cb)?;
    drop(cb);
    fx::Host::start_slow(&mut env, 600)?;
    let until = Instant::now() + Duration::from_millis(400);
    while Instant::now() < until {
        fx::Host::gc(&mut env)?;
        std::thread::sleep(Duration::from_millis(20));
        assert!(
            !dropped.load(Ordering::SeqCst),
            "the Rust value was dropped while a call was running"
        );
    }
    assert_eq!(fx::Host::finish(&mut env)?, "slow:done");
    wait_for("the value to be dropped after the call", || {
        dropped.load(Ordering::SeqCst)
    });
    Ok(())
}
