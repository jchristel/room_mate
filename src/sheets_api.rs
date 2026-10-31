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

use crate::service::blueprint_capture::qualify_room;
use crate::service::elevation_facing::{self, Elevations};
use crate::service::room_locator::RoomRef;
use crate::service::sheet_suggest::{self, Suggestions};
use crate::sheets::{
    self, DecideRequest, PushOutcome, PushRequest, RoomSummary, SheetError, SheetRecord, SheetRoom, SheetSummary,
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

/// Which wall each elevation on a sheet looks at, worked out now from the sheet's room.
pub async fn http_sheet_elevations(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
) -> Result<Json<Elevations>, (StatusCode, String)> {
    let record = sheets::load(&dir(&state)?, &project_id, &id).map_err(to_http)?;
    elevation_facing::for_sheet(&state, &project_id, &record)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// The room's summary as the server holds it now (plan decision 25). `None` when the room is not
/// on the server: the sheet is still stored, and is ranked from its room as it is read later.
pub(crate) fn summary_of(state: &Shared, project: &str, room: &SheetRoom) -> Option<RoomSummary> {
    let room = RoomRef { model_id: room.model_id.clone(), room_id: room.room_id.clone() };
    match sheet_suggest::room_summary(state, project, &room) {
        Ok(found) => found,
        Err(e) => {
            tracing::warn!("room summary for a sheet push failed: {e}");
            None
        }
    }
}

/// The extractor's push. 200 with what happened to each sheet: created, updated, or stashed
/// as pending because its number changed. Each sheet that names a room also stores a summary of
/// it, read from the rooms and FF&E pushed already.
pub async fn http_push_sheets(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(mut request): Json<PushRequest>,
) -> Result<Json<Vec<PushOutcome>>, (StatusCode, String)> {
    let mut seen: std::collections::HashMap<(String, String), Option<RoomSummary>> = std::collections::HashMap::new();
    for sheet in &mut request.sheets {
        // Figures the extractor measured are kept; the server reads them only for a named room without any.
        if sheet.room_summary.is_none()
            && let Some(room) = &sheet.room
        {
            let key = (room.model_id.clone(), room.room_id.clone());
            sheet.room_summary = *seen.entry(key).or_insert_with(|| summary_of(&state, &project_id, room));
        }
    }
    sheets::push(&dir(&state)?, &project_id, request).map(Json).map_err(to_http)
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
    /// The room's model; left out, looked up among the project's rooms (see `qualify_room`).
    #[serde(default)]
    pub model_id: Option<String>,
    pub room_id: String,
}

/// The library sheets closest to one room, closest first. Offers; applies nothing.
pub async fn http_suggest_sheets(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Query(params): Query<SuggestParams>,
) -> Result<Json<Suggestions>, (StatusCode, String)> {
    let projects = dir(&state)?;
    let mut records = sheets::load_all(&projects, &project_id).map_err(to_http)?;
    // A sheet bound to a blueprint that is not in use is not offered (plan decision 27).
    let retired: std::collections::HashSet<String> = crate::blueprints::list(&projects, &project_id)
        .unwrap_or_default()
        .into_iter()
        .filter(|b| !b.in_use)
        .map(|b| b.id)
        .collect();
    records.retain(|r| r.blueprint.as_deref().is_none_or(|b| !retired.contains(b)));
    let room = match qualify_room(&state, &project_id, &params.room_id, params.model_id.as_deref()) {
        Ok(Some(room)) => room,
        Ok(None) => return Err((StatusCode::NOT_FOUND, "no such room in this project".to_string())),
        Err(crate::service::ServiceError::Invalid(msg)) => return Err((StatusCode::BAD_REQUEST, msg)),
        Err(crate::service::ServiceError::Internal(e)) => {
            tracing::error!("suggest error: {e:#}");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string()));
        }
    };
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
