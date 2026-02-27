mod token;

use std::collections::HashMap;
use std::process::ExitCode;

use clap::{Arg, ArgAction, Command};
use miette::{miette, IntoDiagnostic};

use crate::auth;

/// Build the clap Command for `aspect github`.
pub fn command() -> Command {
    let mut token_cmd = Command::new("token")
        .about("Get a short-lived GitHub access token via the Aspect GitHub App")
        .arg(
            Arg::new("repo")
                .required(true)
                .value_name("OWNER/REPO")
                .help("Repository in owner/repo format (e.g. my-org/my-repo)"),
        )
        .arg(
            Arg::new("permissions")
                .long("permissions")
                .short('p')
                .value_name("PERMS")
                .help("Comma-separated permissions (e.g. contents=read,issues=write)"),
        )
        .arg(
            Arg::new("json")
                .long("json")
                .action(ArgAction::SetTrue)
                .help("Output full JSON response instead of just the token"),
        )
        .arg(
            Arg::new("environment")
                .long("environment")
                .hide(true)
                .value_name("ENV"),
        );

    // Only expose --api-url in unstamped dev builds
    if env!("CARGO_PKG_VERSION") == "0.0.0-dev" {
        token_cmd = token_cmd.arg(
            Arg::new("api-url")
                .long("api-url")
                .value_name("URL")
                .hide(true)
                .help("Override the API base URL (dev builds only)"),
        );
    }

    Command::new("github")
        .bin_name("aspect github")
        .about("Interact with the Aspect GitHub App")
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
        .subcommand(token_cmd)
}

/// Entry point for `aspect github <subcommand>`.
pub async fn handle(args: Vec<String>) -> miette::Result<ExitCode> {
    let cmd = command();
    let matches = match cmd.try_get_matches_from(&args[1..]) {
        Ok(m) => m,
        Err(e) => {
            e.print().into_diagnostic()?;
            return Ok(ExitCode::from(e.exit_code() as u8));
        }
    };

    match matches.subcommand() {
        Some(("token", sub_matches)) => {
            let repo_arg = sub_matches.get_one::<String>("repo").unwrap();
            let (owner, repo) = parse_owner_repo(repo_arg)?;

            let permissions = sub_matches
                .get_one::<String>("permissions")
                .map(|s| parse_permissions(s))
                .transpose()?;

            let json_output = sub_matches.get_flag("json");

            // Resolve API URL: --api-url override (dev builds) > --environment > default
            let api_url = sub_matches
                .get_one::<String>("api-url")
                .cloned()
                .unwrap_or_else(|| {
                    let env_str = sub_matches
                        .get_one::<String>("environment")
                        .map(|s| s.as_str());
                    // resolve_auth_env defaults to production if env_str is None
                    auth::resolve_auth_env(env_str)
                        .map(|env| env.api_url.to_string())
                        .unwrap_or_else(|_| "https://api.aspect.build".to_string())
                });

            token::run(&owner, &repo, permissions, &api_url, json_output).await
        }
        _ => unreachable!("arg_required_else_help ensures a subcommand"),
    }
}

/// Parse "owner/repo" into (owner, repo).
fn parse_owner_repo(input: &str) -> miette::Result<(String, String)> {
    let parts: Vec<&str> = input.splitn(2, '/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(miette!(
            "invalid repository format: {:?}\n\nExpected format: OWNER/REPO (e.g. my-org/my-repo)",
            input
        ));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Parse "contents=read,issues=write" into a HashMap.
fn parse_permissions(input: &str) -> miette::Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for pair in input.split(',') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').ok_or_else(|| {
            miette!(
                "invalid permission format: {:?}\n\nExpected format: key=value (e.g. contents=read)",
                pair
            )
        })?;
        map.insert(key.to_string(), value.to_string());
    }
    if map.is_empty() {
        return Err(miette!("empty permissions string"));
    }
    Ok(map)
}
