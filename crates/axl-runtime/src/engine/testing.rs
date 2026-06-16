//! Built-in test framework for AXL — POC.
//!
//! This is a proof-of-concept implementation of the design discussed for
//! giving AXL a native, pytest-style testing story. It demonstrates the
//! load-bearing decisions end to end:
//!
//!   1. **Test-only globals.** `*_test.axl` files are evaluated against an
//!      augmented globals surface (base AXL + the `asserts` namespace). The
//!      vocabulary exists *only* in test files (see `eval::api::get_test_globals`
//!      and the loader's per-file globals selection in `eval::load`), so it
//!      can never leak into production `config.axl` / builtins.
//!
//!   2. **Convention discovery.** A test is a top-level `def test_*(t)`
//!      function. The runner enumerates the module's `test_*` callables —
//!      the same shape as `eval::task::FrozenTaskModuleLike::tasks()`, which
//!      filters module names by value kind.
//!
//!   3. **A bazel-free harness `t`.** Each test receives a zero-state handle
//!      `t` carrying assertions-adjacent fixtures: `t.env` (an in-memory
//!      environment overlay), `t.std` (the real `std` surface), and `t.ctx`
//!      (a *real* `TaskContext` — same Rust type production uses — wired over
//!      the mock backends). `t` deliberately has no bazel surface.
//!
//!   4. **Mock-by-backend-swap, not by masquerade.** `t.ctx.std.env` is the
//!      genuine `std.Env` type; it reads the in-memory overlay only because
//!      the runner installs a `test_env` on `eval.extra` for the duration of
//!      the test. The type is unchanged; only the backend it consults differs.
//!      This is what keeps the mock's contract identical to reality.
//!
//!   5. **Per-test isolation, run in parallel.** Each test runs with a fresh
//!      overlay; a failed assertion (which raises) is caught per-test and
//!      recorded, so one failure never aborts the run — pytest semantics.
//!      Tests fan out across `min(tests, cpus)` worker threads, each with its
//!      own Starlark heap (heaps are `!Send`), and results merge back into
//!      definition order. This is sound precisely because per-test state lives
//!      on the test's own values, never in a process-global — so concurrent
//!      workers share no mutable state.
//!
//! Not yet built (tracked in `docs/testing.md`): the `aspect test` runner as
//! an AXL task, snapshot golden files, fs/process/http mock backends, and the
//! LSP knowing about the `_test.axl` surface.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic, Module};
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;
use starlark::values::list::AllocList;
use starlark::values::none::{NoneOr, NoneType};
use starlark::values::{
    Heap, NoSerialize, ProvidesStaticType, StarlarkValue, Value, starlark_value,
};
use starlark::{starlark_module, starlark_simple_value, values};

use crate::engine::arguments::Arguments;
use crate::engine::bazel::Bazel;
use crate::engine::std::Std;
use crate::engine::store::{Env as RuntimeEnv, TestEnvMap};
use crate::engine::task_context::TaskContext;
use crate::engine::task_info::TaskInfo;
use crate::engine::trait_map::TraitMap;

// ─── The `asserts` namespace (test-only global) ──────────────────────────────

