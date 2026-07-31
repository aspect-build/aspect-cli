use reqwest::header::HeaderName;
use reqwest::redirect::Policy;
use reqwest::{self, Method, StatusCode};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use std::env::var;
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

pub fn do_not_track() -> bool {
    var("DO_NOT_TRACK").is_ok()
}

/// Salted SHA-1 of `data`, mirroring `tools_telemetry`'s `hash` helper. Honors
/// the `ASPECT_TOOLS_TELEMETRY_SALT` env var so a single salt covers both
/// sources.
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

/// Organization slug from CI env, mirroring `tools_telemetry`'s `_repo_org`.
fn repo_org() -> Option<String> {
    first_env(&[
        "BUILDKITE_ORGANIZATION_SLUG",
        "GITHUB_REPOSITORY_OWNER",
        "CI_PROJECT_NAMESPACE",
        "CIRCLE_PROJECT_USERNAME",
        "DRONE_REPO_NAMESPACE",
        "CI_REPO_OWNER",
        "TRAVIS_REPO_SLUG",
    ])
}

/// Salted hash of the user, mirroring `tools_telemetry`'s `_repo_user`. Without
/// a stable repo `id` to salt with, we fall back to the configured telemetry
/// salt only.
fn repo_user() -> Option<String> {
    let raw = first_env(&[
        "BUILDKITE_BUILD_AUTHOR_EMAIL",
        "GITHUB_ACTOR",
        "GITLAB_USER_EMAIL",
        "CIRCLE_USERNAME",
        "DRONE_COMMIT_AUTHOR",
        "DRONE_COMMIT_AUTHOR_EMAIL",
        "CI_COMMIT_AUTHOR",
        "CI_COMMIT_AUTHOR_EMAIL",
        "LOGNAME",
        "USER",
    ])?;
    Some(salted_hash(&raw))
}

fn shell() -> Option<String> {
    first_env(&["SHELL"])
}

/// Build the JSON body posted to the ingest endpoint.
///
/// Breadcrumb: keys here intentionally mirror `tools_telemetry`'s key vocabulary
/// (`os`, `arch`, `ci`, `runner`, `counter`, `org`, `user`, `shell`) so that a
/// single aggregator can group both sources. When adding new fields, prefer
/// reusing names from https://github.com/aspect-build/tools_telemetry rather
/// than inventing new ones. Bazel-specific fields are intentionally omitted —
/// `tools_telemetry` already covers those.
fn build_payload() -> Value {
    let mut payload = json!({
        "version": cargo_pkg_version(),
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
    if let Some(v) = repo_org() {
        obj.insert("org".into(), Value::String(v));
    }
    if let Some(v) = repo_user() {
        obj.insert("user".into(), Value::String(v));
    }
    if let Some(v) = shell() {
        obj.insert("shell".into(), Value::String(v));
    }
    json!({ "aspect-cli": payload })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_is_wrapped_under_aspect_cli_with_core_keys() {
        let v = build_payload();
        let inner = v
            .get("aspect-cli")
            .and_then(Value::as_object)
            .expect("aspect-cli envelope");
        for k in ["version", "os", "arch", "ci"] {
            assert!(inner.contains_key(k), "missing key: {k}");
        }
        assert!(inner.get("ci").unwrap().is_boolean());
    }

    #[test]
    fn salted_hash_is_stable_and_hex() {
        // Test the no-salt path; setting env vars in tests is racy so we don't.
        // SAFETY: ensure the salt is unset for this assertion path.
        // SAFETY: setting/removing env vars in tests is technically unsafe in
        // multithreaded contexts; we only read it here.
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
