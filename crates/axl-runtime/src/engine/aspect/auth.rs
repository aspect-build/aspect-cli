use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use allocative::Allocative;
use derive_more::Display;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use starlark::StarlarkResultExt;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value_as_type::StarlarkValueAsType;
use starlark::values::{self, NoSerialize, ProvidesStaticType, ValueLike, starlark_value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::runtime::Handle;

use super::credential_store::CredentialStore;

/// A deployment the CLI can authenticate against: the built-in Aspect
/// deployment or a configured self-hosted one. `configure` discovers these from a
/// deployment endpoint's `/.well-known/oauth-protected-resource` document and
/// records them in `config.json`; `login` mints a token against the selected one.
///
/// `issuer`/`client_id` are the OAuth config the CLI mints a bearer against
/// (interactive PKCE or API-token exchange); absent only when the endpoint
/// advertises no authorization server. `hosts` are the Bazel-facing endpoints
/// (remote cache, BES) whose traffic receives this deployment's login JWT.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Deployment {
    name: String,
    #[serde(default, skip_serializing_if = "is_false")]
    default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_id: Option<String>,
    /// Aspect-cloud API base (`ctx.aspect.auth.api_url`), used by Aspect-cloud
    /// features (GitHub/GitLab token exchange, status comments, budget). Present
    /// only for the built-in Aspect account; self-hosted deployments don't run
    /// these endpoints, so it is absent for configured deployments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    api_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hosts: Vec<String>,
    /// OAuth scopes the login flow requests, from the deployment's advertised
    /// `scopes_supported` — overridable per deployment for a bring-your-own IdP
    /// that doesn't accept the standard OIDC set. Empty falls back to
    /// [`DEFAULT_LOGIN_SCOPES`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    scopes: Vec<String>,
    /// The capability → host map the deployment advertised (`cache`, `bes`, and
    /// `exec` when it serves remote execution), so `aspect <build|test>
    /// --deployment <name>` wires each to the right Bazel flag. Absent for the
    /// built-in Aspect seed (it advertises none yet) and for a hand-edited
    /// `config.json` entry that omits them — `configure` requires an advertised
    /// `aspect_endpoints` map, so a discovered deployment always has this.
    #[serde(default, skip_serializing_if = "Endpoints::is_empty")]
    endpoints: Endpoints,
}

/// The OAuth scopes the login flow requests when the deployment advertises none.
/// `offline_access` requests a refresh token so the credential renews without
/// re-login.
const DEFAULT_LOGIN_SCOPES: &[&str] = &["openid", "profile", "email", "offline_access"];

/// The Bazel-facing endpoints a deployment serves, keyed by capability. `cache`
/// is the remote cache (`--remote_cache`), `bes` the build event stream, `exec`
/// the remote executor (`--remote_executor`), present only when the deployment
/// serves remote execution. Any field may be empty (the capability isn't served
/// or predates endpoint advertisement).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct Endpoints {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    cache: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    bes: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    exec: String,
}

impl Endpoints {
    fn is_empty(&self) -> bool {
        self.cache.is_empty() && self.bes.is_empty() && self.exec.is_empty()
    }
}

fn is_false(b: &bool) -> bool {
    !b
}

/// Resolved issuer + client_id for the PKCE / api-token / refresh flows,
/// produced from the selected [`Deployment`]. A deployment that advertised no
/// authorization server has neither, so resolving it for a login flow is an error.
#[derive(Debug, Clone)]
struct AuthEnv {
    domain: String,
    client_id: String,
    /// OAuth scopes to request at login (the deployment's advertised
    /// `scopes_supported`, or [`DEFAULT_LOGIN_SCOPES`] when it advertised none).
    scopes: Vec<String>,
}

/// The built-in Aspect production deployment, seeded so a fresh install can
/// `aspect auth login` with no `configure` step. It backs Aspect Cloud services
/// (not a Bazel remote cache/BES/exec), so it owns no endpoint hosts and `hosts`
/// is left empty. `config.json` entries add to — and by `name` override — this seed.
const DEFAULT_DEPLOYMENT_NAME: &str = "aspect";
const DEFAULT_ISSUER: &str = "https://auth.aspect.build";
const DEFAULT_CLIENT_ID: &str = "771ff228-18a1-43f0-bc83-62c9df0d72ca";
const DEFAULT_API_URL: &str = "https://api.aspect.build";

fn default_deployment() -> Deployment {
    Deployment {
        name: DEFAULT_DEPLOYMENT_NAME.to_string(),
        default: true,
        issuer: Some(DEFAULT_ISSUER.to_string()),
        client_id: Some(DEFAULT_CLIENT_ID.to_string()),
        api_url: Some(DEFAULT_API_URL.to_string()),
        hosts: Vec::new(),
        // The Aspect cloud reissues access tokens without a refresh token, so the
        // built-in flow requests the base OIDC scopes (no offline_access).
        scopes: ["openid", "profile", "email"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        endpoints: Endpoints::default(),
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DeploymentsConfig {
    #[serde(default)]
    deployments: Vec<Deployment>,
}

fn config_path() -> anyhow::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("unable to determine home directory"))?;
    Ok(home.join(".aspect").join("config.json"))
}

fn load_config_file(path: &PathBuf) -> anyhow::Result<Vec<Deployment>> {
    match fs::read_to_string(path) {
        Ok(content) => {
            let cfg: DeploymentsConfig = serde_json::from_str(&content)
                .map_err(|e| anyhow::anyhow!("failed to parse {}: {}", path.display(), e))?;
            Ok(cfg.deployments)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(anyhow::anyhow!("failed to read {}: {}", path.display(), e)),
    }
}

/// The effective deployment list: the built-in default, overlaid by the repo's
/// `.aspect/config.json` (when `$ASPECT_WORKSPACE` names one), overlaid by the
/// user's `~/.aspect/config.json`. Overlay is by `name` (a later entry with the
/// same name replaces an earlier one), so a user can override the seed or a
/// repo-declared deployment. Exactly one entry stays `default` — [`select_deployment`]
/// re-derives the default rather than trusting the flag on every entry.
fn load_deployments() -> anyhow::Result<Vec<Deployment>> {
    let mut merged: Vec<Deployment> = vec![default_deployment()];
    let overlay = |merged: &mut Vec<Deployment>, entries: Vec<Deployment>| {
        for entry in entries {
            if let Some(existing) = merged.iter_mut().find(|d| d.name == entry.name) {
                *existing = entry;
            } else {
                merged.push(entry);
            }
        }
    };
    if let Some(repo_cfg) = repo_config_path() {
        overlay(&mut merged, load_config_file(&repo_cfg)?);
    }
    overlay(&mut merged, load_config_file(&config_path()?)?);
    reconcile_seed_default(&mut merged);
    Ok(merged)
}

/// The seed is re-created `default = true` on every load, but a configured
/// deployment marked default must win. So clear the seed's default whenever any
/// configured deployment claims it — leaving the seed as default only when
/// nothing configured does (which is also the "logged out of the default" state).
fn reconcile_seed_default(deployments: &mut [Deployment]) {
    let configured_default = deployments
        .iter()
        .any(|d| d.default && d.name != DEFAULT_DEPLOYMENT_NAME);
    if configured_default {
        if let Some(seed) = deployments
            .iter_mut()
            .find(|d| d.name == DEFAULT_DEPLOYMENT_NAME)
        {
            seed.default = false;
        }
    }
}

/// The repo-level `.aspect/config.json`, checked in so a team shares a
/// deployment. Located via `$ASPECT_WORKSPACE` (the CLI's workspace root); absent
/// when the CLI runs outside a workspace.
fn repo_config_path() -> Option<PathBuf> {
    let root = std::env::var_os("ASPECT_WORKSPACE")?;
    Some(PathBuf::from(root).join(".aspect").join("config.json"))
}

/// Select a deployment by name, or the default when `name` is `None`. An
/// explicit name must match a configured deployment. With no name, the entry
/// marked `default` wins (the first configured deployment claims `default` when
/// written, so a single configured deployment is the default), falling back to
/// the built-in seed. The default flag is the single source of truth —
/// configuring a deployment does not implicitly hijack selection away from an
/// explicit default.
fn select_deployment(deployments: &[Deployment], name: Option<&str>) -> anyhow::Result<Deployment> {
    if let Some(name) = name.filter(|n| !n.is_empty()) {
        return deployments
            .iter()
            .find(|d| d.name == name)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown deployment: {:?}\n\nConfigure it with `aspect auth configure <host>`.",
                    name
                )
            });
    }
    deployments
        .iter()
        .find(|d| d.default)
        .or_else(|| {
            deployments
                .iter()
                .find(|d| d.name == DEFAULT_DEPLOYMENT_NAME)
        })
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no deployments configured"))
}

/// Selection for the `auth login`/`logout` commands, which target the Aspect
/// *account* by default rather than the default *deployment*: an explicit
/// `name` picks that deployment (erroring if unknown); an empty `name` resolves
/// the built-in Aspect account (the seed). The default-deployment concept (set by
/// `auth use`) governs builds (`--aspect-remote`), not the account login — so a
/// bare `auth login` always means the account, never a configured default.
fn select_account_or_deployment(
    deployments: &[Deployment],
    name: Option<&str>,
) -> anyhow::Result<Deployment> {
    match name.filter(|n| !n.is_empty()) {
        Some(_) => select_deployment(deployments, name),
        None => select_deployment(deployments, Some(DEFAULT_DEPLOYMENT_NAME)),
    }
}

/// The name of the configured deployment that owns `host` — an exact or
/// dot-anchored suffix match against any deployment's `hosts`. Returns `None`
/// when no configured deployment claims the host. Used so endpoint auth attaches
/// the token of the deployment serving a given cache/BES endpoint, whichever
/// profile it was logged in under.
fn deployment_name_for_host(deployments: &[Deployment], host: &str) -> Option<String> {
    // Normalize a trailing dot so an absolute FQDN (`host.example.com.`) still
    // matches its configured host.
    let host = host.trim_end_matches('.').to_lowercase();
    deployments
        .iter()
        .find(|d| {
            d.hosts.iter().any(|h| {
                let h = h
                    .trim()
                    .trim_start_matches('.')
                    .trim_end_matches('.')
                    .to_lowercase();
                !h.is_empty() && (host == h || host.ends_with(&format!(".{h}")))
            })
        })
        .map(|d| d.name.clone())
}

/// Resolve the issuer + client_id for a login / api-token / refresh flow against
/// the selected deployment. Errors when the deployment advertised no login config
/// (no issuer/client_id in its discovery document).
fn resolve_auth_env(name: Option<&str>) -> anyhow::Result<AuthEnv> {
    let deployments = load_deployments()?;
    let deployment = select_deployment(&deployments, name)?;
    auth_env_from(&deployment)
}

/// The Aspect-cloud API base for `ctx.aspect.auth.api_url`, used by Aspect-cloud
/// features (GitHub/GitLab token exchange, status comments, budget). Independent
/// of which deployment serves the cache/BES endpoints: the first deployment that
/// carries an `api_url` (the built-in Aspect seed, unless overridden), then the
/// compile-time default.
fn resolve_api_url() -> anyhow::Result<String> {
    Ok(load_deployments()?
        .into_iter()
        .find_map(|d| d.api_url)
        .unwrap_or_else(|| DEFAULT_API_URL.to_string()))
}

fn auth_env_from(deployment: &Deployment) -> anyhow::Result<AuthEnv> {
    match (&deployment.issuer, &deployment.client_id) {
        (Some(domain), Some(client_id)) => Ok(AuthEnv {
            domain: domain.trim_end_matches('/').to_string(),
            client_id: client_id.clone(),
            // A self-hosted deployment owns endpoint hosts and relies on refresh;
            // the built-in cloud seed (no hosts) reissues without a refresh token
            // and deliberately omits offline_access, so honor its scopes verbatim.
            scopes: login_scopes(&deployment.scopes, !deployment.hosts.is_empty()),
        }),
        _ => Err(anyhow::anyhow!(
            "deployment {:?} advertised no login config, so the CLI cannot log in \
             against it; add its issuer and client_id to ~/.aspect/config.json",
            deployment.name
        )),
    }
}

/// The scopes the login flow requests: the deployment's advertised scopes when it
/// has any (a BYO-IdP override), else the standard OIDC set. When `needs_refresh`
/// (a self-hosted deployment), `offline_access` is guaranteed so the credential
/// can refresh — a discovery document that omits it from `scopes_supported` would
/// otherwise silently disable refresh, turning every expiry into a forced
/// re-login. An IdP that doesn't support it ignores it (OIDC), so it's safe to
/// add. The built-in cloud flow passes `needs_refresh = false` to keep its
/// deliberate no-`offline_access` scope set.
fn login_scopes(advertised: &[String], needs_refresh: bool) -> Vec<String> {
    let mut scopes: Vec<String> = if advertised.is_empty() {
        DEFAULT_LOGIN_SCOPES.iter().map(|s| s.to_string()).collect()
    } else {
        advertised.to_vec()
    };
    if needs_refresh && !scopes.iter().any(|s| s == "offline_access") {
        scopes.push("offline_access".to_string());
    }
    scopes
}

fn resolve_aspect_env() -> anyhow::Result<AuthEnv> {
    resolve_auth_env(None)
}

/// Path of the discovery document a deployment serves (RFC 9728, OAuth 2.0
/// Protected Resource Metadata). Both the remote-cache and BES edges serve it.
const PROTECTED_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource";

/// One entry of the discovery document's `aspect_authorization_servers` list —
/// an OAuth issuer bundled with the PKCE `client_id` and `scopes` to use against
/// it. Pairing the client and scopes with each issuer lets a deployment advertise
/// more than one authorization server, which the flat top-level
/// `client_id`/`scopes_supported` cannot express.
#[derive(Debug, Deserialize)]
struct AspectAuthServer {
    #[serde(default)]
    issuer: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    scopes: Vec<String>,
}

/// A usable OAuth config the CLI can log in with: an issuer paired with its PKCE
/// client and scopes. Owned (not borrowed from the [`ProtectedResource`]) so it
/// can cross the discover → prompt → persist boundary, where a deployment that
/// advertises more than one is resolved to the user's choice.
#[derive(Debug, Clone, PartialEq)]
struct AuthServer {
    issuer: String,
    client_id: String,
    scopes: Vec<String>,
}

/// Whether two issuers name the same authorization server, tolerant of how a
/// user types the `--issuer` flag: scheme and trailing slash are ignored and the
/// host compares case-insensitively (`Auth.Dev.Aspect.Build`, `auth.dev.aspect.build`,
/// and `https://auth.dev.aspect.build/` all match).
fn issuers_match(a: &str, b: &str) -> bool {
    fn normalize(s: &str) -> String {
        s.trim()
            .trim_end_matches('/')
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_ascii_lowercase()
    }
    normalize(a) == normalize(b)
}

