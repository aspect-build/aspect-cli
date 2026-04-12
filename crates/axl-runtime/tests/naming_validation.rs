/// Integration tests for task(), feature(), and arg naming validation.
///
/// These tests evaluate real Starlark snippets through the AXL eval stack and assert
/// on the resulting errors or successful outcomes, covering the full path from
/// Starlark source → validation → ConfiguredTask name derivation.
use axl_runtime::eval::eval_snippet;

fn eval(code: &str) -> Result<(), String> {
    eval_snippet(code).map_err(|e| e.to_string())
}

fn eval_err(code: &str) -> String {
    eval(code).expect_err("expected evaluation to fail")
}

// ── Minimal valid Starlark snippets ─────────────────────────────────────────

const VALID_TASK: &str = r#"
def _impl(ctx):
    pass

ValidTask = task(implementation = _impl, args = {})
"#;

const VALID_FEATURE: &str = r#"
def _impl(ctx):
    pass

ValidFeature = feature(implementation = _impl)
"#;

// ── task() — name validation ─────────────────────────────────────────────────

#[test]
fn task_valid_explicit_name() {
    assert!(
        eval(
            r#"
def _impl(ctx): pass
T = task(implementation = _impl, args = {}, name = "my-task")
"#
        )
        .is_ok()
    );
}

#[test]
fn task_name_underscore_rejected() {
    let err = eval_err(
        r#"
def _impl(ctx): pass
T = task(implementation = _impl, args = {}, name = "bad_name")
"#,
    );
    assert!(
        err.contains("invalid character") || err.contains("_"),
        "expected underscore error, got: {}",
        err
    );
}

#[test]
fn task_name_uppercase_rejected() {
    let err = eval_err(
        r#"
def _impl(ctx): pass
T = task(implementation = _impl, args = {}, name = "BadName")
"#,
    );
    assert!(
        err.contains("lowercase"),
        "expected lowercase error, got: {}",
        err
    );
}

#[test]
fn task_name_leading_digit_rejected() {
    let err = eval_err(
        r#"
def _impl(ctx): pass
T = task(implementation = _impl, args = {}, name = "1task")
"#,
    );
    assert!(
        err.contains("lowercase"),
        "expected lowercase error, got: {}",
        err
    );
}

// ── task() — group validation ─────────────────────────────────────────────────

#[test]
fn task_valid_group() {
    assert!(
        eval(
            r#"
def _impl(ctx): pass
T = task(implementation = _impl, args = {}, group = ["axl", "tools"])
"#
        )
        .is_ok()
    );
}

#[test]
fn task_group_underscore_rejected() {
    let err = eval_err(
        r#"
def _impl(ctx): pass
T = task(implementation = _impl, args = {}, group = ["bad_group"])
"#,
    );
    assert!(
        err.contains("invalid character") || err.contains("_"),
        "expected underscore error in group, got: {}",
        err
    );
}

#[test]
fn task_group_uppercase_rejected() {
    let err = eval_err(
        r#"
def _impl(ctx): pass
T = task(implementation = _impl, args = {}, group = ["BadGroup"])
"#,
    );
    assert!(
        err.contains("lowercase"),
        "expected lowercase error in group, got: {}",
        err
    );
}

// ── task() — arg name validation ─────────────────────────────────────────────

#[test]
fn task_valid_args() {
    assert!(eval(VALID_TASK).is_ok());
    assert!(
        eval(
            r#"
def _impl(ctx): pass
T = task(implementation = _impl, args = {
    "target_pattern": args.string(),
    "bazel_flag": args.string_list(),
    "dry_run": args.boolean(default = False),
})
"#
        )
        .is_ok()
    );
}

#[test]
fn task_arg_uppercase_rejected() {
    let err = eval_err(
        r#"
def _impl(ctx): pass
T = task(implementation = _impl, args = {"BadArg": args.string()})
"#,
    );
    assert!(
        err.contains("lowercase"),
        "expected lowercase error for arg name, got: {}",
        err
    );
}

#[test]
fn task_arg_dash_rejected() {
    let err = eval_err(
        r#"
def _impl(ctx): pass
T = task(implementation = _impl, args = {"bad-arg": args.string()})
"#,
    );
    assert!(
        err.contains("invalid character"),
        "expected invalid character error for dash in arg name, got: {}",
        err
    );
}

#[test]
fn task_arg_leading_digit_rejected() {
    let err = eval_err(
        r#"
def _impl(ctx): pass
T = task(implementation = _impl, args = {"1arg": args.string()})
"#,
    );
    assert!(
        err.contains("lowercase"),
        "expected lowercase error for digit-leading arg name, got: {}",
        err
    );
}

