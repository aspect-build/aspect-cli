//! Generated bindings under test: `fixture` always, `bazel` when Bazel 9.0.1 is
//! in the buildx cache. See `tests/`.

pub mod fixture {
    include!(concat!(env!("OUT_DIR"), "/fixture/mod.rs"));
}

#[cfg(not(no_bazel))]
pub mod bazel {
    include!(concat!(env!("OUT_DIR"), "/bazel/mod.rs"));
}

/// The `libjvm` to host the tests in: `$BLAZON_LIBJVM`, the JDK embedded in
/// the cached Bazel release, or `$JAVA_HOME`.
pub fn libjvm() -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os("BLAZON_LIBJVM") {
        return Some(p.into());
    }
    let name = if cfg!(target_os = "macos") {
        "libjvm.dylib"
    } else {
        "libjvm.so"
    };
    let mut candidates = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(
            std::path::PathBuf::from(home)
                .join("Library/Caches/buildx/bazel/9.0.1/embedded_tools/jdk/lib/server"),
        );
    }
    if let Some(java_home) = std::env::var_os("JAVA_HOME") {
        candidates.push(std::path::PathBuf::from(java_home).join("lib/server"));
    }
    candidates
        .into_iter()
        .map(|d| d.join(name))
        .find(|p| p.exists())
}
