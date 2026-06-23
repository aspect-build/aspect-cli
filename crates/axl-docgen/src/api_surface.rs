//! A flat, signature-only projection of the entire public AXL API surface.
//!
//! Every public type, builtin module, function, property, parameter, and
//! return type is rendered to a single deterministic line. Prose docstrings
//! are deliberately excluded so the output only moves when the *contract*
//! moves — a removed builtin, a renamed parameter, a newly-required argument,
//! or a changed return type.
//!
//! The `--api-surface` flag of `axl-docgen` prints this; CI snapshots it and
//! pings #aspect-cli when it drifts, so breaking changes to the surface that
//! downstream extensions (e.g. the `rbe` module) depend on are caught at the
//! commit that introduces them rather than by a customer.

use starlark::docs::{DocFunction, DocItem, DocMember, DocModule, DocParam, DocProperty};

/// Render the whole API surface (Rust-defined globals + embedded builtin
/// modules) to a sorted, de-duplicated, prose-free string — one line per
/// public symbol. Stable across runs of the same CLI build.
pub fn render_api_surface(types: &DocModule, builtins: &[(String, String, DocModule)]) -> String {
    let mut lines = Vec::new();
    walk_module("types", types, &mut lines);
    for (module, name, dm) in builtins {
        walk_module(&format!("@{module}//{name}"), dm, &mut lines);
    }
    lines.sort();
    lines.dedup();
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// True if `name` is part of the public surface (skips `_`-private symbols and
/// internal `#`-markers, matching the docgen traversal).
fn is_public(name: &str) -> bool {
    !name.starts_with('_') && !name.starts_with('#')
}

fn walk_module(path: &str, module: &DocModule, out: &mut Vec<String>) {
    for (name, item) in module.members.iter() {
        if !is_public(name) {
            continue;
        }
        let qualified = format!("{path}.{name}");
        match item {
            DocItem::Member(DocMember::Function(f)) => out.push(render_function(&qualified, f)),
            DocItem::Member(DocMember::Property(p)) => out.push(render_property(&qualified, p)),
            DocItem::Type(ty) => {
                out.push(format!("type {qualified}"));
                for (member_name, member) in ty.members.iter() {
                    if !is_public(member_name) {
                        continue;
                    }
                    let mq = format!("{qualified}.{member_name}");
                    match member {
                        DocMember::Function(f) => out.push(render_function(&mq, f)),
                        DocMember::Property(p) => out.push(render_property(&mq, p)),
                    }
                }
            }
            DocItem::Module(sub) => walk_module(&qualified, sub, out),
        }
    }
}

// KNOWN GAP: a property whose value is a method-bearing builtin *type* renders
// only its `Ty` — e.g. `@std//base64.base64` becomes
// `struct(decode = function, ...)`. Field add/remove/retype still moves the
// line, but a param/return change inside `base64.decode` does NOT, because the
// method signatures live behind `BuiltinsBase64::get_methods()` and that type
// is only reachable via the `_`-private `__builtins__` entrypoint, so the docs
// tree never surfaces it. Module-level builtins (`@std//time`, `json`,
// `grpc.Server`) are exposed as functions/namespaces and ARE captured.
// Accepted limitation: closing it would require `docs::documentation()` to
// emit a DocType (via `Methods::documentation()`) for method-bearing values.
fn render_property(name: &str, prop: &DocProperty) -> String {
    format!("{name}: {}", prop.typ)
}

/// `@bazel//bazel.Bazel.query(expr: str, /, *, rc: RunCommand = None) -> bazel.query.Query`
fn render_function(name: &str, f: &DocFunction) -> String {
    let render_param = |p: &DocParam| match &p.default_value {
        Some(default) => format!("{}: {} = {}", p.name, p.typ, default),
        None => format!("{}: {}", p.name, p.typ),
    };

    let mut parts: Vec<String> = Vec::new();
    for p in &f.params.pos_only {
        parts.push(render_param(p));
    }
    if !f.params.pos_only.is_empty() {
        parts.push("/".to_string());
    }
    for p in &f.params.pos_or_named {
        parts.push(render_param(p));
    }
    if let Some(args) = &f.params.args {
        parts.push(format!("*{}: {}", args.name, args.typ));
    } else if !f.params.named_only.is_empty() {
        parts.push("*".to_string());
    }
    for p in &f.params.named_only {
        parts.push(render_param(p));
    }
    if let Some(kwargs) = &f.params.kwargs {
        parts.push(format!("**{}: {}", kwargs.name, kwargs.typ));
    }

    format!("{name}({}) -> {}", parts.join(", "), f.ret.typ)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axl_runtime::docs;

    /// Renders the live surface and asserts the invariants CI relies on:
    /// non-empty, sorted, de-duplicated, and covering the `@bazel//` builtins
    /// whose removal originally broke the `rbe` extension. No filesystem access,
    /// so it is safe under the Bazel test sandbox.
    #[tokio::test]
    async fn api_surface_is_deterministic_and_covers_bazel() {
        let docs = docs::documentation().expect("collect documentation");
        let surface = render_api_surface(&docs.types, &docs.builtins);

        assert!(!surface.trim().is_empty(), "surface should not be empty");

        let lines: Vec<&str> = surface.lines().collect();
        let mut sorted = lines.clone();
        sorted.sort();
        assert_eq!(lines, sorted, "surface lines must be sorted");

        let mut deduped = lines.clone();
        deduped.dedup();
        assert_eq!(lines.len(), deduped.len(), "surface lines must be unique");

        assert!(
            surface.contains("@bazel//"),
            "surface should include @bazel// builtins, got:\n{surface}"
        );
        assert!(
            surface.contains(".query("),
            "surface should include the bazel query builtin, got:\n{surface}"
        );
    }
}
