//! `ctx.aspect.mcp` — an MCP (Model Context Protocol) server over a Workflows
//! deployment's `/api/v1` build-results REST API.
//!
//! The server speaks MCP over stdio (stdout carries the protocol; every
//! diagnostic goes to stderr) and exposes read-only tools for the build data
//! behind the Build & Test UI. It authenticates each upstream request with the
//! bearer `aspect auth login` minted for the deployment, resolved fresh per
//! call so the CLI's refresh flow keeps a long-lived session alive.
//!
//! The API host comes from the deployment's advertised build-results viewer URL
//! (`aspect_bes_results_url`, recorded by `aspect auth configure`). The REST
//! API ships in Aspect Workflows 6.1 and is opt-in (`webapp.web.api_enabled`),
//! and the CLI reaches customers before their deployments upgrade — so when the
//! one startup probe (the RFC 9728 discovery document, the only unauthenticated
//! path) finds no API, the server still starts and every tool returns the
//! version-gating message instead. An agent then relays an actionable answer
//! rather than the session dying on a transport error.

use std::sync::Arc;

use allocative::Allocative;
use derive_more::Display;
use rmcp::ServiceExt;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData as McpError,
    Implementation, InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities,
    ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value_as_type::StarlarkValueAsType;
use starlark::values::{self, NoSerialize, ProvidesStaticType, starlark_value};

use super::auth;

/// The customer docs page the version-gating error points at. Also where the
/// `.mcp.json` snippet and the tool list are documented.
const DOCS_URL: &str =
    "https://aspect.build/docs/aspect-workflows/using-workflows/build-results-api";

/// RFC 9728 protected-resource metadata: the one unauthenticated path an
/// API-enabled deployment serves on the web host, so it doubles as the "can
/// this deployment serve the API at all" probe.
const DISCOVERY_PATH: &str = "/.well-known/oauth-protected-resource";

/// Ceiling on `limit` params, mirroring the API's own bound. Stated in the tool
/// schemas so an agent does not learn the bound by tripping a 400.
const MAX_LIMIT: u64 = 100;

/// The deployment cannot serve the REST API (probe failed): name the fix, not
/// the transport. This is the support-ticket deflector — most CLI installs will
/// be newer than the deployments they point at.
fn api_unavailable_message(deployment: &str, api_origin: &str) -> String {
    format!(
        "The deployment '{deployment}' ({api_origin}) does not expose the REST API the MCP server \
         needs. It requires Aspect Workflows 6.1 or later with `webapp.web.api_enabled = true` — \
         see {DOCS_URL}. Ask your Workflows operator to enable it."
    )
}

/// No stored credential: the exact commands that fix it.
fn not_logged_in_message(deployment: &str) -> String {
    format!(
        "Not logged in to the Aspect Workflows deployment '{deployment}'. Run `{}` in a terminal, \
         then retry.",
        auth::login_hint(deployment)
    )
}

/// One tool the server publishes: the curated MCP-facing contract plus how it
/// maps onto the REST route. Descriptions are written for the calling agent —
/// they are the tool's entire documentation, so they carry the non-obvious
/// usage facts (id vs invocation_id, label encoding, range values).
struct ToolDef {
    name: &'static str,
    description: &'static str,
    /// JSON Schema for the arguments, hand-written rather than derived: the
    /// REST parameter surface is deliberately trimmed and re-described for
    /// agent consumption.
    schema: fn() -> serde_json::Value,
    /// Build the request path + query from validated arguments.
    route: fn(&Args) -> Result<String, String>,
}

/// Accessor over a tool call's `arguments` object with uniform error text.
struct Args<'a>(Option<&'a serde_json::Map<String, serde_json::Value>>);

