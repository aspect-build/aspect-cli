use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use miette::miette;

use super::credentials;
use super::login::decode_jwt_claims;

pub async fn run() -> miette::Result<ExitCode> {
    let creds = credentials::load_valid().await?;
    let Some(creds) = creds else {
        return Err(miette!(
            "not logged in\n\nRun `aspect auth login` to authenticate."
        ));
    };

    // Decode user info from the stored JWT
    let user_info = decode_jwt_claims(&creds.access_token)?;

    println!("Logged in as:");
    println!("  Name:   {}", user_info.name);
    println!("  Email:  {}", user_info.email);
    println!("  Tenant: {}", user_info.tenant_id);

    // Show token expiry status
    if let Some(exp) = credentials::expiry_timestamp(&creds) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if now >= exp {
            println!("  Token:  expired");
        } else {
            let remaining = exp - now;
            let hours = remaining / 3600;
            let minutes = (remaining % 3600) / 60;
            if hours > 0 {
                println!("  Token:  expires in {}h {}m", hours, minutes);
            } else {
                println!("  Token:  expires in {}m", minutes);
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}
