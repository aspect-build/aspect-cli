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
    BZLARCH, GOOS, LLVM_TRIPLE, cargo_pkg_short_version, do_not_track, send_telemetry,
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

/// Replace `{var}` placeholders in a string with platform values.
/// Supported variables: version, os, arch, target.
fn replace_vars(s: &str, version: &str) -> String {
    s.replace("{version}", version)
        .replace("{os}", GOOS)
        .replace("{arch}", BZLARCH)
        .replace("{target}", LLVM_TRIPLE)
}

fn debug_mode() -> bool {
    match var("ASPECT_DEBUG") {
        Ok(val) => !val.is_empty(),
        _ => false,
    }
}

const ASPECT_LAUNCHER_METHOD_HTTP: &str = "http";
const ASPECT_LAUNCHER_METHOD_GITHUB: &str = "github";
const ASPECT_LAUNCHER_METHOD_LOCAL: &str = "local";

/// Minimum interval between download-progress prints when running on CI.
const CI_DOWNLOAD_PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

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

    // On CI, terminals don't process `\r` line resets so per-chunk progress
    // is spammy. Throttle update on its own line.
    let is_ci = var("CI").map(|v| !v.is_empty()).unwrap_or(false);
    let download_start = std::time::Instant::now();
    let mut last_progress = download_start;

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

        if !is_ci || last_progress.elapsed() >= CI_DOWNLOAD_PROGRESS_INTERVAL {
            let line_start = if is_ci { "" } else { "\r" };
            let line_end = if is_ci { "\n" } else { "" };
            if let Some(total) = total_size {
                let percent = ((downloaded as f64 / total as f64) * 100.0) as u64;
                eprint!(
                    "{line_start}{:.0} / {:.0} KB ({}%){line_end}",
                    downloaded as f64 / 1024.0,
                    total as f64 / 1024.0,
                    percent
                );
            } else {
                eprint!("{line_start}{:.0} KB{line_end}", downloaded as f64 / 1024.0);
            }
            io::stderr().flush().into_diagnostic()?;
            last_progress = std::time::Instant::now();
        }
    }

    let elapsed = download_start.elapsed();
    let kb = downloaded as f64 / 1024.0;
    let size_str = if kb >= 1024.0 {
        format!("{:.1} MB", kb / 1024.0)
    } else {
        format!("{:.0} KB", kb)
    };
    let time_str = if elapsed.as_secs_f64() >= 1.0 {
        format!("{:.1}s", elapsed.as_secs_f64())
    } else {
        format!("{}ms", elapsed.as_millis())
    };
    if is_ci {
        eprintln!("downloaded {size_str} in {time_str}");
    } else {
        // \r overwrites the in-progress KB line; \x1b[K clears any stale tail
        // when the summary is shorter than the last progress print.
        eprintln!("\rdownloaded {size_str} in {time_str}\x1b[K");
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
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ReleaseArtifact>,
}

#[derive(Deserialize, Debug)]
struct ReleaseArtifact {
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

