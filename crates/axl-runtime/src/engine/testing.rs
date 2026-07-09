//! Built-in test framework for AXL.
//!
//! Gives AXL a native, pytest-style testing story, backing `aspect axl test`.
//! The load-bearing decisions:
//!
//!   1. **Test-only globals.** `*.test.axl` files are evaluated against an
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
//!      genuine `std.Env` type; it reads the in-memory overlay because the
//!      `std.Env` value is *minted carrying* the test's overlay `Rc` (the
//!      mock route is value-carried, not ambient on `eval.extra`). The type
//!      and its method table are unchanged; only the map a given instance
//!      consults differs. This is what keeps the mock's contract identical to
//!      reality — and the one shared `Rc` is what makes `t.env`, `t.std.env`,
//!      and `t.ctx.std.env` observe the same map.
//!
//!   5. **Per-test isolation.** Each test runs with a fresh overlay; a failed
//!      assertion (which raises) is caught per-test and recorded, so one
//!      failure never aborts the run — pytest semantics.
//!
//! ## Two runners
//!
//! * [`run_tests`] backs `aspect axl test`. The CLI driver holds the live
//!   `AxlLoader`, so each `*.test.axl` file is loaded *through the normal load
//!   path* — resolving the file's own `load(...)`s — then its `test_*`
//!   functions run against the harness. This is what lets a test file import
//!   the module it exercises.
//! * [`run_test_source`] is the in-process engine used by the Rust unit tests
//!   in this module: it parses inline source (no loader) and fans tests out
//!   across `min(tests, cpus)` worker threads, each with its own `!Send` heap,
//!   merging results back into definition order. It exercises the harness,
//!   isolation, and the bazel `Fake` backend without touching the filesystem.
//!
//! Not yet built (tracked in `docs/testing.md`): snapshot golden files,
//! fs/process/http mock backends, and the LSP knowing about the `.test.axl`
//! surface.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::{
    FrozenModule, GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic, Module,
};
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;
use starlark::values::none::{NoneOr, NoneType};
use starlark::values::tuple::UnpackTuple;
use starlark::values::{
    Heap, NoSerialize, ProvidesStaticType, StarlarkValue, Value, ValueLike, starlark_value,
};
use starlark::{starlark_module, starlark_simple_value, values};

use crate::engine::arguments::Arguments;
use crate::engine::bazel::Bazel;
use crate::engine::bazel::backend::BazelBackend;
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

    /// Fails the test unless `needle` is a member of `haystack`.
    ///
    /// Backed by the `in` operator, so it works for any container Starlark
    /// supports: a substring of a string, an element of a list/tuple/set, or a
    /// key of a dict.
    fn contains<'v>(
        #[starlark(require = pos)] haystack: Value<'v>,
        #[starlark(require = pos)] needle: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        let present = haystack
            .is_in(needle)
            .map_err(|e| anyhow::anyhow!("asserts.contains: membership test failed: {e}"))?;
        if present {
            Ok(NoneType)
        } else {
            Err(anyhow::anyhow!(
                "asserts.contains failed: {} not found in {}",
                needle.to_repr(),
                haystack.to_repr()
            ))
        }
    }

    /// Fails the test unless `needle` is *absent* from `haystack` — the inverse
    /// of `contains`, over the same containers.
    fn not_contains<'v>(
        #[starlark(require = pos)] haystack: Value<'v>,
        #[starlark(require = pos)] needle: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        let present = haystack
            .is_in(needle)
            .map_err(|e| anyhow::anyhow!("asserts.not_contains: membership test failed: {e}"))?;
        if present {
            Err(anyhow::anyhow!(
                "asserts.not_contains failed: {} unexpectedly found in {}",
                needle.to_repr(),
                haystack.to_repr()
            ))
        } else {
            Ok(NoneType)
        }
    }

    /// Fails the test unless `got > want` (Starlark ordering).
    fn gt<'v>(
        #[starlark(require = pos)] got: Value<'v>,
        #[starlark(require = pos)] want: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        compare_assert(got, want, "gt", &[Ordering::Greater])
    }

    /// Fails the test unless `got >= want` (Starlark ordering).
    fn ge<'v>(
        #[starlark(require = pos)] got: Value<'v>,
        #[starlark(require = pos)] want: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        compare_assert(got, want, "ge", &[Ordering::Greater, Ordering::Equal])
    }

    /// Fails the test unless `got < want` (Starlark ordering).
    fn lt<'v>(
        #[starlark(require = pos)] got: Value<'v>,
        #[starlark(require = pos)] want: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        compare_assert(got, want, "lt", &[Ordering::Less])
    }

    /// Fails the test unless `got <= want` (Starlark ordering).
    fn le<'v>(
        #[starlark(require = pos)] got: Value<'v>,
        #[starlark(require = pos)] want: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        compare_assert(got, want, "le", &[Ordering::Less, Ordering::Equal])
    }

    /// Fails the test unless calling `f()` raises. The pytest `raises` analogue
    /// for the common no-argument case. Pass `contains = "substr"` to also
    /// require the raised error's message to contain that substring — the
    /// common "it failed *for the right reason*" check.
    fn fails<'v>(
        #[starlark(require = pos)] f: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] contains: NoneOr<&str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        match eval.eval_function(f, &[], &[]) {
            Ok(_) => Err(anyhow::anyhow!(
                "asserts.fails: expected the callable to raise, but it returned normally"
            )),
            Err(e) => match contains {
                NoneOr::Other(sub) => {
                    // Match against the bare error message, not the rendered
                    // diagnostic — the full `Display` embeds the source frame
                    // (including this very `asserts.fails(...)` call), which
                    // would let the call's own text spuriously satisfy the
                    // substring check.
                    let msg = e.without_diagnostic().to_string();
                    if msg.contains(sub) {
                        Ok(NoneType)
                    } else {
                        Err(anyhow::anyhow!(
                            "asserts.fails: raised error did not contain {sub:?}\n  error: {msg}"
                        ))
                    }
                }
                NoneOr::None => Ok(NoneType),
            },
        }
    }
}