#[starlark_module]
fn assert_namespace(globals: &mut GlobalsBuilder) {
    /// Fails the test unless `got == want` (Starlark equality).
    fn eq<'v>(
        #[starlark(require = pos)] got: Value<'v>,
        #[starlark(require = pos)] want: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        let equal = got
            .equals(want)
            .map_err(|e| anyhow::anyhow!("asserts.eq: comparison failed: {e}"))?;
        if equal {
            Ok(NoneType)
        } else {
            Err(anyhow::anyhow!(
                "asserts.eq failed:\n  got:  {}\n  want: {}",
                got.to_repr(),
                want.to_repr()
            ))
        }
    }

    /// Fails the test unless `got != unwanted`.
    fn ne<'v>(
        #[starlark(require = pos)] got: Value<'v>,
        #[starlark(require = pos)] unwanted: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        let equal = got
            .equals(unwanted)
            .map_err(|e| anyhow::anyhow!("asserts.ne: comparison failed: {e}"))?;
        if equal {
            Err(anyhow::anyhow!(
                "asserts.ne failed: both values are {}",
                got.to_repr()
            ))
        } else {
            Ok(NoneType)
        }
    }

    /// Fails the test unless `value` is truthy.
    fn is_true<'v>(#[starlark(require = pos)] value: Value<'v>) -> anyhow::Result<NoneType> {
        if value.to_bool() {
            Ok(NoneType)
        } else {
            Err(anyhow::anyhow!(
                "asserts.is_true failed: {} is falsy",
                value.to_repr()
            ))
        }
    }

    /// Fails the test unless `value` is falsy.
    fn is_false<'v>(#[starlark(require = pos)] value: Value<'v>) -> anyhow::Result<NoneType> {
        if value.to_bool() {
            Err(anyhow::anyhow!(
                "asserts.is_false failed: {} is truthy",
                value.to_repr()
            ))
        } else {
            Ok(NoneType)
        }
    }

    /// Fails the test unless the `needle` substring appears in `haystack`.
    ///
    /// POC scope: string containment only. Collection membership
    /// (`needle in haystack` for lists/dicts) is a planned extension.
    fn contains<'v>(
        #[starlark(require = pos)] haystack: Value<'v>,
        #[starlark(require = pos)] needle: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        let h = haystack
            .unpack_str()
            .ok_or_else(|| anyhow::anyhow!("asserts.contains (POC): haystack must be a string"))?;
        let n = needle
            .unpack_str()
            .ok_or_else(|| anyhow::anyhow!("asserts.contains (POC): needle must be a string"))?;
        if h.contains(n) {
            Ok(NoneType)
        } else {
            Err(anyhow::anyhow!(
                "asserts.contains failed: {:?} not found in {:?}",
                n,
                h
            ))
        }
    }

    /// Fails the test unless calling `f()` raises. The pytest `raises` analogue
    /// for the common no-argument case.
    fn fails<'v>(
        #[starlark(require = pos)] f: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        match eval.eval_function(f, &[], &[]) {
            Ok(_) => Err(anyhow::anyhow!(
                "asserts.fails: expected the callable to raise, but it returned normally"
            )),
            Err(_) => Ok(NoneType),
        }
    }
}

/// Register the test-only globals onto a builder. Called by
/// `eval::api::get_test_globals` to build the surface used for `*_test.axl`
/// files and by the test runner.
///
/// NOTE: the namespace is `asserts`, not `assert` — `assert` is a reserved
/// keyword in the AXL/Starlark dialect and cannot be used as an identifier,
/// so the plural `asserts` is used instead.
pub fn register_test_globals(globals: &mut GlobalsBuilder) {
    globals.namespace("asserts", assert_namespace);
}

// ─── The harness value `t` ───────────────────────────────────────────────────

/// The per-test harness handed to every `def test_*(t)`. Carries no state
/// itself — its fixtures operate on the `test_env` overlay that the runner
/// installs on `eval.extra`, exactly as the real `std` surface reaches the
/// process via `eval.extra`.
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<Test>")]
pub struct Test {}

#[starlark_value(type = "Test")]
impl<'v> StarlarkValue<'v> for Test {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(test_methods)
    }
}

starlark_simple_value!(Test);

#[starlark_module]
fn test_methods(registry: &mut MethodsBuilder) {
    /// In-memory environment fixture for this test. Mutations are visible
    /// through `t.ctx.std.env` / `t.std.env` and never touch the real process.
    #[starlark(attribute)]
    fn env<'v>(#[allow(unused)] this: Value<'v>) -> anyhow::Result<TestEnv> {
        Ok(TestEnv {})
    }

    /// The real `std` surface (filesystem, env, io, …). Under the runner its
    /// env backend reads the test overlay.
    #[starlark(attribute)]
    fn std<'v>(#[allow(unused)] this: Value<'v>) -> anyhow::Result<Std> {
        Ok(Std {})
    }

    /// A real `TaskContext` wired over this test's mock backends. Same Rust
    /// type production uses, so functions annotated `ctx: TaskContext` accept
    /// it with no drift.
    #[starlark(attribute)]
    fn ctx<'v>(#[allow(unused)] this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let startup_flags = heap.alloc(AllocList(Vec::<String>::new()));
        let bazel = heap.alloc(Bazel { startup_flags });
        let args = heap.alloc(Arguments::new());
        let traits = heap.alloc(TraitMap::new());
        let task_info = heap.alloc(TaskInfo::new(
            "test".to_string(),
            Vec::new(),
            "test".to_string(),
            "test".to_string(),
        ));
        Ok(heap.alloc(TaskContext::new(args, traits, task_info, bazel)))
    }
}