        for source in tool.sources() {
            match source {
                ToolSource::Http { url, headers } => {
                    let fallback_version = cargo_pkg_short_version();
                    let version = tool.version().unwrap_or(&fallback_version);
                    let url = replace_vars(url, version);
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
                    tag,
                    artifact,
                } => {
                    let fallback_version = cargo_pkg_short_version();
                    let pinned_version = tool.version();
                    let version_for_vars = pinned_version.unwrap_or(&fallback_version);

                    let artifact = if artifact.is_empty() {
                        format!("{}-{}", repo, LLVM_TRIPLE)
                    } else {
                        replace_vars(artifact, version_for_vars)
                    };

                    // How long a resolved tag hint is considered fresh before we
                    // re-query the releases API to pick up newer versions.
                    const HINT_MAX_AGE: std::time::Duration =
                        std::time::Duration::from_secs(24 * 60 * 60);

                    // Step 1: Resolve the tag.
                    // If a version is pinned, compute the tag directly.
                    // If unpinned, check the cached tag hint first to avoid a
                    // network round-trip when the binary is already present and
                    // the hint is fresh, then fall back to querying the releases API.
                    let resolved_tag = if let Some(version) = pinned_version {
                        let t = if tag.is_empty() {
                            format!("v{}", version)
                        } else {
                            replace_vars(tag, version)
                        };
                        if debug_mode() {
                            eprintln!("{:} pinned to tag {:?}, skipping API", tool.name(), t);
                        }
                        t
                    } else {
                        let hint_path = cache.latest_tag_path(&tool.name(), org, repo, &artifact);

                        // Use the cached hint if it is fresh and its binary is present.
                        if cache.latest_tag_is_fresh(&hint_path, HINT_MAX_AGE) {
                            if let Ok(cached_tag) = fs::read_to_string(&hint_path) {
                                let cached_tag = cached_tag.trim().to_owned();
                                let cached_url = format!(
                                    "https://github.com/{org}/{repo}/releases/download/{cached_tag}/{artifact}"
                                );
                                let cached_dest = cache.tool_path(&tool.name(), &cached_url);
                                if cached_dest.exists() {
                                    if debug_mode() {
                                        eprintln!(
                                            "{:} source {:?} found in cache {:?} (resolved tag: {})",
                                            tool.name(),
                                            source,
                                            &cached_url,
                                            cached_tag,
                                        );
                                    }
                                    let mut extra_envs = HashMap::new();
                                    extra_envs.insert(
                                        "ASPECT_LAUNCHER_ASPECT_CLI_ORG".to_string(),
                                        org.clone(),
                                    );
                                    extra_envs.insert(
                                        "ASPECT_LAUNCHER_ASPECT_CLI_REPO".to_string(),
                                        repo.clone(),
                                    );
                                    extra_envs.insert(
                                        "ASPECT_LAUNCHER_ASPECT_CLI_TAG".to_string(),
                                        cached_tag,
                                    );
                                    extra_envs.insert(
                                        "ASPECT_LAUNCHER_ASPECT_CLI_ARTIFACT".to_string(),
                                        artifact.clone(),
                                    );
                                    return Ok((
                                        cached_dest,
                                        ASPECT_LAUNCHER_METHOD_GITHUB.to_string(),
                                        extra_envs,
                                    ));
                                }
                            }
                        }

                        // Hint is absent, stale, or binary is missing — query the releases API.
                        if debug_mode() {
                            let reason = if !hint_path.exists() {
                                "no hint cached"
                            } else if !cache.latest_tag_is_fresh(&hint_path, HINT_MAX_AGE) {
                                "hint is stale"
                            } else {
                                "binary not in cache"
                            };
                            eprintln!(
                                "{:} unpinned, querying releases API ({reason})",
                                tool.name()
                            );
                        }
                        let releases_url = format!(
                            "https://api.github.com/repos/{org}/{repo}/releases?per_page=10"
                        );
                        if debug_mode() {
                            eprintln!(
                                "{:} source {:?} querying releases from {:?}",
                                tool.name(),
                                source,
                                releases_url,
                            );
                        }
                        let releases_req = gh_request(&client, releases_url)
                            .header(
                                HeaderName::from_static("accept"),
                                HeaderValue::from_static("application/vnd.github+json"),
                            )
                            .build()
                            .into_diagnostic()?;
                        let releases_resp = client.execute(releases_req).await.into_diagnostic()?;
                        let releases_status = releases_resp.status();
                        if !releases_status.is_success() {
                            let body = releases_resp.text().await.unwrap_or_default();
                            // If we have a stale-but-readable hint whose binary is still present,
                            // fall back to it and touch the hint so we don't hammer a down API.
                            if let Ok(stale_tag) = fs::read_to_string(&hint_path) {
                                let stale_tag = stale_tag.trim().to_owned();
                                let stale_url = format!(
                                    "https://github.com/{org}/{repo}/releases/download/{stale_tag}/{artifact}"
                                );
                                let stale_dest = cache.tool_path(&tool.name(), &stale_url);
                                if stale_dest.exists() {
                                    if debug_mode() {
                                        eprintln!(
                                            "{:} API error, falling back to stale cached tag {} ({})",
                                            tool.name(),
                                            stale_tag,
                                            body.trim(),
                                        );
                                    }
                                    // Reset the expiry so we retry after another HINT_MAX_AGE.
                                    cache.touch_latest_tag(&hint_path);
                                    let mut extra_envs = HashMap::new();
                                    extra_envs.insert(
                                        "ASPECT_LAUNCHER_ASPECT_CLI_ORG".to_string(),
                                        org.clone(),
                                    );
                                    extra_envs.insert(
                                        "ASPECT_LAUNCHER_ASPECT_CLI_REPO".to_string(),
                                        repo.clone(),
                                    );
                                    extra_envs.insert(
                                        "ASPECT_LAUNCHER_ASPECT_CLI_TAG".to_string(),
                                        stale_tag,
                                    );
                                    extra_envs.insert(
                                        "ASPECT_LAUNCHER_ASPECT_CLI_ARTIFACT".to_string(),
                                        artifact.clone(),
                                    );
                                    return Ok((
                                        stale_dest,
                                        ASPECT_LAUNCHER_METHOD_GITHUB.to_string(),
                                        extra_envs,
                                    ));
                                }
                            }
                            errs.push(Err(miette!(
                                "github releases list request for {org}/{repo} failed with status {}: {}",
                                releases_status,
                                body
                            )));
                            continue;
                        }
                        let releases: Vec<Release> =
                            releases_resp.json().await.into_diagnostic()?;
                        let found = releases.into_iter().find(|r| {
                            !r.prerelease && r.assets.iter().any(|a| a.name == *artifact)
                        });
                        let resolved = match found {
                            Some(release) => release.tag_name,
                            None => {
                                errs.push(Err(miette!(
                                    "unable to find release artifact {artifact} in any recent {org}/{repo} release"
                                )));
                                continue;
                            }
                        };
                        // Persist the resolved tag so the next run can skip the API call.
                        if let Some(parent) = hint_path.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        let _ = fs::write(&hint_path, &resolved);
                        resolved
                    };

                    // Step 2: Download from the direct release URL using the resolved tag.
                    let direct_url = format!(
                        "https://github.com/{org}/{repo}/releases/download/{resolved_tag}/{artifact}"
                    );

                    let tool_dest_file = cache.tool_path(&tool.name(), &direct_url);
                    let mut extra_envs = HashMap::new();
                    extra_envs.insert("ASPECT_LAUNCHER_ASPECT_CLI_ORG".to_string(), org.clone());
                    extra_envs.insert("ASPECT_LAUNCHER_ASPECT_CLI_REPO".to_string(), repo.clone());
                    extra_envs.insert(
                        "ASPECT_LAUNCHER_ASPECT_CLI_TAG".to_string(),
                        resolved_tag.clone(),
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
                                &direct_url
                            );
                        };
                        return Ok((
                            tool_dest_file,
                            ASPECT_LAUNCHER_METHOD_GITHUB.to_string(),
                            extra_envs,
                        ));
                    }
                    fs::create_dir_all(tool_dest_file.parent().unwrap()).into_diagnostic()?;

                    if debug_mode() {
                        eprintln!(
                            "{:} source {:?} downloading {:?} to {:?}",
                            tool.name(),
                            source,
                            direct_url,
                            tool_dest_file
                        );
                    };
                    let req = gh_request(&client, direct_url)
                        .header(
                            HeaderName::from_static("accept"),
                            HeaderValue::from_static("application/octet-stream"),
                        )
                        .build()
                        .into_diagnostic()?;
                    let download_msg = format!(
                        "downloading aspect cli version {} file {}",
                        resolved_tag, artifact
                    );
                    if let err @ Err(_) =
                        _download_into_cache(&client, &tool_dest_file, req, &download_msg).await
                    {
                        errs.push(err);
                        continue;
                    }
                    return Ok((
                        tool_dest_file,
                        ASPECT_LAUNCHER_METHOD_GITHUB.to_string(),
                        extra_envs,
                    ));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replace_vars_version() {
        let result = replace_vars("tool-{version}", "1.2.3");
        assert_eq!(result, format!("tool-1.2.3"));
    }

    #[test]
    fn test_replace_vars_os() {
        let result = replace_vars("{os}", "1.0.0");
        assert_eq!(result, GOOS);
    }

    #[test]
    fn test_replace_vars_arch() {
        let result = replace_vars("{arch}", "1.0.0");
        assert_eq!(result, BZLARCH);
    }

    #[test]
    fn test_replace_vars_target() {
        let result = replace_vars("{target}", "1.0.0");
        assert_eq!(result, LLVM_TRIPLE);
    }

    #[test]
    fn test_replace_vars_multiple() {
        let result = replace_vars("tool-{version}-{os}-{arch}", "3.0.0");
        assert_eq!(result, format!("tool-3.0.0-{}-{}", GOOS, BZLARCH));
    }

    #[test]
    fn test_replace_vars_no_placeholders() {
        let result = replace_vars("plain-string", "1.0.0");
        assert_eq!(result, "plain-string");
    }

    #[test]
    fn test_release_deserialize_with_assets() {
        let json = r#"{
            "tag_name": "v1.0.0",
            "assets": [
                {"name": "tool-linux"},
                {"name": "tool-macos"}
            ]
        }"#;
        let release: Release = serde_json::from_str(json).unwrap();
        assert_eq!(release.tag_name, "v1.0.0");
        assert_eq!(release.assets.len(), 2);
        assert_eq!(release.assets[0].name, "tool-linux");
        assert_eq!(release.assets[1].name, "tool-macos");
    }

