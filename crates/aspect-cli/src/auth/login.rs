use std::process::ExitCode;

use miette::{miette, Context, IntoDiagnostic};
use reqwest::Client;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use serde::Deserialize;

use super::credentials::{self, Credentials, JwtClaims, TokenResponse, UserInfo};
use super::AuthEnv;

/// Interactive browser-based login (OAuth2 + PKCE).
pub async fn run_browser(env: AuthEnv) -> miette::Result<ExitCode> {
    // 1. Bind a TCP listener on a fixed port (auth provider requires exact redirect URI match)
    let port: u16 = 19556;
    let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
        .await
        .into_diagnostic()
        .wrap_err(format!("failed to bind localhost:{} — is another login in progress?", port))?;
    let redirect_uri = format!("http://localhost:{}/callback", port);

    // 2. Generate PKCE code verifier and challenge
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);

    // 3. Build the authorize URL with PKCE challenge
    let authorize_url = format!(
        "{}/oauth/authorize?client_id={}&redirect_uri={}&response_type=code&scope=openid+profile+email&code_challenge={}&code_challenge_method=S256",
        env.domain,
        env.client_id,
        urlencode(&redirect_uri),
        urlencode(&code_challenge),
    );

    // 4. Open the browser
    eprintln!("Opening browser to log in...");
    eprintln!(
        "If the browser doesn't open, visit:\n  {}",
        authorize_url
    );
    let _ = open_browser(&authorize_url);

    // 5. Wait for the callback
    eprintln!("Waiting for authentication...");
    let auth_code = accept_callback(listener).await?;

    // 6. Exchange code for tokens (with PKCE verifier)
    let client = Client::new();
    let token_response = exchange_code(&client, &auth_code, &redirect_uri, &code_verifier, env).await?;

    // 7. Decode user info from JWT claims
    let user_info = decode_jwt_claims(&token_response.access_token)?;

    // 8. Save credentials
    let creds = Credentials {
        access_token: token_response.access_token,
        refresh_token: token_response.refresh_token,
        email: user_info.email.clone(),
        name: user_info.name.clone(),
        tenant_id: user_info.tenant_id,
    };
    credentials::save(&creds)?;

    eprintln!("Logged in as {} ({})", user_info.name, user_info.email);
    Ok(ExitCode::SUCCESS)
}

/// Accept a single HTTP request on the listener, extract the `code` query parameter,
/// send back an HTML response, and close the connection.
async fn accept_callback(listener: TcpListener) -> miette::Result<String> {
    let (mut stream, _addr) = listener.accept().await.into_diagnostic()?;

    // Read the HTTP request (enough to get the first line with the path)
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.into_diagnostic()?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // Parse the first line: "GET /callback?code=XXXX HTTP/1.1"
    let first_line = request
        .lines()
        .next()
        .ok_or_else(|| miette!("empty HTTP request from browser callback"))?;
    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| miette!("malformed HTTP request line"))?;

    // Extract the code query parameter
    let code = extract_query_param(path, "code").ok_or_else(|| {
        let error = extract_query_param(path, "error").unwrap_or("unknown".to_string());
        let desc = extract_query_param(path, "error_description").unwrap_or_default();
        miette!("authentication failed: {} {}", error, desc)
    })?;

    // Send a response to the browser
    let html = "<html><body><h2>Authentication successful!</h2><p>You can close this tab and return to the terminal.</p><script>window.close()</script></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    stream
        .write_all(response.as_bytes())
        .await
        .into_diagnostic()?;

    Ok(code)
}

/// Exchange the authorization code for tokens via /oauth/token (with PKCE verifier)
async fn exchange_code(
    client: &Client,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
    env: AuthEnv,
) -> miette::Result<TokenResponse> {
    let resp = client
        .post(format!("{}/oauth/token", env.domain))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", env.client_id),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .into_diagnostic()?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(miette!(
            "token exchange failed (HTTP {}): {}",
            status,
            body
        ));
    }

    resp.json::<TokenResponse>().await.into_diagnostic()
}

// ── Direct JWT login (--with-token, for CI/headless) ──

