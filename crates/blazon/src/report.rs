//! The shape report: what moved between Bazel versions, for the members a
//! spec selects and for the Starlark surface of the classes it binds.

use crate::ir::{Class, Implementation};
use crate::options::Schema;
use std::fmt::Write;

/// One version's view of a class: each selector with its shape.
type Entries = Vec<(String, String)>;

fn entries(c: &Class) -> Entries {
    let mut out: Entries = c
        .members
        .iter()
        .map(|m| {
            (
                m.selector.clone(),
                format!(
                    "{}{}",
                    m.descriptor,
                    if m.public { "" } else { " (non-public)" }
                ),
            )
        })
        .collect();
    out.extend(c.missing.iter().map(|s| (s.clone(), "missing".to_string())));
    out.extend(c.conditional.iter().map(|s| (s.clone(), "n/a".to_string())));
    if let Some(f) = &c.starlark {
        out.extend(f.shapes.iter().cloned());
    }
    out
}

/// Whether `sel` is left out of version `label` by a `when` condition.
fn is_conditional(per_version: &[(&str, Option<Entries>)], label: &str, sel: &str) -> bool {
    per_version
        .iter()
        .any(|(l, e)| *l == label && e.iter().flatten().any(|(s, d)| s == sel && d == "n/a"))
}

pub fn shape_report(
    versions: &[(String, Vec<Class>)],
    schemas: &[(String, Option<Schema>)],
    implemented: &[(String, Vec<Implementation>)],
) -> String {
    let mut out = String::new();
    let labels: Vec<&str> = versions.iter().map(|(l, _)| l.as_str()).collect();
    let _ = writeln!(out, "blazon: shape report across {}", labels.join(", "));
    let mut order: Vec<String> = Vec::new();
    for (_, classes) in versions {
        for c in classes {
            if !order.contains(&c.java) {
                order.push(c.java.clone());
            }
        }
    }
    for java in order {
        let per_version: Vec<(&str, Option<Entries>)> = versions
            .iter()
            .map(|(l, classes)| {
                (
                    l.as_str(),
                    classes.iter().find(|c| c.java == java).map(entries),
                )
            })
            .collect();
        let mut selectors: Vec<String> = Vec::new();
        for (_, e) in &per_version {
            for (s, _) in e.iter().flatten() {
                if !selectors.contains(s) {
                    selectors.push(s.clone());
                }
            }
        }
        for sel in selectors {
            let shapes: Vec<(&str, Option<String>)> = per_version
                .iter()
                .map(|(l, e)| {
                    let found = e.as_ref().and_then(|e| {
                        e.iter()
                            .find(|(s, d)| s == &sel && d != "missing" && d != "n/a")
                            .map(|(_, d)| d.clone())
                    });
                    (*l, found)
                })
                .collect();
            if shapes
                .iter()
                .any(|(l, _)| is_conditional(&per_version, l, &sel))
            {
                let present: Vec<&str> = shapes
                    .iter()
                    .filter(|(_, s)| s.is_some())
                    .map(|(l, _)| *l)
                    .collect();
                let first = shapes.iter().find_map(|(_, s)| s.as_ref());
                if shapes
                    .iter()
                    .filter_map(|(_, s)| s.as_ref())
                    .all(|s| Some(s) == first)
                {
                    let _ = writeln!(out, "  only in {}  {java}.{sel}", present.join(","));
                    continue;
                }
            }
            let first = &shapes[0].1;
            if first.is_some() && shapes.iter().all(|(_, s)| s == first) {
                let _ = writeln!(out, "  same     {java}.{sel}");
                continue;
            }
            let _ = writeln!(out, "  differs  {java}.{sel}");
            for (label, shape) in shapes {
                let _ = writeln!(
                    out,
                    "           {label}: {}",
                    shape.as_deref().unwrap_or("missing")
                );
            }
        }
    }
    out.push_str(&implementations_report(implemented));
    out.push_str(&options_report(schemas));
    out
}

/// The abstract methods each implemented type has, per version: one added,
/// removed or with another descriptor is `differs`, and the trait changes with it.
fn implementations_report(implemented: &[(String, Vec<Implementation>)]) -> String {
    let mut out = String::new();
    let mut order: Vec<&str> = Vec::new();
    for (_, impls) in implemented {
        for i in impls {
            if !order.contains(&i.java.as_str()) {
                order.push(&i.java);
            }
        }
    }
    for java in order {
        let per_version: Vec<(&str, Option<&Implementation>)> = implemented
            .iter()
            .map(|(l, impls)| (l.as_str(), impls.iter().find(|i| i.java == java)))
            .collect();
        let mut selectors: Vec<&str> = Vec::new();
        for (_, i) in &per_version {
            for m in i.iter().flat_map(|i| &i.methods) {
                if !selectors.contains(&m.selector.as_str()) {
                    selectors.push(&m.selector);
                }
            }
        }
        for sel in selectors {
            let shapes: Vec<(&str, Option<String>)> = per_version
                .iter()
                .map(|(l, i)| {
                    let shape = i
                        .and_then(|i| i.methods.iter().find(|m| m.selector == sel))
                        .map(|m| m.descriptor.clone());
                    (*l, shape)
                })
                .collect();
            let first = &shapes[0].1;
            if first.is_some() && shapes.iter().all(|(_, s)| s == first) {
                let _ = writeln!(out, "  same     implement {java}.{sel}");
                continue;
            }
            let _ = writeln!(out, "  differs  implement {java}.{sel}");
            for (label, shape) in shapes {
                let _ = writeln!(
                    out,
                    "           {label}: {}",
                    shape.as_deref().unwrap_or("missing")
                );
            }
        }
    }
    out
}

/// What changed in the option schema: an option added, removed, or whose type,
/// default, class or tags changed is `differs`.
fn options_report(schemas: &[(String, Option<Schema>)]) -> String {
    let mut out = String::new();
    if schemas.iter().all(|(_, s)| s.is_none()) {
        return out;
    }
    let mut names: Vec<&str> = Vec::new();
    for (_, s) in schemas {
        for o in s.iter().flat_map(|s| &s.options) {
            if !names.contains(&o.name.as_str()) {
                names.push(&o.name);
            }
        }
    }
    names.sort();
    for name in names {
        let shapes: Vec<(&str, Option<String>)> = schemas
            .iter()
            .map(|(l, s)| {
                let shape = s
                    .as_ref()
                    .and_then(|s| s.options.iter().find(|o| o.name == name))
                    .map(|o| o.shape());
                (l.as_str(), shape)
            })
            .collect();
        let first = &shapes[0].1;
        if first.is_some() && shapes.iter().all(|(_, s)| s == first) {
            let _ = writeln!(out, "  same     option {name}");
            continue;
        }
        let _ = writeln!(out, "  differs  option {name}");
        for (label, shape) in shapes {
            let _ = writeln!(
                out,
                "           {label}: {}",
                shape.as_deref().unwrap_or("missing")
            );
        }
    }
    out
}
