//! Generates `$OUT_DIR/bindings/` from `bindings.toml` for every Bazel
//! version the spec lists, fetching releases into the blazon cache on first
//! build. A generation error names the member and version and fails the build.

use std::path::{Path, PathBuf};

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("bindings");
    match blazon::build_script(Path::new("bindings.toml"), &out) {
        Ok(report) => {
            if let Some(shape) = report.shape {
                eprint!("{shape}");
            }
        }
        Err(e) => {
            eprintln!("error: bindings.toml: {e:#}");
            std::process::exit(1);
        }
    }
}
