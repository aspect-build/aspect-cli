//! Test helpers for evaluating AXL Starlark snippets.
//!
//! Single fluent entry point: `eval(code)` returns an `EvalBuilder` with four
//! terminals depending on what the test wants back:
//!
//! - `.check()` — parse + run, return `Ok(())` or the eval error.
//! - `.repr()` — run, return `to_repr()` of the module's last expression value.
//! - `.with_value(sym, f)` — run, look up `sym`, call `f` with its `Value`.
//! - `.run_task(idx)` — discover tasks via `MultiPhaseEval`, execute task `idx`.
//!
//! `.with_loader()` opts into a real `AxlLoader` so `load("@std//...")` works.
//!
//! Macros at the crate root cover the two most common one-liners:
//! `axl_eval!(c)` (= `eval(c).with_loader().repr()`) and
//! `axl_check!(c)` (= `eval(c).check()`).

use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::anyhow;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;
use starlark::values::Value;
use tokio::runtime::Runtime;

use crate::engine::arguments::Arguments;
use crate::engine::store::Env;
use crate::eval::api::{dialect, get_globals};
use crate::eval::{Loader, ModuleEnv, MultiPhaseEval};
use crate::module::Mod;

pub fn eval(code: &str) -> EvalBuilder {
    EvalBuilder {
        code: code.to_string(),
        with_loader: false,
        with_fake_bazel: false,
    }
}

pub struct EvalBuilder {
    code: String,
    with_loader: bool,
    with_fake_bazel: bool,
}

impl EvalBuilder {
    /// Wire up `AxlLoader` so `load("@std//...")` resolves. CARGO_MANIFEST_DIR
    /// is used as the synthetic repo root.
    pub fn with_loader(mut self) -> Self {
        self.with_loader = true;
        self
    }

    /// Install the basil fake-bazel binary as `BAZEL_REAL` before running.
    /// Use on any builder whose snippet calls `ctx.bazel.build` so the
    /// runtime spawns basil instead of shelling out to a real `bazel`.
    /// Idempotent across tests; see `install_basil` for details.
    pub fn with_fake_bazel(mut self) -> Self {
        self.with_fake_bazel = true;
        self
    }

    /// Parse + evaluate, discarding the resulting value.
    pub fn check(self) -> starlark::Result<()> {
        let ast = AstModule::parse("<snippet>", self.code, &dialect())?;
        let globals = get_globals().build();
        let rt = Runtime::new().map_err(|e| anyhow!("failed to create runtime: {}", e))?;
        let _g = rt.enter();
        let env_store = Env::new("test".to_string(), PathBuf::from("/"));
        ModuleEnv::with(|env| {
            let mut eval = Evaluator::new(&env.0);
            eval.extra = Some(&env_store);
            eval.eval_module(ast, &globals).map(|_| ())
        })
    }

    /// Parse + evaluate, return `to_repr()` of the module's last value.
    pub fn repr(self) -> anyhow::Result<String> {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let rt = Runtime::new()?;
        let _g = rt.enter();
        let loader = Loader::new("test".to_string(), manifest_dir, &[]);
        let ast = AstModule::parse("test", self.code, &dialect()).map_err(|e| anyhow!("{}", e))?;
        Module::with_temp_heap(|module| {
            let mut eval = Evaluator::new(&module);
            if self.with_loader {
                eval.set_loader(&loader);
            }
            let val = eval
                .eval_module(ast, &loader.globals)
                .map_err(|e| anyhow!("{}", e))?;
            Ok(val.to_repr())
        })
    }

