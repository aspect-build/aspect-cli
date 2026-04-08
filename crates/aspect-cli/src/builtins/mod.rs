use std::path::PathBuf;

#[cfg(not(debug_assertions))]
use include_dir::{Dir, DirEntry, include_dir};

#[cfg(not(debug_assertions))]
static ASPECT_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/builtins/aspect");

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

    // Hash all embedded file content to detect staleness across versions
    let content_hash = {
        let mut combined = String::new();
        for entry in ASPECT_DIR.find("**/*").unwrap() {
            if let DirEntry::File(f) = entry {
                combined.push_str(f.path().to_str().unwrap());
                combined.push_str(f.contents_utf8().unwrap_or(""));
            }
        }
        sha256::digest(combined)
    };

    let builtins_root = broot.join(content_hash);
    let dir = builtins_root.join("aspect");

    if !dir.exists() {
        for entry in ASPECT_DIR.find("**/*").unwrap() {
            if let DirEntry::File(f) = entry {
                let out_path = dir.join(f.path());
                fs::create_dir_all(out_path.parent().unwrap())?;
                fs::write(&out_path, f.contents())?;
            }
        }
    }

    Ok(vec![("aspect".to_string(), dir)])
}
