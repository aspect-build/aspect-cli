use aspect_config::{cargo_pkg_version, BZLARCH, BZLOS, TELURL};
use reqwest::header::HeaderName;
use reqwest::redirect::Policy;
use reqwest::{self, Method};
use std::env::var;
use std::time::Duration;

pub async fn send_telemetry() -> std::result::Result<(), ()> {
    // Honor DO_NOT_TRACK
    if var("DO_NOT_TRACK").is_ok() {
        return Ok(());
    }

    // Report telemetry
    let v = cargo_pkg_version();
    let body = format!(
        "{{\"cli\": {{\"version\": \"{v}\", \"os\": \"{BZLOS}\", \"arch\": \"{BZLARCH}\"}}}}"
    );

    let url = TELURL.to_string();
    let client = reqwest::Client::builder()
        .redirect(Policy::limited(10))
        .build()
        .unwrap();

    let req = client
        .request(Method::POST, &url)
        .query(&[("source", "aspect-cli")])
        .header(HeaderName::from_static("content-type"), "application/json")
        .header(HeaderName::from_static("user-agent"), "reqwest;aspect-cli")
        .body(body.clone())
        .timeout(Duration::from_secs(5));

    req.send().await.map_err(|_| ())?;
    Ok(())
}
