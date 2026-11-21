//! roommate's MCP server: exposes the read side as MCP tools over stdio, one
//! per existing HTTP read route -- `list_projects`, `list_buildings`,
//! `get_rooms`, `get_validation`, `get_hierarchy_areas`, `get_adjacency`,
//! `get_connectivity`, `get_reach`, `list_connections`, `get_stack_candidates`,
//! `list_snapshots`, `get_latest_snapshot`, `get_pending_snapshot`,
//! `list_milestones`, `compare_milestones`, `list_reference_snapshots`,
//! `get_reference_snapshot`, `get_doors`, `get_windows`, `get_ffe`, `get_spaces`,
//! `get_ceilings`, `get_floors`, `build_report`, `list_report_columns`,
//! `list_saved_reports`, `get_saved_report`, `list_blueprints`, `get_blueprint`,
//! `capture_blueprint`, `resolve_blueprint`, `solve_blueprint`, `get_blueprint_bundle`, `list_sheets`, `get_sheet`,
//! `get_sheet_elevations`, `suggest_sheets`, `solve_sheets`, `list_assignments`, `list_tasks`, `get_task`,
//! `preview_rls`, `preview_rls_set`, `list_tag_rules`, `get_tag_rule` --
//! plus three settings *reads* off `settings_api`'s transport-agnostic core
//! (`list_project_settings`, `get_project_settings`, `resolve_project_settings`)
//! and the one forwarded mutation (`upload_reference`, below). Forty-nine in
//! total (`mcp_bounds::TOOL_KINDS` is the list a test holds the router to; this count had drifted to "forty-two"). "One per existing HTTP read route" was untrue twice: while `/api/settings/resolve/{id}` had no tool,
//! and until 2026-11-14, when five reads that answer a POST and store nothing (the two RLS previews, the two
//! solves, a blueprint's bundle) had none, which is the kind of quiet overclaim `scripts/weekly_review.py` exists
//! to catch. **One read is left out on purpose**: `POST .../blueprints/reshape` answers the blueprint page's box
//! editor with a draft's items re-anchored, a step inside an edit and never a question a model asks. Keep this list
//! and STRATEGY-MCP.md's in step when adding one. Each tool is a thin adapter over
//! `roommate::service` -- parse params, call one service function, serialize
//! the result -- exactly like the Axum handlers in `roommate::handlers`, just a
//! second transport over the same domain layer.
//!
//! Ingest (`POST /rooms`) has no MCP equivalent here: an MCP client asking an
//! LLM to push a full room snapshot isn't a realistic flow, and the HTTP
//! server remains the ingest path.
//!
//! The one mutating tool, `upload_reference`, doesn't break that rule: it never
//! writes this process's state or the store — it reads a CSV file and
//! *forwards it over HTTP* to the running server (`--server-url`, default the
//! shared default address), which stays the single writer and hot-swaps
//! its own registry. The `reqwest` dependency this adds is an HTTP *client*;
//! the "no transport crate leaks into the other binary" rule is about server
//! frameworks (`mcp.rs` still never imports `axum`), and `main.rs` still
//! never imports `rmcp` or `reqwest`.
//!
//! `--http <port>` serves the same tools at `/mcp` for several people on a closed network, instead of
//! one client-spawned process each; the framework side is `roommate::mcp_http`. One long-lived process,
//! so settings are read once at start (restart it after a settings save); `upload_reference` is off
//! there unless `--allow-upload`.
//!
//! Run as a client-spawned subprocess (e.g. from an MCP host's config) --
//! stdout is reserved for the JSON-RPC stream, so all logging goes to
//! stderr. This is a distinct OS process from any running HTTP server: it
//! only sees the same room data if pointed at the same `[storage]` root via
//! `--server-settings`, since `MemStore` state isn't shared across processes.

use std::collections::HashMap;
use std::path::PathBuf;

use clap::Parser;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServerHandler, ServiceExt,
};

use roommate::bootstrap::build_state;
use roommate::contract::{CeilingPayload, FloorPayload};
use roommate::default_http_addr;
use roommate::mcp_bounds::{self, Detail, Page};
use roommate::reports_api;
use roommate::service::{
    adjacency, areas, comparison, connectivity, items, milestones, openings, projects, reach, reference, reports,
    rooms, snapshots, spaces, surfaces, validation, ServiceError,
};
use roommate::settings_api::{self, SettingsError};
use roommate::state::Shared;

