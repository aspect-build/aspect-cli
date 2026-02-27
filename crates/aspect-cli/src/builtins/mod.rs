use std::path::PathBuf;

/// A builtin module: name and its embedded files (relative path, content).
#[cfg(not(debug_assertions))]
struct Builtin {
    name: &'static str,
    files: &'static [(&'static str, &'static str)],
}

#[cfg(not(debug_assertions))]
const ASPECT: Builtin = Builtin {
    name: "aspect",
    files: &[
        ("bazel.axl", include_str!("./aspect/bazel.axl")),
        ("build.axl", include_str!("./aspect/build.axl")),
        ("fragments.axl", include_str!("./aspect/fragments.axl")),
        ("test.axl", include_str!("./aspect/test.axl")),
        ("axl_add.axl", include_str!("./aspect/axl_add.axl")),
        ("MODULE.aspect", include_str!("./aspect/MODULE.aspect")),
        // config/
        (
            "config/builtins.axl",
            include_str!("./aspect/config/builtins.axl"),
        ),
        (
            "config/delivery.axl",
            include_str!("./aspect/config/delivery.axl"),
        ),
        ("config/lint.axl", include_str!("./aspect/config/lint.axl")),
        (
            "config/nolint.axl",
            include_str!("./aspect/config/nolint.axl"),
        ),
        (
            "config/artifacts.axl",
            include_str!("./aspect/config/artifacts.axl"),
        ),
        // tasks/
        (
            "tasks/delivery.axl",
            include_str!("./aspect/tasks/delivery.axl"),
        ),
        (
            "tasks/dummy_lint.axl",
            include_str!("./aspect/tasks/dummy_lint.axl"),
        ),
        (
            "tasks/dummy_format.axl",
            include_str!("./aspect/tasks/dummy_format.axl"),
        ),
        // lib/
        (
            "lib/deliveryd.axl",
            include_str!("./aspect/lib/deliveryd.axl"),
        ),
        ("lib/github.axl", include_str!("./aspect/lib/github.axl")),
        ("lib/linting.axl", include_str!("./aspect/lib/linting.axl")),
        (
            "lib/platform.axl",
            include_str!("./aspect/lib/platform.axl"),
        ),
        ("lib/sarif.axl", include_str!("./aspect/lib/sarif.axl")),
        (
            "lib/health_check.axl",
            include_str!("./aspect/lib/health_check.axl"),
        ),
        (
            "lib/artifacts.axl",
            include_str!("./aspect/lib/artifacts.axl"),
        ),
        ("lib/tar.axl", include_str!("./aspect/lib/tar.axl")),
        (
            "lib/environment.axl",
            include_str!("./aspect/lib/environment.axl"),
        ),
        (
            "lib/build_metadata.axl",
            include_str!("./aspect/lib/build_metadata.axl"),
        ),
    ],
};

#[cfg(not(debug_assertions))]
const ALL: &[&Builtin] = &[&ASPECT];

#[cfg(debug_assertions)]
pub fn expand_builtins(
    _root_dir: PathBuf,
    _broot: PathBuf,
) -> std::io::Result<Vec<(String, PathBuf)>> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Ok(vec![(
        "aspect".to_string(),
        manifest_dir.join("src/builtins/aspect"),
    )])
}

#[cfg(not(debug_assertions))]
pub fn expand_builtins(
    _root_dir: PathBuf,
    broot: PathBuf,
) -> std::io::Result<Vec<(String, PathBuf)>> {
    use std::fs;

    // Hash all builtin content to detect staleness across versions
    let content_hash = {
        let mut combined = String::new();
        for builtin in ALL {
            combined.push_str(builtin.name);
            for (path, content) in builtin.files {
                combined.push_str(path);
                combined.push_str(content);
            }
        }
        sha256::digest(combined)
    };

    let builtins_root = broot.join(content_hash);

    // Extract each builtin into its own directory
    for builtin in ALL {
        let dir = builtins_root.join(builtin.name);
        if !dir.exists() {
            for (path, content) in builtin.files {
                let out_path = dir.join(path);
                fs::create_dir_all(out_path.parent().unwrap())?;
                fs::write(&out_path, content)?;
            }
        }
    }

    Ok(ALL
        .iter()
        .map(|b| (b.name.to_string(), builtins_root.join(b.name)))
        .collect())
}