impl<'a> Args<'a> {
    fn str(&self, key: &str) -> Option<&'a str> {
        self.0.and_then(|m| m.get(key)).and_then(|v| v.as_str())
    }

    fn required_str(&self, key: &str) -> Result<&'a str, String> {
        self.str(key)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("missing required argument `{key}`"))
    }

    fn u64(&self, key: &str) -> Option<u64> {
        self.0.and_then(|m| m.get(key)).and_then(|v| v.as_u64())
    }

    fn bool(&self, key: &str) -> Option<bool> {
        self.0.and_then(|m| m.get(key)).and_then(|v| v.as_bool())
    }
}

fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// Append `key=value` (value URL-encoded) to a query string under construction.
fn push_param(query: &mut String, key: &str, value: &str) {
    query.push(if query.is_empty() { '?' } else { '&' });
    query.push_str(key);
    query.push('=');
    query.push_str(&encode(value));
}

/// Copy the caller's optional paging/filter params through to the query,
/// clamping `limit` to the API's bound rather than forwarding a 400.
fn push_common(args: &Args, query: &mut String, keys: &[&str]) {
    for key in keys {
        match *key {
            "limit" | "offset" => {
                if let Some(n) = args.u64(key) {
                    let n = if *key == "limit" { n.min(MAX_LIMIT) } else { n };
                    push_param(query, key, &n.to_string());
                }
            }
            "is_test" => {
                if let Some(b) = args.bool(key) {
                    push_param(query, key, if b { "true" } else { "false" });
                }
            }
            _ => {
                if let Some(v) = args.str(key).filter(|v| !v.is_empty()) {
                    push_param(query, key, v);
                }
            }
        }
    }
}

/// Schema fragment helpers, so the tool table below reads as data.
fn prop(desc: &str, ty: &str) -> serde_json::Value {
    serde_json::json!({"type": ty, "description": desc})
}

fn obj(props: serde_json::Value, required: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": props,
        "required": required,
        "additionalProperties": false,
    })
}

fn id_prop() -> serde_json::Value {
    prop(
        "The build's id — the `id` field of an invocation from list_invocations (NOT the \
         `invocation_id` Bazel printed; resolve that with list_invocations first).",
        "string",
    )
}

fn limit_prop(default: u64) -> serde_json::Value {
    serde_json::json!({
        "type": "integer",
        "description": format!("Maximum results per page (default {default}, maximum {MAX_LIMIT})."),
        "minimum": 1,
        "maximum": MAX_LIMIT,
    })
}

fn offset_prop() -> serde_json::Value {
    serde_json::json!({
        "type": "integer",
        "description": "Number of results to skip, for paging. Every list response carries the unpaginated `total`.",
        "minimum": 0,
    })
}

fn range_prop() -> serde_json::Value {
    serde_json::json!({
        "type": "string",
        "description": "Lookback window for the statistics.",
        "enum": ["d1", "d3", "d7", "m1", "m3", "y1"],
    })
}

fn label_prop() -> serde_json::Value {
    prop(
        "The Bazel target label, exactly as Bazel prints it (e.g. `//pkg:name`). Passed as a \
         query parameter; do not URL-encode it yourself.",
        "string",
    )
}

