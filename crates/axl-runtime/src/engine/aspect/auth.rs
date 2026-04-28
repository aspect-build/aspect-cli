use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use allocative::Allocative;
use derive_more::Display;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use starlark::StarlarkResultExt;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value_as_type::StarlarkValueAsType;
use starlark::values::{self, NoSerialize, ProvidesStaticType, ValueLike, starlark_value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::runtime::Handle;

#[derive(Debug, Clone, Copy)]
struct AuthEnv {
    domain: &'static str,
    client_id: &'static str,
    #[allow(dead_code)]
    api_url: &'static str,
}

const ENV_PRODUCTION: AuthEnv = AuthEnv {
    domain: "https://auth.aspect.build",
    client_id: "771ff228-18a1-43f0-bc83-62c9df0d72ca",
    api_url: "https://api.aspect.build",
};

const ENV_STAGING: AuthEnv = AuthEnv {
    domain: "https://auth.staging.aspect.build",
    client_id: "76f292c5-e1b8-40a5-bc92-f30e521251f1",
    api_url: "https://api-dev.aspect.build",
};

const ENV_DEV: AuthEnv = AuthEnv {
    domain: "https://auth.dev.aspect.build",
    client_id: "a1c8902d-fbbe-43bf-8c4e-f58f79310e7a",
    api_url: "https://api-dev.aspect.build",
};

fn resolve_aspect_env() -> anyhow::Result<AuthEnv> {
    resolve_auth_env(std::env::var("__ASPECT_ENVIRONMENT__").ok().as_deref())
}

fn resolve_auth_env(env: Option<&str>) -> anyhow::Result<AuthEnv> {
    match env {
        None | Some("") | Some("production") | Some("prod") => Ok(ENV_PRODUCTION),
        Some("staging") => Ok(ENV_STAGING),
        Some("dev") | Some("development") => Ok(ENV_DEV),
        Some(other) => Err(anyhow::anyhow!(
            "unknown environment: {:?}\n\nValid values: production (default), staging, dev",
            other
        )),
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct CredentialsEntry {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    email: String,
    name: String,
    tenant_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_client_id: Option<String>,
}

fn credentials_path() -> anyhow::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("unable to determine home directory"))?;
    Ok(home.join(".aspect").join("credentials.json"))
}

fn load_all_credentials() -> anyhow::Result<HashMap<String, CredentialsEntry>> {
    let path = credentials_path()?;
    match fs::read_to_string(&path) {
        Ok(content) => {
            // Try new format (HashMap<profile, entry>) first
            if let Ok(map) = serde_json::from_str::<HashMap<String, CredentialsEntry>>(&content) {
                return Ok(map);
            }
            // Migrate from old flat format (single entry → "default" profile)
            if let Ok(entry) = serde_json::from_str::<CredentialsEntry>(&content) {
                let mut map = HashMap::new();
                map.insert("default".to_string(), entry);
                return Ok(map);
            }
            Err(anyhow::anyhow!(
                "failed to parse credentials file: unrecognized format"
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(e) => Err(anyhow::anyhow!("failed to read credentials: {}", e)),
    }
}

fn save_all_credentials(map: &HashMap<String, CredentialsEntry>) -> anyhow::Result<()> {
    let path = credentials_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("failed to create ~/.aspect directory: {}", e))?;
    }
    let json = serde_json::to_string_pretty(map)
        .map_err(|e| anyhow::anyhow!("failed to serialize credentials: {}", e))?;
    fs::write(&path, &json)
        .map_err(|e| anyhow::anyhow!("failed to write credentials file: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&path, perms)
            .map_err(|e| anyhow::anyhow!("failed to set credentials permissions: {}", e))?;
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct JwtClaims {
    email: Option<String>,
    name: Option<String>,
    #[serde(rename = "tenantId")]
    tenant_id: String,
    exp: Option<u64>,
}

fn decode_jwt_claims(token: &str) -> anyhow::Result<JwtClaims> {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(anyhow::anyhow!(
            "invalid JWT: expected 3 parts, got {}",
            parts.len()
        ));
    }
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| anyhow::anyhow!("failed to decode JWT payload: {}", e))?;
    serde_json::from_slice(&payload_bytes)
        .map_err(|e| anyhow::anyhow!("failed to parse JWT claims: {}", e))
}

fn format_token_status(exp: Option<u64>) -> String {
    let Some(exp) = exp else {
        return "no expiry".to_string();
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if now >= exp {
        "expired".to_string()
    } else {
        let remaining = exp - now;
        let hours = remaining / 3600;
        let minutes = (remaining % 3600) / 60;
        if hours > 0 {
            format!("expires in {}h {}m", hours, minutes)
        } else {
            format!("expires in {}m", minutes)
        }
    }
}

fn is_expired_jwt(entry: &CredentialsEntry) -> bool {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let parts: Vec<&str> = entry.access_token.split('.').collect();
    if parts.len() != 3 {
        return true;
    }
    let Ok(payload_bytes) = URL_SAFE_NO_PAD.decode(parts[1]) else {
        return true;
    };
    #[derive(Deserialize)]
    struct ExpOnly {
        exp: Option<u64>,
    }
    let Ok(claims) = serde_json::from_slice::<ExpOnly>(&payload_bytes) else {
        return true;
    };
    let Some(exp) = claims.exp else {
        return false;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now + 60 >= exp
}

fn can_refresh(entry: &CredentialsEntry) -> bool {
    !entry.refresh_token.is_empty() && entry.auth_domain.is_some() && entry.auth_client_id.is_some()
}

fn generate_code_verifier() -> String {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use rand::RngCore;

    let mut buf = [0u8; 96];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

fn generate_code_challenge(verifier: &str) -> String {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

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

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    Handle::current().block_on(fut)
}

/// Process-level cache for API-token-exchange results. A single `aspect`
/// task can call `ctx.aspect.auth.credentials()` multiple times (once per
/// `github.authenticate` call); without caching each call hits Frontegg's
/// `/api-token` endpoint. The cache stores the most recently successful
/// exchange keyed on the source token; entries are invalidated automatically
/// once the JWT's `exp` claim falls inside the 60-second skew buffer
/// (`is_expired_jwt`), so we don't hand out tokens that are about to be rejected.
struct ApiTokenCacheEntry {
    /// Source token (the raw `client_id:secret` value) the cached entry was
    /// minted from. If the env var changes mid-run we mint a fresh one.
    source_token: String,
    entry: CredentialsEntry,
}

static API_TOKEN_CACHE: OnceLock<Mutex<Option<ApiTokenCacheEntry>>> = OnceLock::new();

fn api_token_cache() -> &'static Mutex<Option<ApiTokenCacheEntry>> {
    API_TOKEN_CACHE.get_or_init(|| Mutex::new(None))
}

/// Exchange an ASPECT_API_TOKEN from an explicit source — the env var or the
/// Buildkite secret store — for a fresh credentials entry. Returns Ok(None)
/// only when no such source is present; a malformed token or a failed
/// exchange is surfaced as an error so CI misconfigurations fail loudly
/// instead of falling through to whatever happens to be cached on disk
/// (e.g. from a previous job on a persistent runner).
///
/// Subsequent calls within the same process reuse the cached exchange
/// result until the JWT approaches expiry.
fn credentials_from_api_token_env() -> anyhow::Result<Option<CredentialsEntry>> {
    let token = std::env::var("ASPECT_API_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(buildkite_aspect_api_token);
    let Some(token) = token else {
        return Ok(None);
    };

    // Fast path: a cached exchange for the same source token whose JWT
    // hasn't entered the 60-second pre-expiry buffer.
    if let Ok(guard) = api_token_cache().lock() {
        if let Some(cached) = guard.as_ref() {
            if cached.source_token == token && !is_expired_jwt(&cached.entry) {
                return Ok(Some(cached.entry.clone()));
            }
        }
    }

    let (client_id, secret) = token
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("ASPECT_API_TOKEN must be in 'client_id:secret' format"))?;
    let env = resolve_aspect_env()?;
    let entry = block_on(exchange_api_token(client_id, secret, env))?;

    // Store for subsequent calls. Clearing on lock-poison is fine — the
    // next caller will just miss and re-exchange.
    if let Ok(mut guard) = api_token_cache().lock() {
        *guard = Some(ApiTokenCacheEntry {
            source_token: token,
            entry: entry.clone(),
        });
    }
    Ok(Some(entry))
}

/// Read ASPECT_API_TOKEN from Buildkite's secret store when running on a
/// Buildkite agent. Returns None if not on a Buildkite agent, the CLI is
/// missing, or the secret is not defined — callers should fall through to
/// the "not authenticated" path rather than surface the error.
fn buildkite_aspect_api_token() -> Option<String> {
    // BUILDKITE_AGENT_ACCESS_TOKEN is injected into every job on a Buildkite
    // agent; checking it avoids shelling out when we're not on Buildkite.
    if std::env::var_os("BUILDKITE_AGENT_ACCESS_TOKEN").is_none() {
        return None;
    }
    let output = std::process::Command::new("buildkite-agent")
        .args(["secret", "get", "ASPECT_API_TOKEN"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if token.is_empty() { None } else { Some(token) }
}

async fn exchange_api_token(
    client_id: &str,
    secret: &str,
    env: AuthEnv,
) -> anyhow::Result<CredentialsEntry> {
    #[derive(Deserialize)]
    struct ApiTokenResponse {
        #[serde(rename = "accessToken", alias = "access_token")]
        access_token: String,
    }
    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/identity/resources/auth/v1/api-token",
            env.domain
        ))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "clientId": client_id, "secret": secret }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("API token exchange failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "API token exchange failed (HTTP {}): {}",
            status,
            body
        ));
    }
    let data: ApiTokenResponse = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("failed to parse API token response: {}", e))?;
    let claims = decode_jwt_claims(&data.access_token)?;
    Ok(CredentialsEntry {
        access_token: data.access_token,
        refresh_token: String::new(),
        email: claims.email.unwrap_or_else(|| "api-token".to_string()),
        name: claims.name.unwrap_or_else(|| "Unknown".to_string()),
        tenant_id: claims.tenant_id,
        auth_domain: None,
        auth_client_id: None,
    })
}

async fn accept_callback(listener: TcpListener) -> anyhow::Result<String> {
    let (mut stream, _addr) = listener
        .accept()
        .await
        .map_err(|e| anyhow::anyhow!("failed to accept OAuth callback: {}", e))?;
    let mut buf = vec![0u8; 4096];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| anyhow::anyhow!("failed to read OAuth callback: {}", e))?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request
        .lines()
        .next()
        .ok_or_else(|| anyhow::anyhow!("empty HTTP request from browser callback"))?;
    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("malformed HTTP request line"))?;
    let code = extract_query_param(path, "code").ok_or_else(|| {
        let error = extract_query_param(path, "error").unwrap_or_else(|| "unknown".to_string());
        let desc = extract_query_param(path, "error_description").unwrap_or_default();
        anyhow::anyhow!("authentication failed: {} {}", error, desc)
    })?;
    let html = r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Authenticated — Aspect Build</title>
  <style>
    * { margin: 0; padding: 0; box-sizing: border-box; }
    body {
      font-family: 'Inter', -apple-system, BlinkMacSystemFont, sans-serif;
      background: #f5f6f8;
      color: #1f2e35;
      min-height: 100vh;
      display: flex;
      align-items: center;
      justify-content: center;
    }
    .card {
      text-align: center;
      padding: 48px 56px;
      border: 1px solid #e2e6ea;
      border-radius: 12px;
      background: #fff;
      max-width: 340px;
      width: 100%;
    }
    .logo { margin-bottom: 28px; }
    h2 {
      font-size: 17px;
      font-weight: 600;
      color: #1f2e35;
      margin-bottom: 8px;
    }
    p {
      font-size: 13px;
      color: #6b7e87;
    }
  </style>