mod errors;
mod params;
mod results;
use errors::*;
use params::*;
use results::*;

mod tools_authored;
mod tools_data;
mod tools_geometry;
mod tools_settings;
mod tools_snapshots;

#[allow(dead_code)]
#[derive(Clone)]
struct RoommateMcp {
    state: Shared,
    server_url: String,
    /// Whether `upload_reference` may run. On for a client-spawned process (it is the
    /// user's own), off by default over HTTP, where any caller could use it.
    allow_upload: bool,
    tool_router: ToolRouter<RoommateMcp>,
}

impl RoommateMcp {
    fn new(state: Shared, server_url: String) -> Self {
        Self { state, server_url, allow_upload: true, tool_router: Self::all_tools() }
    }

    /// Every tool: the routers of the `tools_*` modules joined (each module declares its own with `#[tool_router]`).
    fn all_tools() -> ToolRouter<RoommateMcp> {
        Self::tool_router_data()
            + Self::tool_router_snapshots()
            + Self::tool_router_geometry()
            + Self::tool_router_authored()
            + Self::tool_router_settings()
    }

    fn with_upload(mut self, allow: bool) -> Self {
        self.allow_upload = allow;
        self
    }

    /// The `--project-settings` directory this process was started with --
    /// always present for this binary (the arg is required), so the error arm
    /// is defensive only.
    fn projects_dir(&self) -> Result<std::path::PathBuf, McpError> {
        self.state
            .projects_dir()
            .cloned()
            .ok_or_else(|| McpError::internal_error("no project settings directory configured", None))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for RoommateMcp {
    fn get_info(&self) -> ServerConfig {
        // Not `Implementation::from_build_env()` -- it's a plain fn whose body
        // bakes in `env!()` at *rmcp's own* compile time, so it always reports
        // "rmcp"/rmcp's version rather than ours (confirmed via a stdio smoke
        // test). Name and version explicitly instead.
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("roommate-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Read-only access to roommate's stored room and reference data -- this process never \
                 writes its own state or the store. The one mutating tool, upload_reference, forwards \
                 the CSV over HTTP to the running roommate server, which stays the single writer \
                 and hot-swaps its own registry. Requires the same [storage] root as the HTTP \
                 server (via --server-settings) to see real data -- this process does not share \
                 memory with it. "
                    .to_string()
                    + &mcp_bounds::bounding_note(),
            )
    }
}

#[derive(Parser)]
struct Args {
    /// Path to the server-wide TOML settings file (same file the HTTP server
    /// uses via `--server-settings`).
    #[arg(long)]
    server_settings: PathBuf,

    /// Path to the directory of per-project TOML settings files (same
    /// directory the HTTP server uses via `--project-settings`).
    #[arg(long)]
    project_settings: PathBuf,

    /// Base URL of the running roommate HTTP server, used only by the
    /// `upload_reference` tool (which forwards uploads to it). Defaults to the
    /// address the server binary binds by default.
    ///
    /// It tracks the server's *default* port, not its actual one: the server
    /// takes `--port` (and `$PORT`), and nothing here can observe that choice.
    /// Move the server and this flag has to move with it — deliberately loud
    /// rather than a second env lookup that would silently disagree with
    /// whatever the server actually did.
    #[arg(long, default_value_t = format!("http://{}", default_http_addr()))]
    server_url: String,

    /// Serve MCP over HTTP at `/mcp` on this port instead of over stdio, so several people on a
    /// closed network can use one server (they add it to their client by URL). One long-lived
    /// process: it reads settings once at start, so restart it after saving settings in the UI.
    /// `--bind` decides who can reach it; read the warning there.
    #[arg(long)]
    http: Option<u16>,

    /// The interface to listen on with `--http`. Loopback by default; `0.0.0.0` is every
    /// interface. Also read from `ROOMMATE_BIND`, like the HTTP server's.
    #[arg(long, env = "ROOMMATE_BIND", default_value = roommate::DEFAULT_HTTP_HOST)]
    bind: std::net::IpAddr,

    /// An extra `Host` name to answer to with `--http` on a network (repeat or comma-separate;
    /// also `ROOMMATE_ALLOW_HOSTS`), as for the HTTP server.
    #[arg(long = "allow-host", env = "ROOMMATE_ALLOW_HOSTS", value_delimiter = ',')]
    allow_host: Vec<String>,

    /// A shared secret clients must send as `Authorization: Bearer <token>` with `--http`. Read
    /// it from `ROOMMATE_MCP_TOKEN` rather than the command line, which other users of the
    /// machine can see. Without one, anyone who can reach the port can read every project.
    #[arg(long, env = "ROOMMATE_MCP_TOKEN", hide_env_values = true)]
    token: Option<String>,

    /// A file holding the shared secret (one line), for `--http`, instead of `ROOMMATE_MCP_TOKEN`. Also read from
    /// `ROOMMATE_MCP_TOKEN_FILE`. Used only when no token is given directly. Keep it readable by you alone.
    #[arg(long, env = "ROOMMATE_MCP_TOKEN_FILE")]
    token_file: Option<std::path::PathBuf>,

    /// Let `upload_reference` run. Always on over stdio (the process is the user's own); over
    /// `--http` it is off unless this is given, because any caller could use it to replace a
    /// project's reference data.
    #[arg(long)]
    allow_upload: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // stderr, never stdout -- stdout is the JSON-RPC transport. Filter on the
    // *current* crate name (this said "revit_viewer", the crate's old name,
    // which silently dropped every log event). RUST_LOG still wins when set.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("roommate=info,mcp=info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    let state = build_state(&args.server_settings, &args.project_settings)?;

    if let Some(port) = args.http {
        return serve_http(state, &args, port).await;
    }

    let service = RoommateMcp::new(state, args.server_url).serve(stdio()).await.inspect_err(|e| {
        tracing::error!("serving error: {e:?}");
    })?;
    service.waiting().await?;

    Ok(())
}

/// The shared secret `--http` enforces: a token given directly wins, else the first line of `--token-file`.
///
/// A file that was asked for and cannot be used is an ERROR, not "no token": the operator meant the server to be
/// protected, and starting it open on a network because a path was mistyped is the wrong way to fail.
fn effective_token(token: Option<&str>, file: Option<&std::path::Path>) -> anyhow::Result<Option<String>> {
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        return Ok(Some(t.to_string()));
    }
    let Some(path) = file else { return Ok(None) };
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("the token file {} could not be read: {}", path.display(), e.kind()))?;
    let token = text.trim();
    if token.is_empty() {
        anyhow::bail!("the token file {} is empty", path.display());
    }
    Ok(Some(token.to_string()))
}

