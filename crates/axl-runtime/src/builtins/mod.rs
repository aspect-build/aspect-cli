#[cfg(debug_assertions)]
use std::path::Path;
use std::path::PathBuf;

#[cfg(debug_assertions)]
pub fn expand_builtins(
    repo_root: impl AsRef<Path>,
    _broot: PathBuf,
) -> std::io::Result<Vec<(String, PathBuf)>> {
    Ok(vec![(
        "aspect".to_string(),
        repo_root
            .as_ref()
            .join("crates/axl-runtime/src/builtins/aspect"),
    )])
}

#[cfg(not(debug_assertions))]
pub fn expand_builtins(
    _repo_root: PathBuf,
    broot: PathBuf,
) -> std::io::Result<Vec<(String, PathBuf)>> {
    use aspect_config::cli_version;
    use std::fs;

    let builtins_root = broot.join(sha256::digest(cli_version()));
    fs::create_dir_all(&builtins_root)?;

    let builtins = vec![
        ("aspect/build.axl", include_str!("./aspect/build.axl")),
        ("aspect/common.axl", include_str!("./aspect/common.axl")),
        ("aspect/query.axl", include_str!("./aspect/query.axl")),
        ("aspect/test.axl", include_str!("./aspect/test.axl")),
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
