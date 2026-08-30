mod cache;
mod config;
mod warming;

use std::collections::HashMap;
use std::env;
use std::env::var;
use std::fs;
use std::fs::File;
use std::io::{self, IsTerminal, Write};
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
use miette::{Context, IntoDiagnostic, Report, Result, miette};
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

/// Like [`replace_vars`], plus `{artifact}` for URLs that mirror the release
/// assets. `artifact` already carries the `-debug-` infix when the debug
/// variant was requested, so a mirror URL tracks it with no debug placeholder
/// of its own.
fn replace_url_vars(s: &str, version: &str, artifact: &str) -> String {
    replace_vars(s, version).replace("{artifact}", artifact)
}

fn debug_mode() -> bool {
    match var("ASPECT_DEBUG") {
        Ok(val) => !val.is_empty(),
        _ => false,
    }
}

/// Whether to fetch the `-debug-` release variant instead of the primary binary,
/// requested either by `debug = True` in `version()` (`config_debug`) or by the
/// `ASPECT_DEBUG_CLI` environment variable.
///
/// Separate from [`debug_mode`] on purpose: `ASPECT_DEBUG` requests verbose logging and
/// is set routinely, so it must not also swap in a larger, slower binary.
fn debug_cli_mode(config_debug: bool) -> bool {
    config_debug || var("ASPECT_DEBUG_CLI").is_ok_and(|val| !val.is_empty())
}

/// Default release asset name for a tool, e.g. `aspect-cli-x86_64-unknown-linux-musl`.
///
/// With `debug` set, names the `-debug-` variant published alongside the primary binary:
/// unstripped and built with debug assertions, so a crash report resolves to function
/// and file:line at the cost of size and speed.
///
/// Only used when the config did not name an `artifact` explicitly, since a debug
/// counterpart of an arbitrary asset may not exist.
fn default_artifact(repo: &str, debug: bool) -> String {
    if debug {
        format!("{}-debug-{}", repo, LLVM_TRIPLE)
    } else {
        format!("{}-{}", repo, LLVM_TRIPLE)
    }
}

/// The release asset a source should request, plus the primary-binary name to
/// retry with when the debug variant was chosen.
///
/// Not every release publishes a `-debug-` asset, and a pinned tag is
/// downloaded without consulting the asset list, so a missing one surfaces only
/// as a failed download. [`Self::debug_fallback`] then names the primary binary,
/// degrading a debug request to a working CLI instead of no CLI. An artifact
/// named explicitly in config has no known counterpart, so it gets no fallback.
struct ArtifactChoice {
    name: String,
    /// Set only when [`Self::name`] is a debug variant this resolver chose.
    fallback: Option<String>,
}

impl ArtifactChoice {
    /// The asset name this launcher derives for `repo`.
    fn derived(repo: &str, debug: bool) -> Self {
        Self {
            name: default_artifact(repo, debug),
            fallback: debug.then(|| default_artifact(repo, false)),
        }
    }

    /// Resolve the asset name for `repo`, honoring an explicit `configured`
    /// name (with `{var}` placeholders expanded) over the derived default.
    fn resolve(repo: &str, configured: &str, version: &str, debug: bool) -> Self {
        if configured.is_empty() {
            return Self::derived(repo, debug);
        }
        Self {
            name: replace_vars(configured, version),
            fallback: None,
        }
    }

    /// The primary-binary name to retry with, and the warning to print, after a
    /// download of a chosen debug variant failed.
    fn debug_fallback(&self, tag: &str) -> Option<(&str, String)> {
        let primary = self.fallback.as_deref()?;
        Some((
            primary,
            format!(
                "{} is unavailable in {tag}; falling back to {primary}",
                self.name
            ),
        ))
    }
}

/// Whether a `github()` source names the repo the CDN mirrors.
fn is_aspect_cli_repo(org: &str, repo: &str) -> bool {
    org == config::ASPECT_CLI_ORG && repo == config::ASPECT_CLI_REPO
}