</head>
<body>
  <div class="card">
    <div class="logo">
      <svg xmlns="http://www.w3.org/2000/svg" width="44" height="40" viewBox="0 0 44 40" fill="none">
        <path fill-rule="evenodd" d="M37.854 39.784H24.115l6.825-11.923-6.825-11.923h13.739l6.874 11.923-6.874 11.923zm-30.978 0h13.719l6.825-11.932H13.77L6.896 15.919.07 27.852l6.806 11.932zm1.882-26.865l6.884 11.923 6.825-11.942h13.739L29.371.977H15.622L8.758 12.919z" fill="#176acc"/>
      </svg>
    </div>
    <h2>Login successful</h2>
    <p>You can close this tab and return to your terminal.</p>
  </div>
  <script>setTimeout(window.close, 10000)</script>
</body>
</html>"##;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|e| anyhow::anyhow!("failed to write OAuth response: {}", e))?;
    Ok(code)
}

#[derive(Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: String,
}

async fn exchange_code(
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
    env: AuthEnv,
) -> anyhow::Result<OAuthTokenResponse> {
    let client = reqwest::Client::new();
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
        .map_err(|e| anyhow::anyhow!("token exchange request failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "token exchange failed (HTTP {}): {}",
            status,
            body
        ));
    }
    resp.json::<OAuthTokenResponse>()
        .await
        .map_err(|e| anyhow::anyhow!("failed to parse token response: {}", e))
}

