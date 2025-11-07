use reqwest::header::HeaderName;
use reqwest::redirect::Policy;
use reqwest::{self, Method, StatusCode};
use std::env::var;
use std::time::Duration;

// The Bazel arch and os per @platforms and //bazel/platforms
pub static BZLOS: &str = env!("BUILD_BZLOS");
pub static BZLARCH: &str = env!("BUILD_BZLARCH");

// And the GOOS/GOARCH equivalents
pub static GOOS: &str = env!("BUILD_GOOS");
pub static GOARCH: &str = env!("BUILD_GOARCH");
pub static LLVM_TRIPLE: &str = env!("LLVM_TRIPLE");

static TELURL: &str = "https://telemetry2.aspect.build/ingest";

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

pub fn do_not_track() -> bool {
    var("DO_NOT_TRACK").is_ok()
}

pub async fn send_telemetry() -> std::result::Result<(), ()> {
    // Honor DO_NOT_TRACK
    if do_not_track() {
        return Ok(());
    }

    // Report telemetry
    let v = cargo_pkg_version();
    let body = format!(
        "{{\"cli\": {{\"version\": \"{v}\", \"os\": \"{BZLOS}\", \"arch\": \"{BZLARCH}\"}}}}"
    );

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