/// Shared body for the ordering asserts (`gt`/`ge`/`lt`/`le`). Compares `got`
/// against `want` and passes only when the resulting [`Ordering`] is in
/// `allowed`; otherwise raises a message naming the failed `op`.
fn compare_assert<'v>(
    got: Value<'v>,
    want: Value<'v>,
    op: &str,
    allowed: &[Ordering],
) -> anyhow::Result<NoneType> {
    let ord = got
        .compare(want)
        .map_err(|e| anyhow::anyhow!("asserts.{op}: comparison failed: {e}"))?;
    if allowed.contains(&ord) {
        Ok(NoneType)
    } else {
        Err(anyhow::anyhow!(
            "asserts.{op} failed:\n  got:  {}\n  want: {}",
            got.to_repr(),
            want.to_repr()
        ))
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

/// The per-test harness handed to every `def test_*(t)`. Carries the test's
/// in-memory env overlay **on the value itself**: `t.env`, `t.std`, and
/// `t.ctx.std.env` are all minted from this one shared `Rc`, so a mutation
/// through any of them is observed through the others. Nothing is fished out
/// of `eval.extra` — the mock route is value-carried, never ambient.
///
/// Its bazel fixture is likewise value-carried: a per-test
/// [`PendingExpectation`] cell + the located fake-bazel binary, so a declared
/// `t.bazel.expect_build(...)` reaches the `Fake` backend minted by `t.ctx`
/// (state on the value, never a global — decisions 6/8).
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<Test>")]
pub struct Test {
    #[allocative(skip)]
    overlay: TestEnvMap,
    /// Fake-bazel binary path. `None` → `t.ctx.bazel` is `Real` (production
    /// behavior; the bazel mock is opt-in per the runner that constructs it).
    #[allocative(skip)]
    fake_bin: Option<Arc<str>>,
    /// The expectation declared by `t.bazel.expect_build(...)` for the next
    /// `t.ctx.bazel.build(...)`. Shared (by `Arc` clone) with the `t.bazel`
    /// handle and read by `t.ctx`.
    #[allocative(skip)]
    expectation: PendingExpectation,
}

/// Shared per-test cell holding the declared `BazelExpectation`, if any.
type PendingExpectation = Arc<Mutex<Option<basil_core::BazelExpectation>>>;

impl Test {
    /// A harness carrying `overlay`. `fake_bin = Some` wires `t.ctx.bazel` to a
    /// `Fake` backend driving that binary; `None` leaves it `Real`.
    fn new(overlay: TestEnvMap, fake_bin: Option<Arc<str>>) -> Self {
        Self {
            overlay,
            fake_bin,
            expectation: Arc::new(Mutex::new(None)),
        }
    }
}

#[starlark_value(type = "Test")]
impl<'v> StarlarkValue<'v> for Test {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(test_methods)
    }
}

starlark_simple_value!(Test);

/// Read the shared overlay `Rc` carried on a `t` (`Test`) value.
fn test_overlay<'v>(this: Value<'v>) -> anyhow::Result<TestEnvMap> {
    let t = this
        .downcast_ref::<Test>()
        .ok_or_else(|| anyhow::anyhow!("test harness method called on a non-Test value"))?;
    Ok(t.overlay.clone())
}

