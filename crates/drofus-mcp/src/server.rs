//! The MCP server: its identity and its tools.
//!
//! **One tool per method, a thin adapter over a plain function.** A tool parses its parameters,
//! calls one function that knows nothing about MCP, and serialises the answer, so what a tool does
//! can be tested without a transport (the tests below call the methods directly).
//!
//! **The dRofus tools are rows, registered in a loop.** `server_info` is a plain `#[tool]` method; the
//! other 89 are built from [`ENDPOINTS`] and added to the router at start-up, each with its own name,
//! description and argument schema. An attribute macro cannot be generated from a `macro_rules` table
//! (`#[tool_router]` reads the impl block before any inner macro expands), so the table is data and the
//! generic path in `client` runs every row. What a tool does is [`answer_to_result`] over
//! `Client::call`, both free of MCP types where it matters, so tests need no transport.
//!
//! **Read-only.** Every row is a GET or an OPTIONS and each tool says so in its annotations.

use std::sync::Arc;

use rmcp::{
    handler::server::{
        router::tool::{ToolRoute, ToolRouter},
        tool::ToolCallContext,
    },
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo, Tool, ToolAnnotations},
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use serde::Serialize;

use crate::{
    approval::Gate,
    auth::{Authenticator, Unauthenticated},
    client::{Answer, Client},
    config::Config,
    endpoints::{Endpoint, ToolError, ENDPOINTS},
};

/// What `server_info` answers.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Info {
    pub name: &'static str,
    pub version: &'static str,
    /// The dRofus address in use, or null when none was given.
    pub base_url: Option<String>,
    /// The database and project used when a tool is not given `db` / `pr`.
    pub database: Option<String>,
    pub project: Option<String>,
    /// Whether a credential was given. The credential itself is never returned. Nothing reads it yet.
    pub api_key_set: bool,
    /// Where authentication stands, so a client knows why every dRofus tool stops with an error.
    pub authentication: &'static str,
    /// How many dRofus tools are registered (all read-only).
    pub drofus_tools: usize,
    /// Whether writes are possible. There is no write tool today, so this is the gate's state only.
    pub writes: &'static str,
    /// How tools take their arguments.
    pub usage: &'static str,
    /// What still has to be configured before dRofus tools can work.
    pub missing: Vec<&'static str>,
}

/// The facts `server_info` reports, from the configuration alone.
pub fn info(config: &Config, gate: &Gate) -> Info {
    Info {
        name: "drofus-mcp",
        version: env!("CARGO_PKG_VERSION"),
        base_url: config.base_url.clone(),
        database: config.database.clone(),
        project: config.project.clone(),
        api_key_set: config.api_key.is_some(),
        authentication: "NOT IMPLEMENTED: every dRofus tool stops with an error before any request is sent (see AUTH.md)",
        drofus_tools: ENDPOINTS.len(),
        writes: if gate.enabled() {
            "approval gate enabled (--allow-writes), but no write tool exists: every tool is read-only"
        } else {
            "disabled: every tool is read-only"
        },
        usage: "Every dRofus tool takes optional `db` and `pr` (overriding the configured database and project). List \
                tools take `select`, `orderby`, `filter`, `top` (default 50, ceiling 200) and `skip`, named without the \
                `$` that dRofus's own query options carry; get tools take `select`. Query values are passed to dRofus \
                unchanged.",
        missing: config.missing(),
    }
}

/// What a dRofus tool answers: the JSON as text, an image as an image block (with a line saying what
/// it is), or a tool error. A failure is the tool's answer, not a protocol error, so the model sees it.
pub fn answer_to_result(outcome: Result<Answer, ToolError>) -> CallToolResult {
    match outcome {
        Ok(Answer::Json(v)) => CallToolResult::success(vec![ContentBlock::text(v.to_string())]),
        Ok(Answer::Image { mime, bytes, base64 }) => CallToolResult::success(vec![
            ContentBlock::text(format!("{mime}, {bytes} bytes")),
            ContentBlock::image(base64, mime),
        ]),
        Err(e) => CallToolResult::error(vec![ContentBlock::text(e.0)]),
    }
}

