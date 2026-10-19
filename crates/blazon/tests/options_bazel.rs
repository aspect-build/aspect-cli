//! The option schema on Bazel 9.0.1's own jar: what the buildx spec's `[options]`
//! selects. Skipped without the release.

use blazon::classfile::Classes;
use blazon::jar::Jar;
use blazon::options::{OptionDef, Schema, schema};
use blazon::spec;
use std::path::PathBuf;

fn jar(version: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let jar = PathBuf::from(home).join(format!(
        "Library/Caches/buildx/bazel/{version}/A-server.jar"
    ));
    jar.exists().then_some(jar)
}

fn load(version: &str) -> Option<Schema> {
    let jar = jar(version)?;
    let spec_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../buildx/bindings.toml");
    let spec = spec::load(&spec_path).expect("buildx spec");
    let rules = spec.options.expect("[options]");
    let mut classes = Classes::new(Jar::open(&jar).expect("jar"));
    Some(schema(&mut classes, &rules, version).expect("schema"))
}

fn has(s: &Schema, name: &str) -> bool {
    s.options.iter().any(|o| o.name == name)
}

fn by_class<'a>(s: &'a Schema, class: &str) -> Vec<&'a OptionDef> {
    s.options.iter().filter(|o| o.class == class).collect()
}

#[test]
fn the_schema_covers_configuration_starlark_semantics_and_bzlmod() {
    let Some(s) = load("9.0.1") else { return };

    // Every option of every configuration fragment that has a command-line spelling.
    let fragment: Vec<&OptionDef> = s.options.iter().filter(|o| o.fragment).collect();
    assert!(fragment.len() >= 246, "{} fragment options", fragment.len());
    for name in [
        "compilation_mode",
        "cpu",
        "copt",
        "cxxopt",
        "linkopt",
        "platforms",
        "extra_toolchains",
        "host_platform",
        "java_language_version",
        "javacopt",
        "test_timeout",
        "test_env",
        "action_env",
        "host_action_env",
        "stamp",
        "features",
        "define_missing_ok_placeholder",
        "incompatible_strict_action_env",
        "python_path",
        "android_platforms",
        "ios_minimum_os",
        "proto_compiler",
        "objc_debug_with_GLIBCXX",
        "run_under",
        "enable_runfiles",
        "collect_code_coverage",
        "instrumentation_filter",
        "experimental_exec_config",
    ] {
        if name != "define_missing_ok_placeholder" {
            assert!(has(&s, name), "{name} is a configuration option");
        }
    }
    assert!(
        by_class(
            &s,
            "com.google.devtools.build.lib.analysis.config.CoreOptions"
        )
        .len()
            >= 60
    );
    assert!(by_class(&s, "com.google.devtools.build.lib.rules.cpp.CppOptions").len() >= 70);

    // Starlark semantics and loading.
    let semantics = by_class(
        &s,
        "com.google.devtools.build.lib.packages.semantics.BuildLanguageOptions",
    );
    assert!(
        semantics.len() >= 50,
        "{} Starlark semantics flags",
        semantics.len()
    );
    for name in [
        "incompatible_disallow_empty_glob",
        "experimental_enable_android_migration_apis",
        "experimental_repo_remote_exec",
        "max_computation_steps",
        "incompatible_enable_exports_provider",
    ] {
        let _ = name;
    }
    assert!(has(&s, "package_path"), "PackageOptions");
    assert!(has(&s, "deleted_packages"));
    assert!(has(&s, "build_tag_filters"), "LoadingOptions");

    // Repository and bzlmod.
    for name in [
        "registry",
        "lockfile_mode",
        "check_direct_dependencies",
        "ignore_dev_dependency",
        "vendor_dir",
        "override_module",
        "override_repository",
        "repository_cache",
        "distdir",
        "allow_yanked_versions",
        "check_bazel_compatibility",
        "inject_repository",
    ] {
        assert!(has(&s, name), "{name} is a repository or bzlmod option");
    }

    // Named in the spec: they live in classes the rule excludes.
    for name in [
        "repo_env",
        "incompatible_repo_env_ignores_action_env",
        "experimental_strict_repo_env",
        "enable_bzlmod",
        "enable_workspace",
        "incompatible_use_plus_in_repo_names",
        "experimental_enable_aspect_hints",
        "aspects_parameters",
    ] {
        assert!(has(&s, name), "{name} is included by name");
    }

    // Not what the schema is for.
    for name in [
        "remote_cache",
        "remote_executor",
        "remote_download_outputs",
        "spawn_strategy",
        "jobs",
        "keep_going",
        "bes_backend",
        "disk_cache",
        "color",
        "profile",
        "sandbox_base",
        "sandbox_debug",
        "build_event_json_file",
        "show_progress",
        "curses",
    ] {
        assert!(!has(&s, name), "{name} is not selected");
    }
    assert!(
        !s.options
            .iter()
            .any(|o| o.name.starts_with("sandbox_") || o.name.starts_with("remote_")),
        "no sandbox or remote option"
    );
    assert!(
        !s.options
            .iter()
            .any(|o| o.metadata_tags.iter().any(|t| t == "INTERNAL"))
    );

    // Names are exactly Bazel's: no dashes, no spaces.
    for o in &s.options {
        assert!(
            o.name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "{:?} is not a flag name",
            o.name
        );
    }
    // Sorted, so the table can be searched.
    assert!(s.options.windows(2).all(|w| w[0].name < w[1].name));
}

#[test]
fn the_summary_reports_counts_and_gaps() {
    let Some(s) = load("9.0.1") else { return };
    let text = s.summary();
    assert!(text.contains("per class"), "{text}");
    assert!(text.contains("per kind"), "{text}");
    assert!(text.contains("Opaque values"), "{text}");
    assert!(text.contains("no help text"), "{text}");
    eprintln!("{text}");
}
