//! How a store or service error becomes an MCP error: invalid input is the caller's to fix, anything else is internal. One function per kind of store.

use super::*;

/// Serialize any service response into a single text content block -- the
/// same `Serialize` types the HTTP handlers already return as JSON, just
/// wrapped for MCP instead of `axum::Json`.
/// `ReportsError` -> `McpError`, the same split `settings_to_mcp_error` makes:
/// a caller fault carries its message, an internal one is logged and not
/// echoed.
pub(super) fn reports_to_mcp_error(err: roommate::reports_api::ReportsError) -> McpError {
    use roommate::reports_api::ReportsError;
    match err {
        ReportsError::NotFileBacked => McpError::internal_error(
            "this MCP server was not started with --project-settings, so it has no saved reports to read".to_string(),
            None,
        ),
        ReportsError::NotFound(msg) | ReportsError::Invalid(msg) => McpError::invalid_params(msg, None),
        ReportsError::Internal(e) => {
            tracing::error!("saved reports error: {e:#}");
            McpError::internal_error("internal error".to_string(), None)
        }
    }
}

pub(super) fn connections_to_mcp_error(err: roommate::connections::ConnectionsError) -> McpError {
    use roommate::connections::ConnectionsError;
    match err {
        ConnectionsError::NotFileBacked => McpError::internal_error(
            "this MCP server was not started with --project-settings, so it has no connections to read".to_string(),
            None,
        ),
        ConnectionsError::Invalid(msg) | ConnectionsError::Conflict(msg) => McpError::invalid_params(msg, None),
        ConnectionsError::Internal(e) => {
            tracing::error!("connections error: {e:#}");
            McpError::internal_error("internal error".to_string(), None)
        }
    }
}

pub(super) fn blueprints_to_mcp_error(err: roommate::blueprints::BlueprintError) -> McpError {
    use roommate::blueprints::BlueprintError;
    match err {
        BlueprintError::NotFileBacked => McpError::internal_error(
            "this MCP server was not started with --project-settings, so it has no blueprints to read".to_string(),
            None,
        ),
        BlueprintError::Invalid(msg) | BlueprintError::Conflict(msg) | BlueprintError::NotFound(msg) => {
            McpError::invalid_params(msg, None)
        }
        BlueprintError::Internal(e) => {
            tracing::error!("blueprints error: {e:#}");
            McpError::internal_error("internal error".to_string(), None)
        }
    }
}

pub(super) fn tasks_to_mcp_error(err: roommate::tasks::TaskError) -> McpError {
    use roommate::tasks::TaskError;
    match err {
        TaskError::NotFileBacked => McpError::internal_error(
            "this MCP server was not started with --project-settings, so it has no tasks to read".to_string(),
            None,
        ),
        TaskError::Invalid(msg) | TaskError::Conflict(msg) | TaskError::NotFound(msg) => {
            McpError::invalid_params(msg, None)
        }
        TaskError::Internal(e) => {
            tracing::error!("tasks error: {e:#}");
            McpError::internal_error("internal error".to_string(), None)
        }
    }
}

pub(super) fn tag_rules_to_mcp_error(err: roommate::tag_rules::TagRuleError) -> McpError {
    use roommate::tag_rules::TagRuleError;
    match err {
        TagRuleError::NotFileBacked => McpError::internal_error(
            "this MCP server was not started with --project-settings, so it has no tag rules to read".to_string(),
            None,
        ),
        TagRuleError::Invalid(msg) | TagRuleError::NotFound(msg) => McpError::invalid_params(msg, None),
        TagRuleError::Internal(e) => {
            tracing::error!("tag rules error: {e:#}");
            McpError::internal_error("internal error".to_string(), None)
        }
    }
}

pub(super) fn sheets_to_mcp_error(err: roommate::sheets::SheetError) -> McpError {
    use roommate::sheets::SheetError;
    match err {
        SheetError::NotFileBacked => McpError::internal_error(
            "this MCP server was not started with --project-settings, so it has no sheets to read".to_string(),
            None,
        ),
        SheetError::Invalid(msg) | SheetError::Conflict(msg) | SheetError::NotFound(msg) => {
            McpError::invalid_params(msg, None)
        }
        SheetError::Internal(e) => {
            tracing::error!("sheets error: {e:#}");
            McpError::internal_error("internal error".to_string(), None)
        }
    }
}

pub(super) fn json_result<T: serde::Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let text = mcp_bounds::bounded(value, "", GENERIC_ADVICE).map_err(|m| McpError::invalid_params(m, None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

/// `ServiceError` -> `McpError`: an internal failure becomes `internal_error`,
/// a malformed request `invalid_params` -- the MCP counterpart of the HTTP
/// adapter's 500/400 split, mapped here in the adapter rather than in the
/// domain layer.
pub(super) fn to_mcp_error(err: ServiceError) -> McpError {
    match err {
        ServiceError::Internal(e) => {
            tracing::error!("internal service error: {e:#}");
            McpError::internal_error(e.to_string(), None)
        }
        // Caller-addressable, and worth passing verbatim: the message names
        // the offending predicate, which is what lets the client fix it
        // instead of guessing why a result was empty.
        ServiceError::Invalid(msg) => McpError::invalid_params(msg, None),
    }
}

/// `SettingsError` -> `McpError`: caller-addressable problems (unknown id,
/// invalid input) become `invalid_params`; the rest `internal_error`.
pub(super) fn settings_to_mcp_error(err: SettingsError) -> McpError {
    match err {
        SettingsError::NotFound(msg) | SettingsError::Invalid(msg) | SettingsError::Conflict(msg) => {
            McpError::invalid_params(msg, None)
        }
        SettingsError::NotFileBacked => McpError::internal_error("no project settings directory configured", None),
        SettingsError::Internal(e) => {
            tracing::error!("settings read error: {e:#}");
            McpError::internal_error(e.to_string(), None)
        }
    }
}