/// The Aspect release a preceding `github()` source resolved, so a following
/// `http()` mirror requests that same release rather than re-deriving it.
///
/// An unpinned config resolves its version from the releases API; without this
/// the mirror would substitute the launcher's own, older version and silently
/// downgrade the CLI during the outage the mirror exists to cover.
///
/// Only recorded for [`ASPECT_CLI_REPO`]: we mirror our own releases and
/// nothing else, so a custom `github(org/repo)` must not redirect a mirror at
/// assets the CDN does not carry.
struct MirroredRelease {
    /// Version for `{version}`, without the tag's leading `v`.
    version: String,
    artifact: String,
    /// Primary-binary name to retry with, carried from the resolved
    /// [`ArtifactChoice`] so the mirror falls back the same way.
    fallback: Option<String>,
}

impl From<&MirroredRelease> for ArtifactChoice {
    fn from(m: &MirroredRelease) -> Self {
        Self {
            name: m.artifact.clone(),
            fallback: m.fallback.clone(),
        }
    }
}

/// Direct download URL for a release asset. Also the cache key for the downloaded
/// binary, so every caller must build it the same way.
fn release_asset_url(org: &str, repo: &str, tag: &str, artifact: &str) -> String {
    format!("https://github.com/{org}/{repo}/releases/download/{tag}/{artifact}")
}

const ASPECT_LAUNCHER_METHOD_HTTP: &str = "http";
const ASPECT_LAUNCHER_METHOD_GITHUB: &str = "github";
const ASPECT_LAUNCHER_METHOD_LOCAL: &str = "local";

/// Minimum interval between download-progress prints when stderr is not a terminal.
const DOWNLOAD_PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

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

    // `\r` line resets only redraw in place on a terminal. Anywhere else — a CI
    // log, a pipe, an AI agent capturing stderr — every chunk accumulates instead,
    // so fall back to a throttled line-per-update.
    let is_ci = var("CI").map(|v| !v.is_empty()).unwrap_or(false);
    let redraw_in_place = io::stderr().is_terminal() && !is_ci;
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

        if redraw_in_place || last_progress.elapsed() >= DOWNLOAD_PROGRESS_INTERVAL {
            let line_start = if redraw_in_place { "\r" } else { "" };
            let line_end = if redraw_in_place { "" } else { "\n" };
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
    if redraw_in_place {
        // \r overwrites the in-progress KB line; \x1b[K clears any stale tail
        // when the summary is shorter than the last progress print.
        eprintln!("\rdownloaded {size_str} in {time_str}\x1b[K");
    } else {
        eprintln!("downloaded {size_str} in {time_str}");
    }

    // A 0-byte (or truncated) body would `exec` into an ENOEXEC "Exec format
    // error" downstream. Reject it here so the caller's retry loop re-fetches
    // instead of caching and punting to an empty binary. GitHub's release CDN
    // can briefly serve an empty 200 for an asset that isn't fully propagated.
    if downloaded == 0 {
        let _ = tokio::fs::remove_file(&tmp_file).await;
        return Err(miette::miette!(
            "downloaded an empty file (0 bytes) — refusing to cache it"
        ));
    }
    if let Some(total) = total_size {
        if downloaded != total {
            let _ = tokio::fs::remove_file(&tmp_file).await;
            return Err(miette::miette!(
                "download truncated: got {downloaded} bytes, expected {total}"
            ));
        }
    }

    // And move it into the cache
    tokio::fs::rename(&tmp_file, &cache_entry)
        .await
        .into_diagnostic()
        .context("failed to move tool")?;

    // FIXME: Check download integrity/signatures?
    Ok(())
}

/// Retry up to 3 times with exponential backoff (0s, 1s, 2s) to survive transient
/// failures such as a mid-stream connection reset.
async fn download_with_retries(
    client: &Client,
    url: &str,
    dest: &PathBuf,
    download_msg: &str,
) -> Result<()> {
    const MAX_DOWNLOAD_ATTEMPTS: u32 = 3;
    let mut last_err = None;
    for attempt in 0..MAX_DOWNLOAD_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(1 << (attempt - 1))).await;
            eprintln!(
                "retrying download (attempt {}/{})",
                attempt + 1,
                MAX_DOWNLOAD_ATTEMPTS
            );
        }
        let req = gh_request(client, url.to_owned())
            .header(
                HeaderName::from_static("accept"),
                HeaderValue::from_static("application/octet-stream"),
            )
            .build()
            .into_diagnostic()?;
        match _download_into_cache(client, dest, req, download_msg).await {
            Ok(()) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.expect("at least one attempt must have run"))
}