/// The `t.env` fixture handle. Stateless — it mutates the `test_env` overlay
/// carried on `eval.extra`.
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<Test.env>")]
pub struct TestEnv {}

#[starlark_value(type = "Test.env")]
impl<'v> StarlarkValue<'v> for TestEnv {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(test_env_methods)
    }
}

starlark_simple_value!(TestEnv);

fn test_env_map<'v>(eval: &Evaluator<'v, '_, '_>) -> anyhow::Result<TestEnvMap> {
    let env = RuntimeEnv::from_eval(eval)?;
    env.test_env
        .clone()
        .ok_or_else(|| anyhow::anyhow!("t.env is only available under the AXL test runner"))
}

#[starlark_module]
fn test_env_methods(registry: &mut MethodsBuilder) {
    /// Set an environment variable in the in-memory overlay.
    fn set<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] key: values::StringValue<'v>,
        #[starlark(require = pos)] value: values::StringValue<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        test_env_map(eval)?
            .borrow_mut()
            .insert(key.as_str().to_string(), value.as_str().to_string());
        Ok(NoneType)
    }

    /// Read an environment variable from the overlay (`None` if unset).
    fn get<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] key: values::StringValue<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneOr<values::StringValue<'v>>> {
        let resolved = test_env_map(eval)?.borrow().get(key.as_str()).cloned();
        let heap = eval.heap();
        Ok(NoneOr::from_option(
            resolved.map(|v| heap.alloc_str(v.as_str())),
        ))
    }

    /// Remove a variable from the overlay.
    fn remove<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] key: values::StringValue<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        test_env_map(eval)?.borrow_mut().remove(key.as_str());
        Ok(NoneType)
    }

    /// Clear the overlay back to empty.
    fn reset<'v>(
        #[allow(unused)] this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        test_env_map(eval)?.borrow_mut().clear();
        Ok(NoneType)
    }
}

// ─── Discovery + runner ──────────────────────────────────────────────────────

/// Outcome of a single `test_*` function.
#[derive(Debug, Clone)]
pub struct TestOutcome {
    pub name: String,
    pub passed: bool,
    /// The failure message (assertion or unexpected error) when `!passed`.
    pub message: Option<String>,
}

/// Aggregate result of running a test module.
#[derive(Debug, Clone, Default)]
pub struct TestSummary {
    pub outcomes: Vec<TestOutcome>,
}

impl TestSummary {
    pub fn passed(&self) -> usize {
        self.outcomes.iter().filter(|o| o.passed).count()
    }

    pub fn failed(&self) -> usize {
        self.outcomes.iter().filter(|o| !o.passed).count()
    }

    /// Render a human-readable summary, the shape the `aspect test` AXL runner
    /// will eventually produce (here in Rust for the POC).
    pub fn report(&self) -> String {
        let mut out = String::new();
        for o in &self.outcomes {
            if o.passed {
                out.push_str(&format!("  ok   {}\n", o.name));
            } else {
                out.push_str(&format!("  FAIL {}\n", o.name));
                if let Some(msg) = &o.message {
                    for line in msg.lines() {
                        out.push_str(&format!("         {line}\n"));
                    }
                }
            }
        }
        out.push_str(&format!(
            "\n{} passed, {} failed\n",
            self.passed(),
            self.failed()
        ));
        out
    }
}

/// Parse `source` into the test AST against the test globals dialect.
fn parse_test_source(source: &str) -> anyhow::Result<AstModule> {
    AstModule::parse(
        "<axl-test>",
        source.to_string(),
        &crate::eval::api::dialect(),
    )
    .map_err(|e| anyhow::anyhow!("parse error: {e}"))
}