/// `--http`: the same tools behind `/mcp`. See `roommate::mcp_http` for what stands in front.
async fn serve_http(state: Shared, args: &Args, port: u16) -> anyhow::Result<()> {
    use roommate::network::{self, HostPolicy};

    let hostname = network::machine_hostname();
    let hosts = HostPolicy::for_bind(args.bind, hostname.as_deref(), &args.allow_host);
    let (url, allow_upload) = (args.server_url.clone(), args.allow_upload);
    let token = effective_token(args.token.as_deref(), args.token_file.as_deref())?;
    let app = roommate::mcp_http::router(
        move || Ok(RoommateMcp::new(state.clone(), url.clone()).with_upload(allow_upload)),
        hosts,
        token.clone(),
    );
    let addr = std::net::SocketAddr::new(args.bind, port);
    for base in network::reachable_urls(addr, hostname.as_deref()) {
        tracing::info!("MCP endpoint: {base}{}", roommate::mcp_http::MCP_PATH);
    }
    if network::serves_network(args.bind) && token.is_none() {
        tracing::warn!(
            "serving a network with no token: anyone who can reach this port can read every project. Set ROOMMATE_MCP_TOKEN or ROOMMATE_MCP_TOKEN_FILE."
        );
    }
    if !allow_upload {
        tracing::info!("upload_reference is off over HTTP (--allow-upload turns it on)");
    }
    roommate::mcp_http::serve(addr, app).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_token_comes_directly_then_from_the_file_and_a_bad_file_is_an_error() {
        let dir = std::env::temp_dir().join(format!("mcp-token-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t");
        std::fs::write(&file, "from-file\n").unwrap();
        assert_eq!(super::effective_token(None, Some(&file)).unwrap().as_deref(), Some("from-file"));
        assert_eq!(super::effective_token(Some("direct"), Some(&file)).unwrap().as_deref(), Some("direct"));
        assert_eq!(super::effective_token(Some(""), Some(&file)).unwrap().as_deref(), Some("from-file"));
        assert_eq!(super::effective_token(None, None).unwrap(), None);
        std::fs::write(&file, " \n").unwrap();
        assert!(super::effective_token(None, Some(&file)).is_err(), "an empty file must not mean no token");
        assert!(super::effective_token(None, Some(&dir.join("missing"))).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Every tool declares how it bounds its answer, and the declaration is the tool list: a tool added
    /// without one, or a declared tool that no longer exists, fails here.
    #[test]
    fn test_every_tool_declares_how_it_is_bounded() {
        let mut registered: Vec<String> =
            RoommateMcp::all_tools().list_all().iter().map(|t| t.name.to_string()).collect();
        let mut declared: Vec<String> = mcp_bounds::TOOL_KINDS.iter().map(|(n, _)| (*n).to_string()).collect();
        registered.sort();
        declared.sort();
        assert_eq!(registered, declared);
    }

    #[test]
    fn test_the_instructions_name_the_paged_tools() {
        let text = mcp_bounds::bounding_note();
        assert!(text.contains("get_rooms") && text.contains("list_tasks") && text.contains("get_validation"));
    }

    #[derive(serde::Serialize)]
    struct FakeList {
        revision: String,
        things: Vec<serde_json::Value>,
    }

    impl Listing for FakeList {
        type Element = serde_json::Value;
        const KEY: &'static str = "things";
        const SUMMARY: &'static [&'static str] = &["id", "name"];
        fn elements(&mut self) -> &mut Vec<serde_json::Value> {
            &mut self.things
        }
    }

    fn fake(n: usize) -> FakeList {
        let things = (0..n)
            .map(|i| serde_json::json!({"id": i, "name": format!("n{i}"), "loops": [[1, 2, 3]]}))
            .collect();
        FakeList { revision: "r".into(), things }
    }

    fn text_of(r: Result<CallToolResult, McpError>) -> serde_json::Value {
        let r = r.unwrap();
        serde_json::from_str(r.content[0].as_text().unwrap().text.as_str()).unwrap()
    }

    #[test]
    fn test_a_listing_summary_keeps_the_summary_keys_and_says_what_it_omitted() {
        let a = text_of(listing_result(fake(5), None, Some(2), None));
        assert_eq!(a["detail"], "summary");
        assert_eq!(
            (a["total"].as_u64(), a["returned"].as_u64(), a["next_offset"].as_u64()),
            (Some(5), Some(2), Some(2))
        );
        assert_eq!(a["things"][0], serde_json::json!({"id": 0, "name": "n0"}));
        assert!(a["omitted"].is_array() && a["more"].is_string());
    }

    #[test]
    fn test_a_listing_full_page_keeps_whole_records_and_pages_on() {
        let a = text_of(listing_result(fake(5), Some("full"), Some(2), Some(2)));
        assert_eq!(a["detail"], "full");
        assert_eq!(a["things"].as_array().unwrap().len(), 2);
        assert_eq!(a["things"][0]["id"], 2);
        assert!(a["things"][0].get("loops").is_some() && a.get("omitted").is_none());
        assert_eq!(a["next_offset"], 4);
    }

    #[test]
    fn test_a_listing_refuses_a_bad_detail() {
        assert!(listing_result(fake(1), Some("everything"), None, None).is_err());
    }

    use super::*;
    use roommate::state::{AppState, ProjectSettings};
    use roommate::storage::MemStore;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// A registered project configuring `sources` reference sources by name.
    fn state_with(sources: &[&str]) -> Shared {
        let bundle = ProjectSettings {
            spaces: Default::default(),
            reference: sources
                .iter()
                .map(|name| {
                    (
                        name.to_string(),
                        roommate::state::ProjectReferenceSource {
                            entity: roommate::settings::ReferenceEntity::Rooms,
                            data: None,
                            fields: vec![],
                        },
                    )
                })
                .collect(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec![],
            milestones: vec![],
            anchor_model: None,
            tag_type_code: None,
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            windows: Default::default(),
            ffe: Default::default(),
            hierarchy_exclusions: vec![],
            routing: Default::default(),
        };
        Arc::new(AppState::new(
            Box::new(MemStore::new()),
            HashMap::from([("p1".to_string(), bundle)]),
            None,
        ))
    }

    /// The common case: one configured source, so a caller that names none
    /// still gets the right one. This is what keeps `source` optional instead
    /// of forcing a `get_project_settings` round trip before every call.
    #[test]
    fn test_sole_source_resolves_without_being_named() {
        assert_eq!(resolve_source(&state_with(&["drofus"]), "p1", None).unwrap(), "drofus");
    }

    /// An explicit source always wins — including one the project does not
    /// configure. Whether that source has data is the service's answer to
    /// give (a soft-empty listing), not this resolver's to pre-empt.
    #[test]
    fn test_explicit_source_wins() {
        let state = state_with(&["drofus", "ffe"]);
        assert_eq!(resolve_source(&state, "p1", Some("ffe")).unwrap(), "ffe");
        assert_eq!(resolve_source(&state, "p1", Some("nope")).unwrap(), "nope");
    }

    /// **Ambiguity is refused, not guessed.** Silently taking the first source
    /// would answer confidently about the wrong dataset, which is worse than
    /// an error. The message names the real choices so the retry is obvious.
    #[test]
    fn test_ambiguous_source_is_refused_and_names_the_choices() {
        let err = resolve_source(&state_with(&["drofus", "ffe"]), "p1", None).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("drofus") && msg.contains("ffe"), "must name the choices: {msg}");
    }

    /// A project with no reference source, and an unregistered project, both
    /// fail with a caller-addressable message rather than defaulting to a
    /// name that would then 404 deeper in.
    #[test]
    fn test_no_configured_source_is_an_error() {
        assert!(resolve_source(&state_with(&[]), "p1", None).is_err());
        assert!(resolve_source(&state_with(&["drofus"]), "ghost", None).is_err());
    }

    fn http_app(allow_upload: bool, token: Option<&str>) -> roommate::mcp_http::Router {
        let state = state_with(&["drofus"]);
        roommate::mcp_http::router(
            move || Ok(RoommateMcp::new(state.clone(), "http://127.0.0.1:1".into()).with_upload(allow_upload)),
            roommate::network::HostPolicy::loopback_only(),
            token.map(String::from),
        )
    }

    async fn rpc(app: roommate::mcp_http::Router, token: Option<&str>, body: serde_json::Value) -> (u16, String) {
        roommate::mcp_http::call(app, token, &body.to_string()).await
    }

    fn init() -> serde_json::Value {
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"0"}}})
    }

    /// The tools answer over HTTP as they do over stdio: the handshake works and a call returns
    /// what the service returns. One POST per message, so nothing depends on a session surviving.
    #[tokio::test]
    async fn test_http_answers_initialize_and_a_tool_call() {
        let (status, body) = rpc(http_app(false, None), None, init()).await;
        assert_eq!(status, 200, "{body}");
        assert!(body.contains("roommate-mcp"), "{body}");

        let call = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"list_projects","arguments":{}}});
        let (status, body) = rpc(http_app(false, None), None, call).await;
        assert_eq!(status, 200, "{body}");
        assert!(body.contains("\"result\""), "{body}");
    }

    /// With a token set, a call without it never reaches a tool.
    #[tokio::test]
    async fn test_http_refuses_a_call_without_the_token() {
        assert_eq!(rpc(http_app(false, Some("s3cret")), None, init()).await.0, 401);
        assert_eq!(rpc(http_app(false, Some("s3cret")), Some("s3cret"), init()).await.0, 200);
    }

    /// Over HTTP the one mutating tool is off unless asked for, and says so rather than failing
    /// on the network call it would otherwise make.
    #[tokio::test]
    async fn test_upload_is_off_over_http_by_default() {
        let call = serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"upload_reference","arguments":{"project_id":"p1","path":"/nope.csv"}}});
        let (status, body) = rpc(http_app(false, None), None, call).await;
        assert_eq!(status, 200, "{body}");
        assert!(body.contains("turned off"), "{body}");
    }

    /// Path and query components are percent-encoded before they reach the
    /// forwarded upload URL — a source or project name with a space or slash
    /// must not split the path.
    #[test]
    fn test_urlencode_escapes_path_hostile_characters() {
        assert_eq!(urlencode("drofus"), "drofus");
        assert_eq!(urlencode("a b/c"), "a%20b%2Fc");
        assert_eq!(urlencode("2026-01-01T10:00:00+10:00"), "2026-01-01T10%3A00%3A00%2B10%3A00");
    }
}
