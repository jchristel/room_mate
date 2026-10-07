//! HTTP adapters for the authored connections document — the thin half of
//! `crate::connections`, as `reports_api` is of `reports`.
//!
//! `GET /projects/{id}/connections` reads; `PUT` replaces the whole list and
//! names the version it read (`base`), answering 409 when the document has moved.
//! Writes are HTTP-only, for the reason settings writes are: the MCP server is a
//! separate process and reads the same file.

use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    Json,
};

use crate::connections::{self, ConnectionsDocument, ConnectionsError, SaveRequest};
use crate::state::Shared;

fn to_http(err: ConnectionsError) -> (StatusCode, String) {
    match err {
        ConnectionsError::NotFileBacked => (
            StatusCode::SERVICE_UNAVAILABLE,
            "this server was not started from a settings directory".to_string(),
        ),
        ConnectionsError::Invalid(msg) => (StatusCode::BAD_REQUEST, msg),
        ConnectionsError::Conflict(msg) => (StatusCode::CONFLICT, msg),
        ConnectionsError::Internal(e) => {
            tracing::error!("connections error: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
        }
    }
}

pub async fn http_get_connections(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
) -> Result<Json<ConnectionsDocument>, (StatusCode, String)> {
    let dir = state.projects_dir().ok_or(ConnectionsError::NotFileBacked).map_err(to_http)?;
    connections::load(dir, &project_id).map(Json).map_err(to_http)
}

pub async fn http_put_connections(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(request): Json<SaveRequest>,
) -> Result<Json<ConnectionsDocument>, (StatusCode, String)> {
    let dir = state.projects_dir().cloned().ok_or(ConnectionsError::NotFileBacked).map_err(to_http)?;
    connections::save(&state, &dir, &project_id, request).map(Json).map_err(to_http)
}