#[starlark_module]
fn test_methods(registry: &mut MethodsBuilder) {
    /// In-memory environment fixture for this test. Mutations are visible
    /// through `t.ctx.std.env` / `t.std.env` and never touch the real process —
    /// all three share the one overlay `Rc` carried on `t`.
    #[starlark(attribute)]
    fn env<'v>(this: Value<'v>) -> anyhow::Result<TestEnv> {
        Ok(TestEnv {
            overlay: test_overlay(this)?,
        })
    }

    /// The real `std` surface (filesystem, env, io, …). Its `env` reads/writes
    /// this test's overlay because the `Std` is minted carrying that `Rc`.
    #[starlark(attribute)]
    fn std<'v>(this: Value<'v>) -> anyhow::Result<Std> {
        Ok(Std::with_env_overlay(test_overlay(this)?))
    }

    /// The bazel mock fixture. Declare expected invocations with
    /// `t.bazel.expect_build(...)`; the declaration is consumed by the next
    /// `t.ctx.bazel.build(...)` via the `Fake` backend.
    #[starlark(attribute)]
    fn bazel<'v>(this: Value<'v>) -> anyhow::Result<TestBazel> {
        let t = this
            .downcast_ref::<Test>()
            .ok_or_else(|| anyhow::anyhow!("t.bazel called on a non-Test value"))?;
        Ok(TestBazel {
            expectation: t.expectation.clone(),
        })
    }

    /// A real `TaskContext` wired over this test's mock backends. Same Rust
    /// type production uses, so functions annotated `ctx: TaskContext` accept
    /// it with no drift. The context carries this test's overlay `Rc`, so
    /// `t.ctx.std.env` observes the same map as `t.env` / `t.std.env`.
    #[starlark(attribute)]
    fn ctx<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let t = this
            .downcast_ref::<Test>()
            .ok_or_else(|| anyhow::anyhow!("t.ctx called on a non-Test value"))?;
        let overlay = t.overlay.clone();
        // Mint the bazel backend from the harness: a `Fake` pointing at the
        // located fake binary + the expectation declared so far (defaulting to
        // a clean passing build), or `Real` when no fake binary was installed.
        let backend = match &t.fake_bin {
            Some(fake_bin) => {
                let exp = t.expectation.lock().unwrap().clone().unwrap_or_else(|| {
                    basil_core::BazelExpectation::new(
                        Vec::new(),
                        basil_core::BuildResult::Passed,
                        None,
                    )
                });
                BazelBackend::Fake {
                    fake_bin: fake_bin.to_string(),
                    expectation: Arc::new(exp),
                }
            }
            None => BazelBackend::Real,
        };
        let bazel = heap.alloc(Bazel {
            active_rc: std::cell::RefCell::new(None),
            backend,
        });
        let args = heap.alloc(Arguments::new());
        let traits = heap.alloc(TraitMap::new());
        let task_info = heap.alloc(TaskInfo::new(
            "test".to_string(),
            Vec::new(),
            "test".to_string(),
            "test".to_string(),
        ));
        Ok(heap.alloc(TaskContext::new(args, traits, task_info, bazel).with_env_overlay(overlay)))
    }
}

// ─── The `t.bazel` fixture ────────────────────────────────────────────────────

/// The `t.bazel` fixture handle. Carries (by `Arc` clone) the per-test
/// expectation cell that `t.ctx`'s `Fake` backend reads.
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<Test.bazel>")]
pub struct TestBazel {
    #[allocative(skip)]
    expectation: PendingExpectation,
}

starlark_simple_value!(TestBazel);

#[starlark_value(type = "Test.bazel")]
impl<'v> StarlarkValue<'v> for TestBazel {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(test_bazel_methods)
    }
}