async fn refresh_access_token(entry: &CredentialsEntry) -> anyhow::Result<CredentialsEntry> {
    let auth_domain = entry
        .auth_domain
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no auth_domain stored — cannot refresh"))?;
    let client_id = entry
        .auth_client_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no auth_client_id stored — cannot refresh"))?;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/oauth/token", auth_domain))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &entry.refresh_token),
            ("client_id", client_id),
        ])
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("token refresh request failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "token refresh failed (HTTP {}): {}",
            status,
            body
        ));
    }
    let token_resp: OAuthTokenResponse = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("failed to parse refresh response: {}", e))?;
    let claims = decode_jwt_claims(&token_resp.access_token)?;
    Ok(CredentialsEntry {
        access_token: token_resp.access_token,
        refresh_token: token_resp.refresh_token,
        email: claims.email.unwrap_or_else(|| "api-token".to_string()),
        name: claims.name.unwrap_or_else(|| "Unknown".to_string()),
        tenant_id: claims.tenant_id,
        auth_domain: entry.auth_domain.clone(),
        auth_client_id: entry.auth_client_id.clone(),
    })
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative, Clone)]
#[display("<AuthCredentials>")]
pub struct AuthCredentials {
    pub email: String,
    pub name: String,
    pub tenant_id: String,
    pub access_token: String,
    pub token_status: String,
    // Internal fields preserved for persist()
    pub(crate) refresh_token: String,
    pub(crate) auth_domain: Option<String>,
    pub(crate) auth_client_id: Option<String>,
}

