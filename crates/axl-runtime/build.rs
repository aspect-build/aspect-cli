//! Refuse to build against a stale patched starlark.
//!
//! axl-runtime relies on the patches in `patches/`. Bazel applies them itself
//! and never runs this script. A cargo build gets them only through the copy
//! `cargo patch-crate` prepares under `target/patch/`, which
//! `.cargo/config.toml` points cargo at. A missing copy already fails in cargo
//! itself, before any build script runs; what this catches is a copy older
//! than its patch, which would otherwise compile the previous patch.
//!
//! It cannot apply the patches: cargo chooses every dependency's source
//! before any build script runs.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

fn main() -> io::Result<()> {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let root = manifest.join("../..");
    let patches = root.join("patches");

    let mut out = io::stdout().lock();
    writeln!(out, "cargo::rerun-if-changed={}", patches.display())?;

    let mut entries = match fs::read_dir(&patches) {
        Ok(entries) => entries
            .map(|e| e.map(|e| e.path()))
            .collect::<io::Result<Vec<_>>>()?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    entries.sort();

    for patch in entries {
        // `cargo patch-crate` names patches `<crate>+<version>.patch` and
        // writes the patched crate to `target/patch/<crate>-<version>`.
        let Some(stem) = patch
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".patch"))
        else {
            continue;
        };
        let Some((name, version)) = stem.split_once('+') else {
            continue;
        };
        let copy = root.join("target/patch").join(format!("{name}-{version}"));
        writeln!(out, "cargo::rerun-if-changed={}", patch.display())?;
        writeln!(out, "cargo::rerun-if-changed={}", copy.display())?;

        let problem = match fs::metadata(&copy) {
            Err(_) => Some(format!("the patched `{name}` {version} is missing")),
            Ok(copied) => (fs::metadata(&patch)?.modified()? > copied.modified()?).then(|| {
                format!("the patched `{name}` {version} is older than patches/{stem}.patch")
            }),
        };
        if let Some(problem) = problem {
            writeln!(
                out,
                "cargo::error={problem}; run `cargo patch-crate --force` from the repository root"
            )?;
            out.flush()?;
            std::process::exit(1);
        }
    }
    out.flush()
}
