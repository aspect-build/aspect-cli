//! Generates the fixture bindings always, and Bazel 9.0.1's when its server
//! jar is in the buildx cache; without it `cfg(no_bazel)` hides those tests.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(no_bazel)");
    println!("cargo:rerun-if-changed=fixture.toml");
    println!("cargo:rerun-if-changed=bazel.toml");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../blazon/tests/fixture/fixture.jar");
    generate("fixture", "fixture", &fixture, &out);
    match bazel_jar() {
        Some(jar) => generate("bazel", "9.0.1", &jar, &out),
        None => {
            println!("cargo:rustc-cfg=no_bazel");
            std::fs::create_dir_all(out.join("bazel")).expect("mkdir");
            std::fs::write(out.join("bazel/mod.rs"), "").expect("write");
        }
    }
}

fn bazel_jar() -> Option<PathBuf> {
    let cache = match std::env::var_os("BLAZON_BAZEL_CACHE") {
        Some(c) => PathBuf::from(c),
        None => PathBuf::from(std::env::var_os("HOME")?).join("Library/Caches/buildx"),
    };
    let jar = cache.join("bazel/9.0.1/A-server.jar");
    jar.exists().then_some(jar)
}

fn generate(name: &str, label: &str, jar: &Path, out: &Path) {
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("{name}.toml"));
    let version = blazon::Version {
        label: label.to_string(),
        jar: jar.to_path_buf(),
    };
    if let Err(e) = blazon::generate(&spec, &[version], &out.join(name)) {
        eprintln!("error: {name}.toml: {e:#}");
        std::process::exit(1);
    }
}