/// Map the AXL `result=` string onto the typed [`basil_core::BuildResult`].
/// Mirrors a `BuildResult` enum (`passed | failed | cache_evicted`); an unknown
/// value fails fast with the legal set, exactly as an `enum(...)` would.
fn parse_build_result(s: &str) -> anyhow::Result<basil_core::BuildResult> {
    match s {
        "passed" => Ok(basil_core::BuildResult::Passed),
        "failed" => Ok(basil_core::BuildResult::Failed),
        "cache_evicted" => Ok(basil_core::BuildResult::CacheEvicted),
        other => Err(anyhow::anyhow!(
            "t.bazel.expect_build: unknown result {other:?}; expected one of \
             \"passed\", \"failed\", \"cache_evicted\""
        )),
    }
}

#[starlark_module]
fn test_bazel_methods(registry: &mut MethodsBuilder) {
    /// Declare the expected outcome of the next `t.ctx.bazel.build(...)`.
    ///
    /// The fake bazel synthesizes a consistent BES stream from this:
    /// `BuildStarted` → one `TargetComplete` per target (pass/fail per
    /// `result`) → `BuildFinished` carrying the exit code — then exits with
    /// that code. The parent reads it back through the real
    /// `ctx.bazel.build` BES path.
    ///
    /// # Arguments
    /// * `targets` - Target patterns the build "covers"; one `TargetComplete`
    ///   per entry.
    /// * `result` - `"passed"` | `"failed"` | `"cache_evicted"`.
    /// * `exit_code` - Override the process exit code (default: derived from
    ///   `result` — 0 / 1 / 39).
    fn expect_build<'v>(
        this: Value<'v>,
        #[starlark(args)] targets: UnpackTuple<values::StringValue<'v>>,
        #[starlark(require = named, default = "passed")] result: &str,
        #[starlark(require = named, default = NoneOr::None)] exit_code: NoneOr<i32>,
    ) -> anyhow::Result<NoneType> {
        let tb = this
            .downcast_ref::<TestBazel>()
            .ok_or_else(|| anyhow::anyhow!("expected Test.bazel"))?;
        let result = parse_build_result(result)?;
        let targets: Vec<String> = targets
            .items
            .iter()
            .map(|t| t.as_str().to_string())
            .collect();
        let exp = basil_core::BazelExpectation::new(targets, result, exit_code.into_option());
        *tb.expectation.lock().unwrap() = Some(exp);
        Ok(NoneType)
    }
}

/// The `t.env` fixture handle. Carries the test's overlay `Rc` directly, so
/// its mutations are observed through `t.std.env` / `t.ctx.std.env`.
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<Test.env>")]
pub struct TestEnv {
    #[allocative(skip)]
    overlay: TestEnvMap,
}

#[starlark_value(type = "Test.env")]
impl<'v> StarlarkValue<'v> for TestEnv {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(test_env_methods)
    }
}

starlark_simple_value!(TestEnv);

/// Read the overlay `Rc` carried on a `t.env` (`TestEnv`) value.
fn test_env_map<'v>(this: Value<'v>) -> anyhow::Result<TestEnvMap> {
    let env = this
        .downcast_ref::<TestEnv>()
        .ok_or_else(|| anyhow::anyhow!("t.env method called on a non-Test.env value"))?;
    Ok(env.overlay.clone())
}

