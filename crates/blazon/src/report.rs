//! The shape report: what moved between Bazel versions, for the members a
//! spec selects and for the Starlark surface of the classes it binds.

use crate::ir::Class;
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
    if let Some(f) = &c.starlark {
        out.extend(f.shapes.iter().cloned());
    }
    out
}

pub fn shape_report(versions: &[(String, Vec<Class>)]) -> String {
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
                            .find(|(s, d)| s == &sel && d != "missing")
                            .map(|(_, d)| d.clone())
                    });
                    (*l, found)
                })
                .collect();
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
    out
}
