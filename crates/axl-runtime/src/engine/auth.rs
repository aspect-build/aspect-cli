use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use allocative::Allocative;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use derive_more::Display;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::values::none::{NoneOr, NoneType};
use starlark::values::{NoSerialize, ProvidesStaticType, ValueLike};
use starlark::{starlark_module, starlark_simple_value, values};
use starlark::values::starlark_value;

use super::store::AxlStore;

// ── Production auth environment ──

const PRODUCTION_AUTH_DOMAIN: &str = "https://auth.aspect.build";
const PRODUCTION_API_URL: &str = "https://api.aspect.build";

// ── Credential types (mirrors aspect-cli/src/auth/credentials.rs) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Credentials {
    access_token: String,
    refresh_token: String,
    email: String,
    name: String,
    tenant_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_client_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JwtClaims {
    email: Option<String>,
    name: Option<String>,
    tenant_id: String,
}

#[derive(Debug, Deserialize)]
struct ApiTokenResponse {
    #[serde(alias = "access_token")]
    #[serde(rename = "accessToken")]
    access_token: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct GithubTokenRequest {
    owner: String,
    repo: String,
}

#[derive(Debug, Deserialize)]
struct GithubTokenResponse {
    token: String,
}

// ── Credential operations ──

fn credentials_path() -> anyhow::Result<PathBuf> {
    let base = match std::env::var("ASPECT_HOME") {
        Ok(val) if !val.is_empty() => PathBuf::from(val),
        _ => {
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("unable to determine home directory"))?;
            home.join(".aspect")
        }
    };
    Ok(base.join("credentials.json"))
}

fn load_credentials() -> anyhow::Result<Option<Credentials>> {
    let path = credentials_path()?;
    match fs::read_to_string(&path) {
        Ok(content) => {
            let creds: Credentials = serde_json::from_str(&content)?;
            Ok(Some(creds))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::anyhow!("failed to read credentials: {}", e)),
    }
}

fn save_credentials(creds: &Credentials) -> anyhow::Result<()> {
    let path = credentials_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(creds)?;
    fs::write(&path, &json)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

fn is_expired(creds: &Credentials) -> bool {
    let parts: Vec<&str> = creds.access_token.split('.').collect();
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

fn can_refresh(creds: &Credentials) -> bool {
    !creds.refresh_token.is_empty()
        && creds.auth_domain.is_some()
        && creds.auth_client_id.is_some()
}

async fn refresh_access_token(creds: &Credentials) -> anyhow::Result<Credentials> {
    let auth_domain = creds
        .auth_domain
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no auth_domain stored — cannot refresh"))?;
    let client_id = creds
        .auth_client_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no auth_client_id stored — cannot refresh"))?;

    let client = Client::new();
    let resp = client
        .post(format!("{}/oauth/token", auth_domain))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", creds.refresh_token.as_str()),
            ("client_id", client_id),
        ])
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "token refresh failed (HTTP {}): {}",
            status,
            body
        ));
    }

    let token_response: TokenResponse = resp.json().await?;
    let claims = decode_jwt_claims(&token_response.access_token)?;

    Ok(Credentials {
        access_token: token_response.access_token,
        refresh_token: token_response.refresh_token,
        email: claims.email.unwrap_or_else(|| "api-token".to_string()),
        name: claims.name.unwrap_or_else(|| "Unknown".to_string()),
        tenant_id: claims.tenant_id,
        auth_domain: creds.auth_domain.clone(),
        auth_client_id: creds.auth_client_id.clone(),
    })
}

async fn load_valid_credentials() -> anyhow::Result<Credentials> {
    let creds = load_credentials()?
        .ok_or_else(|| anyhow::anyhow!("not logged in\n\nRun `aspect auth login` to authenticate."))?;

    if !is_expired(&creds) {
        return Ok(creds);
    }

    if can_refresh(&creds) {
        match refresh_access_token(&creds).await {
            Ok(refreshed) => {
                save_credentials(&refreshed)?;
                return Ok(refreshed);
            }
            Err(e) => {
                eprintln!("Token refresh failed: {}", e);
            }
        }
    }

    Err(anyhow::anyhow!(
        "session expired\n\nRun `aspect auth login` to re-authenticate."
    ))
}

/// Resolve credentials: check in-memory (from `login_with_api_token`) first,
/// then fall back to disk credentials (from `aspect auth login`) with refresh.
async fn resolve_credentials(store: &AxlStore) -> anyhow::Result<Credentials> {
    // Check in-memory credentials first (synchronous lock, released before any .await)
    {
        let guard = store
            .credentials
            .lock()
            .map_err(|e| anyhow::anyhow!("failed to lock credentials: {}", e))?;
        if let Some(ref creds) = *guard {
            if !is_expired(creds) {
                return Ok(creds.clone());
            }
            // API token logins have no refresh_token — expired means re-login needed
            return Err(anyhow::anyhow!(
                "API token session expired. Call ctx.auth.login_with_api_token() again."
            ));
        }
    }

    // No in-memory credentials — fall back to disk
    load_valid_credentials().await
}