/// The published tool surface. Read-only build data only: the API's
/// org/profile/session management routes are deliberately not exposed.
fn tool_defs() -> &'static [ToolDef] {
    &[
        ToolDef {
            name: "list_invocations",
            description: "List builds (invocations), newest first. Filterable by status, Bazel \
                          command, repository, or a Bazel-printed invocation_id (use that filter \
                          to resolve Bazel's UUID to the build's `id`, which every other tool \
                          takes). Each entry carries a `links.self` URL and the summary header: \
                          status, command, duration, VCS info.",
            schema: || {
                obj(
                    serde_json::json!({
                        "limit": limit_prop(20),
                        "offset": offset_prop(),
                        "status": prop("Filter by build status (e.g. `success`, `failure`).", "string"),
                        "invocation_id": prop(
                            "Filter by the Bazel-printed invocation UUID, to resolve it to the build's `id`.",
                            "string"),
                        "command": prop("Filter by Bazel command (e.g. `build`, `test`).", "string"),
                        "repo_name": prop("Filter by repository name.", "string"),
                    }),
                    &[],
                )
            },
            route: |args| {
                let mut q = String::new();
                if args.u64("limit").is_none() {
                    push_param(&mut q, "limit", "20");
                }
                push_common(
                    args,
                    &mut q,
                    &[
                        "limit",
                        "offset",
                        "status",
                        "invocation_id",
                        "command",
                        "repo_name",
                    ],
                );
                Ok(format!("/invocations{q}"))
            },
        },
        ToolDef {
            name: "get_invocation",
            description: "The full detail of one build: header, status, timings, VCS info, and \
                          links to its sub-resources (log, targets, metrics, …).",
            schema: || obj(serde_json::json!({"id": id_prop()}), &["id"]),
            route: |args| Ok(format!("/invocations/{}", encode(args.required_str("id")?))),
        },
        ToolDef {
            name: "get_invocation_configurations",
            description: "The build configurations (platform/config fingerprints) present in one \
                          build. Configuration ids from here feed list_target_artifacts.",
            schema: || obj(serde_json::json!({"id": id_prop()}), &["id"]),
            route: |args| {
                Ok(format!(
                    "/invocations/{}/configurations",
                    encode(args.required_str("id")?)
                ))
            },
        },
        ToolDef {
            name: "get_invocation_metadata",
            description: "The user- and CI-supplied metadata key/values recorded on one build.",
            schema: || obj(serde_json::json!({"id": id_prop()}), &["id"]),
            route: |args| {
                Ok(format!(
                    "/invocations/{}/metadata",
                    encode(args.required_str("id")?)
                ))
            },
        },
        ToolDef {
            name: "get_invocation_metrics",
            description: "One build's performance metrics: action counts, cache hit rates, \
                          critical path, network and memory figures.",
            schema: || obj(serde_json::json!({"id": id_prop()}), &["id"]),
            route: |args| {
                Ok(format!(
                    "/invocations/{}/metrics",
                    encode(args.required_str("id")?)
                ))
            },
        },
        ToolDef {
            name: "get_build_log",
            description: "One page of a build's log. Pages are zero-indexed; every response \
                          reports `page` and `page_count`, so start at page 0 and page forward. \
                          For the end of the log (usually where the failure is), use \
                          tail_build_log instead.",
            schema: || {
                obj(
                    serde_json::json!({
                        "id": id_prop(),
                        "page": {"type": "integer", "description": "Zero-indexed page number (default 0).", "minimum": 0},
                    }),
                    &["id"],
                )
            },
            route: |args| {
                let mut q = String::new();
                if let Some(page) = args.u64("page") {
                    push_param(&mut q, "page", &page.to_string());
                }
                Ok(format!(
                    "/invocations/{}/log{q}",
                    encode(args.required_str("id")?)
                ))
            },
        },
        ToolDef {
            name: "tail_build_log",
            description: "The last page of a build's log in one request — the fastest way to see \
                          why a build failed.",
            schema: || obj(serde_json::json!({"id": id_prop()}), &["id"]),
            route: |args| {
                Ok(format!(
                    "/invocations/{}/log/tail",
                    encode(args.required_str("id")?)
                ))
            },
        },
        ToolDef {
            name: "list_invocation_targets",
            description: "The targets one build built or tested, with per-target status. \
                          Searchable and pageable; filter to tests with is_test.",
            schema: || {
                obj(
                    serde_json::json!({
                        "id": id_prop(),
                        "limit": limit_prop(20),
                        "offset": offset_prop(),
                        "search": prop("Substring filter on the target label.", "string"),
                        "bucket": prop("Filter by outcome bucket (e.g. `failing`, `flaky`).", "string"),
                        "is_test": {"type": "boolean", "description": "Only test targets (true) or only non-test targets (false)."},
                        "order_by": prop("Sort key.", "string"),
                    }),
                    &["id"],
                )
            },
            route: |args| {
                let mut q = String::new();
                if args.u64("limit").is_none() {
                    push_param(&mut q, "limit", "20");
                }
                push_common(
                    args,
                    &mut q,
                    &["limit", "offset", "search", "bucket", "is_test", "order_by"],
                );
                Ok(format!(
                    "/invocations/{}/targets{q}",
                    encode(args.required_str("id")?)
                ))
            },
        },
        ToolDef {
            name: "get_target_summary",
            description: "Aggregate counts of one build's targets by outcome (built, failed, \
                          flaky, …) — read this before listing targets to know what to look for.",
            schema: || {
                obj(
                    serde_json::json!({
                        "id": id_prop(),
                        "search": prop("Substring filter on the target label before aggregating.", "string"),
                    }),
                    &["id"],
                )
            },
            route: |args| {
                let mut q = String::new();
                push_common(args, &mut q, &["search"]);
                Ok(format!(
                    "/invocations/{}/target-summary{q}",
                    encode(args.required_str("id")?)
                ))
            },
        },
        ToolDef {
            name: "get_target",
            description: "The detail of one target within one build: kind, per-configuration \
                          execution results, and test detail when it is a test. Configuration ids \
                          in the response feed list_target_artifacts.",
            schema: || {
                obj(
                    serde_json::json!({"id": id_prop(), "label": label_prop()}),
                    &["id", "label"],
                )
            },
            route: |args| {
                let mut q = String::new();
                push_param(&mut q, "label", args.required_str("label")?);
                Ok(format!(
                    "/invocations/{}/target{q}",
                    encode(args.required_str("id")?)
                ))
            },
        },
        ToolDef {
            name: "list_target_artifacts",
            description: "The output artifacts one target produced in one build, for one \
                          configuration (get the config_id from get_target or \
                          get_invocation_configurations).",
            schema: || {
                obj(
                    serde_json::json!({
                        "id": id_prop(),
                        "label": label_prop(),
                        "config_id": prop("The configuration id the artifacts were built under.", "string"),
                        "limit": limit_prop(20),
                        "offset": offset_prop(),
                    }),
                    &["id", "label", "config_id"],
                )
            },
            route: |args| {
                let mut q = String::new();
                push_param(&mut q, "label", args.required_str("label")?);
                push_param(&mut q, "config_id", args.required_str("config_id")?);
                push_common(args, &mut q, &["limit", "offset"]);
                Ok(format!(
                    "/invocations/{}/target/artifacts{q}",
                    encode(args.required_str("id")?)
                ))
            },
        },
        ToolDef {
            name: "list_target_invocations",
            description: "The builds that built one target over a lookback window — the \
                          target-centric view, for questions like \"when did this target start \
                          failing\". Cross-invocation; takes a label, not a build id.",
            schema: || {
                obj(
                    serde_json::json!({
                        "label": label_prop(),
                        "range": range_prop(),
                        "limit": limit_prop(20),
                        "offset": offset_prop(),
                        "status": prop("Filter by the target's status in the build.", "string"),
                        "cache": prop("Filter by cache outcome.", "string"),
                        "search": prop("Substring filter.", "string"),
                    }),
                    &["label", "range"],
                )
            },
            route: |args| {
                let mut q = String::new();
                push_param(&mut q, "label", args.required_str("label")?);
                push_param(&mut q, "range", args.required_str("range")?);
                if args.u64("limit").is_none() {
                    push_param(&mut q, "limit", "20");
                }
                push_common(
                    args,
                    &mut q,
                    &["limit", "offset", "status", "cache", "search"],
                );
                Ok(format!("/target-invocations{q}"))
            },
        },
        ToolDef {
            name: "get_target_stats",
            description: "Cross-invocation statistics for the organization's targets over a \
                          lookback window: build counts, failure and flake rates, durations. With \
                          `label`, the same shape narrowed to one target — in that form no other \
                          filter or paging parameter is accepted, and the collection-wide \
                          `profiled_invocations_daily` series is empty. A label with no builds in \
                          the window is an empty page, not an error.",
            schema: || {
                obj(
                    serde_json::json!({
                        "range": range_prop(),
                        "label": label_prop(),
                        "repo": prop("Filter to one repository.", "string"),
                        "is_test": {"type": "boolean", "description": "Only test targets (true) or only non-test targets (false)."},
                        "limit": limit_prop(20),
                    }),
                    &["range"],
                )
            },
            route: |args| {
                let mut q = String::new();
                push_param(&mut q, "range", args.required_str("range")?);
                if let Some(label) = args.str("label").filter(|l| !l.is_empty()) {
                    // The API rejects other params alongside `label`; send it alone.
                    push_param(&mut q, "label", label);
                    return Ok(format!("/target-stats{q}"));
                }
                push_common(args, &mut q, &["repo", "is_test", "limit"]);
                Ok(format!("/target-stats{q}"))
            },
        },
    ]
}