/// A deployment endpoint's discovery document, served at
/// [`PROTECTED_RESOURCE_PATH`].
///
/// `aspect_authorization_servers` is the current, preferred shape: a list of
/// {issuer, client_id, scopes} — the OAuth config the CLI mints a bearer against
/// (interactive PKCE or API token). The flat top-level `authorization_servers`
/// (issuer URLs) + `client_id` + `scopes_supported` are the legacy shape, used
/// as a fallback by [`ProtectedResource::auth_servers`] when no usable
/// `aspect_authorization_servers` entry is present (older deployments). `scopes`
/// are overridable per deployment for a bring-your-own IdP that doesn't accept
/// the standard OIDC set, and empty falls back to [`DEFAULT_LOGIN_SCOPES`].
///
/// `aspect_endpoints` is the capability → host map the deployment serves (remote
/// cache + BES share one client, so one login covers every host); the CLI records
/// it so `--deployment` can wire each host to the right Bazel flag.
#[derive(Debug, Deserialize)]
struct ProtectedResource {
    #[serde(default)]
    resource: String,
    #[serde(default)]
    aspect_authorization_servers: Vec<AspectAuthServer>,
    #[serde(default)]
    authorization_servers: Vec<String>,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    scopes_supported: Vec<String>,
    #[serde(default)]
    aspect_endpoints: Endpoints,
}

impl ProtectedResource {
    /// The authorization servers the CLI can log in against, in advertised order:
    /// every issuer-bearing `aspect_authorization_servers` entry when the document
    /// uses the current shape, else the legacy flat fields
    /// (`authorization_servers` + top-level `client_id`/`scopes_supported`).
    /// Empty when neither shape advertises an issuer, so the caller records a
    /// deployment that can't log in.
    fn auth_servers(&self) -> Vec<AuthServer> {
        let nested: Vec<AuthServer> = self
            .aspect_authorization_servers
            .iter()
            .filter(|s| !s.issuer.is_empty())
            .map(|s| AuthServer {
                issuer: s.issuer.clone(),
                client_id: s.client_id.clone(),
                scopes: s.scopes.clone(),
            })
            .collect();
        if !nested.is_empty() {
            return nested;
        }
        self.authorization_servers
            .iter()
            .filter(|s| !s.is_empty())
            .map(|issuer| AuthServer {
                issuer: issuer.clone(),
                client_id: self.client_id.clone(),
                scopes: self.scopes_supported.clone(),
            })
            .collect()
    }
}

/// The outcome of picking which advertised authorization server to configure,
/// given the `candidates` a deployment advertised and an optional `requested`
/// issuer (`--issuer`). See [`resolve_auth_server`].
enum AuthServerChoice {
    /// No authorization server advertised: record a deployment that can't log in.
    None,
    /// One server resolved — either the sole candidate or the `requested` match.
    Selected(AuthServer),
    /// More than one advertised and no `requested` issuer to disambiguate: the
    /// caller must prompt (interactive) or fail (non-interactive).
    Ambiguous,
    /// A `requested` issuer was given but no candidate matches it.
    NotAdvertised,
}

/// Pick the authorization server to configure from the `candidates` a deployment
/// advertised (in advertised order), honoring an optional `requested` issuer:
///   - `requested` set → the matching candidate ([`issuers_match`], host-tolerant),
///     or [`AuthServerChoice::NotAdvertised`] when none matches — even with a
///     single candidate, so a typo'd `--issuer` is caught rather than ignored.
///   - `requested` unset → the sole candidate, [`AuthServerChoice::None`] when
///     there are none, or [`AuthServerChoice::Ambiguous`] when more than one.
fn resolve_auth_server(candidates: &[AuthServer], requested: Option<&str>) -> AuthServerChoice {
    if let Some(requested) = requested {
        return match candidates.iter().find(|c| issuers_match(&c.issuer, requested)) {
            Some(c) => AuthServerChoice::Selected(c.clone()),
            None => AuthServerChoice::NotAdvertised,
        };
    }
    match candidates {
        [] => AuthServerChoice::None,
        [only] => AuthServerChoice::Selected(only.clone()),
        _ => AuthServerChoice::Ambiguous,
    }
}

/// The Starlark-facing view of an advertised [`AuthServer`], for the `configure`
/// task's issuer prompt.
fn auth_server_info(server: AuthServer) -> AuthServerInfo {
    AuthServerInfo {
        issuer: server.issuer,
        client_id: server.client_id,
        scopes: server.scopes,
    }
}

/// The result of probing a host for its protected-resource metadata, so
/// `configure` can tell the two user-facing failure modes apart:
///   - `Reachable` — a valid Aspect Workflows discovery doc (has `aspect_endpoints`).
///   - `Unreachable` — a transport error (DNS/connect/TLS/timeout); the host may
///     well be a real deployment we just can't reach right now.
///   - `NotADeployment` — reached the host, but it isn't an Aspect Workflows
///     deployment endpoint (non-2xx, unparseable, or no `aspect_endpoints`).
enum Discovery {
    Reachable(Box<ProtectedResource>),
    Unreachable(String),
    NotADeployment,
}

/// Probe `host` for its protected-resource metadata. The discovery path stays an
/// implementation detail — the `Unreachable` reason is a plain transport message,
/// not the well-known URL.
async fn probe_protected_resource(host: &str) -> Discovery {
    let url = format!(
        "https://{}{}",
        host.trim_end_matches('/'),
        PROTECTED_RESOURCE_PATH
    );
    let resp = match reqwest::Client::new().get(&url).send().await {
        Ok(resp) => resp,
        // A transport-level failure (DNS, connect, TLS, timeout): can't tell
        // whether it's a deployment, so report unreachable with a terse reason.
        Err(e) => return Discovery::Unreachable(transport_error_reason(&e)),
    };
    if !resp.status().is_success() {
        return Discovery::NotADeployment;
    }
    match resp.json::<ProtectedResource>().await {
        Ok(info) if !info.aspect_endpoints.is_empty() => Discovery::Reachable(Box::new(info)),
        _ => Discovery::NotADeployment,
    }
}

/// A short human-readable reason for a reqwest transport error, without the
/// well-known URL (an implementation detail).
fn transport_error_reason(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "connection timed out".to_string()
    } else if e.is_connect() {
        "could not connect".to_string()
    } else {
        // DNS failures and other transport errors land here.
        "network error".to_string()
    }
}

/// Derive a deployment name from an endpoint host: drop the leading service
/// label (the per-endpoint prefix like `remote`/`bes`) and keep the rest of the
/// subdomain down to — but not including — the registrable domain.
///
///   `bes.gcp.awd-gha-test-dev.aspect.build` → `gcp.awd-gha-test-dev`
///
/// When dropping the first label leaves only the registrable domain, use that
/// domain (a deployment served directly under it, e.g. `remote.aspect.build` →
/// `aspect.build`). The registrable domain is found via the public-suffix list;
/// when the host has no recognized suffix (an unknown TLD, an internal name, or
/// an IP), fall back to dropping the first label and keeping the remainder
/// verbatim.
fn deployment_name_from_host(host: &str) -> String {
    let host = host.trim_matches('.');
    // An IP literal has no meaningful subdomain to strip — use it verbatim (the
    // public-suffix list would otherwise treat the trailing octets as a domain).
    if host.parse::<std::net::IpAddr>().is_ok() {
        return host.to_string();
    }
    let rest = match host.split_once('.') {
        Some((_first, rest)) => rest,
        // A single label (no dot) — nothing to strip; use it as-is.
        None => return host.to_string(),
    };
    match psl::domain_str(host)
        .and_then(|registrable| rest.strip_suffix(registrable).map(|p| (p, registrable)))
    {
        // `rest` is the subdomain-minus-first-label plus the registrable domain;
        // trim the trailing registrable domain to leave the deployment segment.
        // Nothing left (the endpoint sits directly under the registrable domain)
        // → name it after the domain itself.
        Some((prefix, registrable)) => {
            let name = prefix.trim_end_matches('.');
            if name.is_empty() {
                registrable.to_string()
            } else {
                name.to_string()
            }
        }
        // No recognized public suffix, or it isn't a suffix of `rest` (host *is*
        // the registrable domain): keep everything after the first label.
        None => rest.to_string(),
    }
}

/// Build a [`Deployment`] record from a discovered [`ProtectedResource`] and the
/// `selected` authorization server the deployment logs in against (chosen by the
/// caller from [`ProtectedResource::auth_servers`]; `None` when the endpoint
/// advertises no issuer, recording a deployment that can't log in).
///
/// `issuer` + `client_id` + `scopes` come from `selected`. `hosts` are the
/// endpoints this deployment's token covers: the configured host, the advertised
/// `resource` host, and every `aspect_endpoints` host, deduped. The typed
/// `endpoints` map is recorded (hosts normalized) so `--deployment` can wire each
/// to its Bazel flag. `configured_host` is the host the user ran `configure`
/// against (the `resource` may be a canonical alias).
fn deployment_from_discovery(
    name: String,
    configured_host: &str,
    info: &ProtectedResource,
    selected: Option<&AuthServer>,
) -> Deployment {
    let mut hosts: Vec<String> = Vec::new();
    let mut push = |h: &str| {
        let h = endpoint_host_str(h);
        if !h.is_empty() && !hosts.contains(&h) {
            hosts.push(h);
        }
    };
    push(configured_host);
    push(&info.resource);
    // The endpoint hosts share the same credential, so they extend the auth gate
    // too (normalized to bare hosts, matching `hosts`).
    let endpoints = Endpoints {
        cache: endpoint_host_str(&info.aspect_endpoints.cache),
        bes: endpoint_host_str(&info.aspect_endpoints.bes),
        exec: endpoint_host_str(&info.aspect_endpoints.exec),
    };
    push(&endpoints.cache);
    push(&endpoints.bes);
    push(&endpoints.exec);
    Deployment {
        name,
        default: false,
        issuer: selected.map(|s| s.issuer.clone()),
        client_id: selected.map(|s| s.client_id.clone()),
        api_url: None,
        hosts,
        scopes: selected.map(|s| s.scopes.clone()).unwrap_or_default(),
        endpoints,
    }
}

/// Insert or replace `deployment` in `existing` (by name), keeping exactly one
/// entry marked `default`. A deployment becomes the default when it explicitly
/// claims it, when it is the first configured entry, or when it replaces the
/// entry that was already the default (so re-running `configure` on the current
/// default does not silently clear it). Returns whether the written record is
/// the effective default.
fn upsert_deployment(existing: &mut Vec<Deployment>, mut deployment: Deployment) -> bool {
    let replacing_default = existing
        .iter()
        .any(|d| d.name == deployment.name && d.default);
    let none_default = !existing.iter().any(|d| d.default);
    if none_default || replacing_default {
        deployment.default = true;
    }
    if deployment.default {
        for d in existing.iter_mut() {
            d.default = false;
        }
    }
    let is_default = deployment.default;
    if let Some(slot) = existing.iter_mut().find(|d| d.name == deployment.name) {
        *slot = deployment;
    } else {
        existing.push(deployment);
    }
    is_default
}

/// Serialize `deployments` to the user's `~/.aspect/config.json` (creating the
/// directory), replacing the whole file. The built-in seed is not persisted —
/// only user-configured deployments live in the file (the seed is re-added by
/// [`load_deployments`] on read).
fn write_user_config(deployments: Vec<Deployment>) -> anyhow::Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("failed to create ~/.aspect directory: {}", e))?;
    }
    let json = serde_json::to_string_pretty(&DeploymentsConfig { deployments })
        .map_err(|e| anyhow::anyhow!("failed to serialize deployments: {}", e))?;
    fs::write(&path, &json)
        .map_err(|e| anyhow::anyhow!("failed to write {}: {}", path.display(), e))?;
    Ok(())
}

/// Write (or replace by name) a deployment in the user's `~/.aspect/config.json`
/// via [`upsert_deployment`]. Returns whether the written record is the default.
fn save_user_deployment(deployment: Deployment) -> anyhow::Result<bool> {
    let mut existing = load_config_file(&config_path()?)?;
    let is_default = upsert_deployment(&mut existing, deployment);
    write_user_config(existing)?;
    Ok(is_default)
}

/// Make the deployment named `name` the sole default (clearing the flag on all
/// others), or — when `name` is `None` — clear the default entirely (leaving no
/// configured deployment default; selection with no explicit name then falls
/// back to the built-in Aspect seed). Errors if `name` is given but matches no
/// configured deployment. The built-in seed is not written to the file, so
/// setting it default is expressed as "no configured default".
fn set_default_deployment(name: Option<&str>) -> anyhow::Result<()> {
    let mut existing = load_config_file(&config_path()?)?;
    apply_set_default(&mut existing, name)?;
    write_user_config(existing)
}

/// Mutate a configured-deployment list (the file's contents, excluding the seed)
/// so `name` is the sole default, or — for `None` or the built-in seed name —
/// so no configured deployment is default. Errors if `name` is a non-seed name
/// absent from the list. See [`set_default_deployment`].
fn apply_set_default(deployments: &mut [Deployment], name: Option<&str>) -> anyhow::Result<()> {
    // Selecting the built-in seed (or None) means "no configured default".
    let target = name.filter(|n| *n != DEFAULT_DEPLOYMENT_NAME);
    if let Some(target) = target {
        if !deployments.iter().any(|d| d.name == target) {
            return Err(anyhow::anyhow!(
                "unknown deployment: {:?}\n\nConfigure it with `aspect auth configure <host>`.",
                target
            ));
        }
    }
    for d in deployments.iter_mut() {
        d.default = Some(d.name.as_str()) == target;
    }
    Ok(())
}

/// Forget a configured deployment: drop its `~/.aspect/config.json` entry and its
/// stored credential. Errors on the built-in Aspect seed (not configured, can't
/// be removed) or an unknown name. The seed isn't in the file, so it can't be a
/// target. When the removed deployment was the default, the file simply has no
/// default afterward (selection falls back to the seed).
fn remove_deployment(name: &str) -> anyhow::Result<()> {
    if name == DEFAULT_DEPLOYMENT_NAME {
        return Err(anyhow::anyhow!(
            "the built-in {DEFAULT_DEPLOYMENT_NAME:?} deployment cannot be removed"
        ));
    }
    let mut existing = load_config_file(&config_path()?)?;
    let before = existing.len();
    existing.retain(|d| d.name != name);
    if existing.len() == before {
        return Err(anyhow::anyhow!(
            "unknown deployment: {name:?}\n\nRun `aspect auth status` to see configured deployments."
        ));
    }
    write_user_config(existing)?;
    // Also clear its stored credential (the profile is the deployment name).
    let mut creds = load_all_credentials()?;
    if creds.remove(name).is_some() {
        save_all_credentials(&creds)?;
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct CredentialsEntry {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    email: String,
    name: String,
    tenant_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_client_id: Option<String>,
    // True when this session's bearer is the OIDC id_token (the self-hosted
    // endpoint flow), so a subsequent refresh keeps minting an id_token rather
    // than downgrading to the access_token. Absent/false for the Aspect-cloud
    // flow (access_token bearer). Defaulted so entries written before this field
    // existed load as cloud sessions.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    prefer_id_token: bool,
}

impl CredentialsEntry {
    /// Build an entry for `bearer` (the JWT sent to endpoints), decoding its
    /// email/name/tenant claims for display and recording the issuer + client_id
    /// so the credential can be refreshed. `refresh_token` may be empty (api-token
    /// and raw-token logins are not refreshable). `prefer_id_token` records that the
    /// bearer is an id_token (the self-hosted endpoint flow) so refresh preserves
    /// that kind.
    fn from_bearer(
        bearer: String,
        refresh_token: String,
        auth_domain: Option<String>,
        auth_client_id: Option<String>,
        prefer_id_token: bool,
    ) -> anyhow::Result<Self> {
        let claims = decode_jwt_claims(&bearer)?;
        Ok(CredentialsEntry {
            access_token: bearer,
            refresh_token,
            email: claims.email.unwrap_or_default(),
            name: claims.name.unwrap_or_else(|| "Unknown".to_string()),
            tenant_id: claims.tenant_id,
            auth_domain,
            auth_client_id,
            prefer_id_token,
        })
    }
}

fn load_all_credentials() -> anyhow::Result<HashMap<String, CredentialsEntry>> {
    CredentialStore::resolve()?.load_all()
}

fn save_all_credentials(map: &HashMap<String, CredentialsEntry>) -> anyhow::Result<()> {
    CredentialStore::resolve()?.save_all(map)
}

#[derive(Debug, Deserialize)]
struct JwtClaims {
    email: Option<String>,
    name: Option<String>,
    // A self-hosted deployment's issuer may not mint a `tenantId` claim, so treat
    // it as optional (display-only) rather than failing to decode.
    #[serde(rename = "tenantId", default)]
    tenant_id: String,
    exp: Option<u64>,
}

fn decode_jwt_claims(token: &str) -> anyhow::Result<JwtClaims> {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(anyhow::anyhow!(
            "invalid JWT: expected 3 parts, got {}",
            parts.len()
        ));
    }
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| anyhow::anyhow!("failed to decode JWT payload: {}", e))?;
    serde_json::from_slice(&payload_bytes)
        .map_err(|e| anyhow::anyhow!("failed to parse JWT claims: {}", e))
}

