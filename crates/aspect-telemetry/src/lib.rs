//! Usage telemetry for the Aspect CLI and launcher.
//!
//! Builds the report posted to Aspect's ingest endpoint and sends it on a
//! best-effort, fire-and-forget basis. `DO_NOT_TRACK` disables sending
//! entirely.
//!
//! Reports share an aggregation table with the Bazel rulesets'
//! (https://github.com/aspect-build/tools_telemetry), which is why field names
//! mirror that module's vocabulary (`os`, `arch`, `ci`, `runner`, `counter`,
//! `id_day`) and why `id_day` must be derived exactly as the module derives it
//! — two implementations that disagree count the same repository twice. Fields
//! specific to this crate name their subject (`cli_version`), since the table
//! also carries Bazel and module versions. When adding fields, prefer reusing
//! the module's names over inventing new ones; Bazel-specific fields are
//! intentionally omitted because the module already covers those.

use chrono::Utc;
use reqwest::header::HeaderName;
use reqwest::redirect::Policy;
use reqwest::{self, Method, StatusCode};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use std::env::{current_dir, var};
use std::fs::read_to_string;
use std::path::{Path, PathBuf};
use std::time::Duration;

// The Bazel arch and os per @platforms and //bazel/platforms
pub static BZLOS: &str = env!("BUILD_BZLOS");
pub static BZLARCH: &str = env!("BUILD_BZLARCH");

// And the GOOS/GOARCH equivalents
pub static GOOS: &str = env!("BUILD_GOOS");
pub static GOARCH: &str = env!("BUILD_GOARCH");
pub static LLVM_TRIPLE: &str = env!("LLVM_TRIPLE");

static TELURL: &str = "https://telemetry.aspect.build/ingest";

/// Pull the version of the currently running rust binary from CARGO_PKG_VERSION env.  This env
/// is injected into the rust build artifacts with the version_key attribute on rust_library & rust_binary
/// and is set for release builds with stamping. Defaults to "0.0.0-dev" on unstamped builds.
pub fn cargo_pkg_version() -> String {
    option_env!("CARGO_PKG_VERSION")
        .map(|label| {
            if label == "{CARGO_PKG_VERSION}" {
                "0.0.0-dev"
            } else {
                label
            }
        })
        .unwrap_or("0.0.0-dev")
        .into()
}

/// A short variant of the monorepo version. For examples, 2025.34.0 if the monorepo
/// version was 2025.34.0+201b9a8. See https://blog.aspect.build/versioning-releases-from-a-monorepo.
pub fn cargo_pkg_short_version() -> String {
    let s = cargo_pkg_version();
    match s.find('+') {
        Some(i) => s[..i].to_string(),
        None => s,
    }
}

/// Whether this binary is the `-debug-` release variant (or a local dev build).
///
/// The variant is built with `-Cdebug-assertions=y` applied across the whole graph,
/// which the stripped `-c opt` release never sets, so the flag identifies the build
/// itself. Deliberately not an environment check: `ASPECT_DEBUG_CLI` is the launcher's
/// request for the variant, not proof that the running binary is one.
pub fn is_debug_build() -> bool {
    cfg!(debug_assertions)
}

/// Version for display, suffixed when running a build that trades speed for
/// diagnostics so it is obvious which binary produced a log or crash report.
pub fn cargo_pkg_display_version() -> String {
    let v = cargo_pkg_short_version();
    if is_debug_build() {
        format!("{v} (debug build)")
    } else {
        v
    }
}

pub fn do_not_track() -> bool {
    var("DO_NOT_TRACK").is_ok()
}