/// The MCP server: one configured deployment's API origin plus the credential
/// profile that authenticates against it.
struct BuildResultsServer {
    /// Deployment name — also the credential-store profile.
    deployment: String,
    /// Scheme + host of the web/API edge (no path), from the deployment's
    /// advertised results URL.
    api_origin: String,
    /// Whether the startup probe found the discovery document. When false the
    /// server still runs, and every tool answers with the version-gating
    /// message (see the module docs).
    api_available: bool,
    http: reqwest::Client,
}

impl BuildResultsServer {
    /// Resolve the bearer for the deployment, fresh per call so the stored
    /// credential's refresh flow keeps a long-lived MCP session alive.
    ///
    /// On a `spawn_blocking` worker: `resolve_access_token`'s refresh path
    /// calls `Handle::current().block_on`, which panics on a runtime worker
    /// thread — the same arrangement the credential helper uses.
    async fn bearer(&self) -> Result<String, String> {
        let deployment = self.deployment.clone();
        let resolved = tokio::task::spawn_blocking(move || auth::resolve_access_token(&deployment))
            .await
            .map_err(|e| format!("token resolution failed: {e}"))?;
        match resolved {
            Ok(Some(token)) => Ok(token),
            Ok(None) => Err(not_logged_in_message(&self.deployment)),
            // resolve_access_token's error already names the fix (expired
            // session → the login command); pass it through verbatim.
            Err(e) => Err(e.to_string()),
        }
    }