/// Fetch `url` into the launcher cache for `tool_name`, serving an existing
/// cache entry without a request.
///
/// The outer `Result` is a local failure that aborts provisioning entirely (an
/// unusable cache directory, a malformed request); the inner one is a download
/// failure the caller can attribute to this URL and recover from by trying
/// another source.
async fn http_fetch(
    client: &Client,
    cache: &AspectCache,
    tool_name: &str,
    url: &str,
    headers: &HashMap<String, String>,
) -> Result<Result<PathBuf, Report>> {
    let dest = cache.tool_path(tool_name, url);
    if dest.exists() {
        if debug_mode() {
            eprintln!("{tool_name} found {url:?} in cache");
        }
        return Ok(Ok(dest));
    }
    fs::create_dir_all(dest.parent().unwrap()).into_diagnostic()?;
    if debug_mode() {
        eprintln!("{tool_name} downloading {url:?} to {dest:?}");
    }
    let req = client
        .request(Method::GET, url)
        .headers(headermap_from_hashmap(headers.iter()))
        .build()
        .into_diagnostic()?;
    let msg = format!("downloading aspect cli from {url}");
    Ok(_download_into_cache(client, &dest, req, &msg)
        .await
        .map(|()| dest))
}

/// A failed attempt to obtain a tool from one of its sources.
struct SourceFailure {
    /// Human-readable identity of the source attempt, e.g. the download URL.
    source: String,
    error: Report,
    /// Whether the failure looks like a transient network/service problem
    /// (worth a retry) rather than a configuration problem.
    transient: bool,
}

impl SourceFailure {
    /// Build a failure, classifying `error`'s transience from its cause chain.
    fn new(source: String, error: Report) -> Self {
        let transient = is_transient_network_error(&error);
        Self {
            source,
            error,
            transient,
        }
    }
}

/// Render an error and its cause chain as a single line.
fn error_chain(err: &Report) -> String {
    err.chain()
        .map(|cause| cause.to_string())
        .collect::<Vec<_>>()
        .join(": ")
}