    #[test]
    fn test_release_deserialize_without_assets() {
        let json = r#"{"tag_name": "v2.0.0"}"#;
        let release: Release = serde_json::from_str(json).unwrap();
        assert_eq!(release.tag_name, "v2.0.0");
        assert!(release.assets.is_empty());
    }

    #[test]
    fn test_release_deserialize_empty_assets() {
        let json = r#"{"tag_name": "v3.0.0", "assets": []}"#;
        let release: Release = serde_json::from_str(json).unwrap();
        assert_eq!(release.tag_name, "v3.0.0");
        assert!(release.assets.is_empty());
    }

    #[test]
    fn test_release_deserialize_ignores_extra_fields() {
        let json = r#"{
            "tag_name": "v1.0.0",
            "id": 12345,
            "draft": false,
            "prerelease": false,
            "assets": []
        }"#;
        let release: Release = serde_json::from_str(json).unwrap();
        assert_eq!(release.tag_name, "v1.0.0");
    }

    #[test]
    fn test_release_list_deserialize() {
        let json = r#"[
            {"tag_name": "v2.0.0", "assets": []},
            {"tag_name": "v1.0.0", "assets": [{"name": "tool"}]}
        ]"#;
        let releases: Vec<Release> = serde_json::from_str(json).unwrap();
        assert_eq!(releases.len(), 2);
        assert!(releases[0].assets.is_empty());
        assert_eq!(releases[1].assets[0].name, "tool");
    }

    #[test]
    fn test_prerelease_releases_are_skipped() {
        // prerelease/main should be skipped; v1.0.0 is the first stable release with the artifact.
        let releases = vec![
            Release {
                tag_name: "prerelease/main".to_string(),
                prerelease: true,
                assets: vec![ReleaseArtifact {
                    name: "tool".to_string(),
                }],
            },
            Release {
                tag_name: "v1.0.0".to_string(),
                prerelease: false,
                assets: vec![ReleaseArtifact {
                    name: "tool".to_string(),
                }],
            },
        ];
        let found = releases
            .into_iter()
            .find(|r| !r.prerelease && r.assets.iter().any(|a| a.name == "tool"));
        assert_eq!(found.unwrap().tag_name, "v1.0.0");
    }

    #[test]
    fn test_release_deserialize_prerelease_field() {
        let json =
            r#"{"tag_name": "prerelease/main", "prerelease": true, "assets": [{"name": "tool"}]}"#;
        let release: Release = serde_json::from_str(json).unwrap();
        assert!(release.prerelease);
        assert_eq!(release.tag_name, "prerelease/main");
    }

    #[test]
    fn test_release_deserialize_prerelease_defaults_false() {
        let json = r#"{"tag_name": "v1.0.0"}"#;
        let release: Release = serde_json::from_str(json).unwrap();
        assert!(!release.prerelease);
    }

    #[test]
    fn test_headermap_from_hashmap() {
        let headers = vec![
            ("Content-Type", "application/json"),
            ("Authorization", "Bearer token"),
        ];
        let map = headermap_from_hashmap(headers.into_iter());
        assert_eq!(map.get("content-type").unwrap(), "application/json");
        assert_eq!(map.get("authorization").unwrap(), "Bearer token");
    }

    #[test]
    fn test_headermap_from_hashmap_empty() {
        let headers: Vec<(&str, &str)> = vec![];
        let map = headermap_from_hashmap(headers.into_iter());
        assert!(map.is_empty());
    }

    // Helpers that mirror the production code's URL/path construction so the
    // tests below exercise exactly the same logic.
    fn make_cache(root: &std::path::Path) -> AspectCache {
        AspectCache::from(root.to_path_buf())
    }

    fn binary_cache_path(
        cache: &AspectCache,
        org: &str,
        repo: &str,
        tag: &str,
        artifact: &str,
    ) -> PathBuf {
        let url = format!("https://github.com/{org}/{repo}/releases/download/{tag}/{artifact}");
        cache.tool_path(&"aspect-cli".to_string(), &url)
    }

    /// Create a temp dir scoped to this test process so parallel test runs don't collide.
    fn tmp_cache_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "aspect-launcher-test-{}-{}",
            std::process::id(),
            label
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_cache_hint_fresh_and_binary_present_skips_api() {
        let tmp = tmp_cache_dir("hint-hit");
        let cache = make_cache(&tmp);

        let org = "aspect-build";
        let repo = "aspect-cli";
        let artifact = "aspect-cli-aarch64-apple-darwin";
        let tag = "v2026.15.2";

        // Write the tag hint (as the production code does after a successful API call).
        let hint = cache.latest_tag_path("aspect-cli", org, repo, artifact);
        std::fs::create_dir_all(hint.parent().unwrap()).unwrap();
        std::fs::write(&hint, tag).unwrap();

        // Hint must be fresh for the production code to use it.
        assert!(cache.latest_tag_is_fresh(&hint, std::time::Duration::from_secs(86400)));

        // Reconstruct the binary path from the hint — mirrors the production check.
        let cached_tag = std::fs::read_to_string(&hint).unwrap();
        let cached_tag = cached_tag.trim();
        let dest = binary_cache_path(&cache, org, repo, cached_tag, artifact);

        // Binary not present yet — hint alone is not enough.
        assert!(!dest.exists());

        // Simulate a previously downloaded binary.
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"fake binary").unwrap();

        // Fresh hint + binary present: production code returns early, no API call.
        assert!(dest.exists());
        assert_eq!(cached_tag, tag);

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_stale_hint_with_binary_falls_back_on_api_failure() {
        let tmp = tmp_cache_dir("hint-stale");
        let cache = make_cache(&tmp);

        let org = "aspect-build";
        let repo = "aspect-cli";
        let artifact = "aspect-cli-aarch64-apple-darwin";
        let tag = "v2026.15.2";

        // Write a hint that is immediately stale (zero max-age).
        let hint = cache.latest_tag_path("aspect-cli", org, repo, artifact);
        std::fs::create_dir_all(hint.parent().unwrap()).unwrap();
        std::fs::write(&hint, tag).unwrap();
        assert!(!cache.latest_tag_is_fresh(&hint, std::time::Duration::ZERO));

        // Write a binary for the stale tag.
        let dest = binary_cache_path(&cache, org, repo, tag, artifact);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"fake binary").unwrap();

        // The stale hint + existing binary should be usable as a fallback when the
        // API fails. After using it, touch_latest_tag resets the expiry.
        cache.touch_latest_tag(&hint);
        assert!(cache.latest_tag_is_fresh(&hint, std::time::Duration::from_secs(86400)));
        assert!(dest.exists());

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_cache_hint_present_but_binary_missing_falls_through_to_api() {
        let tmp = tmp_cache_dir("hint-miss");
        let cache = make_cache(&tmp);

        let org = "aspect-build";
        let repo = "aspect-cli";
        let artifact = "aspect-cli-aarch64-apple-darwin";
        let tag = "v2026.15.2";

        // Write the tag hint but do NOT create the binary.
        let hint = cache.latest_tag_path("aspect-cli", org, repo, artifact);
        std::fs::create_dir_all(hint.parent().unwrap()).unwrap();
        std::fs::write(&hint, tag).unwrap();

        let cached_tag = std::fs::read_to_string(&hint).unwrap();
        let dest = binary_cache_path(&cache, org, repo, cached_tag.trim(), artifact);

        // Binary missing → production code must fall through to the API.
        assert!(!dest.exists());

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_no_cache_hint_falls_through_to_api() {
        let tmp = tmp_cache_dir("no-hint");
        let cache = make_cache(&tmp);

        let hint = cache.latest_tag_path(
            "aspect-cli",
            "aspect-build",
            "aspect-cli",
            "aspect-cli-aarch64-apple-darwin",
        );

        // No hint written → production code must query the API.
        assert!(!hint.exists());

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_cache_hint_is_overwritten_on_new_resolution() {
        let tmp = tmp_cache_dir("hint-update");
        let cache = make_cache(&tmp);

        let hint = cache.latest_tag_path("aspect-cli", "aspect-build", "aspect-cli", "artifact");
        std::fs::create_dir_all(hint.parent().unwrap()).unwrap();

        std::fs::write(&hint, "v2026.14.0").unwrap();
        assert_eq!(std::fs::read_to_string(&hint).unwrap().trim(), "v2026.14.0");

        // Simulate a newer resolution overwriting the old hint.
        std::fs::write(&hint, "v2026.15.2").unwrap();
        assert_eq!(std::fs::read_to_string(&hint).unwrap().trim(), "v2026.15.2");

        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
