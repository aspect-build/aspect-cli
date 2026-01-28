use std::path::PathBuf;

#[cfg(debug_assertions)]
pub fn expand_builtins(
    _root_dir: PathBuf,
    _broot: PathBuf,
) -> std::io::Result<Vec<(String, PathBuf)>> {
    // Use CARGO_MANIFEST_DIR to locate builtins relative to this crate's source,
    // not the user's project root (which could be /tmp or anywhere)
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

    const BUILTINS: &[(&str, &str)] = &[
        ("aspect/build.axl", include_str!("./aspect/build.axl")),
        ("aspect/test.axl", include_str!("./aspect/test.axl")),
        ("aspect/axl_add.axl", include_str!("./aspect/axl_add.axl")),
        (
            "aspect/MODULE.aspect",
            include_str!("./aspect/MODULE.aspect"),
        ),
    ];

    // Hash content to ensure staleness is detected when files change,
    // even without a version bump
    let content_hash = {
        let mut combined = String::new();
        for (path, content) in BUILTINS {
            combined.push_str(path);
            combined.push_str(content);
        }
        sha256::digest(combined)
    };

    let builtins_root = broot.join(content_hash);

    // Only write if directory doesn't exist - content hash guarantees correctness
    if !builtins_root.join("aspect").exists() {
        for (path, content) in BUILTINS {
            let out_path = builtins_root.join(path);
            fs::create_dir_all(out_path.parent().unwrap())?;
            fs::write(&out_path, content)?;
        }
    }

    Ok(vec![("aspect".to_string(), builtins_root.join("aspect"))])
}
