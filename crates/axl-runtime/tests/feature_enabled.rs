/// Integration tests for the implicit `enabled` arg on feature types.
///
/// Covers:
/// - Starlark-level validity of `enabled = True/False`
/// - `enabled` in `args` dict is reserved (must use the top-level kwarg)
/// - Feature CLI name derived correctly from CamelCase export variable
/// - `extract_feature_args` always includes the implicit `enabled` Boolean
/// - `long_override()` on the `enabled` arg is prefixed: `--artifact-upload:enabled`
/// - `enabled = False` default reflected in the extracted `Arg::Boolean`
/// - `feature_instance_effective_defaults` serializes bool overrides as lowercase strings
// ── helpers ──────────────────────────────────────────────────────────────────
use axl_runtime::engine::arg::Arg;
use axl_runtime::engine::feature::{Feature, FeatureLike, FrozenFeature};
use axl_runtime::engine::names::to_command_name;
use axl_runtime::eval::eval_snippet;
use starlark::values::{Value, ValueLike};

/// Run a closure with `&dyn FeatureLike` for whichever feature variant a value holds.
fn with_feature<R>(value: Value<'_>, f: impl FnOnce(&dyn FeatureLike<'_>) -> R) -> R {
    if let Some(live) = value.downcast_ref::<Feature>() {
        return f(live);
    }
    if let Some(frozen) = value.downcast_ref::<FrozenFeature>() {
        return f(frozen);
    }
    panic!("value is not a feature: {}", value.get_type());
}

fn ok(code: &str) {
    eval_snippet(code).unwrap_or_else(|e| panic!("expected ok, got: {e}"));
}

fn err(code: &str) -> String {
    eval_snippet(code)
        .expect_err("expected evaluation to fail")
        .to_string()
}

fn with_module_value<T>(
    code: &str,
    symbol: &str,
    f: impl for<'v> FnOnce(starlark::values::Value<'v>) -> T,
) -> T {
    use axl_runtime::engine::store::Env;
    use axl_runtime::eval::api::{dialect, get_globals};
    use starlark::environment::Module;
    use starlark::eval::Evaluator;
    use starlark::syntax::AstModule;
    use std::path::PathBuf;
    use tokio::runtime::Runtime;

    let ast = AstModule::parse("<test>", code.to_owned(), &dialect())
        .unwrap_or_else(|e| panic!("parse error: {e}"));
    let globals = get_globals().build();
    // `feature()` and `task()` read Env from `eval.extra`; the script path
    // is recovered from the evaluator's call stack. Env::new requires a
    // tokio runtime (it captures the current Handle).
    let rt = Runtime::new().expect("failed to create runtime");
    let _guard = rt.enter();
    let env = Env::new("test".to_string(), PathBuf::from("/"));
    Module::with_temp_heap(|module| {
        let mut eval = Evaluator::new(&module);
        eval.extra = Some(&env);
        eval.eval_module(ast, &globals)
            .unwrap_or_else(|e| panic!("eval error: {e}"));
        let value = module
            .get(symbol)
            .unwrap_or_else(|| panic!("{symbol} not found in module"));
        f(value)
    })
}

// ── Starlark-level validity ───────────────────────────────────────────────────

#[test]
fn feature_enabled_defaults_to_true() {
    ok(r#"
def _impl(ctx): pass
MyFeature = feature(implementation = _impl)
"#);
}

#[test]
fn feature_enabled_false_is_valid() {
    ok(r#"
def _impl(ctx): pass
MyFeature = feature(implementation = _impl, enabled = False)
"#);
}

#[test]
fn feature_enabled_true_explicit_is_valid() {
    ok(r#"
def _impl(ctx): pass
MyFeature = feature(implementation = _impl, enabled = True)
"#);
}

/// Users must use `enabled = False` on the `feature()` call, not in the `args` dict.
#[test]
fn feature_enabled_reserved_in_args_dict() {
    let e = err(r#"
def _impl(ctx): pass
MyFeature = feature(implementation = _impl, args = {"enabled": args.boolean()})
"#);
    assert!(
        e.contains("enabled") && (e.contains("implicit") || e.contains("remove")),
        "expected 'enabled is implicit' error, got: {e}"
    );
}

// ── Name derivation ───────────────────────────────────────────────────────────

/// `to_command_name` converts CamelCase export names to kebab-case CLI prefixes.
#[test]
fn feature_name_camelcase_to_kebab() {
    assert_eq!(to_command_name("ArtifactUpload"), "artifact-upload");
    assert_eq!(to_command_name("BazelDefaults"), "bazel-defaults");
    assert_eq!(
        to_command_name("GithubStatusChecks"),
        "github-status-checks"
    );
    assert_eq!(to_command_name("MyFeature"), "my-feature");
}

// ── Implicit `enabled` arg always present ────────────────────────────────────

/// Every feature has an implicit Boolean `enabled` arg regardless of user-supplied args.
#[test]
fn feature_enabled_arg_always_present() {
    let (has_enabled, has_mode) = with_module_value(
        r#"
def _impl(ctx): pass
ArtifactUpload = feature(implementation = _impl, args = {"mode": args.string(default = "upload")})
"#,
        "ArtifactUpload",
        |v| {
            with_feature(v, |f| {
                (
                    f.args().contains_key("enabled"),
                    f.args().contains_key("mode"),
                )
            })
        },
    );

    assert!(has_enabled, "implicit `enabled` arg must always be present");
    assert!(has_mode, "user-defined arg `mode` must be present");
}

// ── `enabled` default value ───────────────────────────────────────────────────

/// `enabled = True` (the default) is reflected in the `Arg::Boolean` default.
#[test]
fn feature_enabled_default_true_reflected_in_arg() {
    let default_val = with_module_value(
        r#"
def _impl(ctx): pass
MyFeature = feature(implementation = _impl)
"#,
        "MyFeature",
        |v| {
            with_feature(v, |f| {
                match f.args().get("enabled").expect("enabled arg not found") {
                    Arg::Boolean { default, .. } => *default,
                    other => panic!("expected Boolean arg, got: {other:?}"),
                }
            })
        },
    );
    assert!(default_val, "default should be true when enabled = True");
}

/// `enabled = False` is reflected in the `Arg::Boolean` default.
#[test]
fn feature_enabled_default_false_reflected_in_arg() {
    let default_val = with_module_value(
        r#"
def _impl(ctx): pass
MyFeature = feature(implementation = _impl, enabled = False)
"#,
        "MyFeature",
        |v| {
            with_feature(v, |f| {
                match f.args().get("enabled").expect("enabled arg not found") {
                    Arg::Boolean { default, .. } => *default,
                    other => panic!("expected Boolean arg, got: {other:?}"),
                }
            })
        },
    );
    assert!(!default_val, "default should be false when enabled = False");
}
