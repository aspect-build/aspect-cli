use crate::axl_check;

/// Starlark has no assertion builtin; the snippets share this one.
const PRELUDE: &str = "def assert_eq(got, want):\n    if got != want:\n        fail(\"want\", repr(want), \"got\", repr(got))\n";

fn ok(code: &str) {
    axl_check!(&format!("{PRELUDE}{code}")).unwrap_or_else(|e| panic!("expected ok, got: {e}"));
}

fn err(code: &str) -> String {
    axl_check!(code)
        .expect_err("expected evaluation to fail")
        .to_string()
}

const TYPES: &str = r#"
DeployError = error.type(fields = {"deployment": str, "attempt": field(int, default = 1)})
DeployTimeout = DeployError.type(fields = {"after_ms": int})
Unrelated = error.type()
"#;

fn with_types(body: &str) -> String {
    format!("{TYPES}\n{body}")
}

#[test]
fn every_error_is_type_error_and_matches_its_whole_chain() {
    ok(&with_types(
        r#"
e = DeployTimeout("no ack", deployment = "prod", after_ms = 30000)
assert_eq(type(e), "error")
assert_eq(isinstance(e, DeployTimeout), True)
assert_eq(isinstance(e, DeployError), True)
assert_eq(isinstance(e, error), True)
assert_eq(isinstance(e, Unrelated), False)
assert_eq(isinstance(DeployError("x", deployment = "d"), DeployTimeout), False)
assert_eq(isinstance("x", error), False)
assert_eq(type(DeployError), "error_type")
assert_eq(isinstance(error("plain"), error), True)
"#,
    ));
}

#[test]
fn annotations_accept_subtypes_and_reject_others() {
    ok(&with_types(
        r#"
def take(e: DeployError) -> str:
    return e.deployment

def any_error(e: error) -> str:
    return e.message

assert_eq(take(DeployTimeout("t", deployment = "prod", after_ms = 1)), "prod")
assert_eq(any_error(Unrelated("u")), "u")
"#,
    ));
    let msg = err(&with_types(
        r#"
def take(e: DeployError):
    pass

take(Unrelated("u"))
"#,
    ));
    assert!(
        msg.contains("does not match the type annotation `DeployError`"),
        "{msg}"
    );
}