fn tool_of(endpoint: &Endpoint) -> Tool {
    Tool::new(endpoint.name, endpoint.description(), endpoint.input_schema()).annotate(
        ToolAnnotations::new()
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(true),
    )
}

fn json_result<T: Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let json = serde_json::to_string(value)
        .map_err(|e| McpError::internal_error(format!("failed to serialize response: {e}"), None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
}

// `tool_router` is read by the `#[tool_handler]`-generated dispatch, which rustc's dead-code
// analysis does not see through (the rmcp examples suppress it the same way).
#[allow(dead_code)]
#[derive(Clone)]
pub struct DrofusMcp {
    config: Config,
    gate: Gate,
    tool_router: ToolRouter<DrofusMcp>,
}

#[tool_router]
impl DrofusMcp {
    /// A server whose requests are authenticated by [`Unauthenticated`]: until the sign-in is decided,
    /// every dRofus call stops at that error.
    pub fn new(config: Config) -> Self {
        Self::with_authenticator(config, Arc::new(Unauthenticated))
    }

    /// The same server with the write gate it was started with. No tool consults it yet.
    pub fn with_gate(mut self, gate: Gate) -> Self {
        self.gate = gate;
        self
    }

    /// The one place an authenticator is chosen.
    pub fn with_authenticator(config: Config, auth: Arc<dyn Authenticator>) -> Self {
        let client = Arc::new(Client::new(&config, auth));
        let mut tool_router = Self::tool_router();
        for endpoint in ENDPOINTS {
            let client = client.clone();
            tool_router.add_route(ToolRoute::new_dyn(tool_of(endpoint), move |ctx: ToolCallContext<'_, Self>| {
                let client = client.clone();
                Box::pin(async move {
                    let args = ctx.arguments.unwrap_or_default();
                    Ok(answer_to_result(client.call(endpoint, &args).await))
                })
            }));
        }
        Self { config, gate: Gate::new(crate::approval::GateConfig::disabled()), tool_router }
    }

    #[tool(
        description = "What this dRofus MCP server is and how it is configured: its name and version, the dRofus address it was given (or null), whether a credential was given (never the credential), and what is still missing. Makes no network call; use it first to see whether the server is ready."
    )]
    fn server_info(&self) -> Result<CallToolResult, McpError> {
        json_result(&info(&self.config, &self.gate))
    }
}

