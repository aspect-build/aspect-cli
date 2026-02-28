use std::fs;
use std::path::PathBuf;

use miette::{miette, Context, IntoDiagnostic};
use serde::{Deserialize, Serialize};

/// On-disk credential format stored at ~/.aspect/credentials.json
#[derive(Debug, Serialize, Deserialize)]
pub struct Credentials {
    pub access_token: String,
    pub refresh_token: String,
    pub email: String,
    pub name: String,
    pub tenant_id: String,
}

/// OAuth /oauth/token response
#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    // pub expires_in: Option<u64>,  // TODO: use for token refresh
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

/// Returns ~/.aspect/credentials.json
pub fn credentials_path() -> miette::Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| miette!("unable to determine home directory"))?;
    Ok(home.join(".aspect").join("credentials.json"))
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