starlark_simple_value!(AuthCredentials);

#[starlark_value(type = "aspect.AuthCredentials")]
impl<'v> values::StarlarkValue<'v> for AuthCredentials {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(auth_credentials_methods)
    }
}

#[starlark_module]
fn auth_credentials_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn email<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(this
            .downcast_ref_err::<AuthCredentials>()
            .into_anyhow_result()?
            .email
            .clone())
    }

    #[starlark(attribute)]
    fn name<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(this
            .downcast_ref_err::<AuthCredentials>()
            .into_anyhow_result()?
            .name
            .clone())
    }

    #[starlark(attribute)]
    fn tenant_id<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(this
            .downcast_ref_err::<AuthCredentials>()
            .into_anyhow_result()?
            .tenant_id
            .clone())
    }

    #[starlark(attribute)]
    fn access_token<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(this
            .downcast_ref_err::<AuthCredentials>()
            .into_anyhow_result()?
            .access_token
            .clone())
    }

    #[starlark(attribute)]
    fn token_status<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(this
            .downcast_ref_err::<AuthCredentials>()
            .into_anyhow_result()?
            .token_status
            .clone())
    }
}

impl AuthCredentials {
    fn from_entry(entry: &CredentialsEntry) -> Self {
        let claims = decode_jwt_claims(&entry.access_token).ok();
        let exp = claims.as_ref().and_then(|c| c.exp);
        AuthCredentials {
            email: entry.email.clone(),
            name: entry.name.clone(),
            tenant_id: entry.tenant_id.clone(),
            access_token: entry.access_token.clone(),
            token_status: format_token_status(exp),
            refresh_token: entry.refresh_token.clone(),
            auth_domain: entry.auth_domain.clone(),
            auth_client_id: entry.auth_client_id.clone(),
        }
    }