#[test]
fn task_arg_leading_underscore_rejected() {
    let err = eval_err(
        r#"
def _impl(ctx): pass
T = task(implementation = _impl, args = {"_private": args.string()})
"#,
    );
    assert!(
        err.contains("lowercase"),
        "expected lowercase error for underscore-leading arg name, got: {}",
        err
    );
}

// ── feature() — arg name validation ──────────────────────────────────────────

#[test]
fn feature_valid() {
    assert!(eval(VALID_FEATURE).is_ok());
    assert!(
        eval(
            r#"
def _impl(ctx): pass
F = feature(implementation = _impl, args = {"upload_bucket": args.string()})
"#
        )
        .is_ok()
    );
}

#[test]
fn feature_arg_uppercase_rejected() {
    let err = eval_err(
        r#"
def _impl(ctx): pass
F = feature(implementation = _impl, args = {"UploadBucket": args.string()})
"#,
    );
    assert!(
        err.contains("lowercase"),
        "expected lowercase error for feature arg, got: {}",
        err
    );
}

#[test]
fn feature_arg_dash_rejected() {
    let err = eval_err(
        r#"
def _impl(ctx): pass
F = feature(implementation = _impl, args = {"upload-bucket": args.string()})
"#,
    );
    assert!(
        err.contains("invalid character"),
        "expected invalid character error for dash in feature arg, got: {}",
        err
    );
}

#[test]
fn feature_positional_arg_rejected() {
    let err = eval_err(
        r#"
def _impl(ctx): pass
F = feature(implementation = _impl, args = {"target": args.positional()})
"#,
    );
    assert!(
        err.contains("positional"),
        "expected positional-not-allowed error, got: {}",
        err
    );
}

#[test]
fn feature_trailing_var_args_rejected() {
    let err = eval_err(
        r#"
def _impl(ctx): pass
F = feature(implementation = _impl, args = {"rest": args.trailing_var_args()})
"#,
    );
    assert!(
        err.contains("positional"),
        "expected positional-not-allowed error for trailing_var_args, got: {}",
        err
    );
}

// ── feature() — CamelCase export name enforcement ────────────────────────────

#[test]
fn feature_camelcase_export_valid() {
    assert!(
        eval(
            r#"
def _impl(ctx): pass
ArtifactUpload = feature(implementation = _impl)
"#
        )
        .is_ok()
    );
    assert!(
        eval(
            r#"
def _impl(ctx): pass
BazelDefaults = feature(implementation = _impl)
"#
        )
        .is_ok()
    );
}

#[test]
fn feature_snake_case_export_rejected() {
    let err = eval_err(
        r#"
def _impl(ctx): pass
artifact_upload = feature(implementation = _impl)
"#,
    );
    assert!(
        err.contains("uppercase"),
        "expected uppercase error for snake_case feature export, got: {}",
        err
    );
}

#[test]
fn feature_lowercase_export_rejected() {
    let err = eval_err(
        r#"
def _impl(ctx): pass
myfeature = feature(implementation = _impl)
"#,
    );
    assert!(
        err.contains("uppercase"),
        "expected uppercase error for lowercase feature export, got: {}",
        err
    );
}

#[test]
fn feature_export_underscore_rejected() {
    let err = eval_err(
        r#"
def _impl(ctx): pass
Artifact_Upload = feature(implementation = _impl)
"#,
    );
    assert!(
        err.contains("invalid character"),
        "expected invalid character error for underscore in feature export, got: {}",
        err
    );
}

// ── trait() — CamelCase export name enforcement ───────────────────────────────

#[test]
fn trait_camelcase_export_valid() {
    assert!(
        eval(
            r#"
MyConfig = trait(
    message = attr(str, "default"),
)
"#
        )
        .is_ok()
    );
}

#[test]
fn trait_snake_case_export_rejected() {
    let err = eval_err(
        r#"
my_config = trait(
    message = attr(str, "default"),
)
"#,
    );
    assert!(
        err.contains("uppercase"),
        "expected uppercase error for snake_case trait export, got: {}",
        err
    );
}

#[test]
fn trait_export_underscore_rejected() {
    let err = eval_err(
        r#"
My_Config = trait(
    message = attr(str, "default"),
)
"#,
    );
    assert!(
        err.contains("invalid character"),
        "expected invalid character error for underscore in trait export, got: {}",
        err
    );
}
