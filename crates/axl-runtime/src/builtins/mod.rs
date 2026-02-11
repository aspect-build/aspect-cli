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
        ("build.axl", include_str!("./aspect/build.axl")),
        ("test.axl", include_str!("./aspect/test.axl")),
        ("axl_add.axl", include_str!("./aspect/axl_add.axl")),
        ("MODULE.aspect", include_str!("./aspect/MODULE.aspect")),
    ],
};

#[cfg(not(debug_assertions))]
const AXEL_F: Builtin = Builtin {
    name: "axel-f",
    files: axel_f::FILES,
};

#[cfg(not(debug_assertions))]
const ALL: &[&Builtin] = &[&ASPECT, &AXEL_F];

#[cfg(debug_assertions)]
pub fn expand_builtins(
    _root_dir: PathBuf,
    _broot: PathBuf,
) -> std::io::Result<Vec<(String, PathBuf)>> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Ok(vec![
        (
            "aspect".to_string(),
            manifest_dir.join("src/builtins/aspect"),
        ),
        ("axel-f".to_string(), manifest_dir.join("../../axel-f")),
    ])
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