    fn to_entry(&self) -> CredentialsEntry {
        CredentialsEntry {
            access_token: self.access_token.clone(),
            refresh_token: self.refresh_token.clone(),
            email: self.email.clone(),
            name: self.name.clone(),
            tenant_id: self.tenant_id.clone(),
            auth_domain: self.auth_domain.clone(),
            auth_client_id: self.auth_client_id.clone(),
        }
    }
}

struct AuthSessionInner {
    listener: Option<TcpListener>,
    code_verifier: String,
    redirect_uri: String,
    env: AuthEnv,
}

#[derive(Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<AuthSession>")]
pub struct AuthSession {
    pub url: String,
    #[allocative(skip)]
    inner: Mutex<Option<AuthSessionInner>>,
}

impl std::fmt::Debug for AuthSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthSession")
            .field("url", &self.url)
            .finish()
    }
}

starlark_simple_value!(AuthSession);

#[starlark_value(type = "aspect.AuthSession")]
impl<'v> values::StarlarkValue<'v> for AuthSession {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(auth_session_methods)
    }
}

#[starlark_module]
fn auth_session_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn url<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(this
            .downcast_ref_err::<AuthSession>()
            .into_anyhow_result()?
            .url
            .clone())
    }

    fn wait<'v>(this: values::Value<'v>) -> anyhow::Result<AuthCredentials> {
        let session = this
            .downcast_ref_err::<AuthSession>()
            .into_anyhow_result()?;
        let mut guard = session
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("auth session already consumed or poisoned"))?;
        let inner = guard.take().ok_or_else(|| {
            anyhow::anyhow!("auth session already consumed (wait() called twice)")
        })?;
        let entry = block_on(async move {
            let listener = inner
                .listener
                .ok_or_else(|| anyhow::anyhow!("no listener in auth session"))?;
            let code = accept_callback(listener).await?;
            let token_resp =
                exchange_code(&code, &inner.redirect_uri, &inner.code_verifier, inner.env).await?;
            let claims = decode_jwt_claims(&token_resp.access_token)?;
            Ok::<CredentialsEntry, anyhow::Error>(CredentialsEntry {
                access_token: token_resp.access_token,
                refresh_token: token_resp.refresh_token,
                email: claims.email.unwrap_or_else(|| "api-token".to_string()),
                name: claims.name.unwrap_or_else(|| "Unknown".to_string()),
                tenant_id: claims.tenant_id,
                auth_domain: Some(inner.env.domain.to_string()),
                auth_client_id: Some(inner.env.client_id.to_string()),
            })
        })?;
        Ok(AuthCredentials::from_entry(&entry))
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<aspect.Auth>")]
pub struct Auth {}

starlark_simple_value!(Auth);

#[starlark_value(type = "aspect.Auth")]
impl<'v> values::StarlarkValue<'v> for Auth {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(auth_methods)
    }
}