#[starlark_module]
fn test_env_methods(registry: &mut MethodsBuilder) {
    /// Set an environment variable in the in-memory overlay.
    fn set<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] key: values::StringValue<'v>,
        #[starlark(require = pos)] value: values::StringValue<'v>,
    ) -> anyhow::Result<NoneType> {
        test_env_map(this)?
            .lock()
            .unwrap()
            .insert(key.as_str().to_string(), value.as_str().to_string());
        Ok(NoneType)
    }

    /// Read an environment variable from the overlay (`None` if unset).
    fn get<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] key: values::StringValue<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneOr<values::StringValue<'v>>> {
        let resolved = test_env_map(this)?
            .lock()
            .unwrap()
            .get(key.as_str())
            .cloned();
        let heap = eval.heap();
        Ok(NoneOr::from_option(
            resolved.map(|v| heap.alloc_str(v.as_str())),
        ))
    }

    /// Remove a variable from the overlay.
    fn remove<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] key: values::StringValue<'v>,
    ) -> anyhow::Result<NoneType> {
        test_env_map(this)?.lock().unwrap().remove(key.as_str());
        Ok(NoneType)
    }

    /// Clear the overlay back to empty.
    fn reset<'v>(this: Value<'v>) -> anyhow::Result<NoneType> {
        test_env_map(this)?.lock().unwrap().clear();
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
/// thread. We never move a `RuntimeEnv` (or the per-test overlay) across a
/// thread boundary — the `Module`/heap is `!Send` anyway, so each worker
/// enters the shared tokio runtime handle, mints its own `RuntimeEnv`, and
/// builds its own per-test overlays locally.
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
    fake_bin: Option<Arc<str>>,
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

            // Fresh, isolated overlay per test, carried directly on the harness
            // value `t`. `t.env`, `t.std`, and `t.ctx.std.env` are all minted
            // from this one `Rc`, so they observe one shared map — and because
            // the overlay lives on the value (not in any process-global or
            // ambient `eval.extra`), concurrent workers never contend.
            //
            // `base_env` is still installed on `eval.extra` for the *production*
            // reads `std.env` makes (`aspect_cli_version`, roots) via
            // `RuntimeEnv::from_eval`; only the env-overlay route moved onto the
            // value.
            let overlay: TestEnvMap = Arc::new(Mutex::new(BTreeMap::new()));
            // Fresh harness per test: its own expectation cell, so a declared
            // `t.bazel.expect_build(...)` in one test never bleeds into another
            // (parallel-safe — state lives on the value, not a global).
            let t = module.heap().alloc(Test::new(overlay, fake_bin.clone()));

            let mut eval = Evaluator::new(&module);
            eval.extra = Some(base_env);
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
    run_test_source_with_jobs(source, base_env, names, jobs, None)
}

/// Like [`run_test_source`] but installs a fake-bazel binary so
/// `t.ctx.bazel.build(...)` drives the `Fake` backend (spawns `fake_bin`,
/// feeds it the declared `BazelExpectation`). `fake_bin` is located the way
/// `crate::test::basil_bin` does today; a shipped self-exec subcommand is
/// roadmap item 6.
pub fn run_test_source_with_fake_bazel(
    source: &str,
    base_env: &RuntimeEnv,
    fake_bin: Arc<str>,
) -> anyhow::Result<TestSummary> {
    let names = discover_test_names(source, base_env)?;
    let jobs = default_jobs(names.len());
    run_test_source_with_jobs(source, base_env, names, jobs, Some(fake_bin))
}

/// Like [`run_test_source`] but with an explicit worker count (the `--jobs`
/// knob). `jobs <= 1` runs serially on the calling thread; higher values fan
/// the tests out across that many threads, each with its own Starlark heap.
/// `fake_bin`, when set, wires every harness's `t.ctx.bazel` to a `Fake`
/// backend driving that binary.
fn run_test_source_with_jobs(
    source: &str,
    base_env: &RuntimeEnv,
    names: Vec<String>,
    jobs: usize,
    fake_bin: Option<Arc<str>>,
) -> anyhow::Result<TestSummary> {
    let jobs = jobs.max(1);

    // Serial fast path: no threads, run everything on the calling thread (which
    // is already inside the tokio runtime context the caller established).
    if jobs <= 1 || names.len() <= 1 {
        let shard: Vec<(usize, String)> = names.into_iter().enumerate().collect();
        let mut outcomes = run_shard(base_env, source, shard, fake_bin)?;
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
            let fake_bin = fake_bin.clone();
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
                run_shard(&env, &source, shard, fake_bin)
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

// ─── Native `aspect axl test` runner ─────────────────────────────────────────
//
// `aspect axl test` is served natively rather than by AXL: the CLI driver holds
// the live `AxlLoader`, so it can load each `*.test.axl` file *through the normal
// load path* (resolving the file's own `load(...)`s), then run its `test_*`
// functions against the same harness the in-process runner uses. This is why a
// test file can `load()` the module it exercises — it is loaded exactly like any
// other AXL module.

/// True for a `*.test.axl` filename.
fn is_test_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".test.axl"))
}

/// Bound on directory-walk iterations; directory symlinks are not followed
/// (`file_type` reports the link, not its target), so this only guards against
/// a pathologically deep real tree.
const WALK_LIMIT: usize = 1_000_000;