    async fn call(&self, def: &ToolDef, args: &Args<'_>) -> Result<String, String> {
        if !self.api_available {
            return Err(api_unavailable_message(&self.deployment, &self.api_origin));
        }
        let path = (def.route)(args)?;
        let token = self.bearer().await?;
        let url = format!("{}/api/v1{}", self.api_origin, path);
        let response = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| format!("request to {url} failed: {e}"))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("reading the response from {url} failed: {e}"))?;
        if status.is_success() {
            return Ok(body);
        }
        // The API's error bodies are {"code", "message"} and already
        // user-readable; frame them with the status and enough context to act.
        let detail = body.trim();
        let hint = match status.as_u16() {
            // A bearer the edge redirects (302 → HTML) or the gateway rejects:
            // either way the fix is a fresh login.
            401 | 403 => format!(
                " — the credential was not accepted; run `{}`",
                auth::login_hint(&self.deployment)
            ),
            404 => format!(
                " — no such resource on deployment '{}'; ids come from list_invocations",
                self.deployment
            ),
            _ => String::new(),
        };
        Err(format!("HTTP {status} from {path}{hint}: {detail}"))
    }
}

impl ServerHandler for BuildResultsServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build());
        info.server_info =
            Implementation::new("aspect-workflows-build-results", env!("CARGO_PKG_VERSION"))
                .with_title("Aspect Workflows build results");
        info.instructions = Some(format!(
            "Read-only build and test results from the Aspect Workflows deployment '{}'. \
             Builds are addressed by their `id` from list_invocations — when you only have \
             the invocation UUID Bazel printed, resolve it first with \
             list_invocations(invocation_id=...). Responses carry `links` objects; follow \
             them rather than constructing URLs.",
            self.deployment
        ));
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools = tool_defs()
            .iter()
            .map(|def| {
                let schema = match (def.schema)() {
                    serde_json::Value::Object(map) => map,
                    _ => unreachable!("tool schemas are objects by construction"),
                };
                Tool::new(def.name, def.description, Arc::new(schema))
            })
            .collect();
        Ok(ListToolsResult {
            tools,
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let Some(def) = tool_defs().iter().find(|d| d.name == request.name) else {
            return Err(McpError::invalid_params(
                format!("unknown tool: {}", request.name),
                None,
            ));
        };
        let args = Args(request.arguments.as_ref());
        Ok(match self.call(def, &args).await {
            Ok(body) => CallToolResult::success(vec![ContentBlock::text(body)]),
            Err(message) => CallToolResult::error(vec![ContentBlock::text(message)]),
        }
        .into())
    }
}

