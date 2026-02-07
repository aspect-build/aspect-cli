mod cache;
mod config;

use std::collections::HashMap;
use std::env;
use std::env::var;
use std::fs;
use std::fs::File;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command as UnixCommand;
use std::process::ExitCode;
use std::str::FromStr;

use aspect_telemetry::{
    BZLARCH, BZLOS, GOARCH, GOOS, LLVM_TRIPLE, cargo_pkg_short_version, do_not_track,
    send_telemetry,
};
use clap::{Arg, Command, arg};
use fork::{Fork, fork};
use futures_util::TryStreamExt;
use miette::{Context, IntoDiagnostic, Result, miette};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{self, Client, Method, Request, RequestBuilder};
use serde::Deserialize;
use tokio::runtime;
use tokio::task::{self, JoinHandle};

use crate::cache::AspectCache;
use crate::config::{ToolSource, ToolSpec, autoconf};

fn debug_mode() -> bool {
    match var("ASPECT_DEBUG") {
        Ok(val) => !val.is_empty(),
        _ => false,
    }
}

const ASPECT_LAUNCHER_METHOD_HTTP: &str = "http";
const ASPECT_LAUNCHER_METHOD_GITHUB: &str = "github";
const ASPECT_LAUNCHER_METHOD_LOCAL: &str = "local";

async fn _download_into_cache(
    client: &Client,
    cache_entry: &PathBuf,
    req: Request,
    download_msg: &str,
) -> Result<()> {
    // Stream to a tempfile
    let tmp_file = cache_entry.with_extension("tmp");
    let tmpf = File::create(&tmp_file)
        .into_diagnostic()
        .context("failed to create temporary file")?;

    let metadata = fs::metadata(&tmp_file).into_diagnostic()?;
    let mut permissions = metadata.permissions();
    let new_mode = 0o755;
    permissions.set_mode(new_mode);
    fs::set_permissions(&tmp_file, permissions).into_diagnostic()?;

    let mut tmp_writer = tokio::fs::File::from(tmpf);
    let response = client
        .execute(req)
        .await
        .into_diagnostic()?
        .error_for_status()
        .into_diagnostic()?;

    eprintln!("{}", download_msg);

    let total_size = response.content_length();
    let mut byte_stream = response.bytes_stream();

    let mut downloaded: u64 = 0;

    while let Some(item) = byte_stream
        .try_next()
        .await
        .into_diagnostic()
        .wrap_err("failed to stream content")?
    {
        let chunk_size = item.len() as u64;
        tokio::io::copy(&mut item.as_ref(), &mut tmp_writer)
            .await
            .into_diagnostic()
            .wrap_err("failed to slab stream to file")?;

        downloaded += chunk_size;

        if let Some(total) = total_size {
            let percent = ((downloaded as f64 / total as f64) * 100.0) as u64;
            eprint!(
                "\r{:.0} / {:.0} KB ({}%)",
                downloaded as f64 / 1024.0,
                total as f64 / 1024.0,
                percent
            );
            io::stderr().flush().into_diagnostic()?;
        } else {
            eprint!("\r{:.0} KB", downloaded as f64 / 1024.0);
            io::stderr().flush().into_diagnostic()?;
        }
    }

    eprintln!();

    // And move it into the cache
    tokio::fs::rename(&tmp_file, &cache_entry)
        .await
        .into_diagnostic()
        .context("failed to move tool")?;

    // FIXME: Check download integrity/signatures?
    Ok(())
}

#[derive(Deserialize, Debug)]
struct Release {
    assets: Vec<ReleaseArtifact>,
}

#[derive(Deserialize, Debug)]
struct ReleaseArtifact {
    url: String,
    name: String,
}

fn headermap_from_hashmap<'a, I, S>(headers: I) -> HeaderMap
where
    I: Iterator<Item = (S, S)> + 'a,
    S: AsRef<str> + 'a,
{
    headers
        .map(|(name, val)| {
            (
                HeaderName::from_str(name.as_ref()),
                HeaderValue::from_str(val.as_ref()),
            )
        })
        // We ignore the errors here. If you want to get a list of failed conversions, you can use Iterator::partition
        // to help you out here
        .filter(|(k, v)| k.is_ok() && v.is_ok())
        .map(|(k, v)| (k.unwrap(), v.unwrap()))
        .collect()
}

fn gh_request(client: &Client, url: String) -> RequestBuilder {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("user-agent"),
        HeaderValue::from_static("aspect-launcher v0.0.1"),
    );
    headers.insert(
        HeaderName::from_static("x-github-api-version"),
        HeaderValue::from_static("2022-11-28"),
    );

    let mut builder = client.request(Method::GET, url).headers(headers);

    if let Ok(val) = env::var("GITHUB_TOKEN")
        && !val.is_empty()
    {
        builder = builder.bearer_auth(&val);
    }

    builder
}