/// Every `*.test.axl` file at or under each root (a file or directory),
/// de-duplicated and sorted for a deterministic report. Hidden directories
/// (`.git`, …) and directory symlinks (Bazel output trees) are skipped.
fn discover_test_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found: BTreeSet<PathBuf> = BTreeSet::new();
    for root in roots {
        let Ok(meta) = std::fs::metadata(root) else {
            continue;
        };
        if !meta.is_dir() {
            if is_test_file(root) {
                found.insert(root.clone());
            }
            continue;
        }
        let mut stack = vec![root.clone()];
        let mut budget = WALK_LIMIT;
        while let Some(dir) = stack.pop() {
            if budget == 0 {
                break;
            }
            budget -= 1;
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let Ok(ft) = entry.file_type() else { continue };
                if ft.is_dir() {
                    if !name.starts_with('.') {
                        stack.push(entry.path());
                    }
                } else if name.ends_with(".test.axl") {
                    found.insert(entry.path());
                }
            }
        }
    }
    found.into_iter().collect()
}

/// Run every top-level `def test_*(t)` in an already-loaded test module, each
/// with a fresh harness, sequentially on a scratch heap. `base_env` backs the
/// production reads `t.std`/`t.ctx` make (roots, cli version).
fn run_frozen_module(
    module: &FrozenModule,
    base_env: &RuntimeEnv,
) -> anyhow::Result<TestSummary> {
    let mut names: Vec<String> = module
        .names()
        .map(|s| s.as_str().to_string())
        .filter(|n| n.starts_with("test_"))
        .filter(|n| {
            module
                .get(n)
                .ok()
                .map(|o| o.value().to_value().get_type() == "function")
                .unwrap_or(false)
        })
        .collect();
    names.sort();

    let mut outcomes = Vec::with_capacity(names.len());
    Module::with_temp_heap(|live| -> anyhow::Result<()> {
        for name in names {
            let owned = module
                .get(&name)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let func = live.heap().access_owned_frozen_value(&owned);
            // Fresh, isolated overlay per test (same contract as the in-process
            // runner); real bazel (`fake_bin = None`).
            let overlay: TestEnvMap = Arc::new(Mutex::new(BTreeMap::new()));
            let t = live.heap().alloc(Test::new(overlay, None));
            let mut eval = Evaluator::new(&live);
            eval.extra = Some(base_env);
            let outcome = match eval.eval_function(func, &[t], &[]) {
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
            outcomes.push(outcome);
        }
        Ok(())
    })?;
    Ok(TestSummary { outcomes })
}

/// ANSI palette; blank when stdout is not a TTY so piped output stays clean.
struct Palette {
    green: &'static str,
    red: &'static str,
    dim: &'static str,
    bold: &'static str,
    reset: &'static str,
}

impl Palette {
    fn detect() -> Self {
        use std::io::IsTerminal;
        if std::io::stdout().is_terminal() {
            Palette {
                green: "\x1b[32m",
                red: "\x1b[31m",
                dim: "\x1b[2m",
                bold: "\x1b[1m",
                reset: "\x1b[0m",
            }
        } else {
            Palette {
                green: "",
                red: "",
                dim: "",
                bold: "",
                reset: "",
            }
        }
    }
}

/// Discover, load, and run every `*.test.axl` file under `paths` (defaulting to
/// the workspace root), printing a per-file report. Returns the process exit
/// code: `0` when everything passed, `1` on any test failure or load error.
///
/// Loading goes through the caller's live `loader`, so a test file's own
/// `load(...)`s resolve exactly as they would in any other module — this is the
/// whole point of running natively where the loader is in hand.
pub fn run_tests(loader: &crate::eval::Loader, paths: &[String]) -> anyhow::Result<i32> {
    let root = loader.aspect_root();
    let roots: Vec<PathBuf> = if paths.is_empty() {
        vec![root.to_path_buf()]
    } else {
        paths.iter().map(PathBuf::from).collect()
    };
    let files = discover_test_files(&roots);
    let pal = Palette::detect();

    if files.is_empty() {
        println!("No *.test.axl files found.");
        return Ok(0);
    }

    let rel = |p: &Path| p.strip_prefix(root).unwrap_or(p).display().to_string();
    let (mut total_pass, mut total_fail, mut file_errors) = (0usize, 0usize, 0usize);

    for path in &files {
        match loader.load_file(path) {
            Err(e) => {
                println!("{}{}ERROR{} {}", pal.red, pal.bold, pal.reset, rel(path));
                for line in format!("{e:#}").lines() {
                    println!("    {line}");
                }
                file_errors += 1;
            }
            Ok(module) => {
                let summary = run_frozen_module(&module, &loader.env)?;
                let (p, f) = (summary.passed(), summary.failed());
                total_pass += p;
                total_fail += f;
                let status = if f == 0 {
                    format!("{}ok  {}", pal.green, pal.reset)
                } else {
                    format!("{}FAIL{}", pal.red, pal.reset)
                };
                println!(
                    "{} {}{}{} ({} passed, {} failed)",
                    status,
                    pal.dim,
                    rel(path),
                    pal.reset,
                    p,
                    f
                );
                for o in &summary.outcomes {
                    if !o.passed {
                        println!("    {}FAIL{} {}", pal.red, pal.reset, o.name);
                        if let Some(m) = &o.message {
                            for line in m.lines() {
                                println!("         {line}");
                            }
                        }
                    }
                }
            }
        }
    }

    println!();
    let mut line = format!(
        "{} passed, {} failed across {} file(s)",
        total_pass,
        total_fail,
        files.len()
    );
    if file_errors > 0 {
        line.push_str(&format!(", {file_errors} file error(s)"));
    }
    println!("{}{}{}", pal.bold, line, pal.reset);

    Ok(if total_fail == 0 && file_errors == 0 {
        0
    } else {
        1
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let summary = run_test_source_with_jobs(&src, &base_env(), names, 8, None)
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
    fn discover_finds_dot_test_axl_and_skips_hidden() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join("sub")).unwrap();
        std::fs::create_dir(root.join(".hidden")).unwrap();
        std::fs::write(root.join("a.test.axl"), "").unwrap();
        std::fs::write(root.join("sub/b.test.axl"), "").unwrap();
        // `_test.axl` (the old convention) must NOT match the `.test.axl` suffix.
        std::fs::write(root.join("sub/legacy_test.axl"), "").unwrap();
        std::fs::write(root.join(".hidden/c.test.axl"), "").unwrap();

        let files = discover_test_files(&[root.to_path_buf()]);
        let names: Vec<String> = files
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().display().to_string())
            .collect();
        assert_eq!(names, vec!["a.test.axl".to_string(), "sub/b.test.axl".to_string()]);
    }

    /// The load-bearing property of the native runner: a `*.test.axl` file can
    /// `load()` the module it exercises, because the runner loads it through the
    /// real `AxlLoader` (resolving its `load()`s) rather than re-parsing source
    /// with no loader.
    #[test]
    fn native_runner_loads_module_under_test() {
        let rt = Runtime::new().unwrap();
        let _g = rt.enter();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("lib.axl"), "def double(x):\n    return x * 2\n").unwrap();
        std::fs::write(
            root.join("math.test.axl"),
            "load(\"./lib.axl\", \"double\")\n\
             \n\
             def test_double_ok(t):\n    asserts.eq(double(21), 42)\n\
             \n\
             def test_double_wrong(t):\n    asserts.eq(double(1), 3)\n",
        )
        .unwrap();

        let root_mod = crate::module::Mod::new(
            root.to_path_buf(),
            crate::module::AXL_ROOT_MODULE_NAME.to_string(),
            root.to_path_buf(),
        );
        let modules: Vec<crate::module::Mod> = vec![];
        let loader = crate::eval::Loader::new(
            "test".to_string(),
            root.to_path_buf(),
            root.to_path_buf(),
            None,
            &root_mod,
            &modules,
        );

        let module = loader
            .load_file(&root.join("math.test.axl"))
            .expect("test file (and its load()) should load");
        let summary = run_frozen_module(&module, &base_env()).expect("run ok");
        assert_eq!(summary.passed(), 1, "report:\n{}", summary.report());
        assert_eq!(summary.failed(), 1, "report:\n{}", summary.report());
        let failed: Vec<&str> = summary
            .outcomes
            .iter()
            .filter(|o| !o.passed)
            .map(|o| o.name.as_str())
            .collect();
        assert_eq!(failed, vec!["test_double_wrong"]);
    }

    #[test]
    fn expanded_assertion_vocabulary() {
        let rt = Runtime::new().unwrap();
        let _g = rt.enter();
        // Exercises the assertions beyond eq/ne/is_true: container membership
        // over every supported shape, its inverse, the ordering family, and
        // message-matching on `fails`. Each test is written to pass; a single
        // regression in any assertion would flip its test to failed.
        let src = r#"
def test_contains_over_containers(t):
    asserts.contains("hello world", "world")   # substring
    asserts.contains([1, 2, 3], 2)              # list element
    asserts.contains((1, 2, 3), 3)              # tuple element
    asserts.contains({"a": 1, "b": 2}, "a")     # dict key

def test_not_contains(t):
    asserts.not_contains("hello", "z")
    asserts.not_contains([1, 2, 3], 9)
    asserts.not_contains({"a": 1}, "b")

def test_ordering(t):
    asserts.gt(2, 1)
    asserts.ge(2, 2)
    asserts.lt(1, 2)
    asserts.le(2, 2)
    asserts.lt("abc", "abd")

def test_fails_with_message_match(t):
    asserts.fails(lambda: fail("boom: bad input"), contains = "bad input")
    asserts.fails(lambda: fail("boom"))
"#;
        let summary = run_test_source(src, &base_env()).expect("runner ok");
        assert_eq!(
            summary.failed(),
            0,
            "all expanded-assertion tests should pass; report:\n{}",
            summary.report()
        );
        assert_eq!(summary.passed(), 4, "report:\n{}", summary.report());
    }

    #[test]
    fn assertions_fail_when_expected() {
        let rt = Runtime::new().unwrap();
        let _g = rt.enter();
        // The mirror image: each test drives one assertion into its failure
        // path, proving the failure branches actually raise (a no-op assert
        // would let these pass and break the framework silently).
        let src = r#"
def test_contains_fails(t):
    asserts.contains([1, 2], 3)

def test_not_contains_fails(t):
    asserts.not_contains([1, 2], 1)

def test_gt_fails(t):
    asserts.gt(1, 2)

def test_fails_wrong_message(t):
    asserts.fails(lambda: fail("boom"), contains = "not-present")

def test_fails_when_no_raise(t):
    asserts.fails(lambda: 1 + 1)
"#;
        let summary = run_test_source(src, &base_env()).expect("runner ok");
        assert_eq!(
            summary.passed(),
            0,
            "every assertion should reach its failure path; report:\n{}",
            summary.report()
        );
        assert_eq!(summary.failed(), 5, "report:\n{}", summary.report());
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

    /// End-to-end proof of the bazel `Fake` backend (increment 2): a test
    /// declares a typed `BazelExpectation` via `t.bazel.expect_build(...)`, the
    /// `Fake` backend on `t.ctx.bazel` fork+execs the fake-bazel binary
    /// (basil), hands it the expectation over the inherited socketpair control
    /// channel, and the fake synthesizes a real BES stream onto the
    /// `--build_event_binary_file` the parent already wires. The assertions run
    /// entirely through the production `ctx.bazel.build` read path
    /// (`BuildEventIter` + `build.wait()`), so the mock's contract is the real
    /// one.
    #[test]
    fn fake_bazel_backend_synthesizes_declared_expectation() {
        let rt = Runtime::new().unwrap();
        let _g = rt.enter();
        let fake_bin: Arc<str> = Arc::from(crate::test::basil_bin());

        let src = r#"
def test_passing_build_synthesizes_events_and_exit(t):
    t.bazel.expect_build("//a:b", "//c:d", result = "passed")
    iter = bazel.build_events.iterator()
    build = t.ctx.bazel.build(build_events = [iter], stderr = None)
    started = 0
    completed = 0
    finished = 0
    for event in iter:
        kind = event.kind
        if kind == "build_started":
            started += 1
        elif kind == "target_completed":
            completed += 1
        elif kind == "build_finished":
            finished += 1
    status = build.wait()
    asserts.is_true(status.success)
    asserts.eq(status.code, 0)
    asserts.eq(started, 1)
    asserts.eq(completed, 2)
    asserts.eq(finished, 1)

def test_failing_build_surfaces_declared_exit_code(t):
    t.bazel.expect_build("//x:y", result = "failed", exit_code = 7)
    build = t.ctx.bazel.build(build_events = True, stderr = None)
    status = build.wait()
    asserts.is_false(status.success)
    asserts.eq(status.code, 7)
"#;
        let summary = run_test_source_with_fake_bazel(src, &base_env(), fake_bin)
            .expect("fake-bazel runner ok");
        assert_eq!(
            summary.failed(),
            0,
            "expected all fake-bazel tests to pass; report:\n{}",
            summary.report()
        );
        assert_eq!(summary.passed(), 2, "report:\n{}", summary.report());
    }
}
