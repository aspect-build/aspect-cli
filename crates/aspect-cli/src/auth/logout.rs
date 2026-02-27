use std::process::ExitCode;

use super::credentials;

pub async fn run() -> miette::Result<ExitCode> {
    credentials::delete()?;
    eprintln!("Logged out successfully.");
    Ok(ExitCode::SUCCESS)
}