async fn configure_tool_task(
    cache: AspectCache,
    root_dir: PathBuf,
    tool: Box<dyn ToolSpec + Send>,
) -> JoinHandle<Result<(PathBuf, String, HashMap<String, String>)>> {
    task::spawn((async move |cache: AspectCache,
                             root_dir: PathBuf,
                             tool: Box<dyn ToolSpec + Send>|
                -> Result<(
        PathBuf,
        String,
        HashMap<String, String>,
    )> {
        let mut errs: Vec<Result<()>> = Vec::new();

        let client = reqwest::Client::new();

        let liquid_globals = liquid::object!({
            "version": tool.version(),
            // Per @platforms, sigh
            "bzlos": BZLOS.to_string(),
            "bzlarch": BZLARCH.to_string(),
            // Per golang
            "goos": GOOS.to_string(),
            "goarch": GOARCH.to_string(),
            "llvm_triple": LLVM_TRIPLE.to_string(),
        });

        let liquid_parser = liquid::ParserBuilder::new().build().into_diagnostic()?;

        for source in tool.sources() {
            match source {
                ToolSource::Http { url, headers } => {
                    let url = liquid_parser
                        .parse(&url)
                        .into_diagnostic()?
                        .render(&liquid_globals)
                        .into_diagnostic()?;
                    let req_headers = headermap_from_hashmap(headers.iter());
                    let req = client
                        .request(Method::GET, &url)
                        .headers(req_headers)
                        .build()
                        .into_diagnostic()?;
                    let tool_dest_file = cache.tool_path(&tool.name(), &url);
                    let mut extra_envs = HashMap::new();
                    extra_envs.insert("ASPECT_LAUNCHER_ASPECT_CLI_URL".to_string(), url.clone());
                    if tool_dest_file.exists() {
                        if debug_mode() {
                            eprintln!(
                                "{:} source {:?} found in cache {:?}",
                                tool.name(),
                                source,
                                url
                            );
                        };
                        return Ok((
                            tool_dest_file,
                            ASPECT_LAUNCHER_METHOD_HTTP.to_string(),
                            extra_envs,
                        ));
                    }
                    fs::create_dir_all(tool_dest_file.parent().unwrap()).into_diagnostic()?;
                    if debug_mode() {
                        eprintln!(
                            "{:} source {:?} downloading {:?} to {:?}",
                            tool.name(),
                            source,
                            url,
                            tool_dest_file
                        );
                    };
                    let download_msg = format!("downloading aspect cli from {}", url);
                    if let err @ Err(_) =
                        _download_into_cache(&client, &tool_dest_file, req, &download_msg).await
                    {
                        errs.push(err);
                        continue;
                    };
                    return Ok((
                        tool_dest_file,
                        ASPECT_LAUNCHER_METHOD_HTTP.to_string(),
                        extra_envs,
                    ));
                }
                ToolSource::GitHub {
                    org,
                    repo,
                    release,
                    artifact,
                } => {
                    let release = liquid_parser
                        .parse(&release)
                        .into_diagnostic()?
                        .render(&liquid_globals)
                        .into_diagnostic()?;
                    let artifact = liquid_parser
                        .parse(&artifact)
                        .into_diagnostic()?
                        .render(&liquid_globals)
                        .into_diagnostic()?;

                    let url = format!(
                        "https://api.github.com/repos/{org}/{repo}/releases/tags/{release}"
                    );

                    let tool_dest_file = cache.tool_path(&tool.name(), &url);
                    let mut extra_envs = HashMap::new();
                    extra_envs.insert("ASPECT_LAUNCHER_ASPECT_CLI_ORG".to_string(), org.clone());
                    extra_envs.insert("ASPECT_LAUNCHER_ASPECT_CLI_REPO".to_string(), repo.clone());
                    extra_envs.insert(
                        "ASPECT_LAUNCHER_ASPECT_CLI_RELEASE".to_string(),
                        release.clone(),
                    );
                    extra_envs.insert(
                        "ASPECT_LAUNCHER_ASPECT_CLI_ARTIFACT".to_string(),
                        artifact.clone(),
                    );
                    if tool_dest_file.exists() {
                        if debug_mode() {
                            eprintln!(
                                "{:} source {:?} found in cache {:?}",
                                tool.name(),
                                source,
                                &url
                            );
                        };
                        return Ok((
                            tool_dest_file,
                            ASPECT_LAUNCHER_METHOD_GITHUB.to_string(),
                            extra_envs,
                        ));
                    }
                    fs::create_dir_all(tool_dest_file.parent().unwrap()).into_diagnostic()?;

                    let req = gh_request(&client, url)
                        .header(
                            HeaderName::from_static("accept"),
                            HeaderValue::from_static("application/vnd.github+json"),
                        )
                        .build()
                        .into_diagnostic()?;

                    let resp = client
                        .execute(req.try_clone().unwrap())
                        .await
                        .into_diagnostic()?;
                    let release_data: Release = resp.json::<Release>().await.into_diagnostic()?;
                    for asset in release_data.assets {
                        if asset.name == *artifact {
                            if debug_mode() {
                                eprintln!(
                                    "{:} source {:?} downloading {:?} to {:?}",
                                    tool.name(),
                                    source,
                                    asset.url,
                                    tool_dest_file
                                );
                            };
                            let req = gh_request(&client, asset.url)
                                .header(
                                    HeaderName::from_static("accept"),
                                    HeaderValue::from_static("application/octet-stream"),
                                )
                                .build()
                                .into_diagnostic()?;
                            let download_msg = format!(
                                "downloading aspect cli version {} file {}",
                                release, artifact
                            );
                            if let err @ Err(_) =
                                _download_into_cache(&client, &tool_dest_file, req, &download_msg)
                                    .await
                            {
                                errs.push(err);
                                break;
                            }
                            return Ok((
                                tool_dest_file,
                                ASPECT_LAUNCHER_METHOD_GITHUB.to_string(),
                                extra_envs,
                            ));
                        }
                    }
                    errs.push(Err(miette!("unable to find a release artifact in github!")));
                    continue;
                }
                ToolSource::Local { path } => {
                    let tool_dest_file = cache.tool_path(&tool.name(), path);
                    // Don't pull local sources from the cache since the local development flow will
                    // always be to copy the latest
                    fs::create_dir_all(tool_dest_file.parent().unwrap()).into_diagnostic()?;

                    let full_path = root_dir.join(path);
                    if fs::exists(&full_path).into_diagnostic()? {
                        if fs::exists(&tool_dest_file).into_diagnostic()? {
                            tokio::fs::remove_file(&tool_dest_file)
                                .await
                                .into_diagnostic()?;
                        }

                        // We use copies because Bazel nukes the output tree on build errors and we want to resist that
                        tokio::fs::copy(&full_path, &tool_dest_file)
                            .await
                            .into_diagnostic()?;

                        if debug_mode() {
                            eprintln!(
                                "{:} source {:?} copying from {:?} to {:?}",
                                tool.name(),
                                source,
                                full_path,
                                tool_dest_file
                            );
                        };

                        let metadata = fs::metadata(&tool_dest_file).into_diagnostic()?;
                        let mut permissions = metadata.permissions();
                        let new_mode = 0o755;
                        permissions.set_mode(new_mode);
                        fs::set_permissions(&tool_dest_file, permissions).into_diagnostic()?;
                        let mut extra_envs = HashMap::new();
                        extra_envs
                            .insert("ASPECT_LAUNCHER_ASPECT_CLI_PATH".to_string(), path.clone());
                        return Ok((
                            tool_dest_file,
                            ASPECT_LAUNCHER_METHOD_LOCAL.to_string(),
                            extra_envs,
                        ));
                    }
                }
            }
        }
        Err(miette!(format!(
            "exhausted tool sources {:?}; errors occurred {:?}",
            tool.sources(),
            errs
        )))
    })(cache.clone(), root_dir.clone(), tool))
}