/// Resolve the deployment, probe the API, and serve MCP over stdio until the
/// client disconnects. Blocking; returns the process's exit code.
fn serve_blocking(deployment_name: Option<&str>) -> anyhow::Result<i32> {
    let deployments = auth::load_deployments()?;
    let deployment = auth::select_deployment(&deployments, deployment_name)?;
    let results_url = deployment.endpoints.results_url.clone();
    if results_url.is_empty() {
        anyhow::bail!(
            "deployment '{}' does not advertise a build-results URL, so it has no web/API host to \
             serve build results from. Re-run `aspect auth configure <remote-host>` against a \
             deployment with the Build & Test UI enabled.",
            deployment.name
        );
    }
    let origin = url_origin(&results_url).ok_or_else(|| {
        anyhow::anyhow!(
            "deployment '{}' advertises an invalid results URL: {results_url}",
            deployment.name
        )
    })?;

    let http = reqwest::Client::builder()
        // A no-bearer request to a cookie-fronted edge 302s to the IdP; seeing
        // that status (rather than the IdP's HTML) is what the probe and the
        // per-call error mapping key on.
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

    let probe_url = format!("{origin}{DISCOVERY_PATH}");
    let api_available = auth::block_on(async {
        match http.get(&probe_url).send().await {
            Ok(r) if r.status().is_success() => true,
            Ok(r) => {
                eprintln!(
                    "warning: {probe_url} answered HTTP {} — serving anyway; every tool will \
                     explain what the deployment is missing.",
                    r.status()
                );
                false
            }
            Err(e) => {
                eprintln!("warning: could not probe {probe_url} ({e}) — serving anyway.");
                false
            }
        }
    });

    let server = BuildResultsServer {
        deployment: deployment.name.clone(),
        api_origin: origin.clone(),
        api_available,
        http,
    };

    eprintln!(
        "aspect mcp: serving build results for deployment '{}' ({origin}) over stdio",
        deployment.name
    );

    auth::block_on(async {
        let service = server.serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        Ok::<_, anyhow::Error>(())
    })?;
    Ok(0)
}

/// `scheme://host[:port]` of `url`, with any path/query dropped.
fn url_origin(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let mut origin = format!("{}://{host}", parsed.scheme());
    if let Some(port) = parsed.port() {
        origin.push_str(&format!(":{port}"));
    }
    Some(origin)
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<aspect.Mcp>")]
pub struct Mcp {}

starlark_simple_value!(Mcp);

#[starlark_value(type = "aspect.Mcp")]
impl<'v> values::StarlarkValue<'v> for Mcp {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(mcp_methods)
    }
}

