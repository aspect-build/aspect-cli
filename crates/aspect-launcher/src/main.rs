use aspect_cache::AspectCache;
use aspect_config::{
    autoconf, cli_version, debug_mode, ToolSource, ToolSpec, BZLARCH, BZLOS, GOARCH, GOOS,
    LLVM_TRIPLE, TELURL,
};
use clap::{arg, Arg, Command};
use fork::{fork, Fork};
use futures_util::TryStreamExt;
use miette::{miette, Context, IntoDiagnostic, Result};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::redirect::Policy;
use reqwest::{self, Client, Method, Request, RequestBuilder, StatusCode};
use serde::Deserialize;
use std::env::{self, var};
use std::fs;
use std::fs::File;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command as UnixCommand;
use std::process::ExitCode;
use std::str::FromStr;
use std::time::Duration;
use tokio::runtime;
use tokio::task::{self, JoinHandle};

async fn _download_into_cache(client: &Client, cache_entry: &PathBuf, req: Request) -> Result<()> {
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
    let mut byte_stream = client
        .execute(req)
        .await
        .into_diagnostic()?
        .error_for_status()
        .into_diagnostic()?
        .bytes_stream();

    while let Some(item) = byte_stream
        .try_next()
        .await
        .into_diagnostic()
        .wrap_err("Failed to stream content")?
    {
        tokio::io::copy(&mut item.as_ref(), &mut tmp_writer)
            .await
            .into_diagnostic()
            .wrap_err("Failed to slab stream to file")?;
    }

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
    repo_dir: PathBuf,
    tool: Box<dyn ToolSpec + Send>,
) -> JoinHandle<Result<()>> {
    task::spawn((async move |cache: AspectCache,
                             repo_dir: PathBuf,
                             tool: Box<dyn ToolSpec + Send>|
                -> Result<()> {
        let tool_dest_file = cache.tool_path(&(*tool));
        fs::create_dir_all(tool_dest_file.parent().unwrap()).into_diagnostic()?;

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

        if tool_dest_file.exists() {
            if debug_mode() {
                eprintln!("Tool {tool:?} already in cache");
            };
            return Ok(());
        }

        for source in tool.sources() {
            match source {
                ToolSource::Http { url, headers } => {
                    let url = liquid_parser
                        .parse(url)
                        .into_diagnostic()?
                        .render(&liquid_globals)
                        .into_diagnostic()?;
                    let req_headers = headermap_from_hashmap(headers.iter());
                    let req = client
                        .request(Method::GET, url)
                        .headers(req_headers)
                        .build()
                        .into_diagnostic()?;
                    if let err @ Err(_) = _download_into_cache(&client, &tool_dest_file, req).await
                    {
                        errs.push(err);
                    };
                }
                ToolSource::Github {
                    org,
                    repo,
                    release,
                    artifact,
                } => {
                    let release = liquid_parser
                        .parse(release)
                        .into_diagnostic()?
                        .render(&liquid_globals)
                        .into_diagnostic()?;
                    let artifact = liquid_parser
                        .parse(artifact)
                        .into_diagnostic()?
                        .render(&liquid_globals)
                        .into_diagnostic()?;

                    let req = gh_request(
                        &client,
                        format!(
                            "https://api.github.com/repos/{org}/{repo}/releases/tags/{release}"
                        ),
                    )
                    .header(
                        HeaderName::from_static("accept"),
                        HeaderValue::from_static("application/vnd.github+json"),
                    )
                    .build()
                    .into_diagnostic()?;

                    // FIXME: Accumulate errors
                    let resp = client
                        .execute(req.try_clone().unwrap())
                        .await
                        .into_diagnostic()?;
                    let release: Release = resp.json::<Release>().await.into_diagnostic()?;
                    for asset in release.assets {
                        if asset.name == *artifact {
                            let req = gh_request(&client, asset.url)
                                .header(
                                    HeaderName::from_static("accept"),
                                    HeaderValue::from_static("application/octet-stream"),
                                )
                                .build()
                                .into_diagnostic()?;
                            if let err @ Err(_) =
                                _download_into_cache(&client, &tool_dest_file, req).await
                            {
                                errs.push(err);
                            }
                        }
                    }
                    errs.push(Err(miette!(
                        "Unable to find a matching release artifact in github!"
                    )));
                    continue;
                }
                ToolSource::Local { path } => {
                    let path = repo_dir.join(path);

                    if fs::exists(&path).into_diagnostic()? {
                        if fs::exists(&tool_dest_file).into_diagnostic()? {
                            tokio::fs::remove_file(&tool_dest_file)
                                .await
                                .into_diagnostic()?;
                        }

                        // We use copies because Bazel nukes the output tree on build errors and we want to resist that
                        tokio::fs::copy(&path, &tool_dest_file)
                            .await
                            .into_diagnostic()?;

                        let metadata = fs::metadata(&tool_dest_file).into_diagnostic()?;
                        let mut permissions = metadata.permissions();
                        let new_mode = 0o755;
                        permissions.set_mode(new_mode);
                        fs::set_permissions(&tool_dest_file, permissions).into_diagnostic()?;
                    }
                }
            }
            if tool_dest_file.exists() {
                if debug_mode() {
                    eprintln!("Hit in {source:?}");
                };
                return Ok(());
            }
        }

        Err(miette!(format!(
            "Exhausted tool sources {:?}; errors occurred {:?}",
            tool.sources(),
            errs
        )))
    })(cache.clone(), repo_dir.clone(), tool))
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
        let v = cli_version();
        println!("Aspect CLI Launcher {v:}");
        return Ok(ExitCode::SUCCESS);
    }

    // Fork the launcher and report usage
    match fork().unwrap() {
        Fork::Child => {
            // Honor DO_NOT_TRACK
            if var("DO_NOT_TRACK").is_ok() {
                return Ok(ExitCode::SUCCESS);
            }

            let threaded_rt = runtime::Runtime::new().into_diagnostic()?;
            threaded_rt.block_on(async {
                // Report telemetry
                let v = cli_version();
                let body = format!(
                    "{{\"cli\": {{\"version\": \"{v}\", \"os\": \"{BZLOS}\", \"arch\": \"{BZLARCH}\"}}}}"
                );
                let mut url = TELURL.to_string();
                let client = reqwest::Client::builder().redirect(Policy::none()).build().unwrap();

                loop {
                    let req = client
                        .request(Method::POST, &url)
                        .query(&[("source", "aspect-cli")])
                        .header(HeaderName::from_static("content-type"), "application/json")
                        .header(HeaderName::from_static("user-agent"), "reqwest;aspect-cli")
                        .body(body.clone())
                        .timeout(Duration::from_secs(5));

                    let send_res = req.send().await;

                    let send_res = match send_res {
                        Ok(r) => r,
                        Err(_) => break,
                    };

                    match send_res.status() {
                        StatusCode::FOUND | StatusCode::PERMANENT_REDIRECT | StatusCode::TEMPORARY_REDIRECT => {
                            if let Some(loc) = send_res.headers().get("location") {
                                if let Ok(loc_str) = loc.to_str() {
                                    url = loc_str.to_owned();
                                    continue;
                                }
                            }
                            break;
                        },
                        _ => break
                    };
                }
            });

            Ok(ExitCode::SUCCESS)
        }
        Fork::Parent(_) => {
            // Deal with the config bits
            let (repo_dir, config) = autoconf();
            let cache: AspectCache = AspectCache::default()?;

            let threaded_rt = runtime::Runtime::new().into_diagnostic()?;
            threaded_rt.block_on(async {
                let cli_task = configure_tool_task(
                    cache.clone(),
                    repo_dir.clone(),
                    Box::new(config.tools.cli.clone()),
                )
                .await;
                let bazelisk_task = configure_tool_task(
                    cache.clone(),
                    repo_dir.clone(),
                    Box::new(config.tools.bazelisk.clone()),
                )
                .await;

                // Wait for fetches
                let cli = &config.tools.cli;
                if debug_mode() {
                    eprintln!("Attempting to provision {cli:?}");
                };

                let _: Result<()> = cli_task.await.into_diagnostic()?;

                let bazelisk = &config.tools.bazelisk;
                if debug_mode() {
                    eprintln!("Attempting to provision {bazelisk:?}");
                };
                let _: Result<()> = bazelisk_task.await.into_diagnostic()?;

                let path = cache.tool_path(&config.tools.cli);
                if debug_mode() {
                    eprintln!("Attempting to run {path:?}");
                };

                // Punt
                let mut cmd = UnixCommand::new(&path);
                if let Some(args) = matches.get_many::<String>("args") {
                    cmd.args(args);
                };
                let err = cmd.exec();
                Err::<(), _>(miette!(format!(
                    "Failed to punt to the `aspect-cli`, {:?}",
                    err
                )))
            })?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
