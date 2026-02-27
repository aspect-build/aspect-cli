use std::collections::HashMap;
use std::process::ExitCode;

use miette::{miette, Context, IntoDiagnostic};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::auth::credentials;

#[derive(Serialize)]
struct TokenRequest {
    owner: String,
    repo: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    permissions: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TokenResponse {
    token: String,
    expires_at: String,
    permissions: serde_json::Value,
    repository: String,
}

pub async fn run(
    owner: &str,
    repo: &str,
    permissions: Option<HashMap<String, String>>,
    api_url: &str,
    json_output: bool,
) -> miette::Result<ExitCode> {
    let creds = credentials::load_valid().await?;
    let Some(creds) = creds else {
        return Err(miette!(
            "not logged in\n\nRun `aspect auth login` to authenticate."
        ));
    };

    let client = Client::new();
    let url = format!("{}/github/token", api_url);

    let body = TokenRequest {
        owner: owner.to_string(),
        repo: repo.to_string(),
        permissions,
    };

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", creds.access_token))
        .json(&body)
        .send()
        .await
        .into_diagnostic()
        .wrap_err("failed to connect to Aspect API")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let raw_body = resp.text().await.unwrap_or_default();

        // Try to extract the "error" field from JSON response
        let error_msg = serde_json::from_str::<serde_json::Value>(&raw_body)
            .ok()
            .and_then(|v| v["error"].as_str().map(String::from))
            .unwrap_or(raw_body);

        return match status.as_u16() {
            401 => Err(miette!(
                "authentication failed: {}\n\nRun `aspect auth login` to re-authenticate.",
                error_msg
            )),
            403 => Err(miette!("access denied: {}", error_msg)),
            404 => Err(miette!(
                "{}\n\nLink a GitHub App installation at https://docs.aspect.build/github-app",
                error_msg
            )),
            _ => Err(miette!(
                "API request failed (HTTP {}): {}",
                status,
                error_msg
            )),
        };
    }

    let token_response: TokenResponse = resp
        .json()
        .await
        .into_diagnostic()
        .wrap_err("failed to parse API response")?;

    if json_output {
        let json = serde_json::to_string_pretty(&token_response).into_diagnostic()?;
        println!("{}", json);
    } else {
        println!("{}", token_response.token);
    }

    Ok(ExitCode::SUCCESS)
}
