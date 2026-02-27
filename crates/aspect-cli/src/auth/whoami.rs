use std::process::ExitCode;

use miette::miette;

use super::credentials;
use super::login::decode_jwt_claims;

pub async fn run() -> miette::Result<ExitCode> {
    let creds = credentials::load()?;
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

    Ok(ExitCode::SUCCESS)
}