// `router = self.tool_router`: the macro's default is a fresh `Self::tool_router()`, which holds only
// `server_info` and would silently leave the 89 table-built tools unlisted and uncallable.
#[tool_handler(router = self.tool_router)]
impl ServerHandler for DrofusMcp {
    fn get_info(&self) -> ServerInfo {
        // Named explicitly: `Implementation::from_build_env()` would report rmcp's own name and version.
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("drofus-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Read-only access to a dRofus database through its Project API. Call server_info first: it says \
                 whether the dRofus address, database and project have been configured, and that authentication is \
                 not implemented yet, so every dRofus tool currently ends with an error saying so."
                    .to_string(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTED: &str = include_str!("../operations.txt");

    #[test]
    fn an_unconfigured_server_says_what_is_missing() {
        let i = info(&Config::default(), &Gate::new(crate::approval::GateConfig::disabled()));
        assert_eq!(i.name, "drofus-mcp");
        assert!(!i.api_key_set);
        assert_eq!(i.base_url, None);
        assert_eq!(i.missing.len(), 3);
        assert!(i.authentication.contains("NOT IMPLEMENTED"));
    }

    #[test]
    fn a_configured_server_reports_the_address_and_never_the_key() {
        let config = Config::resolve(
            Some("https://d.example".into()),
            Some("hunter2".into()),
            Some("db".into()),
            Some("pr".into()),
            |_| None,
        );
        let i = info(&config, &Gate::new(crate::approval::GateConfig::disabled()));
        assert_eq!(i.base_url.as_deref(), Some("https://d.example"));
        assert!(i.api_key_set && i.missing.is_empty());
        let json = serde_json::to_string(&i).unwrap();
        assert!(!json.contains("hunter2"), "{json}");
    }

    /// The checked-in list (`operations.txt`, from the OpenAPI definition by `gen_endpoints.py`) against
    /// what is registered: a missing operation, or a tool for one that is not listed, fails here.
    #[test]
    fn every_listed_operation_has_exactly_one_tool_and_nothing_else_is_registered() {
        let server = DrofusMcp::new(Config::default());
        let tools = server.tool_router.list_all();
        let mut by_op: Vec<String> = ENDPOINTS.iter().map(|e| format!("{} {}", e.kind.method(), e.path)).collect();
        by_op.sort();
        let listed: Vec<String> = LISTED.lines().map(str::to_string).collect();
        assert_eq!(by_op, listed, "the table and operations.txt disagree");
        assert_eq!(listed.len(), 89);
        assert_eq!(tools.len(), 89 + 1, "89 operations plus server_info");
        assert!(tools.iter().any(|t| t.name == "server_info"));
        let get = listed.iter().filter(|l| l.starts_with("GET ")).count();
        assert_eq!((get, listed.len() - get), (73, 16), "73 GET and 16 OPTIONS, as ENDPOINTS.md says");
        assert!(
            listed.iter().all(|l| l.starts_with("GET ") || l.starts_with("OPTIONS ")),
            "no write operation"
        );
    }

    #[test]
    fn every_drofus_tool_is_annotated_read_only_and_described() {
        let server = DrofusMcp::new(Config::default());
        for t in server.tool_router.list_all().iter().filter(|t| t.name != "server_info") {
            assert_eq!(t.annotations.as_ref().and_then(|a| a.read_only_hint), Some(true), "{}", t.name);
            let d = t.description.as_deref().unwrap();
            assert!(d.contains("Read-only") && d.contains("Needs authentication"), "{}", t.name);
        }
    }

    #[test]
    fn an_answer_becomes_text_an_image_block_or_a_tool_error() {
        let ok = answer_to_result(Ok(Answer::Json(serde_json::json!({"a": 1}))));
        assert_ne!(ok.is_error, Some(true));
        let img = answer_to_result(Ok(Answer::Image { mime: "image/png".into(), bytes: 3, base64: "AAEC".into() }));
        assert_eq!(img.content.len(), 2);
        assert!(matches!(img.content[1], ContentBlock::Image(_)));
        let err = answer_to_result(Err(ToolError("nope".into())));
        assert_eq!(err.is_error, Some(true));
    }

    /// Through the `ServerHandler`, which is what a client reaches: `#[tool_handler]` once ignored the
    /// field and served a fresh router holding only `server_info`, while the router tests passed.
    #[test]
    fn the_handler_serves_the_table_built_tools() {
        let server = DrofusMcp::new(Config::default());
        assert!(ServerHandler::get_tool(&server, "drofus_rooms_list").is_some());
        assert!(ServerHandler::get_tool(&server, "drofus_users_get").is_some());
    }

    #[test]
    fn a_route_carries_its_endpoints_own_schema() {
        let server = DrofusMcp::new(Config::default());
        let route = server.tool_router.map.get("drofus_rooms_get").expect("registered");
        assert_eq!(route.attr.input_schema["required"], serde_json::json!(["id"]));
        assert!(route.attr.input_schema["properties"].get("select").is_some());
    }

    #[test]
    fn server_info_answers_without_a_transport() {
        let server = DrofusMcp::new(Config::default());
        let result = server.server_info().unwrap();
        assert_ne!(result.is_error, Some(true));
    }
}