pub async fn run_with_token(token: &str) -> miette::Result<ExitCode> {
    eprintln!("Validating token...");
    let user_info = decode_jwt_claims(token)?;

    let creds = Credentials {
        access_token: token.to_string(),
        refresh_token: String::new(),
        email: user_info.email.clone(),
        name: user_info.name.clone(),
        tenant_id: user_info.tenant_id,
    };
    credentials::save(&creds)?;

    eprintln!("Logged in as {} ({})", user_info.name, user_info.email);
    Ok(ExitCode::SUCCESS)
}

// ── API token login (--with-api-token, for CI/headless) ──

/// Response from the API token exchange endpoint.
#[derive(Debug, Deserialize)]
struct ApiTokenResponse {
    #[serde(alias = "access_token")]
    #[serde(rename = "accessToken")]
    access_token: String,
}

pub async fn run_with_api_token(client_id: &str, secret: &str, env: AuthEnv) -> miette::Result<ExitCode> {
    eprintln!("Exchanging API token...");

    // The vendor host is the hostname portion of the auth domain
    let vendor_host = env.domain
        .strip_prefix("https://")
        .unwrap_or(env.domain);

    let client = Client::new();
    let resp = client
        .post(format!("{}/identity/resources/auth/v1/api-token", env.domain))
        .header("Content-Type", "application/json")
        .header("frontegg-vendor-host", vendor_host)
        .json(&serde_json::json!({
            "clientId": client_id,
            "secret": secret,
        }))
        .send()
        .await
        .into_diagnostic()
        .wrap_err("failed to exchange API token")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(miette!(
            "API token exchange failed (HTTP {}): {}",
            status,
            body
        ));
    }

    let data: ApiTokenResponse = resp
        .json()
        .await
        .into_diagnostic()
        .wrap_err("failed to parse API token response")?;

    // Decode user info from the returned JWT
    let user_info = decode_jwt_claims(&data.access_token)?;

    let creds = Credentials {
        access_token: data.access_token,
        refresh_token: String::new(),
        email: user_info.email.clone(),
        name: user_info.name.clone(),
        tenant_id: user_info.tenant_id,
    };
    credentials::save(&creds)?;

    eprintln!("Logged in as {} ({})", user_info.name, user_info.email);
    Ok(ExitCode::SUCCESS)
}

// ── PKCE helpers ──

/// Generate a random 128-byte code verifier, base64url-encoded (no padding).
fn generate_code_verifier() -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use rand::RngCore;

    let mut buf = [0u8; 96]; // 96 bytes → 128 base64url chars (within 43-128 range per RFC 7636)
    rand::rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// Compute the S256 code challenge: base64url(sha256(code_verifier)).
fn generate_code_challenge(verifier: &str) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;

    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

// ── Shared helpers ──

/// Decode user info from a JWT access token's claims.
pub fn decode_jwt_claims(token: &str) -> miette::Result<UserInfo> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;

    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(miette!("invalid JWT: expected 3 parts, got {}", parts.len()));
    }

    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .into_diagnostic()
        .wrap_err("failed to base64-decode JWT payload")?;

    let claims: JwtClaims = serde_json::from_slice(&payload_bytes)
        .into_diagnostic()
        .wrap_err("failed to parse JWT claims")?;

    Ok(UserInfo {
        email: claims.email.unwrap_or_else(|| "api-token".to_string()),
        name: claims.name.unwrap_or_else(|| "Unknown".to_string()),
        tenant_id: claims.tenant_id,
    })
}

fn open_browser(url: &str) -> miette::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .into_diagnostic()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .into_diagnostic()?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", url])
            .spawn()
            .into_diagnostic()?;
    }
    Ok(())
}

/// Simple percent-encoding for URL parameters.
fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                String::from(b as char)
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}

/// Extract a query parameter value from a URL path like "/callback?code=abc&state=xyz"
fn extract_query_param(path: &str, key: &str) -> Option<String> {
    let query = path.split('?').nth(1)?;
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if kv.next() == Some(key) {
            return kv.next().map(urldecode);
        }
    }
    None
}

/// Simple percent-decoding.
fn urldecode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            }
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}
