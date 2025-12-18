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
    use aspect_telemetry::cargo_pkg_version;
    use std::fs;

    let builtins_root = broot.join(sha256::digest(cargo_pkg_version()));
    fs::create_dir_all(&builtins_root)?;

    let builtins = vec![
        ("aspect/build.axl", include_str!("./aspect/build.axl")),
        ("aspect/test.axl", include_str!("./aspect/test.axl")),
        ("aspect/axl_add.axl", include_str!("./aspect/axl_add.axl")),
        (
            "aspect/MODULE.aspect",
            include_str!("./aspect/MODULE.aspect"),
        ),
    ];

    for (path, content) in builtins {
        let out_path = &builtins_root.join(path);
        fs::create_dir_all(&out_path.parent().unwrap())?;
        fs::write(out_path, content)?;
    }

    Ok(vec![("aspect".to_string(), builtins_root.join("aspect"))])
}
