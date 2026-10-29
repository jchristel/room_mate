//! The MCP server: its identity and its tools.
//!
//! **One tool per method, a thin adapter over a plain function.** A tool parses its parameters,
//! calls one function that knows nothing about MCP, and serialises the answer, so what a tool does
//! can be tested without a transport (the tests below call the methods directly).
//!
//! **Read-only first.** The only tool so far, `server_info`, does not touch the network: it says what
//! this server is and what it has not been told yet. Add the dRofus tools beside it, each with a
//! description a model can choose by (what it returns, what it needs, what it leaves out).

use rmcp::{
    handler::server::router::tool::ToolRouter,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use serde::Serialize;

use crate::config::Config;

/// What `server_info` answers.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Info {
    pub name: &'static str,
    pub version: &'static str,
    /// The dRofus address in use, or null when none was given.
    pub base_url: Option<String>,
    /// Whether a credential was given. The credential itself is never returned.
    pub api_key_set: bool,
    /// What still has to be configured before dRofus tools can work.
    pub missing: Vec<&'static str>,
}

/// The facts `server_info` reports, from the configuration alone.
pub fn info(config: &Config) -> Info {
    Info {
        name: "drofus-mcp",
        version: env!("CARGO_PKG_VERSION"),
        base_url: config.base_url.clone(),
        api_key_set: config.api_key.is_some(),
        missing: config.missing(),
    }
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
    tool_router: ToolRouter<DrofusMcp>,
}

#[tool_router]
impl DrofusMcp {
    pub fn new(config: Config) -> Self {
        Self { config, tool_router: Self::tool_router() }
    }

    #[tool(
        description = "What this dRofus MCP server is and how it is configured: its name and version, the dRofus address it was given (or null), whether a credential was given (never the credential), and what is still missing. Makes no network call; use it first to see whether the server is ready."
    )]
    fn server_info(&self) -> Result<CallToolResult, McpError> {
        json_result(&info(&self.config))
    }
}

#[tool_handler]
impl ServerHandler for DrofusMcp {
    fn get_info(&self) -> ServerInfo {
        // Named explicitly: `Implementation::from_build_env()` would report rmcp's own name and version.
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("drofus-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Read-only access to a dRofus database. Call server_info first: it says whether the dRofus \
                 address and credential have been configured."
                    .to_string(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unconfigured_server_says_what_is_missing() {
        let i = info(&Config::default());
        assert_eq!(i.name, "drofus-mcp");
        assert!(!i.api_key_set);
        assert_eq!(i.base_url, None);
        assert_eq!(i.missing.len(), 2);
    }

    #[test]
    fn a_configured_server_reports_the_address_and_never_the_key() {
        let config = Config::resolve(Some("https://d.example".into()), Some("hunter2".into()), |_| None);
        let i = info(&config);
        assert_eq!(i.base_url.as_deref(), Some("https://d.example"));
        assert!(i.api_key_set && i.missing.is_empty());
        let json = serde_json::to_string(&i).unwrap();
        assert!(!json.contains("hunter2"), "{json}");
    }

    #[test]
    fn the_tool_is_registered_and_answers_without_a_transport() {
        let server = DrofusMcp::new(Config::default());
        let tools = server.tool_router.list_all();
        assert_eq!(tools.iter().map(|t| t.name.as_ref()).collect::<Vec<_>>(), vec!["server_info"]);
        let result = server.server_info().unwrap();
        assert_ne!(result.is_error, Some(true));
    }
}
