//! `--dump`: every member of a class with descriptors and generic signatures,
//! and its Starlark surface, which is how a spec entry is written.

use crate::classfile::{ClassInfo, Classes};
use crate::signature::pretty;
use anyhow::Result;
use std::fmt::Write;

pub fn class(classes: &mut Classes, java: &str) -> Result<String> {
    let internal = java.replace('.', "/");
    let info = classes.require(&internal, java)?;
    let mut out = header(&info);
    for f in &info.fields {
        let _ = writeln!(
            out,
            "  field  {:<16} {} {}  {}",
            f.access.modifiers(),
            f.name,
            f.descriptor,
            pretty(&f.ty)
        );
    }
    for m in &info.methods {
        let params: Vec<String> = m.params.iter().map(pretty).collect();
        let _ = writeln!(
            out,
            "  method {:<16} {}{}  {} {}({}){}{}",
            m.access.modifiers(),
            m.name,
            m.descriptor,
            pretty(&m.ret),
            m.name,
            params.join(", "),
            if m.nullable_ret { "  @Nullable" } else { "" },
            if m.access.varargs { "  varargs" } else { "" },
        );
    }
    out.push_str(&crate::facet::describe(classes, &internal)?);
    Ok(out)
}

fn header(info: &ClassInfo) -> String {
    let mut out = String::new();
    let tps: Vec<String> = info
        .type_params
        .iter()
        .map(|t| {
            if t.bounds.is_empty() {
                t.name.clone()
            } else {
                format!(
                    "{} extends {}",
                    t.name,
                    t.bounds.iter().map(pretty).collect::<Vec<_>>().join(" & ")
                )
            }
        })
        .collect();
    let _ = writeln!(
        out,
        "{} {}{}",
        if info.is_interface {
            "interface"
        } else {
            "class"
        },
        info.name.replace('/', "."),
        if tps.is_empty() {
            String::new()
        } else {
            format!("<{}>", tps.join(", "))
        }
    );
    if let Some(s) = &info.super_class {
        let _ = writeln!(out, "  extends {}", pretty(s));
    }
    for i in &info.interfaces {
        let _ = writeln!(out, "  implements {}", pretty(i));
    }
    out
}