/// Enumerate the `test_*` callables in definition order. Evaluates the module
/// body once in a throwaway heap purely to learn the names; the result is
/// `Send` (plain `String`s), so the caller can shard it across worker threads
/// even though the `Module` itself is `!Send`.
fn discover_test_names(source: &str, base_env: &RuntimeEnv) -> anyhow::Result<Vec<String>> {
    let globals = crate::eval::api::get_test_globals();
    let ast = parse_test_source(source)?;
    Module::with_temp_heap(|module| -> anyhow::Result<Vec<String>> {
        let mut eval = Evaluator::new(&module);
        eval.extra = Some(base_env);
        eval.eval_module(ast, &globals)
            .map_err(|e| anyhow::anyhow!("eval error: {e}"))?;
        Ok(module
            .names()
            .map(|n| n.as_str().to_string())
            .filter(|n| n.starts_with("test_"))
            .filter(|n| {
                module
                    .get(n)
                    .map(|v| v.get_type() == "function")
                    .unwrap_or(false)
            })
            .collect())
    })
}

/// The `Send` slice of a [`RuntimeEnv`] needed to rebuild one on a worker
/// thread. `RuntimeEnv` itself is `!Send` (it carries an `Rc` overlay slot),
/// so we never move one across a thread boundary — instead each worker enters
/// the shared tokio runtime handle and mints its own `RuntimeEnv` locally.
#[derive(Clone)]
struct EnvSeed {
    cli_version: String,
    aspect_root: std::path::PathBuf,
    bazel_root: std::path::PathBuf,
    git_root: Option<std::path::PathBuf>,
    rt: tokio::runtime::Handle,
}

impl EnvSeed {
    fn from_env(env: &RuntimeEnv) -> Self {
        Self {
            cli_version: env.cli_version.clone(),
            aspect_root: env.aspect_root_dir.clone(),
            bazel_root: env.bazel_root_dir.clone(),
            git_root: env.git_root_dir.clone(),
            rt: env.rt.0.clone(),
        }
    }
}

/// Run a slice of tests (carrying their original indices, so results can be
/// merged back into definition order) in a fresh `Module` on the current
/// thread. Each test gets its own isolated env overlay; a raised error fails
/// that test and is recorded — the slice continues (pytest semantics).
fn run_shard(
    base_env: &RuntimeEnv,
    source: &str,
    shard: Vec<(usize, String)>,
) -> anyhow::Result<Vec<(usize, TestOutcome)>> {
    let globals = crate::eval::api::get_test_globals();
    let ast = parse_test_source(source)?;
    Module::with_temp_heap(|module| -> anyhow::Result<Vec<(usize, TestOutcome)>> {
        // Evaluate the module body — this only binds the `def test_*` functions;
        // a well-behaved test file performs no side effects at module scope,
        // which is also what makes re-evaluating it per worker thread safe.
        {
            let mut eval = Evaluator::new(&module);
            eval.extra = Some(base_env);
            eval.eval_module(ast, &globals)
                .map_err(|e| anyhow::anyhow!("eval error: {e}"))?;
        }

        let mut out = Vec::with_capacity(shard.len());
        for (idx, name) in shard {
            let f = match module.get(&name) {
                Some(f) => f,
                None => {
                    out.push((
                        idx,
                        TestOutcome {
                            name,
                            passed: false,
                            message: Some("test disappeared after discovery".to_string()),
                        },
                    ));
                    continue;
                }
            };

            // Fresh, isolated overlay per test; shared between `t.env` and the
            // `std.env` backend via this `eval.extra`. Because the overlay lives
            // on this thread's evaluator (not in any process-global), concurrent
            // workers never contend.
            let overlay: TestEnvMap = Rc::new(RefCell::new(BTreeMap::new()));
            let test_env = base_env.with_test_env(overlay);
            let t = module.heap().alloc(Test {});

            let mut eval = Evaluator::new(&module);
            eval.extra = Some(&test_env);
            let outcome = match eval.eval_function(f, &[t], &[]) {
                Ok(_) => TestOutcome {
                    name,
                    passed: true,
                    message: None,
                },
                Err(e) => TestOutcome {
                    name,
                    passed: false,
                    message: Some(e.to_string()),
                },
            };
            out.push((idx, outcome));
        }
        Ok(out)
    })
}

/// Default worker count: one per test, capped at the available parallelism.
fn default_jobs(n_tests: usize) -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    n_tests.min(cpus).max(1)
}

/// Run every `test_*` function defined in `source`, each with an isolated
/// in-memory environment overlay, across up to `min(tests, cpus)` worker
/// threads. Results are merged back into definition order so the report is
/// deterministic regardless of how tests were sharded.
///
/// `base_env` supplies the non-mocked context (roots, async runtime).
pub fn run_test_source(source: &str, base_env: &RuntimeEnv) -> anyhow::Result<TestSummary> {
    let names = discover_test_names(source, base_env)?;
    let jobs = default_jobs(names.len());
    run_test_source_with_jobs(source, base_env, names, jobs)
}