fn main() -> Result<ExitCode> {
    let cmd = Command::new("aspect")
        .disable_help_flag(true)
        .arg(
            Arg::new("version")
                .short('v')
                .long("version")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            arg!(<args> ...)
                .trailing_var_arg(true)
                .required(false)
                .allow_hyphen_values(true),
        );

    let matches = cmd.get_matches();

    if matches.get_flag("version") {
        let v = cargo_pkg_short_version();
        println!("aspect launcher {v:}");
        return Ok(ExitCode::SUCCESS);
    }

    // Fork the launcher and report usage
    match fork().unwrap() {
        Fork::Child => {
            // Honor DO_NOT_TRACK
            if do_not_track() {
                return Ok(ExitCode::SUCCESS);
            }
            // Report telemetry
            let threaded_rt = runtime::Runtime::new().into_diagnostic()?;
            threaded_rt.block_on(async {
                let _ = send_telemetry().await;
            });
            Ok(ExitCode::SUCCESS)
        }
        Fork::Parent(_) => {
            // Deal with the config bits
            let (root_dir, config) = autoconf()?;
            let cache: AspectCache = AspectCache::default()?;

            let threaded_rt = runtime::Runtime::new().into_diagnostic()?;
            threaded_rt.block_on(async {
                let cli_task = configure_tool_task(
                    cache.clone(),
                    root_dir.clone(),
                    Box::new(config.aspect_cli.clone()),
                )
                .await;

                // Wait for fetches
                let cli = &config.aspect_cli;
                if debug_mode() {
                    eprintln!("attempting to provision {cli:?}");
                };

                let (cli_path, method, extra_envs) = cli_task.await.into_diagnostic()??;
                if debug_mode() {
                    eprintln!("provisioned at {cli_path:?}");
                };

                if debug_mode() {
                    eprintln!("attempting to run {cli_path:?}");
                };

                // Punt
                let mut cmd = UnixCommand::new(&cli_path);
                cmd.env("ASPECT_LAUNCHER", "true");
                cmd.env("ASPECT_LAUNCHER_VERSION", cargo_pkg_short_version());
                cmd.env("ASPECT_LAUNCHER_ASPECT_CLI_METHOD", method);
                for (k, v) in extra_envs {
                    cmd.env(k, v);
                }
                if let Some(args) = matches.get_many::<String>("args") {
                    cmd.args(args);
                };
                let err = cmd.exec();
                Err::<(), _>(miette!(format!(
                    "failed to punt to the `aspect-cli`, {:?}",
                    err
                )))
            })?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