/// Salted SHA-1 of `data`, mirroring `tools_telemetry`'s `hash` helper. Honors
/// the `ASPECT_TOOLS_TELEMETRY_SALT` env var so a single salt covers both
/// sources, and so a repository that sets one is hashed identically by each.
fn salted_hash(data: &str) -> String {
    let mut hasher = Sha1::new();
    if let Ok(salt) = var("ASPECT_TOOLS_TELEMETRY_SALT") {
        hasher.update(salt.as_bytes());
        hasher.update(b";");
    }
    hasher.update(data.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Candidate documentation filenames, in `tools_telemetry`'s search order:
/// directory, then basename, then extension. The order decides which file wins
/// when a repository has several, so it has to match the module's exactly.
const REPO_ID_DIRS: &[&str] = &["", "doc", "docs", "Doc", "Docs"];
const REPO_ID_BASES: &[&str] = &["README", "readme", "Readme", "index"];
const REPO_ID_EXTS: &[&str] = &[
    "",
    ".adoc",
    ".asc",
    ".asciidoc",
    ".markdown",
    ".md",
    ".mdown",
    ".mkdk",
    ".org",
    ".rdoc",
    ".rst",
    ".textile",
    ".txt",
    ".wiki",
];

/// Markers of a project root. For bzlmod repositories this lands on the same
/// directory the module's `repository_ctx.workspace_root` does; the Aspect
/// project markers (mirroring the CLI's own root detection) and the
/// WORKSPACE-era markers extend coverage to repositories the module never
/// runs in, where any consistent root serves deduplication equally well.
const ROOT_MARKERS: &[&str] = &[
    // Aspect project roots: MODULE.aspect per axl-runtime's AXL_MODULE_FILE,
    // plus the version pin the CLI itself anchors on.
    "MODULE.aspect",
    ".aspect/version.axl",
    // Bazel workspace roots.
    "MODULE.bazel",
    "MODULE.bazel.lock",
    "REPO.bazel",
    "WORKSPACE",
    "WORKSPACE.bazel",
];

/// Nearest ancestor of `start` holding a project-root marker.
///
/// Walked here rather than taken from a caller so this crate stays a leaf
/// dependency of the CLI and the launcher.
fn workspace_root_from(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if ROOT_MARKERS.iter().any(|m| dir.join(m).exists()) {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn workspace_root() -> Option<PathBuf> {
    workspace_root_from(&current_dir().ok()?)
}

/// Stable per-repository value, never reported on its own.
///
/// Hashes the first four lines of the repository's documentation, falling back
/// to `MODULE.bazel`, exactly as `tools_telemetry` does — the two have to agree
/// or the same repository counts twice.
fn repo_id(root: &Path) -> Option<String> {
    let mut chosen = None;
    'search: for dir in REPO_ID_DIRS {
        for base in REPO_ID_BASES {
            for ext in REPO_ID_EXTS {
                let candidate = root.join(dir).join(format!("{base}{ext}"));
                if candidate.is_file() {
                    chosen = Some(candidate);
                    break 'search;
                }
            }
        }
    }
    let path = chosen.unwrap_or_else(|| root.join("MODULE.bazel"));
    let content = read_to_string(path).ok()?;
    let head: Vec<&str> = content.split('\n').take(4).collect();
    Some(salted_hash(&head.join("\n")))
}

/// Day-scoped repository value: reports from one repository group together
/// within a UTC day and cannot be linked across days.
fn id_day_for(root: &Path, date: &str) -> Option<String> {
    let id = repo_id(root)?;
    Some(salted_hash(&format!("{id};{date}")))
}

fn id_day() -> Option<String> {
    id_day_for(
        &workspace_root()?,
        &Utc::now().format("%Y-%m-%d").to_string(),
    )
}

/// Returns the first env var from `vars` that is set and non-empty.
fn first_env(vars: &[&str]) -> Option<String> {
    for v in vars {
        if let Ok(val) = var(v) {
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}

fn is_ci() -> bool {
    var("CI").is_ok()
}

/// Identify the CI/CD runner, mirroring `tools_telemetry`'s `_build_runner`.
///
/// Probe order matters: Forgejo and Gitea Actions both set `GITHUB_RUN_NUMBER`
/// for compatibility, so they must be detected before `github-actions` or
/// they'd be misclassified — and the aggregator-side `runner` grouping would
/// disagree with what `tools_telemetry` reports for the same users.
fn runner() -> Option<String> {
    let probes: &[(&str, &str)] = &[
        ("BUILDKITE_BUILD_NUMBER", "buildkite"),
        // We only test presence; the value is never read or transmitted.
        ("FORGEJO_TOKEN", "forgejo"),
        ("GITEA_ACTIONS", "gitea"),
        ("GITHUB_RUN_NUMBER", "github-actions"),
        ("GITLAB_CI", "gitlab"),
        ("CIRCLE_BUILD_NUM", "circleci"),
        ("DRONE_BUILD_NUMBER", "drone"),
        ("BUILD_NUMBER", "jenkins"),
        ("TRAVIS", "travis"),
    ];
    for (env, name) in probes {
        if var(env).is_ok() {
            return Some((*name).to_string());
        }
    }
    first_env(&["CI_SYSTEM_NAME"])
}

/// Build counter from CI env, mirroring `tools_telemetry`'s `_build_counter`.
fn build_counter() -> Option<String> {
    first_env(&[
        "BUILDKITE_BUILD_NUMBER",
        "GITHUB_RUN_NUMBER",
        "CI_PIPELINE_IID",
        "CIRCLE_BUILD_NUM",
        "DRONE_BUILD_NUMBER",
        "BUILD_NUMBER",
        "CI_PIPELINE_NUMBER",
        "TRAVIS_BUILD_NUMBER",
    ])
}

/// Build the JSON body posted to the ingest endpoint. Field names follow the
/// crate-level contract above; optional fields are omitted rather than sent
/// empty.
fn build_payload() -> Value {
    let mut payload = json!({
        "cli_version": cargo_pkg_version(),
        "os": BZLOS,
        "arch": BZLARCH,
        "ci": is_ci(),
    });
    let obj = payload.as_object_mut().expect("object literal");
    if let Some(v) = runner() {
        obj.insert("runner".into(), Value::String(v));
    }
    if let Some(v) = build_counter() {
        obj.insert("counter".into(), Value::String(v));
    }
    if let Some(v) = id_day() {
        obj.insert("id_day".into(), Value::String(v));
    }
    json!({ "aspect-cli": payload })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The suffix must track how this test binary was built, not an env var, so the
    /// marker cannot be faked on or spuriously appear on a release build.
    #[test]
    fn display_version_is_suffixed_only_for_debug_builds() {
        let display = cargo_pkg_display_version();
        let bare = cargo_pkg_short_version();
        if is_debug_build() {
            assert_eq!(display, format!("{bare} (debug build)"));
        } else {
            assert_eq!(display, bare);
        }
    }

    /// Whatever the build, the bare version stays a parseable prefix — telemetry and
    /// AXL compare against it.
    #[test]
    fn display_version_starts_with_the_bare_version() {
        assert!(cargo_pkg_display_version().starts_with(&cargo_pkg_short_version()));
    }

    #[test]
    fn payload_is_wrapped_under_aspect_cli_with_core_keys() {
        let v = build_payload();
        let inner = v
            .get("aspect-cli")
            .and_then(Value::as_object)
            .expect("aspect-cli envelope");
        for k in ["cli_version", "os", "arch", "ci"] {
            assert!(inner.contains_key(k), "missing key: {k}");
        }
        assert!(inner.get("ci").unwrap().is_boolean());
    }

    /// The search order decides which file is hashed, and it has to match the
    /// Bazel module's or the same repository resolves to two different values.
    #[test]
    fn repo_id_prefers_the_module_search_order() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("MODULE.bazel"), "module(name = \"x\")\n").unwrap();
        std::fs::write(dir.join("docs/README.md"), "docs one\n").unwrap();

        // docs/README.md beats the MODULE.bazel fallback.
        let with_docs = repo_id(dir).unwrap();
        std::fs::write(dir.join("README.md"), "root one\n").unwrap();
        // A root README outranks docs/, so the value must change.
        let with_root = repo_id(dir).unwrap();
        assert_ne!(with_docs, with_root);

        // Only the first four lines participate.
        std::fs::write(dir.join("README.md"), "root one\n\n\n\nfifth\n").unwrap();
        let four = repo_id(dir).unwrap();
        std::fs::write(dir.join("README.md"), "root one\n\n\n\ndifferent\n").unwrap();
        assert_eq!(four, repo_id(dir).unwrap());
    }

    /// Known-answer vectors, independently computed with Python's hashlib from
    /// the derivation both implementations share: repo_id is sha1 of the first
    /// four lines, id_day is sha1 of "repo_id;YYYY-MM-DD". If either value
    /// drifts, this implementation has diverged from tools_telemetry and the
    /// same repository will count twice in the aggregates. No-salt path only;
    /// a set salt skips, as in salted_hash_is_stable_and_hex.
    #[test]
    fn id_day_matches_the_shared_derivation() {
        if std::env::var("ASPECT_TOOLS_TELEMETRY_SALT").is_ok() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(
            dir.join("README.md"),
            "# Fixture repo\n\nUsed by the id_day known-answer test.\nLine four.\nLine five is not hashed.\n",
        )
        .unwrap();

        assert_eq!(
            repo_id(dir).unwrap(),
            "a66894ba5f70368a295eb44fec1ff3b272efabd4"
        );
        assert_eq!(
            id_day_for(dir, "2026-01-02").unwrap(),
            "0c96cba5e7efaea4f3a2ea0054a7a6a451229536"
        );
    }

    #[test]
    fn workspace_root_is_the_nearest_marked_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        let nested = root.join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("MODULE.bazel"), "").unwrap();

        assert_eq!(workspace_root_from(&nested).unwrap(), root);
        // A nearer marker wins over the outer one.
        std::fs::write(root.join("a/WORKSPACE"), "").unwrap();
        assert_eq!(workspace_root_from(&nested).unwrap(), root.join("a"));
    }

    /// Aspect projects without any Bazel workspace still resolve a root, so
    /// the CLI reports an id_day for them too.
    #[test]
    fn aspect_project_markers_resolve_a_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        let nested = root.join("src/deep");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(root.join(".aspect")).unwrap();
        std::fs::write(root.join(".aspect/version.axl"), "").unwrap();
        assert_eq!(workspace_root_from(&nested).unwrap(), root);

        let module_root = root.join("src");
        std::fs::write(module_root.join("MODULE.aspect"), "").unwrap();
        assert_eq!(workspace_root_from(&nested).unwrap(), module_root);
    }

    /// Covers the no-salt path only: mutating env vars in tests is racy under
    /// the multi-threaded test runner, so a set salt skips the assertions
    /// instead of being unset.
    #[test]
    fn salted_hash_is_stable_and_hex() {
        if std::env::var("ASPECT_TOOLS_TELEMETRY_SALT").is_ok() {
            return;
        }
        let h = salted_hash("hello");
        assert_eq!(h.len(), 40);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(h, salted_hash("hello"));
    }
}

pub async fn send_telemetry() -> std::result::Result<(), ()> {
    // Honor DO_NOT_TRACK
    if do_not_track() {
        return Ok(());
    }

    let body = build_payload().to_string();

    let mut url = TELURL.to_string();
    let client = reqwest::Client::builder()
        .redirect(Policy::limited(10))
        .build()
        .unwrap();

    loop {
        let req = client
            .request(Method::POST, &url)
            .query(&[("source", "aspect-cli")])
            .header(HeaderName::from_static("content-type"), "application/json")
            .header(HeaderName::from_static("user-agent"), "reqwest;aspect-cli")
            .body(body.clone())
            .timeout(Duration::from_secs(5));

        let send_res = req.send().await;

        let send_res = match send_res {
            Ok(r) => r,
            Err(_) => break,
        };

        match send_res.status() {
            StatusCode::FOUND | StatusCode::PERMANENT_REDIRECT | StatusCode::TEMPORARY_REDIRECT => {
                if let Some(loc) = send_res.headers().get("location") {
                    if let Ok(loc_str) = loc.to_str() {
                        url = loc_str.to_owned();
                        continue;
                    }
                }
                break;
            }
            _ => break,
        };
    }
    Ok(())
}