fn format_token_status(exp: Option<u64>) -> String {
    let Some(exp) = exp else {
        return "no expiry".to_string();
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if now >= exp {
        "expired".to_string()
    } else {
        let remaining = exp - now;
        let hours = remaining / 3600;
        let minutes = (remaining % 3600) / 60;
        if hours > 0 {
            format!("expires in {}h {}m", hours, minutes)
        } else {
            format!("expires in {}m", minutes)
        }
    }
}

fn is_expired_jwt(entry: &CredentialsEntry) -> bool {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let parts: Vec<&str> = entry.access_token.split('.').collect();
    if parts.len() != 3 {
        return true;
    }
    let Ok(payload_bytes) = URL_SAFE_NO_PAD.decode(parts[1]) else {
        return true;
    };
    #[derive(Deserialize)]
    struct ExpOnly {
        exp: Option<u64>,
    }
    let Ok(claims) = serde_json::from_slice::<ExpOnly>(&payload_bytes) else {
        return true;
    };
    let Some(exp) = claims.exp else {
        return false;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now + 60 >= exp
}

fn can_refresh(entry: &CredentialsEntry) -> bool {
    !entry.refresh_token.is_empty() && entry.auth_domain.is_some() && entry.auth_client_id.is_some()
}

fn generate_code_verifier() -> String {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use rand::RngCore;

    let mut buf = [0u8; 96];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

fn generate_code_challenge(verifier: &str) -> String {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                String::from(b as char)
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}

fn urldecode(s: &str) -> String {
    // Accumulate raw bytes so a percent-encoded multi-byte UTF-8 sequence
    // reassembles correctly (rather than mapping each byte to a Latin-1 char).
    let mut bytes: Vec<u8> = Vec::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                bytes.push(byte);
            }
        } else if c == '+' {
            bytes.push(b' ');
        } else {
            let mut buf = [0u8; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn extract_query_param(path: &str, key: &str) -> Option<String> {
    let query = path.split('?').nth(1)?;
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if kv.next() == Some(key) {
            return kv.next().map(urldecode);
        }
    }
    None
}

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    Handle::current().block_on(fut)
}

/// Process-level cache for API-token-exchange results. A single `aspect`
/// task can call `ctx.aspect.auth.credentials()` multiple times (once per
/// `github.authenticate` call); without caching each call hits the auth
/// provider's `/api-token` endpoint. The cache stores the most recently successful
/// exchange keyed on the source token AND the resolved auth-environment
/// domain; entries are invalidated automatically once the JWT's `exp` claim
/// falls inside the 60-second skew buffer (`is_expired_jwt`), so we don't
/// hand out tokens that are about to be rejected.
struct ApiTokenCacheEntry {
    /// Source token (the raw `client_id:secret` value) the cached entry was
    /// minted from. If the env var changes mid-run we mint a fresh one.
    source_token: String,
    /// Issuer the cached entry was exchanged against
    /// (`https://auth.aspect.build`, a self-hosted deployment's issuer, …). Reusing a
    /// token minted by one deployment's issuer against another would hand back
    /// credentials from the wrong issuer, so the issuer is part of the cache key.
    auth_domain: String,
    entry: CredentialsEntry,
}

static API_TOKEN_CACHE: OnceLock<Mutex<Option<ApiTokenCacheEntry>>> = OnceLock::new();

fn api_token_cache() -> &'static Mutex<Option<ApiTokenCacheEntry>> {
    API_TOKEN_CACHE.get_or_init(|| Mutex::new(None))
}

/// Exchange an ASPECT_API_TOKEN from an explicit source — the env var or the
/// Buildkite secret store — for a fresh credentials entry. Returns Ok(None)
/// only when no such source is present; a malformed token or a failed
/// exchange is surfaced as an error so CI misconfigurations fail loudly
/// instead of falling through to whatever happens to be cached on disk
/// (e.g. from a previous job on a persistent runner).
///
/// Subsequent calls within the same process reuse the cached exchange
/// result until the JWT approaches expiry.
fn credentials_from_api_token_env() -> anyhow::Result<Option<CredentialsEntry>> {
    let token = std::env::var("ASPECT_API_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(buildkite_aspect_api_token);
    let Some(token) = token else {
        return Ok(None);
    };

    // Resolve the default deployment's issuer each call so a config change
    // within a single process (test harnesses, multi-step tooling) re-exchanges
    // against the new issuer instead of returning a stale token.
    let env = resolve_aspect_env()?;

    // Fast path: a cached exchange for the same source token AND issuer whose
    // JWT hasn't entered the 60-second pre-expiry buffer.
    if let Ok(guard) = api_token_cache().lock() {
        if let Some(cached) = guard.as_ref() {
            if cached.source_token == token
                && cached.auth_domain == env.domain
                && !is_expired_jwt(&cached.entry)
            {
                return Ok(Some(cached.entry.clone()));
            }
        }
    }

    let (client_id, secret) = token
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("ASPECT_API_TOKEN must be in 'client_id:secret' format"))?;
    let entry = block_on(exchange_api_token(client_id, secret, &env))?;

    // Store for subsequent calls. Clearing on lock-poison is fine — the
    // next caller will just miss and re-exchange.
    if let Ok(mut guard) = api_token_cache().lock() {
        *guard = Some(ApiTokenCacheEntry {
            source_token: token,
            auth_domain: env.domain.clone(),
            entry: entry.clone(),
        });
    }
    Ok(Some(entry))
}

/// Read ASPECT_API_TOKEN from Buildkite's secret store when running on a
/// Buildkite agent. Returns None if not on a Buildkite agent, the CLI is
/// missing, or the secret is not defined — callers should fall through to
/// the "not authenticated" path rather than surface the error.
fn buildkite_aspect_api_token() -> Option<String> {
    // BUILDKITE_AGENT_ACCESS_TOKEN is injected into every job on a Buildkite
    // agent; checking it avoids shelling out when we're not on Buildkite.
    if std::env::var_os("BUILDKITE_AGENT_ACCESS_TOKEN").is_none() {
        return None;
    }
    let output = std::process::Command::new("buildkite-agent")
        .args(["secret", "get", "ASPECT_API_TOKEN"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if token.is_empty() { None } else { Some(token) }
}

async fn exchange_api_token(
    client_id: &str,
    secret: &str,
    env: &AuthEnv,
) -> anyhow::Result<CredentialsEntry> {
    #[derive(Deserialize)]
    struct ApiTokenResponse {
        #[serde(rename = "accessToken", alias = "access_token")]
        access_token: String,
    }
    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/identity/resources/auth/v1/api-token",
            env.domain
        ))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "clientId": client_id, "secret": secret }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("API token exchange failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "API token exchange failed (HTTP {}): {}",
            status,
            body
        ));
    }
    let data: ApiTokenResponse = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("failed to parse API token response: {}", e))?;
    // Not refreshable (the api-token is re-exchanged each run), so no
    // auth_domain/client_id are stored. Access-token bearer (Aspect cloud).
    CredentialsEntry::from_bearer(data.access_token, String::new(), None, None, false)
}

/// Accept one OAuth callback request on `listener` and return its `code` and
/// `state` query params (the caller validates `state` when it set one). Writes a
/// success page to the browser.
async fn accept_callback(listener: TcpListener) -> anyhow::Result<(String, Option<String>)> {
    let (mut stream, _addr) = listener
        .accept()
        .await
        .map_err(|e| anyhow::anyhow!("failed to accept OAuth callback: {}", e))?;
    let mut buf = vec![0u8; 4096];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| anyhow::anyhow!("failed to read OAuth callback: {}", e))?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request
        .lines()
        .next()
        .ok_or_else(|| anyhow::anyhow!("empty HTTP request from browser callback"))?;
    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("malformed HTTP request line"))?;
    let code = extract_query_param(path, "code").ok_or_else(|| {
        let error = extract_query_param(path, "error").unwrap_or_else(|| "unknown".to_string());
        let desc = extract_query_param(path, "error_description").unwrap_or_default();
        anyhow::anyhow!("authentication failed: {} {}", error, desc)
    })?;
    let state = extract_query_param(path, "state");
    let html = r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Authenticated — Aspect Build</title>
  <style>
    * { margin: 0; padding: 0; box-sizing: border-box; }
    body {
      font-family: 'Inter', -apple-system, BlinkMacSystemFont, sans-serif;
      background: #f5f6f8;
      color: #1f2e35;
      min-height: 100vh;
      display: flex;
      align-items: center;
      justify-content: center;
    }
    .card {
      text-align: center;
      padding: 48px 56px;
      border: 1px solid #e2e6ea;
      border-radius: 12px;
      background: #fff;
      max-width: 340px;
      width: 100%;
    }
    .logo { margin-bottom: 28px; }
    h2 {
      font-size: 17px;
      font-weight: 600;
      color: #1f2e35;
      margin-bottom: 8px;
    }
    p {
      font-size: 13px;
      color: #6b7e87;
    }
  </style>
</head>
<body>
  <div class="card">
    <div class="logo">
      <svg xmlns="http://www.w3.org/2000/svg" width="44" height="40" viewBox="0 0 44 40" fill="none">
        <path fill-rule="evenodd" d="M37.854 39.784H24.115l6.825-11.923-6.825-11.923h13.739l6.874 11.923-6.874 11.923zm-30.978 0h13.719l6.825-11.932H13.77L6.896 15.919.07 27.852l6.806 11.932zm1.882-26.865l6.884 11.923 6.825-11.942h13.739L29.371.977H15.622L8.758 12.919z" fill="#176acc"/>
      </svg>
    </div>
    <h2>Login successful</h2>
    <p>You can close this tab and return to your terminal.</p>
  </div>
  <script>setTimeout(window.close, 10000)</script>
</body>
</html>"##;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|e| anyhow::anyhow!("failed to write OAuth response: {}", e))?;
    Ok((code, state))
}

/// An OAuth token response. The bearer the CLI attaches to endpoints is the
/// `id_token` when present (self-hosted edges validate the OIDC id_token), else
/// the `access_token` (the Aspect-cloud flow).
#[derive(Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    id_token: String,
    #[serde(default)]
    refresh_token: String,
}

impl TokenResponse {
    /// The bearer to attach to endpoints. When `prefer_id_token` (the self-hosted
    /// endpoint flow — those edges validate the OIDC id_token), require the
    /// id_token and error if the grant omitted it, rather than silently returning
    /// the access_token: a refresh grant that drops the id_token (permitted by RFC
    /// 6749 §5.1) would otherwise downgrade a working session to a bearer the edge
    /// rejects with 401. When not preferring the id_token (the Aspect-cloud flow,
    /// which validates the access_token), return the access_token, falling back to
    /// the id_token only if the access_token is absent.
    fn bearer(self, prefer_id_token: bool) -> anyhow::Result<String> {
        if prefer_id_token {
            return if !self.id_token.is_empty() {
                Ok(self.id_token)
            } else {
                Err(anyhow::anyhow!(
                    "token response contained no id_token (self-hosted endpoints \
                     validate the id_token); re-run `aspect auth login`"
                ))
            };
        }
        if !self.access_token.is_empty() {
            Ok(self.access_token)
        } else if !self.id_token.is_empty() {
            Ok(self.id_token)
        } else {
            Err(anyhow::anyhow!(
                "token response contained neither an id_token nor an access_token"
            ))
        }
    }
}

/// POST an `application/x-www-form-urlencoded` body to a token/exchange endpoint
/// and deserialize the JSON response, surfacing a non-2xx status with its body.
async fn post_form<T: serde::de::DeserializeOwned>(
    url: &str,
    form: &[(&str, &str)],
    what: &str,
) -> anyhow::Result<T> {
    let resp = reqwest::Client::new()
        .post(url)
        .form(form)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("{what} request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("{what} failed (HTTP {status}): {body}"));
    }
    resp.json::<T>()
        .await
        .map_err(|e| anyhow::anyhow!("failed to parse {what} response: {e}"))
}

/// Exchange an authorization `code` for tokens at `token_url` (PKCE — sends
/// `code_verifier`, no client secret). The caller picks the bearer from the
/// response (`id_token` for self-hosted edges, `access_token` for Aspect cloud).
async fn exchange_code(
    token_url: &str,
    client_id: &str,
    redirect_uri: &str,
    code: &str,
    code_verifier: &str,
) -> anyhow::Result<TokenResponse> {
    post_form(
        token_url,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
            ("code_verifier", code_verifier),
        ],
        "token exchange",
    )
    .await
}

/// The authorize + token endpoints for an issuer.
struct OidcEndpoints {
    authorize: String,
    token: String,
}

/// Resolve an issuer's authorize + token endpoints, preferring OIDC discovery
/// (`{issuer}/.well-known/openid-configuration`) so a self-hosted provider works
/// without assuming a URL layout, falling back to the conventional
/// `{issuer}/oauth/{authorize,token}` when no discovery document is served.
async fn resolve_oidc_endpoints(issuer: &str) -> OidcEndpoints {
    let issuer = issuer.trim_end_matches('/');
    #[derive(Deserialize)]
    struct Discovery {
        authorization_endpoint: String,
        token_endpoint: String,
    }
    let discovered = reqwest::Client::new()
        .get(format!("{issuer}/.well-known/openid-configuration"))
        .send()
        .await
        .ok()
        .filter(|r| r.status().is_success());
    if let Some(resp) = discovered {
        if let Ok(d) = resp.json::<Discovery>().await {
            // The discovery doc is server-controlled and drives where the
            // authorization code + PKCE verifier are sent, so require https on
            // both endpoints — never POST the code over http or to a downgraded
            // endpoint. A non-https endpoint falls back to the conventional
            // https-derived one rather than trusting it.
            if is_https(&d.authorization_endpoint) && is_https(&d.token_endpoint) {
                return OidcEndpoints {
                    authorize: d.authorization_endpoint,
                    token: d.token_endpoint,
                };
            }
        }
    }
    oidc_endpoints_fallback(issuer)
}

/// Whether `url` is an absolute `https://` URL (case-insensitive scheme).
fn is_https(url: &str) -> bool {
    let scheme_end = url.find("://").unwrap_or(0);
    url[..scheme_end].eq_ignore_ascii_case("https")
}