fn decode_jwt_claims(token: &str) -> anyhow::Result<JwtClaims> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(anyhow::anyhow!(
            "invalid JWT: expected 3 parts, got {}",
            parts.len()
        ));
    }
    let payload_bytes = URL_SAFE_NO_PAD.decode(parts[1])?;
    let claims: JwtClaims = serde_json::from_slice(&payload_bytes)?;
    Ok(claims)
}

/// Derive API base URL from the stored auth_domain.
fn api_url_from_auth_domain(auth_domain: Option<&str>) -> &str {
    match auth_domain {
        Some(d) if d.contains("staging") => "https://api-dev.aspect.build",
        Some(d) if d.contains("dev") => "https://api-dev.aspect.build",
        _ => PRODUCTION_API_URL,
    }
}

/// Detect owner/repo from git remote origin URL.
fn detect_github_repo(root_dir: &Path) -> anyhow::Result<(String, String)> {
    let output = std::process::Command::new("git")
        .args(["-C", &root_dir.to_string_lossy(), "remote", "get-url", "origin"])
        .output()?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "failed to get git remote origin URL — is this a git repository?"
        ));
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_github_remote(&url)
}

/// Parse owner/repo from a git remote URL.
/// Supports:
///   - git@github.com:owner/repo.git
///   - https://github.com/owner/repo.git
///   - https://github.com/owner/repo
fn parse_github_remote(url: &str) -> anyhow::Result<(String, String)> {
    // SSH format: git@github.com:owner/repo.git
    if let Some(path) = url.strip_prefix("git@github.com:") {
        let path = path.strip_suffix(".git").unwrap_or(path);
        return split_owner_repo(path);
    }

    // HTTPS format: https://github.com/owner/repo[.git]
    if let Some(rest) = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
    {
        let path = rest.strip_suffix(".git").unwrap_or(rest);
        return split_owner_repo(path);
    }

    Err(anyhow::anyhow!(
        "unable to parse GitHub owner/repo from remote URL: {}",
        url
    ))
}

fn split_owner_repo(path: &str) -> anyhow::Result<(String, String)> {
    let parts: Vec<&str> = path.splitn(2, '/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(anyhow::anyhow!(
            "expected owner/repo format, got: {}",
            path
        ));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

// ── Auto-login ──

/// Auto-login with `ASPECT_API_TOKEN` if set.
/// Tolerant of failures — prints a warning instead of returning an error.
/// Called from Rust before task execution so Starlark error handling is not needed.
pub fn try_auto_login(store: &AxlStore) {
    let debug = std::env::var("ASPECT_DEBUG").is_ok();
    let token = match std::env::var("ASPECT_API_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => {
            if debug {
                eprintln!("try_auto_login: ASPECT_API_TOKEN not set, skipping");
            }
            return;
        }
    };

    // Skip if credentials are already set (e.g., from a previous call)
    if let Ok(guard) = store.credentials.lock() {
        if guard.is_some() {
            return;
        }
    }

    let parts: Vec<&str> = token.splitn(2, ':').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        eprintln!(
            "Warning: ASPECT_API_TOKEN has invalid format (expected client_id:secret)"
        );
        return;
    }
    let client_id = parts[0].to_string();
    let secret = parts[1].to_string();

    match store.rt.0.block_on(async {
        let client = Client::new();
        let resp = client
            .post(format!(
                "{}/identity/resources/auth/v1/api-token",
                PRODUCTION_AUTH_DOMAIN
            ))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "clientId": client_id,
                "secret": secret,
            }))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("failed to exchange API token: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "API token exchange failed (HTTP {}): {}",
                status,
                body
            ));
        }

        let data: ApiTokenResponse = resp.json().await?;
        let claims = decode_jwt_claims(&data.access_token)?;

        Ok(Credentials {
            access_token: data.access_token,
            refresh_token: String::new(),
            email: claims.email.unwrap_or_else(|| "api-token".to_string()),
            name: claims.name.unwrap_or_else(|| "Unknown".to_string()),
            tenant_id: claims.tenant_id,
            auth_domain: None,
            auth_client_id: None,
        })
    }) {
        Ok(creds) => {
            if debug {
                eprintln!("try_auto_login: success (email={})", creds.email);
            }
            if let Ok(mut guard) = store.credentials.lock() {
                *guard = Some(creds);
            }
        }
        Err(e) => {
            eprintln!(
                "Warning: auto-login with ASPECT_API_TOKEN failed: {}",
                e
            );
        }
    }
}