#[test]
fn fields_are_inherited_defaulted_and_type_checked() {
    ok(&with_types(
        r#"
e = DeployTimeout(message = "t", deployment = "prod", after_ms = 5)
assert_eq(e.message, "t")
assert_eq(e.deployment, "prod")
assert_eq(e.attempt, 1)
assert_eq(e.after_ms, 5)
assert_eq(str(e), "t")
assert_eq(repr(e), 'DeployTimeout(message = "t", deployment = "prod", attempt = 1, after_ms = 5)')
assert_eq(dir(e), ["after_ms", "attempt", "cause", "deployment", "message", "stacktrace"])
"#,
    ));
    let msg = err(&with_types(r#"DeployError("x", deployment = 3)"#));
    assert!(
        msg.contains("field `deployment` expected type `str`"),
        "{msg}"
    );
    let msg = err(&with_types(r#"DeployError("x")"#));
    assert!(msg.contains("missing required field `deployment`"), "{msg}");
    let msg = err(&with_types(
        r#"DeployError("x", deployment = "d", nope = 1)"#,
    ));
    assert!(msg.contains("unexpected field `nope`"), "{msg}");
    let msg = err(&with_types(r#"DeployError(deployment = "d")"#));
    assert!(msg.contains("missing its message"), "{msg}");
}

#[test]
fn fields_take_field_specs_not_attrs() {
    ok(r#"
E = error.type(fields = {"tags": field(list[str], default = [])})
a = E("a")
a.tags.append("x")
assert_eq(E("b").tags, [])
"#);
    let msg = err(r#"E = error.type(fields = {"x": attr(int, default = 1)})"#);
    assert!(msg.contains("field `x`"), "{msg}");
}

#[test]
fn reserved_and_redeclared_fields_are_rejected() {
    let msg = err(r#"E = error.type(fields = {"cause": str})"#);
    assert!(msg.contains("`cause` is reserved"), "{msg}");
    let msg = err(&with_types(
        r#"E = DeployError.type(fields = {"deployment": str})"#,
    ));
    assert!(
        msg.contains("field `deployment` is already declared by DeployError"),
        "{msg}"
    );
}

#[test]
fn cause_chains_errors() {
    ok(&with_types(
        r#"
root = error("disk full")
e = DeployError("upload failed", deployment = "prod", cause = root)
assert_eq(e.cause.message, "disk full")
assert_eq(e.cause.cause, None)
"#,
    ));
    let msg = err(r#"error("x", cause = "not an error")"#);
    assert!(msg.contains("cause must be an error or None"), "{msg}");
}

#[test]
fn stacktrace_names_the_constructing_function() {
    // `error("x")` sits on the snippet's third line, after the prelude.
    let line = PRELUDE.lines().count() + 3;
    ok(&format!(
        r#"
def _build():
    return error("x")

frames = _build().stacktrace
names = [f.name for f in frames]
assert_eq("_build" in names, True)
f = [f for f in frames if f.name == "_build"][0]
assert_eq(f.path, "<snippet>")
assert_eq(f.line, {line})
"#
    ));
}

#[test]
fn string_fail_is_unchanged() {
    let msg = err(r#"fail("oops", 1, False)"#);
    assert!(msg.contains("fail: oops 1 False"), "{msg}");
}

#[test]
fn fail_raises_the_error_with_its_type_name() {
    let msg = err(&with_types(
        r#"fail(DeployError("no ack", deployment = "prod"))"#,
    ));
    assert!(msg.contains("DeployError: no ack"), "{msg}");
    assert!(msg.contains("Traceback"), "{msg}");
}

#[test]
fn catch_passes_success_through_as_the_value() {
    ok(r#"
err, value = __test_future(value = "hello").catch().block()
assert_eq(err, None)
assert_eq(value, "hello")
"#);
}

#[test]
fn catch_turns_a_runtime_failure_into_a_plain_error() {
    ok(r#"
err, value = __test_future(error = "request failed").catch().block()
assert_eq(value, None)
assert_eq(type(err), "error")
assert_eq(isinstance(err, error), True)
assert_eq(err.message, "request failed")
assert_eq(err.cause.message, "root cause")
assert_eq(err.cause.cause, None)
assert_eq(bool(err), True)
"#);
}

#[test]
fn catch_with_types_reraises_other_errors_unchanged() {
    let msg = err(&with_types(
        r#"__test_future(error = "request failed").catch(DeployError).block()"#,
    ));
    assert!(msg.contains("request failed"), "{msg}");
}

#[test]
fn catch_recovers_a_value_raised_in_a_callback() {
    ok(&with_types(
        r#"
def _raise(v):
    fail(DeployTimeout("slow", deployment = "prod", after_ms = 9))

err, value = __test_future(value = "x").map_ok(_raise).catch(DeployError).block()
assert_eq(value, None)
assert_eq(isinstance(err, DeployTimeout), True)
assert_eq(err.after_ms, 9)
assert_eq(err.deployment, "prod")
"#,
    ));
}

/// The caught value is the raised one, still live: its mutable fields
/// stay usable, both through it and through anything else that holds them.
#[test]
fn caught_value_is_the_raised_value_and_stays_mutable() {
    ok(r#"
Collected = error.type(fields = {"items": list})
items = ["a"]
raised = Collected("bad", items = items)

def _raise(v):
    fail(raised)

err, _ = __test_future(value = "x").map_ok(_raise).catch().block()
assert_eq(err == raised, True)
err.items.append("b")
items.append("c")
assert_eq(raised.items, ["a", "b", "c"])
"#);
}

#[test]
fn traceback_is_inherited_unless_overridden() {
    let msg = err(r#"
Refusal = error.type(traceback = False)
Specific = Refusal.type()
fail(Specific("nope"))
"#);
    assert!(
        !msg.contains("Specific:"),
        "a refusal renders its message alone: {msg}"
    );
    let msg = err(r#"
Refusal = error.type(traceback = False)
Loud = Refusal.type(traceback = True)
fail(Loud("nope"))
"#);
    assert!(msg.contains("Loud: nope"), "{msg}");
}

#[test]
fn catch_must_come_last_once_and_take_error_types() {
    let msg = err(r#"__test_future(value = "x").catch().map_ok(str).block()"#);
    assert!(msg.contains("catch() must be the last call"), "{msg}");
    let msg = err(r#"__test_future(value = "x").catch().catch().block()"#);
    assert!(msg.contains("catch() was already called"), "{msg}");
    let msg = err(r#"__test_future(value = "x").catch(str).block()"#);
    assert!(msg.contains("catch() takes error types"), "{msg}");
}

#[test]
fn generic_catch_passes_a_result_through_as_the_value() {
    ok(r#"
def _add(a, b, scale = 1):
    return (a + b) * scale

err, value = catch(_add, 1, 2, scale = 10)
assert_eq(err, None)
assert_eq(value, 30)
"#);
}

#[test]
fn generic_catch_turns_a_string_fail_into_a_plain_error() {
    ok(r#"
def _boom():
    fail("it broke")

err, value = catch(_boom)
assert_eq(value, None)
assert_eq(isinstance(err, error), True)
assert_eq(err.message, "it broke")
assert_eq([f.name for f in err.stacktrace][-1], "_boom")
"#);
}

#[test]
fn generic_catch_returns_a_raised_error_value_itself() {
    ok(&with_types(
        r#"
raised = DeployTimeout("slow", deployment = "prod", after_ms = 9)

def _raise():
    fail(raised)

err, _ = catch(_raise, types = [DeployError])
assert_eq(err == raised, True)
assert_eq(err.after_ms, 9)
"#,
    ));
}

#[test]
fn generic_catch_with_types_reraises_other_errors_unchanged() {
    let msg = err(&with_types(
        r#"
def _raise():
    fail(Unrelated("not mine"))

catch(_raise, types = (DeployError,))
"#,
    ));
    assert!(msg.contains("Unrelated: not mine"), "{msg}");
}

#[test]
fn generic_catch_catches_runtime_failures_of_builtins() {
    ok(r#"
err, value = catch(int, "not a number")
assert_eq(value, None)
assert_eq(type(err), "error")
assert_eq("not a number" in err.message, True)
"#);
}

#[test]
fn generic_catch_takes_error_types_only() {
    let msg = err(r#"catch(len, "x", types = [str])"#);
    assert!(msg.contains("catch() takes error types"), "{msg}");
}

/// Static typechecking of `snippet` against the AXL globals: the errors it
/// reports, rendered.
fn type_errors(snippet: &str) -> Vec<String> {
    use starlark::syntax::AstModule;
    use starlark::typing::AstModuleTypecheck;

    let ast = AstModule::parse(
        "<snippet>",
        snippet.to_owned(),
        &crate::eval::api::dialect(),
    )
    .expect("snippet parses");
    let globals = crate::eval::api::get_globals().build();
    let (errors, ..) = ast.typecheck(&globals, &Default::default());
    errors.iter().map(|e| e.to_string()).collect()
}

#[test]
fn generic_catch_checks_the_call_it_makes() {
    let errors = type_errors(
        r#"
def _parse(x: int) -> str:
    return str(x)

def _call():
    catch(_parse, "not an int")
"#,
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("Expected type `int` but got `str`")),
        "{errors:?}"
    );
}

#[test]
fn generic_catch_types_its_value_as_the_function_result() {
    let errors = type_errors(
        r#"
def _parse(x: int) -> str:
    return str(x)

def _wrong() -> int:
    err, value = catch(_parse, 1)
    return value
"#,
    );
    assert!(
        errors.iter().any(|e| e.contains("None | str")),
        "the value is the function's result or None: {errors:?}"
    );
    assert!(
        type_errors(
            r#"
def _parse(x: int) -> str:
    return str(x)

def _right() -> str | None:
    err, value = catch(_parse, 1)
    return value
"#
        )
        .is_empty()
    );
}

#[test]
fn a_typed_future_types_block_and_catch() {
    let errors = type_errors(
        r#"
def _block() -> int:
    return __test_future(value = "x").block()
"#,
    );
    assert!(
        errors.iter().any(|e| e.contains("`str`")),
        "block() returns the future's value type: {errors:?}"
    );
    let errors = type_errors(
        r#"
def _caught() -> int:
    err, value = __test_future(value = "x").catch().block()
    return value
"#,
    );
    assert!(
        errors.iter().any(|e| e.contains("None | str")),
        "catch().block() pairs the value type: {errors:?}"
    );
}