/// The conventional `{issuer}/oauth/{authorize,token}` endpoints, used when the
/// issuer serves no OIDC discovery document (`issuer` is already trimmed of a
/// trailing slash by [`resolve_oidc_endpoints`]).
fn oidc_endpoints_fallback(issuer: &str) -> OidcEndpoints {
    OidcEndpoints {
        authorize: format!("{issuer}/oauth/authorize"),
        token: format!("{issuer}/oauth/token"),
    }
}

/// The OAuth `state` for a self-hosted login: `"<nonce>.<port>"`. The nonce is
/// the CSRF guard (validated verbatim on callback); the port suffix tells the
/// endpoint's callback page which loopback port to forward to. The nonce is
/// base64url so it never contains the dot separator.
fn login_state(nonce: &str, port: u16) -> String {
    format!("{nonce}.{port}")
}

/// Build an OAuth Authorization-Code + PKCE authorize URL. Every value is
/// url-encoded; `scope` is space-joined; `state` is appended when present (the
/// self-hosted flow uses it, the cloud flow does not). `authorize_endpoint` may
/// already carry a query (OIDC discovery can return one), so the parameter
/// separator is chosen accordingly.
fn build_authorize_url(
    authorize_endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    scopes: &[String],
    code_challenge: &str,
    state: Option<&str>,
) -> String {
    let mut url = format!(
        "{}{}client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}&code_challenge_method=S256",
        authorize_endpoint,
        if authorize_endpoint.contains('?') {
            '&'
        } else {
            '?'
        },
        urlencode(client_id),
        urlencode(redirect_uri),
        urlencode(&scopes.join(" ")),
        urlencode(code_challenge),
    );
    if let Some(state) = state {
        url.push_str(&format!("&state={}", urlencode(state)));
    }
    url
}

fn build_cloud_session(env: AuthEnv) -> anyhow::Result<AuthSession> {
    let port: u16 = 19556;
    let listener = block_on(TcpListener::bind(format!("127.0.0.1:{}", port))).map_err(|e| {
        anyhow::anyhow!(
            "failed to bind localhost:{} — is another login in progress? ({})",
            port,
            e
        )
    })?;
    let redirect_uri = format!("http://localhost:{}/callback", port);
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);
    let authorize_url = build_authorize_url(
        &format!("{}/oauth/authorize", env.domain),
        &env.client_id,
        &redirect_uri,
        &env.scopes,
        &code_challenge,
        None,
    );
    Ok(AuthSession {
        url: authorize_url,
        inner: Mutex::new(Some(AuthSessionInner {
            listener: Some(listener),
            code_verifier,
            redirect_uri,
            env,
            kind: SessionKind::Cloud,
        })),
    })
}

/// Build a configured (self-hosted) deployment's browser session. The redirect is
/// the endpoint's own `https://<host>/oauth2/callback`, which forwards the browser
/// to this CLI's loopback listener; an OS-assigned port is bound up front and
/// carried to the endpoint in `state = "<nonce>.<port>"` so the callback page can
/// forward to it. The authorize endpoint is resolved via OIDC discovery, and the
/// bearer is the OIDC `id_token`.
fn build_endpoint_session(env: AuthEnv, host: &str) -> anyhow::Result<AuthSession> {
    let listener = block_on(TcpListener::bind("127.0.0.1:0"))
        .map_err(|e| anyhow::anyhow!("failed to bind the login callback port: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| anyhow::anyhow!("failed to read the login callback port: {}", e))?
        .port();

    let redirect_uri = format!("https://{}/oauth2/callback", host);
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);
    let state = login_state(&generate_code_verifier(), port);
    let endpoints = block_on(resolve_oidc_endpoints(&env.domain));
    let authorize_url = build_authorize_url(
        &endpoints.authorize,
        &env.client_id,
        &redirect_uri,
        &env.scopes,
        &code_challenge,
        Some(&state),
    );
    Ok(AuthSession {
        url: authorize_url,
        inner: Mutex::new(Some(AuthSessionInner {
            listener: Some(listener),
            code_verifier,
            redirect_uri,
            env,
            kind: SessionKind::Endpoint {
                expected_state: state,
            },
        })),
    })
}

async fn refresh_access_token(entry: &CredentialsEntry) -> anyhow::Result<CredentialsEntry> {
    let auth_domain = entry
        .auth_domain
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no auth_domain stored — cannot refresh"))?;
    let client_id = entry
        .auth_client_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no auth_client_id stored — cannot refresh"))?;
    // Resolve the token endpoint via OIDC discovery (the conventional
    // /oauth/token is the fallback), so a self-hosted issuer whose token endpoint
    // is elsewhere still refreshes.
    let endpoints = resolve_oidc_endpoints(auth_domain).await;
    let token_resp: TokenResponse = post_form(
        &endpoints.token,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &entry.refresh_token),
            ("client_id", client_id),
        ],
        "token refresh",
    )
    .await?;
    let refresh_token = merged_refresh_token(&entry.refresh_token, &token_resp.refresh_token);
    CredentialsEntry::from_bearer(
        token_resp.bearer(entry.prefer_id_token)?,
        refresh_token,
        entry.auth_domain.clone(),
        entry.auth_client_id.clone(),
        entry.prefer_id_token,
    )
}

/// The refresh token to persist after a refresh: the newly-issued `fresh` one, or
/// the `prior` when the response omitted it. RFC 6749 §6 makes the refresh token
/// optional in a refresh response; a provider that doesn't rotate it leaves the
/// original valid, so dropping it would make the next expiry fail `can_refresh`
/// and force a needless re-login.
fn merged_refresh_token(prior: &str, fresh: &str) -> String {
    if fresh.is_empty() {
        prior.to_string()
    } else {
        fresh.to_string()
    }
}

/// The `aspect auth login` invocation that re-authenticates `profile`, naming
/// the deployment so the hint doesn't assume the default. A self-hosted
/// deployment stores under its name, so the profile is the deployment name and
/// gets an explicit `--deployment`; the default profile (the built-in Aspect
/// deployment, or a `$ASPECT_AUTH_PROFILE`) gets a bare `login`.
fn login_hint(profile: &str) -> String {
    if profile == DEFAULT_PROFILE {
        "aspect auth login".to_string()
    } else {
        format!("aspect auth login --deployment {profile}")
    }
}

/// Shown when a stored session is expired and cannot be silently refreshed.
fn session_expired_message(profile: &str) -> String {
    format!(
        "session expired\n\nRun `{}` to re-authenticate.",
        login_hint(profile)
    )
}

/// What to do with a stored credentials entry, decided purely from the entry
/// (no IO). Lets the expiry policy be unit-tested without disk or network.
enum TokenAction {
    /// The token is current; use it as-is.
    Use,
    /// The token is expired but refreshable; mint a new one.
    Refresh,
    /// The token is expired and cannot be refreshed; the caller must error.
    Expired,
}

fn classify_token(entry: &CredentialsEntry) -> TokenAction {
    if !is_expired_jwt(entry) {
        TokenAction::Use
    } else if can_refresh(entry) {
        TokenAction::Refresh
    } else {
        TokenAction::Expired
    }
}

/// Environment variable naming the credentials profile to use when no explicit
/// profile is given. The single selector shared by the `auth` tasks and the
/// Bazel credential helper (which has no way to pass a flag), so one
/// `export ASPECT_AUTH_PROFILE=...` drives both.
pub const PROFILE_ENV: &str = "ASPECT_AUTH_PROFILE";

/// The credentials profile when none is configured.
const DEFAULT_PROFILE: &str = "default";

/// Resolve the effective profile name: an explicit (non-empty) value wins, then
/// [`PROFILE_ENV`] (non-empty), then [`DEFAULT_PROFILE`]. An empty `explicit` is
/// treated as absent (the `auth` tasks pass `""` when `--profile` is omitted),
/// so it still falls through to the env var.
pub fn resolve_profile(explicit: Option<&str>) -> String {
    let non_empty = |s: String| (!s.is_empty()).then_some(s);
    explicit
        .map(str::to_owned)
        .and_then(non_empty)
        .or_else(|| std::env::var(PROFILE_ENV).ok().and_then(non_empty))
        .unwrap_or_else(|| DEFAULT_PROFILE.to_owned())
}

/// Which configured deployment owns the endpoint `uri`, for the credential helper:
/// the `host` parsed from the URI and the `deployment` (name = credential profile)
/// that claims it, or `None` when no configured deployment does. The helper emits
/// a credential only when a deployment owns the host, so it self-scopes by the URI
/// Bazel passes — a global `--credential_helper=aspect` never sends a token to an
/// unclaimed (e.g. third-party) host.
pub struct UriProfile {
    pub host: String,
    pub deployment: Option<String>,
}

/// Resolve the [`UriProfile`] for a credential-helper request, so a Bazel request
/// for a configured deployment's cache/BES gets *that* deployment's token.
pub fn profile_for_uri(uri: &str) -> anyhow::Result<UriProfile> {
    let host = endpoint_host_str(uri);
    let deployment = if host.is_empty() {
        None
    } else {
        deployment_name_for_host(&load_deployments()?, &host)
    };
    Ok(UriProfile { host, deployment })
}

/// Resolve the current access token (JWT) for `profile` (already resolved, e.g.
/// via [`resolve_profile`]). For the default profile an explicit
/// `ASPECT_API_TOKEN` (exchanged against the Aspect account issuer) takes
/// precedence over stored credentials; other profiles always use their stored
/// credential. Auto-refreshes an expired-but-refreshable token (persisting the
/// refresh). Returns `None` when no credential exists for the profile, and errors
/// when the stored token is expired and cannot be refreshed: it never returns a
/// known-expired token, so a consumer (e.g. the credential helper) does not emit
/// one a server will 401. Requires a Tokio runtime (the refresh path blocks on
/// async HTTP).
pub fn resolve_access_token(profile: &str) -> anyhow::Result<Option<String>> {
    // ASPECT_API_TOKEN is exchanged against the default (Aspect account) issuer,
    // so it only stands in for the default profile — a self-hosted deployment's
    // profile must use its own stored credential, not the account token.
    if profile == DEFAULT_PROFILE {
        if let Some(entry) = credentials_from_api_token_env()? {
            return Ok(Some(entry.access_token));
        }
    }
    let mut map = load_all_credentials()?;
    let Some(entry) = map.get(profile).cloned() else {
        return Ok(None);
    };
    match classify_token(&entry) {
        TokenAction::Use => Ok(Some(entry.access_token)),
        TokenAction::Expired => Err(anyhow::anyhow!(session_expired_message(profile))),
        TokenAction::Refresh => {
            let refreshed = block_on(refresh_access_token(&entry))
                .map_err(|_| anyhow::anyhow!(session_expired_message(profile)))?;
            map.insert(profile.to_string(), refreshed.clone());
            save_all_credentials(&map)?;
            Ok(Some(refreshed.access_token))
        }
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative, Clone)]
#[display("<AuthCredentials>")]
pub struct AuthCredentials {
    pub email: String,
    pub name: String,
    pub tenant_id: String,
    pub access_token: String,
    pub token_status: String,
    // Internal fields preserved for persist()
    pub(crate) refresh_token: String,
    pub(crate) auth_domain: Option<String>,
    pub(crate) auth_client_id: Option<String>,
    pub(crate) prefer_id_token: bool,
}

starlark_simple_value!(AuthCredentials);

#[starlark_value(type = "aspect.AuthCredentials")]
impl<'v> values::StarlarkValue<'v> for AuthCredentials {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(auth_credentials_methods)
    }
}

/// Body of a `#[starlark(attribute)]` string getter: downcast `this` to `$ty` and
/// clone its `$field`. The `#[starlark(attribute)] fn …` wrapper stays visible to
/// the `#[starlark_module]` macro; only the copy-pasted body is deduped.
macro_rules! attr_str {
    ($this:expr, $ty:ty, $field:ident) => {
        Ok($this
            .downcast_ref_err::<$ty>()
            .into_anyhow_result()?
            .$field
            .clone())
    };
}

/// Body of a `#[starlark(attribute)]` bool getter (copies rather than clones).
macro_rules! attr_bool {
    ($this:expr, $ty:ty, $field:ident) => {
        Ok($this.downcast_ref_err::<$ty>().into_anyhow_result()?.$field)
    };
}

#[starlark_module]
fn auth_credentials_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn email<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, AuthCredentials, email)
    }

    #[starlark(attribute)]
    fn name<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, AuthCredentials, name)
    }

    #[starlark(attribute)]
    fn tenant_id<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, AuthCredentials, tenant_id)
    }

    #[starlark(attribute)]
    fn access_token<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, AuthCredentials, access_token)
    }

    #[starlark(attribute)]
    fn token_status<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, AuthCredentials, token_status)
    }
}

impl AuthCredentials {
    fn from_entry(entry: &CredentialsEntry) -> Self {
        let claims = decode_jwt_claims(&entry.access_token).ok();
        let exp = claims.as_ref().and_then(|c| c.exp);
        AuthCredentials {
            email: entry.email.clone(),
            name: entry.name.clone(),
            tenant_id: entry.tenant_id.clone(),
            access_token: entry.access_token.clone(),
            token_status: format_token_status(exp),
            refresh_token: entry.refresh_token.clone(),
            auth_domain: entry.auth_domain.clone(),
            auth_client_id: entry.auth_client_id.clone(),
            prefer_id_token: entry.prefer_id_token,
        }
    }

    fn to_entry(&self) -> CredentialsEntry {
        CredentialsEntry {
            access_token: self.access_token.clone(),
            refresh_token: self.refresh_token.clone(),
            email: self.email.clone(),
            name: self.name.clone(),
            tenant_id: self.tenant_id.clone(),
            auth_domain: self.auth_domain.clone(),
            auth_client_id: self.auth_client_id.clone(),
            prefer_id_token: self.prefer_id_token,
        }
    }
}

/// Which browser login a pending [`AuthSession`] runs when `wait()`ed.
enum SessionKind {
    /// Aspect-cloud: loopback redirect registered directly with the IdP; the
    /// bearer is the OAuth `access_token`.
    Cloud,
    /// A configured (self-hosted) deployment: the redirect is the endpoint's own
    /// `/oauth2/callback` (which forwards the browser to this loopback), the
    /// authorization/token endpoints come from OIDC discovery, and the bearer is
    /// the OIDC `id_token`. `state` is validated against the value we generated.
    Endpoint { expected_state: String },
}

struct AuthSessionInner {
    listener: Option<TcpListener>,
    code_verifier: String,
    redirect_uri: String,
    env: AuthEnv,
    kind: SessionKind,
}

#[derive(Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<AuthSession>")]
pub struct AuthSession {
    pub url: String,
    #[allocative(skip)]
    inner: Mutex<Option<AuthSessionInner>>,
}

impl std::fmt::Debug for AuthSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthSession")
            .field("url", &self.url)
            .finish()
    }
}

starlark_simple_value!(AuthSession);

#[starlark_value(type = "aspect.AuthSession")]
impl<'v> values::StarlarkValue<'v> for AuthSession {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(auth_session_methods)
    }
}

