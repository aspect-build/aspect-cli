//! blazon: typed Rust bindings to Bazel's server classes, generated per Bazel
//! version from that release's `A-server.jar`.
//!
//! A spec names the classes and members a consumer uses. For each Bazel
//! version blazon reads the class files, resolves the members' descriptors,
//! access, generic signatures and nullability, and writes one Rust module per
//! version plus a shape report of what moved between versions. The generated
//! code depends on `blazon-rt` at runtime. With the `release` feature,
//! `build_script` is the entry point for a consumer's `build.rs` and
//! `release` locates and extracts Bazel releases.

pub mod annot;
pub mod classfile;
pub mod dump;
pub mod emit;
pub mod facet;
pub mod ir;
pub mod jar;
pub mod jdk;
pub mod lower;
pub mod names;
#[cfg(feature = "release")]
pub mod release;
pub mod report;
pub mod signature;
pub mod spec;

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub struct Version {
    pub label: String,
    pub jar: PathBuf,
}

/// What a generation produced: the resolved classes of the last version and,
/// with more than one version, the shape report.
pub struct Report {
    pub versions: Vec<String>,
    pub classes: Vec<ir::Class>,
    pub shape: Option<String>,
}

/// The `A-server.jar` of a Bazel release, extracted into the cache on first use.
#[cfg(feature = "release")]
pub fn jar_for_bazel(version: &str) -> Result<PathBuf> {
    Ok(release::Release {
        version: version.to_string(),
    }
    .install(None)?
    .server_jar)
}

/// Writes `text` to `path` unless the file already has it, so that a rerun
/// does not touch what did not change and force a rebuild of its consumers.
fn write_if_changed(path: &Path, text: &str) -> Result<()> {
    if fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(());
    }
    fs::write(path, text).with_context(|| format!("write {}", path.display()))
}

pub fn generate(spec_path: &Path, versions: &[Version], out: &Path) -> Result<Report> {
    let spec = spec::load(spec_path)?;
    fs::create_dir_all(out)?;
    let mods: Vec<String> = versions.iter().map(|v| module_name(&v.label)).collect();
    for stale in fs::read_dir(out)?.filter_map(|e| e.ok()) {
        let name = stale.file_name().to_string_lossy().into_owned();
        if name.starts_with('v')
            && name.ends_with(".rs")
            && !mods.iter().any(|m| name == format!("{m}.rs"))
        {
            fs::remove_file(stale.path())?;
        }
    }
    let mut resolved = Vec::new();
    for v in versions {
        let jar = jar::Jar::open(&v.jar)?;
        let mut classes = classfile::Classes::new(jar);
        let ir = lower::lower(&spec, &mut classes, &v.label)?;
        let source = emit::module(&v.label, &ir).with_context(|| format!("Bazel {}", v.label))?;
        write_if_changed(&out.join(format!("{}.rs", module_name(&v.label))), &source)?;
        resolved.push((v.label.clone(), ir));
    }
    let out_abs = out.canonicalize().unwrap_or_else(|_| out.to_path_buf());
    let starlark = resolved
        .iter()
        .any(|(_, cs)| cs.iter().any(|c| c.starlark.is_some()));
    write_if_changed(
        &out.join("mod.rs"),
        &emit::mod_rs(&out_abs, &mods, starlark)?,
    )?;
    let shape = (resolved.len() > 1).then(|| report::shape_report(&resolved));
    Ok(Report {
        versions: versions.iter().map(|v| v.label.clone()).collect(),
        classes: resolved.pop().map(|(_, c)| c).unwrap_or_default(),
        shape,
    })
}

/// For a consumer's `build.rs`: binds the spec's `versions`, fetching releases
/// as needed, into `out`, and prints the cargo rerun directives. Errors are
/// readable as-is; the caller should print them and fail the build.
#[cfg(feature = "release")]
pub fn build_script(spec_path: &Path, out: &Path) -> Result<Report> {
    println!("cargo:rerun-if-changed={}", spec_path.display());
    println!("cargo:rerun-if-env-changed=BLAZON_BAZEL_CACHE");
    let spec = spec::load(spec_path)?;
    if spec.versions.is_empty() {
        anyhow::bail!(
            "{}: `versions = [...]` is required for build-script generation",
            spec_path.display()
        );
    }
    let mut versions = Vec::new();
    for v in &spec.versions {
        let jar = jar_for_bazel(v).with_context(|| format!("Bazel {v}: no release to bind"))?;
        versions.push(Version {
            label: v.clone(),
            jar,
        });
    }
    generate(spec_path, &versions, out)
}

pub fn module_name(label: &str) -> String {
    format!("v{}", label.replace(['.', '-'], "_"))
}

/// Every member of the given classes, one per line, for writing a spec.
pub fn dump(jar: &Path, classes: &[String]) -> Result<String> {
    let mut classes_in_jar = classfile::Classes::new(jar::Jar::open(jar)?);
    let mut out = String::new();
    for java in classes {
        out.push_str(&dump::class(&mut classes_in_jar, java)?);
    }
    Ok(out)
}