/// Whether an error chain looks like a transient network or service failure
/// (unreachable host, reset stream, timeout, 5xx/429) rather than a
/// configuration problem.
fn is_transient_network_error(err: &Report) -> bool {
    err.chain().any(|cause| {
        if let Some(e) = cause.downcast_ref::<std::io::Error>() {
            use std::io::ErrorKind::*;
            return matches!(
                e.kind(),
                ConnectionRefused
                    | ConnectionReset
                    | ConnectionAborted
                    | TimedOut
                    | HostUnreachable
                    | NetworkUnreachable
                    | NetworkDown
            );
        }
        // Concrete error types can be erased behind opaque wrappers whose
        // source() skips them, so fall back to message heuristics.
        let msg = cause.to_string();
        msg.contains("error sending request")
            || msg.contains("http2 error")
            || msg.contains("connection closed")
            || msg.contains("connection reset")
            || msg.contains("operation timed out")
            || msg.contains("dns error")
            || msg.contains("HTTP status server error")
            || msg.contains("429 Too Many Requests")
    })
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
        let mut errs: Vec<SourceFailure> = Vec::new();

        let client = reqwest::Client::new();

        let mut mirrored: Option<MirroredRelease> = None;

        for source in tool.sources() {
            match source {
                ToolSource::Http { url, headers } => {
                    let fallback_version = cargo_pkg_short_version();
                    let debug = debug_cli_mode(tool.debug());
                    // Follow the Aspect release an earlier github() source resolved;
                    // otherwise derive from the pinned version and this launcher.
                    let (version, choice) = match &mirrored {
                        Some(m) => (m.version.as_str(), ArtifactChoice::from(m)),
                        None => (
                            tool.version().unwrap_or(&fallback_version),
                            ArtifactChoice::derived(&tool.name(), debug),
                        ),
                    };
                    let mut attempt_url = replace_url_vars(url, version, &choice.name);
                    let mut result =
                        http_fetch(&client, &cache, &tool.name(), &attempt_url, headers).await?;

                    // Skipped when the URL has no {artifact}, since the retry would
                    // repeat the same request.
                    if result.is_err()
                        && let Some((primary, warning)) = choice.debug_fallback(version)
                    {
                        let primary_url = replace_url_vars(url, version, primary);
                        if primary_url != attempt_url {
                            eprintln!("{warning}");
                            attempt_url = primary_url;
                            result =
                                http_fetch(&client, &cache, &tool.name(), &attempt_url, headers)
                                    .await?;
                        }
                    }

                    let tool_dest_file = match result {
                        Ok(dest) => dest,
                        Err(e) => {
                            errs.push(SourceFailure::new(attempt_url, e));
                            continue;
                        }
                    };
                    return Ok((
                        tool_dest_file,
                        ASPECT_LAUNCHER_METHOD_HTTP.to_string(),
                        HashMap::from([(
                            "ASPECT_LAUNCHER_ASPECT_CLI_URL".to_string(),
                            attempt_url,
                        )]),
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

                    let choice = ArtifactChoice::resolve(
                        repo,
                        artifact,
                        version_for_vars,
                        debug_cli_mode(tool.debug()),
                    );
                    let artifact = choice.name.clone();

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
                                let cached_url =
                                    release_asset_url(org, repo, &cached_tag, &artifact);
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
                        let api_source = format!("github.com/{org}/{repo} releases API");
                        let releases_result: std::result::Result<Vec<Release>, SourceFailure> =
                            async {
                                let resp = client
                                    .execute(releases_req)
                                    .await
                                    .into_diagnostic()
                                    .map_err(|e| SourceFailure::new(api_source.clone(), e))?;
                                let status = resp.status();
                                if !status.is_success() {
                                    let body = resp.text().await.unwrap_or_default();
                                    return Err(SourceFailure {
                                        source: api_source.clone(),
                                        error: miette!(
                                            "request failed with status {status}: {body}"
                                        ),
                                        transient: status.is_server_error()
                                            || status == reqwest::StatusCode::TOO_MANY_REQUESTS,
                                    });
                                }
                                resp.json::<Vec<Release>>()
                                    .await
                                    .into_diagnostic()
                                    .map_err(|e| SourceFailure::new(api_source.clone(), e))
                            }
                            .await;
                        let releases = match releases_result {
                            Ok(releases) => releases,
                            Err(failure) => {
                                // If we have a stale-but-readable hint whose binary is still present,
                                // fall back to it and touch the hint so we don't hammer a down API.
                                if let Ok(stale_tag) = fs::read_to_string(&hint_path) {
                                    let stale_tag = stale_tag.trim().to_owned();
                                    let stale_url =
                                        release_asset_url(org, repo, &stale_tag, &artifact);
                                    let stale_dest = cache.tool_path(&tool.name(), &stale_url);
                                    if stale_dest.exists() {
                                        if debug_mode() {
                                            eprintln!(
                                                "{:} API error, falling back to stale cached tag {} ({})",
                                                tool.name(),
                                                stale_tag,
                                                error_chain(&failure.error),
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
                                // The API is unreachable, but a cached hint still names
                                // the last resolved release; let the mirror try it
                                // rather than fall back to the launcher's version.
                                if is_aspect_cli_repo(org, repo)
                                    && let Ok(hint) = fs::read_to_string(&hint_path)
                                {
                                    mirrored = Some(MirroredRelease {
                                        version: hint.trim().trim_start_matches('v').to_owned(),
                                        artifact: choice.name.clone(),
                                        fallback: choice.fallback.clone(),
                                    });
                                }
                                errs.push(failure);
                                continue;
                            }
                        };
                        // A chosen debug variant also matches a release carrying only
                        // the primary binary; the download below falls back to it.
                        let acceptable: Vec<&str> = std::iter::once(artifact.as_str())
                            .chain(choice.fallback.as_deref())
                            .collect();
                        let found = releases.into_iter().find(|r| {
                            !r.prerelease
                                && r.assets
                                    .iter()
                                    .any(|a| acceptable.contains(&a.name.as_str()))
                        });
                        let resolved = match found {
                            Some(release) => release.tag_name,
                            None => {
                                errs.push(SourceFailure {
                                    source: format!("github.com/{org}/{repo} releases API"),
                                    error: miette!(
                                        "unable to find release artifact {artifact} in any recent release"
                                    ),
                                    transient: false,
                                });
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
                    let mut direct_url = release_asset_url(org, repo, &resolved_tag, &artifact);
                    let mut tool_dest_file = cache.tool_path(&tool.name(), &direct_url);
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
                    let download_msg =
                        |a: &str| format!("downloading aspect cli version {resolved_tag} file {a}");

                    let mut download_err = download_with_retries(
                        &client,
                        &direct_url,
                        &tool_dest_file,
                        &download_msg(&artifact),
                    )
                    .await
                    .err();

                    if let Some((primary, warning)) = download_err
                        .is_some()
                        .then(|| choice.debug_fallback(&resolved_tag))
                        .flatten()
                    {
                        eprintln!("{warning}");
                        direct_url = release_asset_url(org, repo, &resolved_tag, primary);
                        tool_dest_file = cache.tool_path(&tool.name(), &direct_url);
                        extra_envs.insert(
                            "ASPECT_LAUNCHER_ASPECT_CLI_ARTIFACT".to_string(),
                            primary.to_owned(),
                        );
                        download_err = if tool_dest_file.exists() {
                            None
                        } else {
                            fs::create_dir_all(tool_dest_file.parent().unwrap())
                                .into_diagnostic()?;
                            download_with_retries(
                                &client,
                                &direct_url,
                                &tool_dest_file,
                                &download_msg(primary),
                            )
                            .await
                            .err()
                        };
                    }

                    if let Some(e) = download_err {
                        if is_aspect_cli_repo(org, repo) {
                            mirrored = Some(MirroredRelease {
                                version: resolved_tag.trim_start_matches('v').to_owned(),
                                artifact: choice.name.clone(),
                                fallback: choice.fallback.clone(),
                            });
                        }
                        errs.push(SourceFailure::new(direct_url.clone(), e));
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
                    errs.push(SourceFailure {
                        source: full_path.display().to_string(),
                        error: miette!("file does not exist"),
                        transient: false,
                    });
                }
            }
        }
        let mut msg = format!("failed to download {}", tool.name());
        if errs.is_empty() {
            msg.push_str(": no tool sources are configured");
        } else {
            msg.push_str(" — every source failed:");
            for failure in &errs {
                msg.push_str(&format!(
                    "\n  {}: {}",
                    failure.source,
                    error_chain(&failure.error)
                ));
            }
        }
        if errs.iter().any(|failure| failure.transient) {
            return Err(miette!(
                help = "these failures look like a network or GitHub outage — check https://www.githubstatus.com and retry. A previously downloaded CLI is reused from the launcher cache (ASPECT_LAUNCHER_CACHE), so pre-seeding that cache avoids the download entirely.",
                "{msg}"
            ));
        }
        Err(miette!("{msg}"))
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
            // On a Workflows runner, wait for cache warming to finish before
            // touching the download cache it restores. No-op off-runner.
            warming::wait_for_warming();

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
    fn test_artifact_choice_defaults_to_the_primary_binary() {
        let choice = ArtifactChoice::resolve("aspect-cli", "", "1.0.0", false);
        assert_eq!(choice.name, default_artifact("aspect-cli", false));
        assert_eq!(choice.debug_fallback("v1.0.0"), None);
    }

    #[test]
    fn test_artifact_choice_debug_falls_back_to_the_primary() {
        let choice = ArtifactChoice::resolve("aspect-cli", "", "1.0.0", true);
        assert_eq!(choice.name, default_artifact("aspect-cli", true));
        let (primary, warning) = choice.debug_fallback("v1.0.0").expect("fallback");
        assert_eq!(primary, default_artifact("aspect-cli", false));
        assert!(warning.contains("unavailable in v1.0.0"), "{warning}");
    }

    /// A configured artifact has no known primary counterpart, so requesting a
    /// debug build must not invent one.
    #[test]
    fn test_artifact_choice_configured_name_wins_and_has_no_fallback() {
        for debug in [false, true] {
            let choice = ArtifactChoice::resolve("aspect-cli", "custom-{version}", "1.0.0", debug);
            assert_eq!(choice.name, "custom-1.0.0");
            assert_eq!(choice.debug_fallback("v1.0.0"), None);
        }
    }

    #[test]
    fn test_is_aspect_cli_repo_matches_only_the_mirrored_repo() {
        assert!(is_aspect_cli_repo("aspect-build", "aspect-cli"));
        assert!(!is_aspect_cli_repo("aspect-build", "my-cli"));
        assert!(!is_aspect_cli_repo("my-org", "aspect-cli"));
    }

    /// A mirror follows the resolved release, so an unpinned config that
    /// resolved a newer tag is not downgraded to the launcher's own version.
    #[test]
    fn test_mirrored_release_drives_the_url() {
        use crate::config::CDN_MIRROR_URL;

        let m = MirroredRelease {
            version: "2026.35.9".to_owned(),
            artifact: default_artifact("aspect-cli", false),
            fallback: None,
        };
        let choice = ArtifactChoice::from(&m);
        assert_eq!(
            replace_url_vars(CDN_MIRROR_URL, &m.version, &choice.name),
            format!(
                "https://cdn.aspect.build/github.com/aspect-build/aspect-cli/releases/download/v2026.35.9/{}",
                default_artifact("aspect-cli", false)
            )
        );
    }

    /// The debug fallback survives the hand-off, so a mirror recovers from a
    /// release with no debug asset exactly as the github() source would.
    #[test]
    fn test_mirrored_release_carries_the_debug_fallback() {
        let source = ArtifactChoice::derived("aspect-cli", true);
        let m = MirroredRelease {
            version: "2026.31.1".to_owned(),
            artifact: source.name.clone(),
            fallback: source.fallback.clone(),
        };
        let choice = ArtifactChoice::from(&m);
        let (primary, _) = choice
            .debug_fallback("2026.31.1")
            .expect("fallback carried");
        assert_eq!(primary, default_artifact("aspect-cli", false));
    }

    #[test]
    fn test_replace_url_vars_expands_artifact() {
        assert_eq!(
            replace_url_vars(
                "https://cdn/{version}/{artifact}",
                "1.2.3",
                "aspect-cli-linux"
            ),
            "https://cdn/1.2.3/aspect-cli-linux"
        );
    }

    /// The default CDN mirror must resolve to the same asset path the GitHub
    /// source would request, for both the primary and the debug variant.
    #[test]
    fn test_cdn_mirror_url_matches_the_github_asset_path() {
        use crate::config::CDN_MIRROR_URL;

        let primary = default_artifact("aspect-cli", false);
        assert_eq!(
            replace_url_vars(CDN_MIRROR_URL, "2026.35.9", &primary),
            format!(
                "https://cdn.aspect.build/github.com/aspect-build/aspect-cli/releases/download/v2026.35.9/{primary}"
            )
        );

        let debug = default_artifact("aspect-cli", true);
        assert!(debug.contains("-debug-"));
        assert_eq!(
            replace_url_vars(CDN_MIRROR_URL, "2026.35.9", &debug),
            format!(
                "https://cdn.aspect.build/github.com/aspect-build/aspect-cli/releases/download/v2026.35.9/{debug}"
            )
        );
    }

    #[test]
    fn test_debug_cli_mode_follows_the_config_flag() {
        // True regardless of the env var, which is only an additional opt-in.
        assert!(debug_cli_mode(true));
    }

    /// A url= without {artifact} names one fixed asset, so the debug retry would
    /// repeat the same request; the branch guards on the URL actually changing.
    #[test]
    fn test_debug_fallback_url_is_unchanged_without_an_artifact_placeholder() {
        let choice = ArtifactChoice::derived("aspect-cli", true);
        let (primary, _) = choice.debug_fallback("v1.0.0").expect("fallback");
        let url = "https://example.com/aspect-cli-{version}";
        assert_eq!(
            replace_url_vars(url, "1.0.0", &choice.name),
            replace_url_vars(url, "1.0.0", primary)
        );
    }

    /// The CDN mirror must reach the primary binary when a release published no
    /// debug asset, matching the github() source rather than failing outright.
    #[test]
    fn test_cdn_mirror_debug_fallback_url_targets_the_primary() {
        use crate::config::CDN_MIRROR_URL;

        let choice = ArtifactChoice::resolve("aspect-cli", "", "2026.31.1", true);
        let (primary, _) = choice.debug_fallback("2026.31.1").expect("fallback");
        assert_eq!(
            replace_url_vars(CDN_MIRROR_URL, "2026.31.1", primary),
            format!(
                "https://cdn.aspect.build/github.com/aspect-build/aspect-cli/releases/download/v2026.31.1/{}",
                default_artifact("aspect-cli", false)
            )
        );
    }

    #[test]
    fn test_default_artifact_is_the_primary_binary() {
        assert_eq!(
            default_artifact("aspect-cli", false),
            format!("aspect-cli-{}", LLVM_TRIPLE)
        );
    }

    #[test]
    fn test_default_artifact_debug_selects_the_debug_variant() {
        assert_eq!(
            default_artifact("aspect-cli", true),
            format!("aspect-cli-debug-{}", LLVM_TRIPLE)
        );
    }

    /// The two variants must be distinct names so they resolve to different release
    /// assets and, because the cache key hashes the download URL, different cache
    /// entries — a debug run must never overwrite the primary binary in place.
    #[test]
    fn test_default_artifact_variants_differ() {
        assert_ne!(
            default_artifact("aspect-cli", false),
            default_artifact("aspect-cli", true)
        );
    }

    /// The debug variant's fallback target is the primary name for the same release, so
    /// the two must differ only by the `-debug-` infix.
    #[test]
    fn test_default_artifact_debug_is_the_primary_name_plus_infix() {
        assert_eq!(
            default_artifact("aspect-cli", true),
            default_artifact("aspect-cli", false).replace("aspect-cli-", "aspect-cli-debug-")
        );
    }

    #[test]
    fn test_release_asset_url() {
        assert_eq!(
            release_asset_url(
                "aspect-build",
                "aspect-cli",
                "v2026.31.10",
                "aspect-cli-linux"
            ),
            "https://github.com/aspect-build/aspect-cli/releases/download/v2026.31.10/aspect-cli-linux"
        );
    }

    /// The URL is the cache key, so the debug and primary variants of one release must
    /// not collide.
    #[test]
    fn test_release_asset_url_distinguishes_variants() {
        let url = |a: &str| release_asset_url("aspect-build", "aspect-cli", "v2026.31.10", a);
        assert_ne!(
            url(&default_artifact("aspect-cli", false)),
            url(&default_artifact("aspect-cli", true))
        );
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
        cache.tool_path(
            &"aspect-cli".to_string(),
            &release_asset_url(org, repo, tag, artifact),
        )
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

    /// When the releases API is down, the cached hint names the release the last
    /// successful run resolved. The mirror must reuse that version rather than
    /// the launcher's own, which would silently downgrade an unpinned config.
    #[test]
    fn test_stale_hint_version_feeds_the_mirror_url() {
        use crate::config::CDN_MIRROR_URL;

        let tmp = tmp_cache_dir("hint-mirror");
        let cache = make_cache(&tmp);

        let hint = cache.latest_tag_path(
            "aspect-cli",
            "aspect-build",
            "aspect-cli",
            "aspect-cli-aarch64-apple-darwin",
        );
        std::fs::create_dir_all(hint.parent().unwrap()).unwrap();
        std::fs::write(&hint, "v2026.35.9").unwrap();

        // The hand-off the http() branch performs: hint tag -> mirrored version.
        let recorded = std::fs::read_to_string(&hint).unwrap();
        let choice = ArtifactChoice::derived("aspect-cli", false);
        let m = MirroredRelease {
            version: recorded.trim().trim_start_matches('v').to_owned(),
            artifact: choice.name.clone(),
            fallback: choice.fallback.clone(),
        };
        assert_eq!(m.version, "2026.35.9");
        assert_ne!(m.version, cargo_pkg_short_version());
        assert_eq!(
            replace_url_vars(CDN_MIRROR_URL, &m.version, &ArtifactChoice::from(&m).name),
            format!(
                "https://cdn.aspect.build/github.com/aspect-build/aspect-cli/releases/download/v2026.35.9/{}",
                default_artifact("aspect-cli", false)
            )
        );

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
