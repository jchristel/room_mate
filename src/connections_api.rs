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

use crate::connections::{self, ConnectionsDocument, SaveRequest};
use crate::http_error::{projects_dir, to_http};
use crate::state::Shared;

pub async fn http_get_connections(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
) -> Result<Json<ConnectionsDocument>, (StatusCode, String)> {
    let dir = projects_dir(&state)?;
    connections::load(&dir, &project_id).map(Json).map_err(to_http)
}

pub async fn http_put_connections(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(request): Json<SaveRequest>,
) -> Result<Json<ConnectionsDocument>, (StatusCode, String)> {
    let dir = projects_dir(&state)?;
    connections::save(&state, &dir, &project_id, request).map(Json).map_err(to_http)
}