    /// Parse + evaluate, then look up `symbol` in the module and pass its
    /// `Value` to `f`. Panics on parse/eval failure or missing symbol — this
    /// is a test helper, not a library API.
    pub fn with_value<R>(self, symbol: &str, f: impl for<'v> FnOnce(Value<'v>) -> R) -> R {
        let ast = AstModule::parse("<test>", self.code, &dialect())
            .unwrap_or_else(|e| panic!("parse error: {e}"));
        let globals = get_globals().build();
        let rt = Runtime::new().expect("failed to create runtime");
        let _g = rt.enter();
        let env_store = Env::new("test".to_string(), PathBuf::from("/"));
        Module::with_temp_heap(|module| {
            let mut eval = Evaluator::new(&module);
            eval.extra = Some(&env_store);
            eval.eval_module(ast, &globals)
                .unwrap_or_else(|e| panic!("eval error: {e}"));
            let value = module
                .get(symbol)
                .unwrap_or_else(|| panic!("{symbol} not found in module"));
            f(value)
        })
    }

    /// Drive `MultiPhaseEval` Phase 1 (discover tasks) + Phase 4 (execute).
    /// Phase 2 (configs) and Phase 3 (features) are skipped — the snippet must
    /// be self-contained. Task `idx` runs with empty `Arguments`.
    ///
    /// Snippets that call `ctx.bazel.build` should opt in via
    /// `.with_fake_bazel()` so the runtime spawns the basil fake-bazel
    /// instead of a real `bazel`.
    pub fn run_task(self, task_index: usize) -> anyhow::Result<Option<u8>> {
        if self.with_fake_bazel {
            install_basil();
        }
        let tmp = tempfile::tempdir()?;
        let script_path = tmp.path().join("test.axl");
        std::fs::write(&script_path, &self.code)?;

        let rt = Runtime::new()?;
        let _g = rt.enter();

        ModuleEnv::with(|env| -> anyhow::Result<Option<u8>> {
            let modules: Vec<Mod> = vec![];
            let root_mod = Mod::new(
                tmp.path().to_path_buf(),
                "_root".to_string(),
                tmp.path().to_path_buf(),
            );
            let loader = Loader::new("test".to_string(), tmp.path().to_path_buf(), &modules);
            let mut mpe = MultiPhaseEval::new(env, &loader);
            let scripts = vec![script_path];
            mpe.eval(&scripts, &root_mod, &modules)
                .map_err(anyhow::Error::from)?;
            let exit = mpe
                .execute_tasks_with_args(task_index, "_test".to_string(), None, |_t, _h| {
                    Arguments::new()
                })
                .map_err(anyhow::Error::from)?;
            Ok(exit)
        })
    }
}

/// Locate the `basil` binary (the fake `bazel` used by tests that exercise
/// `ctx.bazel.build`).
///
/// Two paths covered:
///   - **Bazel**: `BASIL_BIN` is set by the `rust_test` rule's `env`
///     attribute via `$(rootpath //crates/basil)`. The path is relative to
///     the runfiles root, which is the cwd of a Bazel-run test, so
///     `canonicalize()` resolves it to an absolute path that `Command::new`
///     can spawn.
///   - **Cargo**: `cargo test -p axl-runtime` doesn't build sibling
///     workspace binaries, so we shell out once to `cargo build -p basil`
///     and locate the resulting binary alongside our own test exe. Same
///     recursive-cargo pattern that `assert_cmd`/`escargot` use — by the
///     time test binaries are running, the parent cargo's build lock is
///     released, so the inner `cargo build` succeeds.
pub fn basil_bin() -> &'static str {
    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| {
        if let Ok(p) = std::env::var("BASIL_BIN") {
            return std::fs::canonicalize(&p)
                .unwrap_or_else(|e| panic!("BASIL_BIN={p:?} not found: {e}"))
                .to_string_lossy()
                .into_owned();
        }

        // CARGO is set by cargo for test binaries; CARGO_MANIFEST_DIR points
        // at this crate's Cargo.toml.
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root above crates/axl-runtime")
            .to_path_buf();
        let status = std::process::Command::new(&cargo)
            .args(["build", "--quiet", "-p", "basil"])
            .current_dir(&workspace_root)
            .status()
            .expect("failed to invoke cargo to build basil");
        assert!(
            status.success(),
            "cargo build -p basil failed (status: {status})"
        );

        // Cargo writes the binary alongside the test binary's parent.
        // current_exe() = target/<profile>/deps/<test>-<hash>;
        // basil lives at target/<profile>/basil.
        let test_exe = std::env::current_exe().expect("current_exe");
        let mut path: PathBuf = test_exe.parent().unwrap().to_path_buf();
        if path.ends_with("deps") {
            path.pop();
        }
        path.push("basil");
        assert!(
            path.exists(),
            "basil not found at {} after `cargo build -p basil`",
            path.display()
        );
        path.to_string_lossy().into_owned()
    })
}

/// Point axl-runtime at basil. Idempotent and safe under parallel test
/// execution: every test sets the same value, so there is nothing to race
/// over (no PATH_LOCK needed).
///
/// Per-scenario timing (e.g. the post-open pause that lets a late AXL
/// subscriber land its `.subscribe()` call before basil writes) lives in
/// basil's `scenario()` table, not here.
pub fn install_basil() {
    // SAFETY: process-wide env mutation. All callers in this test binary
    // converge on the same value, so concurrent writes are no-ops.
    unsafe {
        std::env::set_var("BAZEL_REAL", basil_bin());
    }
}