/// Like [`run_test_source`] but with an explicit worker count (the `--jobs`
/// knob). `jobs <= 1` runs serially on the calling thread; higher values fan
/// the tests out across that many threads, each with its own Starlark heap.
fn run_test_source_with_jobs(
    source: &str,
    base_env: &RuntimeEnv,
    names: Vec<String>,
    jobs: usize,
) -> anyhow::Result<TestSummary> {
    let jobs = jobs.max(1);

    // Serial fast path: no threads, run everything on the calling thread (which
    // is already inside the tokio runtime context the caller established).
    if jobs <= 1 || names.len() <= 1 {
        let shard: Vec<(usize, String)> = names.into_iter().enumerate().collect();
        let mut outcomes = run_shard(base_env, source, shard)?;
        outcomes.sort_by_key(|(idx, _)| *idx);
        return Ok(TestSummary {
            outcomes: outcomes.into_iter().map(|(_, o)| o).collect(),
        });
    }

    // Parallel path: shard test names round-robin so each worker gets a roughly
    // even mix, then rebuild an isolated `RuntimeEnv` + `Module` per thread. The
    // `Module`/heap is `!Send`, so each worker re-parses + re-evaluates the
    // (side-effect-free) module body locally rather than sharing one.
    let seed = EnvSeed::from_env(base_env);
    let source: std::sync::Arc<str> = std::sync::Arc::from(source);
    let mut shards: Vec<Vec<(usize, String)>> = vec![Vec::new(); jobs];
    for (idx, name) in names.into_iter().enumerate() {
        shards[idx % jobs].push((idx, name));
    }

    let handles: Vec<_> = shards
        .into_iter()
        .filter(|s| !s.is_empty())
        .map(|shard| {
            let seed = seed.clone();
            let source = source.clone();
            std::thread::spawn(move || -> anyhow::Result<Vec<(usize, TestOutcome)>> {
                // Enter the shared runtime so `RuntimeEnv::new`'s `Handle::current()`
                // resolves, then mint this thread's own env (no `Rc` crosses here).
                let _guard = seed.rt.enter();
                let env = RuntimeEnv::new(
                    seed.cli_version,
                    seed.aspect_root,
                    seed.bazel_root,
                    seed.git_root,
                );
                run_shard(&env, &source, shard)
            })
        })
        .collect();

    let mut merged: Vec<(usize, TestOutcome)> = Vec::new();
    for h in handles {
        let shard_outcomes = h
            .join()
            .map_err(|_| anyhow::anyhow!("a test worker thread panicked"))??;
        merged.extend(shard_outcomes);
    }
    merged.sort_by_key(|(idx, _)| *idx);
    Ok(TestSummary {
        outcomes: merged.into_iter().map(|(_, o)| o).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tokio::runtime::Runtime;

    fn base_env() -> RuntimeEnv {
        // `Env::new` builds an `AsyncRuntime`, which requires a tokio runtime
        // to be entered (matching `crate::test`'s setup).
        RuntimeEnv::new(
            "test".to_string(),
            PathBuf::from("/"),
            PathBuf::from("/"),
            None,
        )
    }

    #[test]
    fn discovers_and_runs_test_functions() {
        let rt = Runtime::new().unwrap();
        let _g = rt.enter();
        let src = r#"
def test_env_overlay_is_isolated(t):
    # overlay starts empty; reads go through the real std.Env type
    asserts.eq(t.std.env.var("BUILDKITE"), None)
    t.env.set("BUILDKITE", "true")
    # same overlay observed through both t.std and a real TaskContext (t.ctx)
    asserts.eq(t.std.env.var("BUILDKITE"), "true")
    asserts.eq(t.ctx.std.env.var("BUILDKITE"), "true")

def test_env_set_and_remove(t):
    t.env.set("FOO", "1")
    asserts.eq(t.env.get("FOO"), "1")
    t.env.remove("FOO")
    asserts.eq(t.env.get("FOO"), None)

def test_contains_and_truthy(t):
    asserts.contains("hello world", "world")
    asserts.is_true(1 == 1)
    asserts.fails(lambda: fail("boom"))

def test_intentional_failure(t):
    asserts.eq(1, 2)

def helper_not_discovered(t):
    fail("helper_* is not a test and must not run")
"#;

        let summary = run_test_source(src, &base_env()).expect("runner should not error");
        eprintln!("{}", summary.report());

        let names: Vec<&str> = summary.outcomes.iter().map(|o| o.name.as_str()).collect();
        assert!(
            !names.contains(&"helper_not_discovered"),
            "only test_* functions should be discovered, got {names:?}"
        );
        assert_eq!(summary.outcomes.len(), 4, "expected 4 discovered tests");
        assert_eq!(summary.passed(), 3, "report:\n{}", summary.report());
        assert_eq!(summary.failed(), 1, "report:\n{}", summary.report());

        let failure = summary
            .outcomes
            .iter()
            .find(|o| !o.passed)
            .expect("one failing test");
        assert_eq!(failure.name, "test_intentional_failure");
        assert!(
            failure
                .message
                .as_deref()
                .unwrap_or("")
                .contains("asserts.eq failed"),
            "failure should carry the assertion message, got {:?}",
            failure.message
        );
    }

    #[test]
    fn runs_tests_in_parallel_shards() {
        let rt = Runtime::new().unwrap();
        let _g = rt.enter();
        // Many tests, each mutating its own overlay under the same key. If the
        // overlay leaked across the concurrent workers, the cross-checks below
        // would observe another test's value and fail. The intentional failure
        // (test_xxx_fail) also proves per-test capture survives sharding.
        let mut src = String::new();
        for i in 0..16 {
            src.push_str(&format!(
                "def test_iso_{i:02}(t):\n    \
                   asserts.eq(t.std.env.var(\"K\"), None)\n    \
                   t.env.set(\"K\", \"{i}\")\n    \
                   asserts.eq(t.ctx.std.env.var(\"K\"), \"{i}\")\n\n"
            ));
        }
        src.push_str("def test_zzz_fail(t):\n    asserts.eq(1, 2)\n");

        let names = discover_test_names(&src, &base_env()).expect("discovery ok");
        assert_eq!(names.len(), 17, "16 isolation tests + 1 failure");

        // Force the parallel path with several workers.
        let summary = run_test_source_with_jobs(&src, &base_env(), names, 8)
            .expect("parallel runner should not error");

        assert_eq!(summary.passed(), 16, "report:\n{}", summary.report());
        assert_eq!(summary.failed(), 1, "report:\n{}", summary.report());

        // Results are merged back into definition order regardless of sharding.
        let ordered: Vec<&str> = summary.outcomes.iter().map(|o| o.name.as_str()).collect();
        let mut expected: Vec<String> = (0..16).map(|i| format!("test_iso_{i:02}")).collect();
        expected.push("test_zzz_fail".to_string());
        assert_eq!(
            ordered,
            expected.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            "outcomes must be in definition order, not completion order"
        );
    }

    #[test]
    fn asserts_surface_is_test_only() {
        // The `asserts` namespace exists in the test surface…
        let test_globals = crate::eval::api::get_test_globals();
        assert!(
            test_globals.names().any(|n| n.as_str() == "asserts"),
            "test globals must expose `asserts`"
        );
        // …and is absent from the production surface, so it can never leak
        // into config.axl / builtins.
        let prod_globals = crate::eval::get_globals().build();
        assert!(
            !prod_globals.names().any(|n| n.as_str() == "asserts"),
            "production globals must NOT expose `asserts`"
        );
    }

    #[test]
    fn overlay_does_not_leak_into_process() {
        let rt = Runtime::new().unwrap();
        let _g = rt.enter();
        // Sanity: a test setting an env var through the overlay must not mutate
        // the real process environment.
        let src = r#"
def test_sets_overlay(t):
    t.env.set("AXL_POC_LEAK_CHECK", "should-not-leak")
    asserts.eq(t.std.env.var("AXL_POC_LEAK_CHECK"), "should-not-leak")
"#;
        let summary = run_test_source(src, &base_env()).expect("runner ok");
        assert_eq!(summary.passed(), 1, "report:\n{}", summary.report());
        assert!(
            std::env::var("AXL_POC_LEAK_CHECK").is_err(),
            "overlay must not leak into the real process env"
        );
    }
}
