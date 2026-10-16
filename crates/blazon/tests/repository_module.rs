//! The private members of Bazel 9.0.1's `BazelRepositoryModule`, which
//! buildx seeds, as typed getters and setters. Skipped without the release.

use blazon::{Version, generate};
use std::path::PathBuf;

const SPEC: &str = r#"
versions = ["9.0.1"]

[[class]]
java = "com.google.devtools.build.lib.bazel.BazelRepositoryModule"
members = [
    "<init>()",
    "repositoryCache",
    "repoEnvSupplier",
    "nonstrictRepoEnvSupplier",
    "registries?",
    "moduleMirrors?",
    "clock?",
    "repositoryFetchFunction?",
    "moduleFileFunction?",
    "repoSpecFunction?",
    "yankedVersionsFunction?",
    "vendorCommand",
]

[[class]]
java = "com.google.devtools.build.lib.bazel.repository.cache.RepositoryCache"
members = []
[[class]]
java = "com.google.devtools.build.lib.skyframe.MutableSupplier"
members = []
[[class]]
java = "com.google.common.collect.ImmutableMap"
members = []
[[class]]
java = "com.google.common.collect.ImmutableSet"
members = []
[[class]]
java = "com.google.devtools.build.lib.clock.Clock"
members = []
[[class]]
java = "com.google.devtools.build.lib.bazel.repository.RepositoryFetchFunction"
members = []
[[class]]
java = "com.google.devtools.build.lib.bazel.bzlmod.ModuleFileFunction"
members = []
[[class]]
java = "com.google.devtools.build.lib.bazel.bzlmod.RepoSpecFunction"
members = []
[[class]]
java = "com.google.devtools.build.lib.bazel.bzlmod.YankedVersionsFunction"
members = []
[[class]]
java = "com.google.devtools.build.lib.bazel.commands.VendorCommand"
members = []
"#;

#[test]
fn private_members_become_typed_accessors() -> anyhow::Result<()> {
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(());
    };
    let jar = PathBuf::from(home).join("Library/Caches/buildx/bazel/9.0.1/A-server.jar");
    if !jar.exists() {
        return Ok(());
    }
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("repository-module");
    std::fs::create_dir_all(&out)?;
    let spec = out.join("spec.toml");
    std::fs::write(&spec, SPEC)?;
    generate(
        &spec,
        &[Version {
            label: "9.0.1".into(),
            jar,
        }],
        &out.join("bindings"),
    )?;
    let code = std::fs::read_to_string(out.join("bindings/v9_0_1.rs"))?;
    let squash = |s: &str| s.split_whitespace().collect::<String>();
    let flat = squash(&code);
    for accessor in [
        "pub fn repository_cache(",
        "blazon_rt::Result<RepositoryCache<'a>>",
        "pub fn set_repository_cache(",
        "pub fn repo_env_supplier(",
        "MutableSupplier<'a, ImmutableMap<'static, JavaString<'static>, JavaString<'static>>",
        "pub fn set_repo_env_supplier(",
        "pub fn nonstrict_repo_env_supplier(",
        "pub fn set_nonstrict_repo_env_supplier(",
        "pub fn registries(",
        "blazon_rt::Result<Option<ImmutableSet<'a, JavaString<'static>>>>",
        "pub fn set_registries(",
        "pub fn module_mirrors(",
        "pub fn clock(",
        "blazon_rt::Result<Option<Clock<'a>>>",
        "pub fn set_clock(",
        "pub fn repository_fetch_function(",
        "pub fn module_file_function(",
        "pub fn repo_spec_function(",
        "pub fn yanked_versions_function(",
        "pub fn vendor_command(",
    ] {
        assert!(flat.contains(&squash(accessor)), "missing `{accessor}`");
    }
    assert!(
        flat.contains("ReflectField::new"),
        "private fields are reached through reflection"
    );
    Ok(())
}