// ── Starlark Auth type ──

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<Auth>")]
pub struct Auth;

starlark_simple_value!(Auth);

#[starlark_value(type = "Auth")]
impl<'v> values::StarlarkValue<'v> for Auth {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(auth_methods)
    }
}

#[starlark_module]
pub(crate) fn auth_methods(registry: &mut MethodsBuilder) {
    /// Authenticate using an API token (client_id:secret format).
    /// Exchanges the token for a JWT and stores credentials in-memory
    /// (scoped to this execution — not persisted to disk).
    fn login_with_api_token<'v>(
        #[allow(unused)] this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
        #[starlark(require = named)] api_token: &str,
    ) -> starlark::Result<NoneType> {
        let store = AxlStore::from_eval(eval)?;

        // Parse client_id:secret
        let parts: Vec<&str> = api_token.splitn(2, ':').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "invalid api_token format: expected 'client_id:secret'"
            )));
        }
        let client_id = parts[0].to_string();
        let secret = parts[1].to_string();

        let creds = store
            .rt
            .0
            .block_on(async {
                let client = Client::new();
                let resp = client
                    .post(format!(
                        "{}/identity/resources/auth/v1/api-token",
                        PRODUCTION_AUTH_DOMAIN
                    ))
                    .header("Content-Type", "application/json")
                    .json(&serde_json::json!({
                        "clientId": client_id,
                        "secret": secret,
                    }))
                    .send()
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to exchange API token: {}", e))?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(anyhow::anyhow!(
                        "API token exchange failed (HTTP {}): {}",
                        status,
                        body
                    ));
                }

                let data: ApiTokenResponse = resp.json().await?;
                let claims = decode_jwt_claims(&data.access_token)?;

                Ok(Credentials {
                    access_token: data.access_token,
                    refresh_token: String::new(),
                    email: claims.email.unwrap_or_else(|| "api-token".to_string()),
                    name: claims.name.unwrap_or_else(|| "Unknown".to_string()),
                    tenant_id: claims.tenant_id,
                    auth_domain: None,
                    auth_client_id: None,
                })
            })
            .map_err(starlark::Error::new_other)?;

        // Store credentials in-memory only — scoped to this execution
        let mut guard = store.credentials.lock().map_err(|e| {
            starlark::Error::new_other(anyhow::anyhow!("failed to lock credentials: {}", e))
        })?;
        *guard = Some(creds);

        Ok(NoneType)
    }

    /// Clear in-memory credentials for this execution.
    fn logout<'v>(
        #[allow(unused)] this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let store = AxlStore::from_eval(eval)?;
        let mut guard = store.credentials.lock().map_err(|e| {
            starlark::Error::new_other(anyhow::anyhow!("failed to lock credentials: {}", e))
        })?;
        *guard = None;
        Ok(NoneType)
    }

    /// Return information about the currently authenticated user.
    fn whoami<'v>(
        #[allow(unused)] this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<WhoAmI> {
        let store = AxlStore::from_eval(eval)?;

        // Check in-memory credentials first
        {
            let guard = store.credentials.lock().map_err(|e| {
                starlark::Error::new_other(anyhow::anyhow!(
                    "failed to lock credentials: {}",
                    e
                ))
            })?;
            if let Some(ref creds) = *guard {
                return Ok(WhoAmI {
                    name: creds.name.clone(),
                    email: creds.email.clone(),
                    tenant_id: creds.tenant_id.clone(),
                });
            }
        }

        // Fall back to disk credentials
        let creds = load_credentials()
            .map_err(starlark::Error::new_other)?
            .ok_or_else(|| {
                starlark::Error::new_other(anyhow::anyhow!(
                    "not logged in\n\nRun `aspect auth login` to authenticate."
                ))
            })?;

        Ok(WhoAmI {
            name: creds.name,
            email: creds.email,
            tenant_id: creds.tenant_id,
        })
    }

    /// Get a short-lived GitHub API token via the Aspect GitHub App.
    /// Auto-detects the repository from git remote origin.
    fn marvin_github_api_token<'v>(
        #[allow(unused)] this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<values::StringValue<'v>> {
        let store = AxlStore::from_eval(eval)?;

        // Detect owner/repo from git remote
        let (owner, repo) =
            detect_github_repo(&store.root_dir).map_err(starlark::Error::new_other)?;

        let token = store
            .rt
            .0
            .block_on(async {
                let creds = resolve_credentials(&store).await?;
                let api_url = api_url_from_auth_domain(creds.auth_domain.as_deref());

                let body = GithubTokenRequest { owner, repo };

                let client = Client::new();
                let resp = client
                    .post(format!("{}/github/token", api_url))
                    .header(
                        "Authorization",
                        format!("Bearer {}", creds.access_token),
                    )
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to connect to Aspect API: {}", e))?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let raw_body = resp.text().await.unwrap_or_default();
                    let error_msg = serde_json::from_str::<serde_json::Value>(&raw_body)
                        .ok()
                        .and_then(|v| v["error"].as_str().map(String::from))
                        .unwrap_or(raw_body);

                    return match status.as_u16() {
                        401 => Err(anyhow::anyhow!(
                            "authentication failed: {}\n\nRun `aspect auth login` to re-authenticate.",
                            error_msg
                        )),
                        403 => Err(anyhow::anyhow!("access denied: {}", error_msg)),
                        404 => Err(anyhow::anyhow!(
                            "{}\n\nSetup the Aspect Workflows GitHub App at https://github-app.aspect.build",
                            error_msg
                        )),
                        _ => Err(anyhow::anyhow!(
                            "API request failed (HTTP {}): {}",
                            status,
                            error_msg
                        )),
                    };
                }

                let token_response: GithubTokenResponse = resp.json().await?;
                Ok(token_response.token)
            })
            .map_err(starlark::Error::new_other)?;

        Ok(eval.heap().alloc_str(&token))
    }

    /// Like `marvin_github_api_token()` but returns `None` instead of raising on failure.
    /// Useful in Starlark hooks that need to gracefully handle missing credentials
    /// (Starlark has no try/except).
    fn try_marvin_github_api_token<'v>(
        #[allow(unused)] this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneOr<values::StringValue<'v>>> {
        let store = AxlStore::from_eval(eval)?;

        // Quick check: do we have any credentials at all?
        {
            let guard = store.credentials.lock().map_err(|e| {
                starlark::Error::new_other(anyhow::anyhow!(
                    "failed to lock credentials: {}",
                    e
                ))
            })?;
            if guard.is_none() {
                // No in-memory creds — check disk
                match load_credentials() {
                    Ok(Some(_)) => {} // disk creds exist, proceed
                    _ => return Ok(NoneOr::None),
                }
            }
        }

        // Try to detect repo — return None if not in a git repo
        let (owner, repo) = match detect_github_repo(&store.root_dir) {
            Ok(r) => r,
            Err(_) => return Ok(NoneOr::None),
        };

        let debug = std::env::var("ASPECT_DEBUG").is_ok();

        match store.rt.0.block_on(async {
            let creds = resolve_credentials(&store).await?;
            let api_url = api_url_from_auth_domain(creds.auth_domain.as_deref());

            if debug {
                eprintln!(
                    "try_marvin_github_api_token: requesting token for {}/{} via {}",
                    owner, repo, api_url
                );
                eprintln!(
                    "try_marvin_github_api_token: creds email={}, tenant_id={}, auth_domain={:?}, token_len={}",
                    creds.email, creds.tenant_id, creds.auth_domain, creds.access_token.len()
                );
            }

            let body = GithubTokenRequest { owner, repo };

            let client = Client::new();
            let resp = client
                .post(format!("{}/github/token", api_url))
                .header(
                    "Authorization",
                    format!("Bearer {}", creds.access_token),
                )
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("failed to connect to Aspect API: {}", e))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let raw_body = resp.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!(
                    "GitHub token request failed (HTTP {}): {}",
                    status,
                    raw_body
                ));
            }

            let token_response: GithubTokenResponse = resp.json().await?;
            Ok(token_response.token)
        }) {
            Ok(token) => Ok(NoneOr::Other(eval.heap().alloc_str(&token))),
            Err(e) => {
                if debug {
                    eprintln!("try_marvin_github_api_token: failed: {}", e);
                }
                Ok(NoneOr::None)
            }
        }
    }
}

// ── Starlark WhoAmI type ──

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<WhoAmI {name} ({email})>")]
pub struct WhoAmI {
    name: String,
    email: String,
    tenant_id: String,
}

starlark_simple_value!(WhoAmI);

#[starlark_value(type = "WhoAmI")]
impl<'v> values::StarlarkValue<'v> for WhoAmI {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(whoami_methods)
    }
}

#[starlark_module]
pub(crate) fn whoami_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn name<'v>(this: values::Value<'v>) -> anyhow::Result<&'v str> {
        Ok(this.downcast_ref_err::<WhoAmI>()?.name.as_str())
    }

    #[starlark(attribute)]
    fn email<'v>(this: values::Value<'v>) -> anyhow::Result<&'v str> {
        Ok(this.downcast_ref_err::<WhoAmI>()?.email.as_str())
    }

    #[starlark(attribute)]
    fn tenant_id<'v>(this: values::Value<'v>) -> anyhow::Result<&'v str> {
        Ok(this.downcast_ref_err::<WhoAmI>()?.tenant_id.as_str())
    }
}