#[starlark_module]
fn auth_session_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn url<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, AuthSession, url)
    }

    fn wait<'v>(this: values::Value<'v>) -> anyhow::Result<AuthCredentials> {
        let session = this
            .downcast_ref_err::<AuthSession>()
            .into_anyhow_result()?;
        let mut guard = session
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("auth session already consumed or poisoned"))?;
        let inner = guard.take().ok_or_else(|| {
            anyhow::anyhow!("auth session already consumed (wait() called twice)")
        })?;
        let entry = block_on(async move {
            let listener = inner
                .listener
                .ok_or_else(|| anyhow::anyhow!("no listener in auth session"))?;
            let (code, state) = accept_callback(listener).await?;
            // The Aspect-cloud flow posts to the conventional token endpoint; the
            // self-hosted flow resolves it via OIDC discovery and first validates
            // the callback `state` (CSRF guard).
            let token_url = match &inner.kind {
                SessionKind::Cloud => format!("{}/oauth/token", inner.env.domain),
                SessionKind::Endpoint { expected_state } => {
                    if state.as_deref() != Some(expected_state.as_str()) {
                        return Err(anyhow::anyhow!(
                            "authentication failed: callback state did not match"
                        ));
                    }
                    resolve_oidc_endpoints(&inner.env.domain).await.token
                }
            };
            let token_resp = exchange_code(
                &token_url,
                &inner.env.client_id,
                &inner.redirect_uri,
                &code,
                &inner.code_verifier,
            )
            .await?;
            let refresh_token = token_resp.refresh_token.clone();
            // Self-hosted edges validate the id_token; the cloud flow the
            // access_token. Record which so refresh keeps minting the same kind.
            let prefer_id_token = matches!(inner.kind, SessionKind::Endpoint { .. });
            CredentialsEntry::from_bearer(
                token_resp.bearer(prefer_id_token)?,
                refresh_token,
                Some(inner.env.domain.clone()),
                Some(inner.env.client_id.clone()),
                prefer_id_token,
            )
        })?;
        Ok(AuthCredentials::from_entry(&entry))
    }
}

/// One authorization server advertised by a deployment's discovery document,
/// exposed to the `configure` task so it can present the choice when more than one
/// is offered. `scopes` are informational for the task; the recorded deployment
/// carries them once an issuer is chosen.
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative, Clone)]
#[display("<aspect.AuthServerInfo>")]
pub struct AuthServerInfo {
    pub issuer: String,
    pub client_id: String,
    pub scopes: Vec<String>,
}

starlark_simple_value!(AuthServerInfo);

#[starlark_value(type = "aspect.AuthServerInfo")]
impl<'v> values::StarlarkValue<'v> for AuthServerInfo {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(auth_server_info_methods)
    }
}

#[starlark_module]
fn auth_server_info_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn issuer<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, AuthServerInfo, issuer)
    }

    #[starlark(attribute)]
    fn client_id<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, AuthServerInfo, client_id)
    }

    #[starlark(attribute)]
    fn scopes<'v>(
        this: values::Value<'v>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let scopes = this
            .downcast_ref_err::<AuthServerInfo>()
            .into_anyhow_result()?
            .scopes
            .clone();
        Ok(heap.alloc(scopes))
    }
}

/// The outcome of `ctx.aspect.auth.configure(host, issuer = ...)`, exposed to the
/// `configure` task so it can report what happened and decide whether to run login.
///
/// `status` distinguishes the outcomes; the two `auth_servers`-bearing statuses
/// record nothing (the task resolves the choice and re-invokes):
///   - `"ok"` — recorded; the other fields are populated.
///   - `"unreachable"` — a transport error; `reason` carries a terse description
///     and the host may still be a real deployment.
///   - `"not_a_deployment"` — reached the host, but it isn't an Aspect Workflows
///     endpoint.
///   - `"needs_issuer"` — the deployment advertised more than one authorization
///     server and no `issuer` was given; `auth_servers` lists the choices to
///     prompt from.
///   - `"issuer_not_advertised"` — the given `issuer` matches none of the
///     advertised servers; `auth_servers` lists the valid choices.
///
/// `can_login` is true when the recorded deployment has an issuer + client_id.
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative, Clone)]
#[display("<aspect.DeploymentInfo>")]
pub struct DeploymentInfo {
    pub status: String,
    pub reason: String,
    pub name: String,
    pub can_login: bool,
    pub is_default: bool,
    pub hosts: Vec<String>,
    /// The advertised authorization servers, populated for `"needs_issuer"` and
    /// `"issuer_not_advertised"` so the task can prompt from — or list — the valid
    /// choices.
    pub auth_servers: Vec<AuthServerInfo>,
}

starlark_simple_value!(DeploymentInfo);

#[starlark_value(type = "aspect.DeploymentInfo")]
impl<'v> values::StarlarkValue<'v> for DeploymentInfo {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(deployment_info_methods)
    }
}

#[starlark_module]
fn deployment_info_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn status<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, DeploymentInfo, status)
    }

    #[starlark(attribute)]
    fn reason<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, DeploymentInfo, reason)
    }

    #[starlark(attribute)]
    fn name<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, DeploymentInfo, name)
    }

    #[starlark(attribute)]
    fn can_login<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        attr_bool!(this, DeploymentInfo, can_login)
    }

    #[starlark(attribute)]
    fn is_default<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        attr_bool!(this, DeploymentInfo, is_default)
    }

    #[starlark(attribute)]
    fn hosts<'v>(
        this: values::Value<'v>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let hosts = this
            .downcast_ref_err::<DeploymentInfo>()
            .into_anyhow_result()?
            .hosts
            .clone();
        Ok(heap.alloc(hosts))
    }

    #[starlark(attribute)]
    fn auth_servers<'v>(
        this: values::Value<'v>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let servers = this
            .downcast_ref_err::<DeploymentInfo>()
            .into_anyhow_result()?
            .auth_servers
            .clone();
        Ok(heap.alloc(servers))
    }
}

/// One row of `ctx.aspect.auth.list()`: a configured deployment (or the built-in
/// Aspect seed) with everything the `auth status` task renders — whether it is the
/// current default, the stored credential's status/identity, the OAuth issuer,
/// and the advertised endpoints. Fields tied to a credential (`email`, `name`,
/// `status`) are empty when the deployment is logged out.
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative, Clone)]
#[display("<aspect.DeploymentSummary>")]
pub struct DeploymentSummary {
    pub name: String,
    pub default: bool,
    pub logged_in: bool,
    pub builtin: bool,
    /// Identity on the stored credential (empty when logged out).
    pub email: String,
    pub display_name: String,
    /// Human-readable token expiry (e.g. "expires in 2h 14m"), empty when logged out.
    pub status: String,
    /// The deployment's OAuth issuer host (empty for a deployment with no issuer).
    pub issuer: String,
    /// Advertised endpoints (bare hosts), empty when not advertised.
    pub cache: String,
    pub bes: String,
    pub exec: String,
}

starlark_simple_value!(DeploymentSummary);

#[starlark_value(type = "aspect.DeploymentSummary")]
impl<'v> values::StarlarkValue<'v> for DeploymentSummary {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(deployment_summary_methods)
    }
}

#[starlark_module]
fn deployment_summary_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn name<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, DeploymentSummary, name)
    }

    #[starlark(attribute)]
    fn default<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        attr_bool!(this, DeploymentSummary, default)
    }

    #[starlark(attribute)]
    fn logged_in<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        attr_bool!(this, DeploymentSummary, logged_in)
    }

    #[starlark(attribute)]
    fn builtin<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        attr_bool!(this, DeploymentSummary, builtin)
    }

    #[starlark(attribute)]
    fn email<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, DeploymentSummary, email)
    }

    #[starlark(attribute)]
    fn display_name<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, DeploymentSummary, display_name)
    }

    #[starlark(attribute)]
    fn status<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, DeploymentSummary, status)
    }

    #[starlark(attribute)]
    fn issuer<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, DeploymentSummary, issuer)
    }

    #[starlark(attribute)]
    fn cache<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, DeploymentSummary, cache)
    }

    #[starlark(attribute)]
    fn bes<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, DeploymentSummary, bes)
    }

    #[starlark(attribute)]
    fn exec<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, DeploymentSummary, exec)
    }
}

/// Build the summaries for `ctx.aspect.auth.list()`: the built-in Aspect account
/// (`builtin`) plus every configured deployment, each tagged with whether it is
/// the default and whether a credential is stored under its profile. The account
/// is always included (the `auth status` task renders it in its own section, logged
/// in or not); when no configured deployment is the default, the account is the
/// effective default (matching [`select_deployment`]).
fn list_deployment_summaries() -> anyhow::Result<Vec<DeploymentSummary>> {
    let deployments = load_deployments()?;
    let creds = load_all_credentials()?;
    let any_configured_default = deployments
        .iter()
        .any(|d| d.default && d.name != DEFAULT_DEPLOYMENT_NAME);
    Ok(deployments
        .iter()
        .map(|d| summarize_deployment(d, &creds, any_configured_default))
        .collect())
}

/// Build the [`DeploymentSummary`] for one deployment: default-marking (the seed
/// is default only when nothing configured claims it), logged-in status, the
/// credential-derived identity/expiry, the issuer host, and the endpoints.
fn summarize_deployment(
    d: &Deployment,
    creds: &HashMap<String, CredentialsEntry>,
    any_configured_default: bool,
) -> DeploymentSummary {
    // "built-in" is the Aspect seed specifically (by name), not any entry that
    // happens to carry no hosts.
    let builtin = d.name == DEFAULT_DEPLOYMENT_NAME;
    let entry = creds.get(&login_profile_for(d));
    let default = if builtin {
        !any_configured_default
    } else {
        d.default
    };
    // Identity + expiry come from the stored credential (when present).
    let (email, display_name, status) = match entry {
        Some(e) => {
            let exp = decode_jwt_claims(&e.access_token).ok().and_then(|c| c.exp);
            (e.email.clone(), e.name.clone(), format_token_status(exp))
        }
        None => (String::new(), String::new(), String::new()),
    };
    DeploymentSummary {
        name: d.name.clone(),
        default,
        logged_in: entry.is_some(),
        builtin,
        email,
        display_name,
        status,
        issuer: d
            .issuer
            .as_deref()
            .map(endpoint_host_str)
            .unwrap_or_default(),
        cache: d.endpoints.cache.clone(),
        bes: d.endpoints.bes.clone(),
        exec: d.endpoints.exec.clone(),
    }
}

/// The [`DeploymentSummary`] for the deployment named `name`, or `None` when no
/// such deployment is configured. Used by `configure` to render the recorded
/// deployment with the same detail as `list`.
fn one_deployment_summary(name: &str) -> anyhow::Result<Option<DeploymentSummary>> {
    let deployments = load_deployments()?;
    let creds = load_all_credentials()?;
    let any_configured_default = deployments
        .iter()
        .any(|d| d.default && d.name != DEFAULT_DEPLOYMENT_NAME);
    Ok(deployments
        .iter()
        .find(|d| d.name == name)
        .map(|d| summarize_deployment(d, &creds, any_configured_default)))
}

/// The resolved Bazel-facing endpoints for `ctx.aspect.auth.deployment_endpoints(name)`:
/// `cache` (→ --remote_cache), `bes` (→ the CLI BES sink / --bes_backend), and
/// `exec` (→ --remote_executor). Each is a bare host, or "" when the deployment
/// doesn't advertise/serve that capability. `name` is the resolved deployment
/// name. Consumed by bazel-spawning tasks' `--deployment` to auto-wire the flags.
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative, Clone)]
#[display("<aspect.DeploymentEndpoints>")]
pub struct DeploymentEndpoints {
    pub name: String,
    pub cache: String,
    pub bes: String,
    pub exec: String,
}

starlark_simple_value!(DeploymentEndpoints);

#[starlark_value(type = "aspect.DeploymentEndpoints")]
impl<'v> values::StarlarkValue<'v> for DeploymentEndpoints {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(deployment_endpoints_methods)
    }
}

#[starlark_module]
fn deployment_endpoints_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn name<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, DeploymentEndpoints, name)
    }

    #[starlark(attribute)]
    fn cache<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, DeploymentEndpoints, cache)
    }

    #[starlark(attribute)]
    fn bes<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, DeploymentEndpoints, bes)
    }

    #[starlark(attribute)]
    fn exec<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        attr_str!(this, DeploymentEndpoints, exec)
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<aspect.Auth>")]
pub struct Auth {}

starlark_simple_value!(Auth);

#[starlark_value(type = "aspect.Auth")]
impl<'v> values::StarlarkValue<'v> for Auth {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(auth_methods)
    }
}

