use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use miette::{Context, IntoDiagnostic, miette};
use serde::{Deserialize, Serialize};

/// On-disk credential format stored at ~/.aspect/credentials.json
#[derive(Debug, Serialize, Deserialize)]
pub struct Credentials {
    pub access_token: String,
    pub refresh_token: String,
    pub email: String,
    pub name: String,
    pub tenant_id: String,
    /// Auth domain used during login (needed for token refresh).
    /// None for --with-token and --with-api-token logins, or legacy credential files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_domain: Option<String>,
    /// OAuth client ID used during login (needed for token refresh).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_client_id: Option<String>,
}

/// OAuth /oauth/token response
#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
}

/// User info decoded from JWT claims
#[derive(Debug, Deserialize)]
pub struct UserInfo {
    pub email: String,
    pub name: String,
    #[serde(rename = "tenantId")]
    pub tenant_id: String,
}

/// JWT claims from an access token.
/// User JWTs have email/name; vendor JWTs (from API tokens) may not.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JwtClaims {
    pub email: Option<String>,
    pub name: Option<String>,
    pub tenant_id: String,
}

/// Returns the credentials file path.
/// Uses `$ASPECT_HOME/credentials.json` if `ASPECT_HOME` is set,
/// otherwise `~/.aspect/credentials.json`.
pub fn credentials_path() -> miette::Result<PathBuf> {
    let base = match std::env::var("ASPECT_HOME") {
        Ok(val) if !val.is_empty() => PathBuf::from(val),
        _ => {
            let home =
                dirs::home_dir().ok_or_else(|| miette!("unable to determine home directory"))?;
            home.join(".aspect")
        }
    };
    Ok(base.join("credentials.json"))
}

/// Check if the access token is expired or will expire within 60 seconds.
/// Returns false if the token has no `exp` claim (can't determine expiry).
pub fn is_expired(creds: &Credentials) -> bool {
    // Decode the JWT to get the exp claim without full validation
    let parts: Vec<&str> = creds.access_token.split('.').collect();
    if parts.len() != 3 {
        return true; // malformed JWT
    }
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
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
        return false; // no exp claim, can't determine — assume valid
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Expired or expiring within 60 seconds
    now + 60 >= exp
}

/// Returns the expiry timestamp (seconds since epoch) from the access token, if available.
pub fn expiry_timestamp(creds: &Credentials) -> Option<u64> {
    let parts: Vec<&str> = creds.access_token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload_bytes = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    #[derive(Deserialize)]
    struct ExpOnly {
        exp: Option<u64>,
    }
    serde_json::from_slice::<ExpOnly>(&payload_bytes).ok()?.exp
}

/// Can this credential set be refreshed? Requires a non-empty refresh_token and auth_domain.
pub fn can_refresh(creds: &Credentials) -> bool {
    !creds.refresh_token.is_empty() && creds.auth_domain.is_some() && creds.auth_client_id.is_some()
}

/// Load credentials from disk. Returns None if file doesn't exist.
pub fn load() -> miette::Result<Option<Credentials>> {
    let path = credentials_path()?;
    match fs::read_to_string(&path) {
        Ok(content) => {
            let creds: Credentials = serde_json::from_str(&content)
                .into_diagnostic()
                .wrap_err("failed to parse credentials file")?;
            Ok(Some(creds))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(miette!(
            "failed to read credentials file at {:?}: {}",
            path,
            e
        )),
    }
}

/// Load credentials, automatically refreshing an expired access_token if possible.
pub async fn load_valid() -> miette::Result<Option<Credentials>> {
    let Some(creds) = load()? else {
        return Ok(None);
    };

    if !is_expired(&creds) {
        return Ok(Some(creds));
    }

    // Token is expired — try to refresh
    if can_refresh(&creds) {
        eprintln!("Token expired, refreshing...");
        match super::login::refresh_access_token(&creds).await {
            Ok(refreshed) => {
                save(&refreshed)?;
                eprintln!("Token refreshed.");
                return Ok(Some(refreshed));
            }
            Err(e) => {
                eprintln!("Token refresh failed: {}", e);
            }
        }
    }

    Err(miette!(
        "session expired\n\nRun `aspect auth login` to re-authenticate."
    ))
}

/// Save credentials to disk, creating ~/.aspect/ if needed.
pub fn save(creds: &Credentials) -> miette::Result<()> {
    let path = credentials_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .into_diagnostic()
            .wrap_err("failed to create ~/.aspect directory")?;
    }
    let json = serde_json::to_string_pretty(creds)
        .into_diagnostic()
        .wrap_err("failed to serialize credentials")?;
    fs::write(&path, &json)
        .into_diagnostic()
        .wrap_err("failed to write credentials file")?;

    // Set file permissions to 0600 (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&path, perms)
            .into_diagnostic()
            .wrap_err("failed to set credentials file permissions")?;
    }

    Ok(())
}

/// Delete the credentials file. Returns Ok even if file doesn't exist.
pub fn delete() -> miette::Result<()> {
    let path = credentials_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(miette!(
            "failed to delete credentials file at {:?}: {}",
            path,
            e
        )),
    }
}