#[starlark_module]
fn mcp_methods(registry: &mut MethodsBuilder) {
    /// Serve the build-results MCP server over stdio for `deployment` (the
    /// default deployment when omitted). Blocks until the MCP client
    /// disconnects; returns the exit code.
    fn serve<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] deployment: NoneOr<String>,
        #[allow(unused)] heap: values::Heap<'v>,
    ) -> anyhow::Result<i32> {
        serve_blocking(
            deployment
                .into_option()
                .as_deref()
                .filter(|d| !d.is_empty()),
        )
    }
}

#[starlark_module]
fn register_types(globals: &mut GlobalsBuilder) {
    const Mcp: StarlarkValueAsType<Mcp> = StarlarkValueAsType::new();
}

pub fn register_globals(globals: &mut GlobalsBuilder) {
    register_types(globals);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(json: serde_json::Value) -> serde_json::Value {
        json
    }

    fn route(name: &str, arguments: serde_json::Value) -> Result<String, String> {
        let def = tool_defs()
            .iter()
            .find(|d| d.name == name)
            .expect("tool exists");
        let map = arguments.as_object().cloned();
        (def.route)(&Args(map.as_ref()))
    }

    #[test]
    fn every_tool_schema_is_a_strict_object() {
        for def in tool_defs() {
            let schema = (def.schema)();
            assert_eq!(schema["type"], "object", "{}", def.name);
            assert_eq!(
                schema["additionalProperties"], false,
                "{}: agents probe loose schemas with junk params",
                def.name
            );
            // Every required key must be a declared property.
            let props = schema["properties"].as_object().unwrap();
            for req in schema["required"].as_array().unwrap() {
                assert!(
                    props.contains_key(req.as_str().unwrap()),
                    "{}: required key {req} not in properties",
                    def.name
                );
            }
        }
    }

    #[test]
    fn tool_names_are_unique() {
        let mut names: Vec<_> = tool_defs().iter().map(|d| d.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), tool_defs().len());
    }

    #[test]
    fn labels_are_query_encoded() {
        let path = route(
            "get_target",
            args(serde_json::json!({"id": "abc", "label": "//pkg:name"})),
        )
        .unwrap();
        assert_eq!(path, "/invocations/abc/target?label=%2F%2Fpkg%3Aname");
    }

    #[test]
    fn list_invocations_defaults_a_limit() {
        let path = route("list_invocations", args(serde_json::json!({}))).unwrap();
        assert_eq!(path, "/invocations?limit=20");
    }

    #[test]
    fn limit_is_clamped_to_the_api_bound() {
        let path = route(
            "list_invocations",
            args(serde_json::json!({"limit": 10_000})),
        )
        .unwrap();
        assert_eq!(path, "/invocations?limit=100");
    }

    #[test]
    fn target_stats_with_label_sends_no_other_filters() {
        let path = route(
            "get_target_stats",
            args(
                serde_json::json!({"range": "d7", "label": "//a:b", "repo": "ignored", "limit": 5}),
            ),
        )
        .unwrap();
        assert_eq!(path, "/target-stats?range=d7&label=%2F%2Fa%3Ab");
    }

    #[test]
    fn missing_required_arguments_name_the_key() {
        let err = route("get_target", args(serde_json::json!({"id": "abc"}))).unwrap_err();
        assert!(err.contains("`label`"), "{err}");
    }

    #[test]
    fn origin_strips_the_viewer_path() {
        assert_eq!(
            url_origin("https://app.example.com/i/").as_deref(),
            Some("https://app.example.com")
        );
        assert_eq!(
            url_origin("http://127.0.0.1:3000/i/").as_deref(),
            Some("http://127.0.0.1:3000")
        );
        assert_eq!(url_origin("not a url"), None);
    }

    #[test]
    fn gating_message_names_version_flag_and_docs() {
        let msg = api_unavailable_message("acme", "https://app.acme.example.com");
        for needle in ["6.1", "webapp.web.api_enabled", DOCS_URL, "acme"] {
            assert!(msg.contains(needle), "missing {needle}: {msg}");
        }
    }
}