#[starlark_module]
fn auth_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn api_url<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(resolve_api_url()?)
    }

    fn login<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named)] token: Option<&str>,
        #[starlark(require = named)] api_token: Option<&str>,
        #[starlark(require = named)] deployment: Option<&str>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let deployments = load_deployments()?;
        let selected = select_account_or_deployment(&deployments, deployment)?;
        let env = auth_env_from(&selected)?;

        if let Some(token) = token {
            let claims = decode_jwt_claims(token)?;
            let entry = CredentialsEntry {
                access_token: token.to_string(),
                refresh_token: String::new(),
                email: claims.email.unwrap_or_else(|| "api-token".to_string()),
                name: claims.name.unwrap_or_else(|| "Unknown".to_string()),
                tenant_id: claims.tenant_id,
                auth_domain: None,
                auth_client_id: None,
                prefer_id_token: false,
            };
            return Ok(heap.alloc(AuthCredentials::from_entry(&entry)));
        }

        if let Some(api_token) = api_token {
            let (client_id, secret) = api_token.split_once(':').ok_or_else(|| {
                anyhow::anyhow!("invalid API token format: expected client_id:secret")
            })?;
            let entry = block_on(exchange_api_token(client_id, secret, &env))?;
            return Ok(heap.alloc(AuthCredentials::from_entry(&entry)));
        }

        // Browser-based OAuth flow. A self-hosted deployment (one that advertises
        // endpoint hosts) uses the endpoint-callback flow: the endpoint forwards
        // the browser to this loopback, so its redirect is the endpoint's own
        // /oauth2/callback and the callback port travels in the OAuth `state`. The
        // built-in Aspect account carries no hosts and registers a
        // loopback redirect directly with the IdP. Keying on `hosts` rather than
        // the name keeps the seed on the cloud flow even if a `config.json` entry
        // overrides it by name.
        let session = match selected.hosts.first() {
            Some(host) => build_endpoint_session(env, host)?,
            None => build_cloud_session(env)?,
        };
        Ok(heap.alloc(session))
    }

    fn persist<'v>(
        #[allow(unused)] this: values::Value<'v>,
        creds: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] profile: NoneOr<String>,
    ) -> anyhow::Result<values::Value<'v>> {
        let creds = creds
            .downcast_ref_err::<AuthCredentials>()
            .into_anyhow_result()?;
        let profile = resolve_profile(profile.into_option().as_deref());
        let mut map = load_all_credentials()?;
        map.insert(profile, creds.to_entry());
        save_all_credentials(&map)?;
        Ok(values::Value::new_none())
    }

    /// Log out. `all` clears every stored credential. Otherwise the target is the
    /// deployment named `deployment`, or — when none is given — the effective
    /// default deployment (a configured default, else the built-in Aspect seed).
    /// An explicit `profile` overrides the resolved profile for advanced use.
    ///
    /// Logging out of the current default configured deployment also clears the
    /// default (there is no fall-through to another configured deployment): a
    /// later command with no `--deployment` then resolves the built-in Aspect
    /// seed, or errors if it needs a login. Returns the name that was logged out
    /// (empty for `all`).
    fn logout<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] deployment: NoneOr<String>,
        #[starlark(require = named, default = NoneOr::None)] profile: NoneOr<String>,
        #[starlark(require = named, default = false)] all: bool,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        if all {
            // Clear every stored profile (empties the file / deletes the keyring entry).
            save_all_credentials(&HashMap::new())?;
            return Ok(heap.alloc(""));
        }
        let deployments = load_deployments()?;
        let selected =
            select_account_or_deployment(&deployments, deployment.into_option().as_deref())?;
        let profile = match profile.into_option() {
            Some(p) if !p.is_empty() => p,
            _ => login_profile_for(&selected),
        };
        let mut map = load_all_credentials()?;
        map.remove(&profile);
        save_all_credentials(&map)?;
        // If the logged-out deployment was the current default, clear the default
        // rather than letting selection fall through to another deployment.
        if selected.default && selected.name != DEFAULT_DEPLOYMENT_NAME {
            set_default_deployment(None)?;
        }
        Ok(heap.alloc(selected.name))
    }

    /// List every effective deployment (built-in seed + configured) as
    /// [`DeploymentSummary`] rows for the `auth status` task.
    fn list<'v>(
        #[allow(unused)] this: values::Value<'v>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let summaries = list_deployment_summaries()?;
        Ok(heap.alloc(summaries))
    }

    /// The [`DeploymentSummary`] for one named deployment (same detail as a `list`
    /// row), or `None` when it isn't configured. Lets `configure` show the
    /// recorded deployment exactly as `list` renders it.
    fn deployment_summary<'v>(
        #[allow(unused)] this: values::Value<'v>,
        deployment: &str,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        Ok(match one_deployment_summary(deployment)? {
            Some(summary) => heap.alloc(summary),
            None => values::Value::new_none(),
        })
    }

    /// Make `deployment` the default (used when a command runs with no explicit
    /// `--deployment`). Selecting the built-in Aspect account is expressed as
    /// clearing every configured default. Errors if `deployment` names no
    /// configured deployment.
    fn set_default<'v>(
        #[allow(unused)] this: values::Value<'v>,
        deployment: &str,
        #[allow(unused)] heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        set_default_deployment(Some(deployment))?;
        Ok(values::Value::new_none())
    }

    /// Forget a configured deployment: drop its `~/.aspect/config.json` entry and
    /// its stored credential. Errors on the built-in Aspect account or an
    /// unknown name.
    fn remove<'v>(
        #[allow(unused)] this: values::Value<'v>,
        deployment: &str,
        #[allow(unused)] heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        remove_deployment(deployment)?;
        Ok(values::Value::new_none())
    }

    fn credentials<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] profile: NoneOr<String>,
        #[starlark(require = named, default = true)] required: bool,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let profile = resolve_profile(profile.into_option().as_deref());
        // Prefer an explicit ASPECT_API_TOKEN over stored credentials, but only for
        // the default profile: the token is exchanged against the Aspect account
        // issuer, so it can't stand in for a self-hosted deployment's profile.
        if profile == DEFAULT_PROFILE {
            if let Some(entry) = credentials_from_api_token_env()? {
                return Ok(heap.alloc(AuthCredentials::from_entry(&entry)));
            }
        }
        let mut map = load_all_credentials()?;
        let Some(entry) = map.get(&profile).cloned() else {
            return Ok(values::Value::new_none());
        };
        // Never hand back a known-expired token (an endpoint would 401 on it).
        // Mirror `resolve_access_token`: refresh when possible, else fail —
        // `required = False` callers (best-effort endpoint auth) get `None` and
        // proceed unauthenticated instead of a hard error.
        let entry = match classify_token(&entry) {
            TokenAction::Use => entry,
            TokenAction::Refresh => match block_on(refresh_access_token(&entry)) {
                Ok(refreshed) => {
                    map.insert(profile.clone(), refreshed.clone());
                    save_all_credentials(&map)?;
                    refreshed
                }
                Err(_) if !required => return Ok(values::Value::new_none()),
                Err(_) => return Err(anyhow::anyhow!(session_expired_message(&profile))),
            },
            TokenAction::Expired if !required => return Ok(values::Value::new_none()),
            TokenAction::Expired => return Err(anyhow::anyhow!(session_expired_message(&profile))),
        };
        Ok(heap.alloc(AuthCredentials::from_entry(&entry)))
    }

    /// Discover a deployment's auth config from its
    /// `/.well-known/oauth-protected-resource` document and record it in
    /// `~/.aspect/config.json` (replacing any entry of the same derived/`name`
    /// deployment). Returns a [`DeploymentInfo`] the caller uses to report the
    /// result and decide whether to run the interactive login.
    ///
    /// `name` overrides the host-derived deployment name. `make_default` forces
    /// this deployment to become the default (taking the crown from any other);
    /// otherwise it becomes the default only when none is set yet — a second
    /// configure never hijacks it. `issuer` selects among advertised authorization
    /// servers ([`resolve_auth_server`]): when set it must match an advertised one
    /// (else `"issuer_not_advertised"`, records nothing), and it is required only
    /// when the deployment advertises more than one — an unset `issuer` there
    /// records nothing and returns `"needs_issuer"` for the task to resolve.
    fn configure<'v>(
        #[allow(unused)] this: values::Value<'v>,
        host: &str,
        #[starlark(require = named, default = NoneOr::None)] name: NoneOr<String>,
        #[starlark(require = named, default = false)] make_default: bool,
        #[starlark(require = named, default = NoneOr::None)] issuer: NoneOr<String>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let host = endpoint_host_str(host);
        // Distinguish the user-facing failure modes (unreachable vs. not an Aspect
        // Workflows deployment) so the task prints the right guidance rather than a
        // Starlark traceback. An `aspect_endpoints` map is the definitive marker of
        // a deployment — probe_protected_resource requires it.
        let mk = |status: &str, reason: String| DeploymentInfo {
            status: status.to_string(),
            reason,
            name: String::new(),
            can_login: false,
            is_default: false,
            hosts: Vec::new(),
            auth_servers: Vec::new(),
        };
        let info = match block_on(probe_protected_resource(&host)) {
            Discovery::Reachable(info) => info,
            Discovery::Unreachable(reason) => return Ok(heap.alloc(mk("unreachable", reason))),
            Discovery::NotADeployment => {
                return Ok(heap.alloc(mk("not_a_deployment", String::new())));
            }
        };
        let name = name
            .into_option()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| deployment_name_from_host(&host));
        let requested = issuer.into_option().filter(|s| !s.is_empty());
        let candidates = info.auth_servers();
        // Hand the advertised choices back so the task can prompt from them or list
        // the valid values when `--issuer` names one that isn't advertised.
        let with_candidates = |status: &str| DeploymentInfo {
            status: status.to_string(),
            reason: String::new(),
            name: name.clone(),
            can_login: false,
            is_default: false,
            hosts: Vec::new(),
            auth_servers: candidates.iter().cloned().map(auth_server_info).collect(),
        };
        let selected = match resolve_auth_server(&candidates, requested.as_deref()) {
            AuthServerChoice::Ambiguous => return Ok(heap.alloc(with_candidates("needs_issuer"))),
            AuthServerChoice::NotAdvertised => {
                return Ok(heap.alloc(with_candidates("issuer_not_advertised")));
            }
            AuthServerChoice::Selected(s) => Some(s),
            AuthServerChoice::None => None,
        };
        let mut deployment = deployment_from_discovery(name.clone(), &host, &info, selected.as_ref());
        // `--default` forces this deployment to take the default crown (upsert
        // then clears it on every other entry); without it, upsert only sets the
        // default when none exists yet, so a second configure doesn't hijack it.
        deployment.default = make_default;
        let can_login = deployment.issuer.is_some() && deployment.client_id.is_some();
        let hosts = deployment.hosts.clone();
        let is_default = save_user_deployment(deployment)?;
        Ok(heap.alloc(DeploymentInfo {
            status: "ok".to_string(),
            reason: String::new(),
            name,
            can_login,
            is_default,
            hosts,
            auth_servers: Vec::new(),
        }))
    }

    /// The Bazel-facing endpoints (`cache`/`bes`/`exec` hosts) advertised by the
    /// deployment named `deployment` (or the default when omitted), so
    /// `aspect <build|test> --deployment <name>` can auto-wire --remote_cache,
    /// the BES backend, and --remote_executor. Errors if the deployment names no
    /// configured deployment. A deployment that advertised no endpoints (an older
    /// discovery, or the built-in Aspect seed) returns empty strings — the task
    /// then injects nothing and relies on the user's own flags.
    fn deployment_endpoints<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] deployment: NoneOr<String>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let deployments = load_deployments()?;
        let selected = select_deployment(&deployments, deployment.into_option().as_deref())?;
        Ok(heap.alloc(DeploymentEndpoints {
            name: selected.name,
            cache: selected.endpoints.cache,
            bes: selected.endpoints.bes,
            exec: selected.endpoints.exec,
        }))
    }

    /// The name of the configured deployment that owns `host` (its advertised
    /// cache/bes/exec host), or `None` when none claims it. Endpoint auth uses
    /// this to attach a deployment's own login credential to its endpoints (the
    /// deployment name is also the credential-store key it logged in under).
    fn deployment_for_host<'v>(
        #[allow(unused)] this: values::Value<'v>,
        host: &str,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let deployments = load_deployments()?;
        Ok(match deployment_name_for_host(&deployments, host) {
            Some(name) => heap.alloc(name),
            None => values::Value::new_none(),
        })
    }

    /// The credentials profile a login against `deployment` should persist under,
    /// so it matches what [`Self::deployment_for_host`] later resolves for that
    /// deployment's endpoints. A configured deployment (one with its own hosts)
    /// stores under its name; the built-in Aspect account — whose endpoints go
    /// through the default profile — stores under the resolved default profile.
    fn login_profile<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] deployment: NoneOr<String>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let deployments = load_deployments()?;
        let selected =
            select_account_or_deployment(&deployments, deployment.into_option().as_deref())?;
        Ok(heap.alloc(login_profile_for(&selected)))
    }
}

/// The credentials profile a login against `selected` persists under: a
/// self-hosted deployment (one with its own endpoint hosts) stores under its
/// name so [`Auth::deployment_for_host`] later resolves the same profile for its
/// endpoints; the built-in Aspect account (no hosts) stores under the
/// resolved default profile. Keyed on `hosts` — not the name — to match the
/// cloud/endpoint flow split in [`Auth::login`].
fn login_profile_for(selected: &Deployment) -> String {
    if selected.hosts.is_empty() {
        resolve_profile(None)
    } else {
        selected.name.clone()
    }
}

/// Extract a bare host from a possibly-URL argument (`https://h/p` → `h`), so
/// `configure` accepts either a host or a full endpoint URL. This is the Rust
/// twin of the Starlark `endpoint_host` in `aspect_endpoint_auth.axl`; the two
/// implement the same parse (strip scheme/userinfo/port/path, lowercase) and
/// must stay in sync — neither handles IPv6 literals (Aspect hosts are DNS names).
fn endpoint_host_str(arg: &str) -> String {
    let mut rest = arg;
    if let Some(idx) = rest.find("://") {
        rest = &rest[idx + 3..];
    }
    rest = rest.split('/').next().unwrap_or(rest);
    if let Some(idx) = rest.rfind('@') {
        rest = &rest[idx + 1..];
    }
    if let Some(idx) = rest.rfind(':') {
        rest = &rest[..idx];
    }
    rest.to_lowercase()
}

// Registers the auth value types usable as Starlark type annotations. Only types
// a caller annotates need registering — `DeploymentSummary` / `DeploymentEndpoints`
// are only ever return values (never annotated), so they're deliberately omitted.
#[starlark_module]
fn register_auth_types(globals: &mut GlobalsBuilder) {
    const Auth: StarlarkValueAsType<Auth> = StarlarkValueAsType::new();
    const AuthSession: StarlarkValueAsType<AuthSession> = StarlarkValueAsType::new();
    const AuthCredentials: StarlarkValueAsType<AuthCredentials> = StarlarkValueAsType::new();
    const DeploymentInfo: StarlarkValueAsType<DeploymentInfo> = StarlarkValueAsType::new();
}