#[starlark_module]
fn auth_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn api_url<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(resolve_aspect_env()?.api_url.to_string())
    }

    fn login<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named)] token: Option<&str>,
        #[starlark(require = named)] api_token: Option<&str>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let env = resolve_aspect_env()?;

        if let Some(token) = token {
            let claims = decode_jwt_claims(token)?;
            let entry = CredentialsEntry {
                access_token: token.to_string(),
                refresh_token: String::new(),
                email: claims.email.unwrap_or_else(|| "api-token".to_string()),
                name: claims.name.unwrap_or_else(|| "Unknown".to_string()),
                tenant_id: claims.tenant_id,
                auth_domain: None,
                auth_client_id: None,
            };
            return Ok(heap.alloc(AuthCredentials::from_entry(&entry)));
        }

        if let Some(api_token) = api_token {
            let (client_id, secret) = api_token.split_once(':').ok_or_else(|| {
                anyhow::anyhow!("invalid API token format: expected client_id:secret")
            })?;
            let entry = block_on(exchange_api_token(client_id, secret, env))?;
            return Ok(heap.alloc(AuthCredentials::from_entry(&entry)));
        }

        // Browser-based OAuth flow
        let port: u16 = 19556;
        let listener = block_on(TcpListener::bind(format!("127.0.0.1:{}", port))).map_err(|e| {
            anyhow::anyhow!(
                "failed to bind localhost:{} — is another login in progress? ({})",
                port,
                e
            )
        })?;
        let redirect_uri = format!("http://localhost:{}/callback", port);
        let code_verifier = generate_code_verifier();
        let code_challenge = generate_code_challenge(&code_verifier);
        let authorize_url = format!(
            "{}/oauth/authorize?client_id={}&redirect_uri={}&response_type=code&scope=openid+profile+email&code_challenge={}&code_challenge_method=S256",
            env.domain,
            env.client_id,
            urlencode(&redirect_uri),
            urlencode(&code_challenge),
        );
        let session = AuthSession {
            url: authorize_url,
            inner: Mutex::new(Some(AuthSessionInner {
                listener: Some(listener),
                code_verifier,
                redirect_uri,
                env,
            })),
        };
        Ok(heap.alloc(session))
    }

    fn persist<'v>(
        #[allow(unused)] this: values::Value<'v>,
        creds: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] profile: NoneOr<String>,
    ) -> anyhow::Result<values::Value<'v>> {
        let creds = creds
            .downcast_ref_err::<AuthCredentials>()
            .into_anyhow_result()?;
        let profile_opt = profile.into_option();
        let profile = profile_opt
            .as_deref()
            .filter(|p| !p.is_empty())
            .unwrap_or("default");
        let mut map = load_all_credentials()?;
        map.insert(profile.to_string(), creds.to_entry());
        save_all_credentials(&map)?;
        Ok(values::Value::new_none())
    }

    fn logout<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] profile: NoneOr<String>,
    ) -> anyhow::Result<values::Value<'v>> {
        let profile_opt = profile.into_option();
        let profile = profile_opt
            .as_deref()
            .filter(|p| !p.is_empty())
            .unwrap_or("default");
        let mut map = load_all_credentials()?;
        map.remove(profile);
        save_all_credentials(&map)?;
        Ok(values::Value::new_none())
    }

    fn credentials<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] profile: NoneOr<String>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let profile_opt = profile.into_option();
        let profile = profile_opt
            .as_deref()
            .filter(|p| !p.is_empty())
            .unwrap_or("default");
        // Prefer an explicit ASPECT_API_TOKEN source over cache login credentials.
        if let Some(entry) = credentials_from_api_token_env()? {
            return Ok(heap.alloc(AuthCredentials::from_entry(&entry)));
        }
        let map = load_all_credentials()?;
        let Some(entry) = map.get(profile).cloned() else {
            return Ok(values::Value::new_none());
        };
        // Auto-refresh if expired and refreshable
        let entry = if is_expired_jwt(&entry) && can_refresh(&entry) {
            match block_on(refresh_access_token(&entry)) {
                Ok(refreshed) => {
                    let mut map = map;
                    map.insert(profile.to_string(), refreshed.clone());
                    save_all_credentials(&map)?;
                    refreshed
                }
                Err(_) => {
                    return Err(anyhow::anyhow!(
                        "session expired\n\nRun `aspect auth login` to re-authenticate."
                    ));
                }
            }
        } else {
            entry
        };
        Ok(heap.alloc(AuthCredentials::from_entry(&entry)))
    }
}

#[starlark_module]
fn register_auth_types(globals: &mut GlobalsBuilder) {
    const Auth: StarlarkValueAsType<Auth> = StarlarkValueAsType::new();
    const AuthSession: StarlarkValueAsType<AuthSession> = StarlarkValueAsType::new();
    const AuthCredentials: StarlarkValueAsType<AuthCredentials> = StarlarkValueAsType::new();
}

pub fn register_globals(globals: &mut GlobalsBuilder) {
    globals.namespace("auth", register_auth_types);
}
