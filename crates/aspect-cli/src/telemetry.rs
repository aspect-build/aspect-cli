use aspect_config::{cargo_pkg_version, BZLARCH, BZLOS, TELURL};
use reqwest::header::HeaderName;
use reqwest::redirect::Policy;
use reqwest::{self, Method, StatusCode};
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
