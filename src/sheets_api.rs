//! HTTP adapters for the room layout sheet library -- the thin half of `crate::sheets`.
//!
//! Reads: the list, one sheet, and **suggest** (the sheets closest to a room, asked for in
//! sheet creation mode only). Writes: the extractor's push, setting or clearing a sheet's
//! room, accepting or rejecting a stashed push, and delete. HTTP-only, for the reason
//! blueprint writes are: the MCP server is a separate process and reads the same files.

use axum::{
    extract::{Path as UrlPath, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use crate::service::room_locator::RoomRef;
use crate::service::sheet_suggest::{self, Suggestions};
use crate::sheets::{
    self, DecideRequest, PushOutcome, PushRequest, SheetError, SheetRecord, SheetSummary, UpdateRequest,
};
use crate::state::Shared;

pub fn to_http(err: SheetError) -> (StatusCode, String) {
    match err {
        SheetError::NotFileBacked => (
            StatusCode::SERVICE_UNAVAILABLE,
            "this server was not started from a settings directory".to_string(),
        ),
        SheetError::Invalid(msg) => (StatusCode::BAD_REQUEST, msg),
        SheetError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
        SheetError::Conflict(msg) => (StatusCode::CONFLICT, msg),
        SheetError::Internal(e) => {
            tracing::error!("sheets error: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
        }
    }
}

fn dir(state: &Shared) -> Result<std::path::PathBuf, (StatusCode, String)> {
    state.projects_dir().cloned().ok_or(SheetError::NotFileBacked).map_err(to_http)
}

pub async fn http_list_sheets(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
) -> Result<Json<Vec<SheetSummary>>, (StatusCode, String)> {
    sheets::list(&dir(&state)?, &project_id).map(Json).map_err(to_http)
}

pub async fn http_get_sheet(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
) -> Result<Json<SheetRecord>, (StatusCode, String)> {
    sheets::load(&dir(&state)?, &project_id, &id).map(Json).map_err(to_http)
}

/// The extractor's push. 200 with what happened to each sheet: created, updated, or stashed
/// as pending because its number changed.
pub async fn http_push_sheets(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(request): Json<PushRequest>,
) -> Result<Json<Vec<PushOutcome>>, (StatusCode, String)> {
    sheets::push(&dir(&state)?, &project_id, request).map(Json).map_err(to_http)
}

pub async fn http_update_sheet(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
    Json(request): Json<UpdateRequest>,
) -> Result<Json<SheetRecord>, (StatusCode, String)> {
    sheets::update(&dir(&state)?, &project_id, &id, request).map(Json).map_err(to_http)
}

pub async fn http_accept_sheet_push(
    State(state): State<Shared>,
    UrlPath((project_id, id, pending_id)): UrlPath<(String, String, u32)>,
    Json(request): Json<DecideRequest>,
) -> Result<Json<SheetRecord>, (StatusCode, String)> {
    sheets::accept(&dir(&state)?, &project_id, &id, pending_id, request)
        .map(Json)
        .map_err(to_http)
}

pub async fn http_reject_sheet_push(
    State(state): State<Shared>,
    UrlPath((project_id, id, pending_id)): UrlPath<(String, String, u32)>,
    Json(request): Json<DecideRequest>,
) -> Result<Json<SheetRecord>, (StatusCode, String)> {
    sheets::reject(&dir(&state)?, &project_id, &id, pending_id, request)
        .map(Json)
        .map_err(to_http)
}

#[derive(Deserialize)]
pub struct DeleteParams {
    pub base: String,
}

pub async fn http_delete_sheet(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
    Query(params): Query<DeleteParams>,
) -> Result<StatusCode, (StatusCode, String)> {
    sheets::delete(&dir(&state)?, &project_id, &id, &params.base).map_err(to_http)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct SuggestParams {
    pub model_id: String,
    pub room_id: String,
}

/// The library sheets closest to one room, closest first. Offers; applies nothing.
pub async fn http_suggest_sheets(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Query(params): Query<SuggestParams>,
) -> Result<Json<Suggestions>, (StatusCode, String)> {
    let records = sheets::load_all(&dir(&state)?, &project_id).map_err(to_http)?;
    let room = RoomRef { model_id: params.model_id, room_id: params.room_id };
    match sheet_suggest::suggest(&state, &project_id, &room, &records) {
        Ok(Some(result)) => Ok(Json(result)),
        Ok(None) => Err((StatusCode::NOT_FOUND, "no such room in this project".to_string())),
        Err(crate::service::ServiceError::Invalid(msg)) => Err((StatusCode::BAD_REQUEST, msg)),
        Err(crate::service::ServiceError::Internal(e)) => {
            tracing::error!("suggest error: {e:#}");
            Err((StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string()))
        }
    }
}