pub fn register_globals(globals: &mut GlobalsBuilder) {
    globals.namespace("auth", register_auth_types);
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    use super::*;

    /// A JWT whose payload is `{"exp": <exp>}` (header and signature are
    /// placeholders; only the payload is decoded). `exp` of `None` omits the
    /// claim entirely.
    fn jwt_with_exp(exp: Option<u64>) -> String {
        let payload = match exp {
            Some(exp) => format!("{{\"exp\":{exp}}}"),
            None => "{}".to_string(),
        };
        format!("header.{}.sig", URL_SAFE_NO_PAD.encode(payload))
    }

    fn entry(access_token: String) -> CredentialsEntry {
        CredentialsEntry {
            access_token,
            refresh_token: String::new(),
            email: "user@example.com".to_string(),
            name: "User".to_string(),
            tenant_id: "tenant".to_string(),
            auth_domain: None,
            auth_client_id: None,
            prefer_id_token: false,
        }
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    #[test]
    fn is_expired_jwt_classifies_by_exp() {
        // A token well in the future is current.
        assert!(!is_expired_jwt(&entry(jwt_with_exp(Some(now() + 3600)))));
        // One in the past is expired, as is one inside the 60s pre-expiry buffer.
        assert!(is_expired_jwt(&entry(jwt_with_exp(Some(now() - 1)))));
        assert!(is_expired_jwt(&entry(jwt_with_exp(Some(now() + 30)))));
        // No `exp` claim means non-expiring.
        assert!(!is_expired_jwt(&entry(jwt_with_exp(None))));
    }

    #[test]
    fn is_expired_jwt_treats_malformed_tokens_as_expired() {
        assert!(is_expired_jwt(&entry("not-a-jwt".to_string())));
        assert!(is_expired_jwt(&entry("only.two".to_string())));
        assert!(is_expired_jwt(&entry("a.!!!.c".to_string())));
    }

    #[test]
    fn can_refresh_requires_token_domain_and_client_id() {
        let mut e = entry(jwt_with_exp(Some(now() - 1)));
        assert!(!can_refresh(&e));
        e.refresh_token = "refresh".to_string();
        assert!(!can_refresh(&e));
        e.auth_domain = Some("https://auth.example".to_string());
        assert!(!can_refresh(&e));
        e.auth_client_id = Some("client".to_string());
        assert!(can_refresh(&e));
    }

    #[test]
    fn resolve_profile_prefers_explicit_then_env_then_default() {
        // An explicit, non-empty profile always wins, regardless of the env.
        assert_eq!(resolve_profile(Some("ci")), "ci");

        // This test mutates the process env, so it must run serially with any
        // other PROFILE_ENV reader; it is currently the only one.
        let saved = std::env::var(PROFILE_ENV).ok();

        // SAFETY: single-threaded test body; restored before returning.
        unsafe { std::env::set_var(PROFILE_ENV, "from-env") };
        // An absent or empty explicit profile falls through to the env var.
        assert_eq!(resolve_profile(None), "from-env");
        assert_eq!(resolve_profile(Some("")), "from-env");
        // An explicit profile still overrides the env.
        assert_eq!(resolve_profile(Some("explicit")), "explicit");

        // With the env unset, everything falls through to the default.
        unsafe { std::env::remove_var(PROFILE_ENV) };
        assert_eq!(resolve_profile(None), DEFAULT_PROFILE);
        assert_eq!(resolve_profile(Some("")), DEFAULT_PROFILE);

        match saved {
            Some(v) => unsafe { std::env::set_var(PROFILE_ENV, v) },
            None => unsafe { std::env::remove_var(PROFILE_ENV) },
        }
    }

    #[test]
    fn classify_token_covers_each_branch() {
        // Current token: use as-is.
        assert!(matches!(
            classify_token(&entry(jwt_with_exp(Some(now() + 3600)))),
            TokenAction::Use
        ));
        // Expired and not refreshable: error (never returned as-is).
        assert!(matches!(
            classify_token(&entry(jwt_with_exp(Some(now() - 1)))),
            TokenAction::Expired
        ));
        // Expired but refreshable: refresh.
        let mut refreshable = entry(jwt_with_exp(Some(now() - 1)));
        refreshable.refresh_token = "refresh".to_string();
        refreshable.auth_domain = Some("https://auth.example".to_string());
        refreshable.auth_client_id = Some("client".to_string());
        assert!(matches!(classify_token(&refreshable), TokenAction::Refresh));
    }

    fn dep(name: &str, default: bool) -> Deployment {
        Deployment {
            name: name.to_string(),
            default,
            issuer: Some(format!("https://{}.auth.aspect.build", name)),
            client_id: Some(format!("client-{}", name)),
            api_url: None,
            hosts: vec![format!("remote.{}.aspect.build", name)],
            scopes: Vec::new(),
            endpoints: Endpoints::default(),
        }
    }

    #[test]
    fn select_deployment_by_explicit_name() {
        let deployments = vec![default_deployment(), dep("acme", false)];
        assert_eq!(
            select_deployment(&deployments, Some("acme")).unwrap().name,
            "acme"
        );
        assert!(select_deployment(&deployments, Some("nope")).is_err());
    }

    #[test]
    fn select_deployment_with_no_name_uses_the_default_flag() {
        // Only the seed → the seed.
        let seed_only = vec![default_deployment()];
        assert_eq!(
            select_deployment(&seed_only, None).unwrap().name,
            DEFAULT_DEPLOYMENT_NAME
        );

        // The entry marked default wins (upsert makes the first configured entry
        // default, so a single configured deployment is selected here).
        let mut one = vec![default_deployment(), dep("acme", true)];
        one[0].default = false;
        assert_eq!(select_deployment(&one, None).unwrap().name, "acme");

        // An explicit default on the seed is respected even with a configured
        // deployment present — configuring does not implicitly hijack selection.
        let explicit_seed = vec![default_deployment(), dep("acme", false)];
        assert_eq!(
            select_deployment(&explicit_seed, None).unwrap().name,
            DEFAULT_DEPLOYMENT_NAME
        );
    }

    #[test]
    fn select_account_or_deployment_defaults_to_the_account_not_the_default_deployment() {
        // Even with a configured deployment marked default (the build default),
        // a bare auth login/logout targets the Aspect account (the seed) — the
        // default-deployment concept governs builds, not the account.
        let mut ds = vec![default_deployment(), dep("acme", true)];
        ds[0].default = false;
        assert_eq!(
            select_account_or_deployment(&ds, None).unwrap().name,
            DEFAULT_DEPLOYMENT_NAME
        );
        // An explicit name still picks that deployment.
        assert_eq!(
            select_account_or_deployment(&ds, Some("acme"))
                .unwrap()
                .name,
            "acme"
        );
        // Unknown name errors.
        assert!(select_account_or_deployment(&ds, Some("nope")).is_err());
    }

    #[test]
    fn login_hint_names_the_deployment_except_for_the_default() {
        // The default profile → bare login (no deployment assumed).
        assert_eq!(login_hint(DEFAULT_PROFILE), "aspect auth login");
        // A configured deployment's profile is its name → explicit --deployment.
        assert_eq!(
            login_hint("gcp.acme"),
            "aspect auth login --deployment gcp.acme"
        );
        assert!(session_expired_message("gcp.acme").contains("--deployment gcp.acme"));
        assert!(!session_expired_message(DEFAULT_PROFILE).contains("--deployment"));
    }

    #[test]
    fn oidc_endpoints_fallback_uses_conventional_paths() {
        let e = oidc_endpoints_fallback("https://acme.auth.aspect.build");
        assert_eq!(
            e.authorize,
            "https://acme.auth.aspect.build/oauth/authorize"
        );
        assert_eq!(e.token, "https://acme.auth.aspect.build/oauth/token");
    }

    #[test]
    fn login_profile_keys_on_hosts_not_name() {
        // The built-in seed (no hosts) → the default profile...
        assert_eq!(login_profile_for(&default_deployment()), DEFAULT_PROFILE);
        // ...even when a config.json entry overrides the seed by name but is
        // still the hostless cloud deployment.
        let mut seed_override = default_deployment();
        seed_override.hosts = Vec::new();
        assert_eq!(login_profile_for(&seed_override), DEFAULT_PROFILE);
        // A self-hosted deployment (has hosts) → its own name.
        assert_eq!(login_profile_for(&dep("acme", true)), "acme");
    }

    #[test]
    fn deployment_name_from_host_keeps_subdomain_minus_first_label() {
        // Drop the first label, keep the rest of the subdomain (down to the
        // registrable domain).
        assert_eq!(
            deployment_name_from_host("remote.acme.aspect.build"),
            "acme"
        );
        assert_eq!(deployment_name_from_host("bes.acme.aspect.build"), "acme");
        assert_eq!(
            deployment_name_from_host("bes.gcp.awd-gha-test-dev.aspect.build"),
            "gcp.awd-gha-test-dev"
        );
        assert_eq!(deployment_name_from_host("cache.app.corp.io"), "app");

        // Only the registrable domain remains after dropping the first label →
        // name it after the domain.
        assert_eq!(
            deployment_name_from_host("remote.aspect.build"),
            "aspect.build"
        );
        assert_eq!(deployment_name_from_host("cache.corp.io"), "corp.io");

        // Multi-part public suffix (.co.uk) is handled by the PSL: the
        // registrable domain is acme.co.uk, so the middle labels survive.
        assert_eq!(deployment_name_from_host("cache.team.acme.co.uk"), "team");

        // An IP literal is used verbatim (no subdomain to strip).
        assert_eq!(deployment_name_from_host("192.168.1.1"), "192.168.1.1");

        // A single label has nothing to strip.
        assert_eq!(deployment_name_from_host("localhost"), "localhost");
    }

    fn nested(issuer: &str, client_id: &str) -> AspectAuthServer {
        AspectAuthServer {
            issuer: issuer.to_string(),
            client_id: client_id.to_string(),
            scopes: vec!["openid".to_string()],
        }
    }

    /// A discovery document serving `cache` at `remote.acme.aspect.build` with the
    /// given authorization servers (nested current shape or legacy flat fields).
    fn protected_resource(
        aspect_authorization_servers: Vec<AspectAuthServer>,
        authorization_servers: Vec<&str>,
        client_id: &str,
        scopes_supported: Vec<&str>,
    ) -> ProtectedResource {
        ProtectedResource {
            resource: "https://remote.acme.aspect.build".to_string(),
            aspect_authorization_servers,
            authorization_servers: authorization_servers.iter().map(|s| s.to_string()).collect(),
            client_id: client_id.to_string(),
            scopes_supported: scopes_supported.iter().map(|s| s.to_string()).collect(),
            aspect_endpoints: Endpoints {
                cache: "remote.acme.aspect.build".to_string(),
                ..Default::default()
            },
        }
    }

    fn configure_from(info: &ProtectedResource, issuer: Option<&str>) -> Deployment {
        let candidates = info.auth_servers();
        let selected = match resolve_auth_server(&candidates, issuer) {
            AuthServerChoice::Selected(s) => Some(s),
            AuthServerChoice::None => None,
            other => panic!(
                "expected a selectable auth server, got {}",
                match other {
                    AuthServerChoice::Ambiguous => "ambiguous",
                    AuthServerChoice::NotAdvertised => "not-advertised",
                    _ => unreachable!(),
                }
            ),
        };
        deployment_from_discovery("acme".to_string(), "remote.acme.aspect.build", info, selected.as_ref())
    }

    #[test]
    fn issuers_match_ignores_scheme_slash_and_case() {
        assert!(issuers_match(
            "https://auth.dev.aspect.build",
            "auth.dev.aspect.build"
        ));
        assert!(issuers_match(
            "https://auth.dev.aspect.build",
            "https://auth.dev.aspect.build/"
        ));
        assert!(issuers_match(
            "https://Auth.Dev.Aspect.Build",
            "http://auth.dev.aspect.build"
        ));
        assert!(!issuers_match(
            "https://auth.dev.aspect.build",
            "https://auth.prod.aspect.build"
        ));
    }

    #[test]
    fn deployment_from_discovery_records_issuer_client_and_all_hosts() {
        // A deployment advertising an authorization server + client_id + endpoints
        // via the legacy flat shape: recorded and loggable. hosts = configured host
        // ∪ resource ∪ every endpoint host, deduped; the typed endpoints map is
        // preserved (and its hosts are normalized to bare hosts).
        let advertised = ProtectedResource {
            resource: "https://remote.acme.aspect.build".to_string(),
            aspect_authorization_servers: vec![],
            authorization_servers: vec!["https://acme.auth.aspect.build".to_string()],
            client_id: "abc".to_string(),
            scopes_supported: vec!["openid".to_string(), "groups".to_string()],
            aspect_endpoints: Endpoints {
                cache: "https://remote.acme.aspect.build".to_string(),
                bes: "bes.acme.aspect.build".to_string(),
                exec: "https://remote.acme.aspect.build".to_string(),
            },
        };
        let d = configure_from(&advertised, None);
        assert_eq!(d.issuer.as_deref(), Some("https://acme.auth.aspect.build"));
        assert_eq!(d.client_id.as_deref(), Some("abc"));
        // A BYO-IdP scope override is recorded verbatim...
        assert_eq!(d.scopes, vec!["openid".to_string(), "groups".to_string()]);
        // ...and drives the login flow, with offline_access guaranteed so the
        // credential can refresh even when the IdP omits it from scopes_supported.
        assert_eq!(
            auth_env_from(&d).unwrap().scopes,
            vec![
                "openid".to_string(),
                "groups".to_string(),
                "offline_access".to_string()
            ]
        );
        assert_eq!(
            d.hosts,
            vec![
                "remote.acme.aspect.build".to_string(),
                "bes.acme.aspect.build".to_string()
            ]
        );
        assert_eq!(d.endpoints.cache, "remote.acme.aspect.build");
        assert_eq!(d.endpoints.bes, "bes.acme.aspect.build");
        assert_eq!(d.endpoints.exec, "remote.acme.aspect.build");
        assert!(auth_env_from(&d).is_ok());

        // An endpoint that advertises no authorization server: hosts recorded, but
        // not loggable. No exec advertised → exec stays empty.
        let bare = ProtectedResource {
            resource: "https://remote.acme.aspect.build".to_string(),
            aspect_authorization_servers: vec![],
            authorization_servers: vec![],
            client_id: String::new(),
            scopes_supported: vec![],
            aspect_endpoints: Endpoints {
                cache: "remote.acme.aspect.build".to_string(),
                bes: "bes.acme.aspect.build".to_string(),
                exec: String::new(),
            },
        };
        let d = configure_from(&bare, None);
        assert!(d.issuer.is_none() && d.client_id.is_none());
        assert_eq!(
            d.hosts,
            vec![
                "remote.acme.aspect.build".to_string(),
                "bes.acme.aspect.build".to_string()
            ]
        );
        assert!(d.endpoints.exec.is_empty());
        assert!(auth_env_from(&d).is_err());
    }

    #[test]
    fn auth_servers_prefers_nested_over_legacy_flat_fields() {
        // The current nested shape wins over the flat legacy fields (which a
        // transitional server may still populate alongside).
        let info = protected_resource(
            vec![AspectAuthServer {
                issuer: "https://auth.dev.aspect.build".to_string(),
                client_id: "nested-client".to_string(),
                scopes: vec!["openid".to_string(), "email".to_string()],
            }],
            vec!["https://legacy.auth.aspect.build"],
            "legacy-client",
            vec!["openid"],
        );
        let servers = info.auth_servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].issuer, "https://auth.dev.aspect.build");
        assert_eq!(servers[0].client_id, "nested-client");
        assert_eq!(servers[0].scopes, vec!["openid".to_string(), "email".to_string()]);
    }

    #[test]
    fn auth_servers_skips_issuerless_nested_entries() {
        // Issuer-less nested entries are dropped; a valid later entry still surfaces.
        let info = protected_resource(
            vec![
                nested("", "skip-me"),
                nested("https://auth.dev.aspect.build", "good-client"),
            ],
            vec![],
            "",
            vec![],
        );
        let servers = info.auth_servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].client_id, "good-client");
    }

    #[test]
    fn auth_servers_falls_back_to_legacy_when_no_usable_nested_entry() {
        // A nested list with no issuer-bearing entry falls back to the flat fields.
        let info = protected_resource(
            vec![nested("", "nested-client")],
            vec!["https://legacy.auth.aspect.build"],
            "legacy-client",
            vec!["openid"],
        );
        let servers = info.auth_servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].issuer, "https://legacy.auth.aspect.build");
        assert_eq!(servers[0].client_id, "legacy-client");
        assert_eq!(servers[0].scopes, vec!["openid".to_string()]);
    }

    #[test]
    fn resolve_auth_server_selects_sole_candidate_without_a_request() {
        let info = protected_resource(
            vec![nested("https://auth.dev.aspect.build", "the-client")],
            vec![],
            "",
            vec![],
        );
        let d = configure_from(&info, None);
        assert_eq!(d.issuer.as_deref(), Some("https://auth.dev.aspect.build"));
        assert_eq!(d.client_id.as_deref(), Some("the-client"));
    }

    #[test]
    fn resolve_auth_server_is_ambiguous_when_multiple_and_no_request() {
        let info = protected_resource(
            vec![
                nested("https://auth.dev.aspect.build", "first"),
                nested("https://auth.corp.example.com", "second"),
            ],
            vec![],
            "",
            vec![],
        );
        assert!(matches!(
            resolve_auth_server(&info.auth_servers(), None),
            AuthServerChoice::Ambiguous
        ));
    }

    #[test]
    fn resolve_auth_server_picks_requested_issuer_among_many() {
        // The requested issuer selects its paired client_id, host-tolerantly (no
        // scheme), even when it isn't the first advertised.
        let info = protected_resource(
            vec![
                nested("https://auth.dev.aspect.build", "first"),
                nested("https://auth.corp.example.com", "second"),
            ],
            vec![],
            "",
            vec![],
        );
        let d = configure_from(&info, Some("auth.corp.example.com"));
        assert_eq!(d.issuer.as_deref(), Some("https://auth.corp.example.com"));
        assert_eq!(d.client_id.as_deref(), Some("second"));
    }

    #[test]
    fn resolve_auth_server_rejects_unadvertised_issuer_even_when_sole() {
        // A requested issuer that isn't advertised is rejected rather than ignored,
        // even when the deployment offers exactly one server.
        let info = protected_resource(
            vec![nested("https://auth.dev.aspect.build", "the-client")],
            vec![],
            "",
            vec![],
        );
        assert!(matches!(
            resolve_auth_server(&info.auth_servers(), Some("https://auth.prod.aspect.build")),
            AuthServerChoice::NotAdvertised
        ));
    }

    #[test]
    fn auth_env_trims_trailing_slash() {
        let d = Deployment {
            name: "acme".to_string(),
            default: false,
            issuer: Some("https://acme.auth.aspect.build/".to_string()),
            client_id: Some("abc".to_string()),
            api_url: None,
            hosts: vec![],
            scopes: vec![],
            endpoints: Endpoints::default(),
        };
        // No advertised scopes → the standard OIDC login set.
        assert_eq!(auth_env_from(&d).unwrap().scopes, DEFAULT_LOGIN_SCOPES);
        assert_eq!(
            auth_env_from(&d).unwrap().domain,
            "https://acme.auth.aspect.build"
        );
    }

    /// A JWT whose payload is the given JSON object literal (header/sig are
    /// placeholders; only the payload is decoded).
    fn jwt_with_payload(payload: &str) -> String {
        format!("header.{}.sig", URL_SAFE_NO_PAD.encode(payload))
    }

    fn token_response(access_token: &str, id_token: &str) -> TokenResponse {
        TokenResponse {
            access_token: access_token.into(),
            id_token: id_token.into(),
            refresh_token: String::new(),
        }
    }

    #[test]
    fn cloud_bearer_prefers_access_token_then_id_token() {
        // The Aspect-cloud flow (prefer_id_token = false) validates the
        // access_token, falling back to the id_token only if absent.
        assert_eq!(token_response("acc", "idt").bearer(false).unwrap(), "acc");
        assert_eq!(token_response("", "idt").bearer(false).unwrap(), "idt");
        assert!(token_response("", "").bearer(false).is_err());
    }

    #[test]
    fn endpoint_bearer_requires_id_token_and_never_downgrades() {
        // The self-hosted endpoint flow (prefer_id_token = true) validates the
        // id_token, so bearer() must return it even when an access_token is also
        // present...
        assert_eq!(token_response("acc", "idt").bearer(true).unwrap(), "idt");
        // ...and must error rather than downgrade to the access_token when a
        // (refresh) grant drops the id_token — the edge would 401 on it.
        assert!(token_response("acc", "").bearer(true).is_err());
    }

    #[test]
    fn from_bearer_decodes_claims_and_defaults() {
        let jwt = jwt_with_payload(r#"{"email":"u@x.io","name":"U","tenantId":"t1"}"#);
        let e = CredentialsEntry::from_bearer(
            jwt.clone(),
            "refresh".into(),
            Some("https://acme.auth.aspect.build".into()),
            Some("abc".into()),
            true,
        )
        .unwrap();
        assert_eq!(e.access_token, jwt);
        assert_eq!(e.email, "u@x.io");
        assert_eq!(e.tenant_id, "t1");
        assert!(e.prefer_id_token);
        assert_eq!(
            e.auth_domain.as_deref(),
            Some("https://acme.auth.aspect.build")
        );

        // A self-hosted id_token without email/tenantId decodes with empty
        // defaults (display-only) rather than failing.
        let bare = jwt_with_payload(r#"{"sub":"user-1"}"#);
        let e = CredentialsEntry::from_bearer(bare, String::new(), None, None, false).unwrap();
        assert_eq!(e.email, "");
        assert_eq!(e.tenant_id, "");
        assert_eq!(e.name, "Unknown");
    }

    #[test]
    fn prefer_id_token_defaults_false_for_legacy_entries() {
        // Entries serialized before `prefer_id_token` existed omit the field; they
        // must load as cloud (access_token) sessions, not error.
        let json = r#"{"access_token":"a","email":"u@x.io","name":"U","tenant_id":"t"}"#;
        let e: CredentialsEntry = serde_json::from_str(json).unwrap();
        assert!(!e.prefer_id_token);
        // And it is omitted from the serialized form when false (no churn).
        let out = serde_json::to_string(&e).unwrap();
        assert!(!out.contains("prefer_id_token"));
    }

    #[test]
    fn build_authorize_url_encodes_params_and_handles_query_and_state() {
        let scopes = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // No prior query → `?` separator; no state; scopes space-joined+encoded.
        let cloud = build_authorize_url(
            "https://auth.aspect.build/oauth/authorize",
            "client-1",
            "http://localhost:19556/callback",
            &scopes(&["openid", "profile", "email"]),
            "chal",
            None,
        );
        assert!(cloud.starts_with("https://auth.aspect.build/oauth/authorize?client_id=client-1&"));
        assert!(cloud.contains("redirect_uri=http%3A%2F%2Flocalhost%3A19556%2Fcallback"));
        assert!(cloud.contains("scope=openid%20profile%20email"));
        assert!(cloud.contains("code_challenge_method=S256"));
        assert!(!cloud.contains("state="));

        // Discovery endpoint that already has a query → `&` separator; state appended.
        let with_state = build_authorize_url(
            "https://acme.auth.aspect.build/authorize?foo=bar",
            "c/2", // reserved char must be encoded
            "https://remote.acme.aspect.build/oauth2/callback",
            &scopes(&["openid", "offline_access"]),
            "chal",
            Some("nonce.5000"),
        );
        assert!(with_state.contains("/authorize?foo=bar&client_id=c%2F2"));
        assert!(with_state.contains("scope=openid%20offline_access"));
        assert!(with_state.ends_with("&state=nonce.5000"));
    }

    #[test]
    fn login_state_encodes_port_after_dot() {
        let s = login_state("abc123", 54321);
        assert_eq!(s, "abc123.54321");
        // The port is recoverable as the segment after the last dot.
        assert_eq!(s.rsplit('.').next(), Some("54321"));
        // A base64url nonce never contains the dot separator.
        assert!(!generate_code_verifier().contains('.'));
    }

    #[test]
    fn endpoint_host_str_extracts_bare_host() {
        assert_eq!(
            endpoint_host_str("https://remote.acme.aspect.build/x"),
            "remote.acme.aspect.build"
        );
        assert_eq!(
            endpoint_host_str("remote.acme.aspect.build:443"),
            "remote.acme.aspect.build"
        );
        assert_eq!(endpoint_host_str("user@Host.ACME.io"), "host.acme.io");
    }

    #[test]
    fn deployment_name_for_host_matches_owning_deployment() {
        let deployments = vec![default_deployment(), dep("acme", false)];
        // dep("acme") owns remote.acme.aspect.build.
        assert_eq!(
            deployment_name_for_host(&deployments, "remote.acme.aspect.build").as_deref(),
            Some("acme")
        );
        // Case-insensitive and dot-anchored: a sub-host of the owned host matches,
        // a lookalike does not.
        assert_eq!(
            deployment_name_for_host(&deployments, "x.Remote.ACME.aspect.build").as_deref(),
            Some("acme")
        );
        assert_eq!(
            deployment_name_for_host(&deployments, "notremote.acme.aspect.build"),
            None
        );
        // An absolute FQDN (trailing dot) still matches its owning deployment.
        assert_eq!(
            deployment_name_for_host(&deployments, "remote.acme.aspect.build.").as_deref(),
            Some("acme")
        );
        // A host no configured deployment claims → None (no token attached).
        assert_eq!(
            deployment_name_for_host(&deployments, "bes.aspect.build"),
            None
        );
    }

    #[test]
    fn upsert_preserves_default_when_replacing_current_default() {
        // Two deployments, `acme` is the default. Re-configuring `acme` with an
        // incoming record that does not claim default must keep it the default.
        let mut existing = vec![dep("acme", true), dep("emca", false)];
        let mut incoming = dep("acme", false);
        incoming.issuer = Some("https://acme.auth.aspect.build/v2".to_string());
        assert!(upsert_deployment(&mut existing, incoming));
        let acme = existing.iter().find(|d| d.name == "acme").unwrap();
        assert!(acme.default, "re-configured current default stays default");
        assert_eq!(existing.iter().filter(|d| d.default).count(), 1);
    }

    #[test]
    fn upsert_first_entry_and_explicit_default() {
        // First configured entry becomes default even without claiming it.
        let mut existing: Vec<Deployment> = vec![];
        assert!(upsert_deployment(&mut existing, dep("acme", false)));

        // A later entry claiming default steals it from the previous one.
        assert!(upsert_deployment(&mut existing, dep("emca", true)));
        assert_eq!(existing.iter().filter(|d| d.default).count(), 1);
        assert!(existing.iter().find(|d| d.name == "emca").unwrap().default);
        assert!(!existing.iter().find(|d| d.name == "acme").unwrap().default);

        // A later non-default entry does not disturb the existing default.
        assert!(!upsert_deployment(&mut existing, dep("third", false)));
        assert!(existing.iter().find(|d| d.name == "emca").unwrap().default);
    }

    #[test]
    fn reconcile_seed_default_yields_to_configured_default() {
        // Seed loads default=true; a configured default clears it.
        let mut merged = vec![default_deployment(), dep("acme", true)];
        reconcile_seed_default(&mut merged);
        assert!(!merged[0].default, "seed yields to configured default");
        assert!(merged[1].default);

        // No configured default → the seed stays default (the fallback state).
        let mut none_configured = vec![default_deployment(), dep("acme", false)];
        reconcile_seed_default(&mut none_configured);
        assert!(none_configured[0].default, "seed is the default fallback");
    }

    #[test]
    fn apply_set_default_switches_and_clears() {
        let mut ds = vec![dep("acme", true), dep("emca", false)];

        // Switch the default to another configured deployment.
        apply_set_default(&mut ds, Some("emca")).unwrap();
        assert!(!ds[0].default && ds[1].default);

        // Selecting the built-in seed clears every configured default (no
        // configured deployment is default → seed is the fallback).
        apply_set_default(&mut ds, Some(DEFAULT_DEPLOYMENT_NAME)).unwrap();
        assert!(ds.iter().all(|d| !d.default));

        // None also clears (the logged-out-of-default state).
        ds[0].default = true;
        apply_set_default(&mut ds, None).unwrap();
        assert!(ds.iter().all(|d| !d.default));

        // An unknown name errors and leaves the list untouched.
        ds[0].default = true;
        assert!(apply_set_default(&mut ds, Some("nope")).is_err());
        assert!(ds[0].default, "failed set_default does not mutate");
    }

    #[test]
    fn summarize_deployment_reflects_credential_and_default() {
        let jwt = jwt_with_payload(r#"{"email":"u@x.io","name":"U"}"#);
        let mut creds = HashMap::new();
        creds.insert(
            "acme".to_string(),
            CredentialsEntry::from_bearer(jwt, String::new(), None, None, false).unwrap(),
        );

        // A logged-in configured deployment: identity from the credential,
        // endpoints from the record, default flag honored.
        let mut acme = dep("acme", true);
        acme.endpoints = Endpoints {
            cache: "remote.acme".to_string(),
            bes: "bes.acme".to_string(),
            exec: String::new(),
        };
        let s = summarize_deployment(&acme, &creds, true);
        assert!(s.logged_in && s.default && !s.builtin);
        assert_eq!(s.email, "u@x.io");
        assert_eq!(s.cache, "remote.acme");
        assert!(s.exec.is_empty());

        // A logged-out deployment: credential-derived fields are empty.
        let s = summarize_deployment(&dep("other", false), &creds, true);
        assert!(!s.logged_in && s.email.is_empty() && s.status.is_empty());

        // The built-in seed is default only when nothing configured claims it.
        assert!(summarize_deployment(&default_deployment(), &creds, false).default);
        assert!(!summarize_deployment(&default_deployment(), &creds, true).default);
    }

    #[test]
    fn is_https_guards_oidc_endpoints() {
        assert!(is_https("https://host/authorize"));
        assert!(is_https("HTTPS://host")); // scheme match is case-insensitive
        assert!(!is_https("http://host")); // rejects the downgrade
        assert!(!is_https("grpcs://host"));
        assert!(!is_https("host/no-scheme")); // no `://` → empty scheme
        assert!(!is_https("httpsx://host")); // must be exactly `https`
    }

    #[test]
    fn login_scopes_guarantees_offline_access_for_self_hosted() {
        // No advertised scopes → the standard set (which includes offline_access).
        assert_eq!(login_scopes(&[], true), DEFAULT_LOGIN_SCOPES);

        // Self-hosted: advertised scopes are honored (BYO-IdP) but offline_access
        // is appended when missing, so refresh always works.
        assert_eq!(
            login_scopes(&["openid".to_string(), "profile".to_string()], true),
            vec!["openid", "profile", "offline_access"]
        );

        // Already present → not duplicated, order preserved.
        assert_eq!(
            login_scopes(&["openid".to_string(), "offline_access".to_string()], true),
            vec!["openid", "offline_access"]
        );

        // Cloud flow (needs_refresh = false): scopes are left exactly as given, so
        // the built-in seed keeps its deliberate no-offline_access set.
        assert_eq!(
            login_scopes(
                &[
                    "openid".to_string(),
                    "profile".to_string(),
                    "email".to_string()
                ],
                false
            ),
            vec!["openid", "profile", "email"]
        );
    }

    #[test]
    fn builtin_seed_login_omits_offline_access() {
        // The built-in cloud seed has no endpoint hosts, so its login must not
        // request offline_access (the Aspect cloud reissues without a refresh
        // token). Guards against forcing offline_access onto the cloud flow.
        let scopes = auth_env_from(&default_deployment()).unwrap().scopes;
        assert!(!scopes.iter().any(|s| s == "offline_access"));
        assert_eq!(scopes, vec!["openid", "profile", "email"]);
    }

    #[test]
    fn merged_refresh_token_keeps_prior_when_response_omits_it() {
        // Provider rotated the token → use the fresh one.
        assert_eq!(merged_refresh_token("old", "new"), "new");
        // Provider omitted it (the common non-rotating case) → keep the prior one,
        // so the next expiry can still refresh instead of forcing a re-login.
        assert_eq!(merged_refresh_token("old", ""), "old");
        // No prior and none returned → empty (nothing to refresh with).
        assert_eq!(merged_refresh_token("", ""), "");
    }

    #[test]
    fn urldecode_reassembles_percent_and_plus() {
        assert_eq!(urldecode("a%20b"), "a b");
        assert_eq!(urldecode("a+b"), "a b");
        // Percent-encoded bytes reassemble into one multi-byte UTF-8 char.
        assert_eq!(urldecode("%E2%82%AC"), "€");
        assert_eq!(urldecode("plain"), "plain");
        // Invalid hex after `%` is skipped rather than emitted.
        assert_eq!(urldecode("%zz"), "");
    }

    #[test]
    fn extract_query_param_reads_callback_values() {
        let path = "/oauth2/callback?code=abc&state=nonce.1234";
        assert_eq!(extract_query_param(path, "code"), Some("abc".to_string()));
        assert_eq!(
            extract_query_param(path, "state"),
            Some("nonce.1234".to_string())
        );
        // Missing key and no query string both yield None.
        assert_eq!(extract_query_param(path, "error"), None);
        assert_eq!(extract_query_param("/oauth2/callback", "code"), None);
        // The value is url-decoded, and an empty value is Some(""), not None.
        assert_eq!(
            extract_query_param("/cb?code=a%20b", "code"),
            Some("a b".to_string())
        );
        assert_eq!(
            extract_query_param("/cb?code=", "code"),
            Some("".to_string())
        );
        // First occurrence wins.
        assert_eq!(
            extract_query_param("/cb?code=first&code=second", "code"),
            Some("first".to_string())
        );
    }
}
