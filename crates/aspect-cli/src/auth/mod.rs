mod credentials;
pub(crate) mod login;
mod logout;
mod whoami;

use std::process::ExitCode;

use clap::{Arg, ArgAction, Command};
use miette::{miette, Context, IntoDiagnostic};

/// Auth environment configuration (domain + OAuth client ID).
#[derive(Debug, Clone, Copy)]
pub struct AuthEnv {
    pub domain: &'static str,
    pub client_id: &'static str,
}

const ENV_PRODUCTION: AuthEnv = AuthEnv {
    domain: "https://auth.aspect.build",
    client_id: "771ff228-18a1-43f0-bc83-62c9df0d72ca",
};

const ENV_STAGING: AuthEnv = AuthEnv {
    domain: "https://auth.staging.aspect.build",
    client_id: "76f292c5-e1b8-40a5-bc92-f30e521251f1",
};

const ENV_DEV: AuthEnv = AuthEnv {
    domain: "https://auth.dev.aspect.build",
    client_id: "a1c8902d-fbbe-43bf-8c4e-f58f79310e7a",
};

/// Build the clap Command for `aspect auth`.
pub fn command() -> Command {
    Command::new("auth")
        .bin_name("aspect auth")
        .about("Authenticate with Aspect")
        .arg_required_else_help(true)
        .disable_help_subcommand(true)
        .subcommand_value_name("COMMAND")
        .help_template("\
{about-with-newline}
{usage-heading} {usage}

\x1b[1;4mCommands:\x1b[0m
{subcommands}

\x1b[1;4mOptions:\x1b[0m
{options}")
        .subcommand(
            Command::new("login")
                .about("Log in to Aspect")
                .arg(
                    Arg::new("with-token")
                        .long("with-token")
                        .help("Read a JWT token from stdin")
                        .action(ArgAction::SetTrue)
                        .conflicts_with("with-api-token"),
                )
                .arg(
                    Arg::new("with-api-token")
                        .long("with-api-token")
                        .help("Read an API token (client_id:secret) from stdin")
                        .action(ArgAction::SetTrue)
                        .conflicts_with("with-token"),
                )
                .arg(
                    Arg::new("environment")
                        .long("environment")
                        .hide(true)
                        .value_name("ENV"),
                ),
        )
        .subcommand(
            Command::new("logout")
                .about("Log out and remove stored credentials"),
        )
        .subcommand(
            Command::new("whoami")
                .about("Show the currently authenticated user"),
        )
}

/// Entry point for `aspect auth <subcommand>`.
/// Called from main.rs before Starlark evaluation.
pub async fn handle(args: Vec<String>) -> miette::Result<ExitCode> {
    // args[0] = "aspect" (or binary path), args[1] = "auth", args[2..] = subcommand + flags
    let cmd = command();
    let matches = match cmd.try_get_matches_from(&args[1..]) {
        Ok(m) => m,
        Err(e) => {
            e.print().into_diagnostic()?;
            return Ok(ExitCode::from(e.exit_code() as u8));
        }
    };

    match matches.subcommand() {
        Some(("login", sub_matches)) => {
            let env_str = sub_matches.get_one::<String>("environment").map(|s| s.as_str());
            let env = resolve_auth_env(env_str)?;

            if sub_matches.get_flag("with-token") {
                let token = read_stdin()?;
                login::run_with_token(&token).await
            } else if sub_matches.get_flag("with-api-token") {
                let input = read_stdin()?;
                let (client_id, secret) = input.split_once(':').ok_or_else(|| {
                    miette!("invalid API token format: expected client_id:secret")
                })?;
                login::run_with_api_token(client_id, secret, env).await
            } else {
                login::run_browser(env).await
            }
        }
        Some(("logout", _)) => logout::run().await,
        Some(("whoami", _)) => whoami::run().await,
        _ => unreachable!("arg_required_else_help ensures a subcommand"),
    }
}

/// Read all of stdin and return trimmed content.
fn read_stdin() -> miette::Result<String> {
    use std::io::Read;
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .into_diagnostic()
        .wrap_err("failed to read from stdin")?;
    let trimmed = buf.trim().to_string();
    if trimmed.is_empty() {
        return Err(miette!("no token provided on stdin"));
    }
    Ok(trimmed)
}

/// Resolve the auth environment from --environment flag value, defaulting to production.
fn resolve_auth_env(env: Option<&str>) -> miette::Result<AuthEnv> {
    match env {
        None | Some("production") | Some("prod") => Ok(ENV_PRODUCTION),
        Some("staging") => Ok(ENV_STAGING),
        Some("dev") | Some("development") => Ok(ENV_DEV),
        Some(other) => Err(miette!(
            "unknown environment: {:?}\n\nValid values: production (default), staging, dev",
            other
        )),
    }
}
