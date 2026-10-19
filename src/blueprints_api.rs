//! HTTP adapters for blueprints -- the thin half of `crate::blueprints`, as
//! `connections_api` is of `connections`.
//!
//! Reads: the list, one blueprint, and a **capture** of a room, which is a read -- it
//! stores nothing and answers what a blueprint made from that room would hold.
//! Writes (create, add a revision, rename or move the current marker, delete) are
//! HTTP-only, for the reason settings and connections writes are: the MCP server is a
//! separate process and reads the same files.

use axum::{
    extract::{Path as UrlPath, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::blueprints::{
    self, AddRevisionRequest, Blueprint, BlueprintError, BlueprintSummary, CreateRequest, DecideRequest, PushRequest,
    UpdateRequest,
};
use crate::service::blueprint_capture::capture_room;
use crate::service::blueprint_model::{RevisionContent, Skipped};
use crate::service::room_locator::RoomRef;
use crate::state::Shared;

pub fn to_http(err: BlueprintError) -> (StatusCode, String) {
    match err {
        BlueprintError::NotFileBacked => (
            StatusCode::SERVICE_UNAVAILABLE,
            "this server was not started from a settings directory".to_string(),
        ),
        BlueprintError::Invalid(msg) => (StatusCode::BAD_REQUEST, msg),
        BlueprintError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
        BlueprintError::Conflict(msg) => (StatusCode::CONFLICT, msg),
        BlueprintError::Internal(e) => {
            tracing::error!("blueprints error: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
        }
    }
}

fn dir(state: &Shared) -> Result<std::path::PathBuf, (StatusCode, String)> {
    state.projects_dir().cloned().ok_or(BlueprintError::NotFileBacked).map_err(to_http)
}

pub async fn http_list_blueprints(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
) -> Result<Json<Vec<BlueprintSummary>>, (StatusCode, String)> {
    blueprints::list(&dir(&state)?, &project_id).map(Json).map_err(to_http)
}

pub async fn http_get_blueprint(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
) -> Result<Json<Blueprint>, (StatusCode, String)> {
    blueprints::load(&dir(&state)?, &project_id, &id).map(Json).map_err(to_http)
}

pub async fn http_create_blueprint(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(request): Json<CreateRequest>,
) -> Result<(StatusCode, Json<Blueprint>), (StatusCode, String)> {
    blueprints::create(&dir(&state)?, &project_id, request)
        .map(|b| (StatusCode::CREATED, Json(b)))
        .map_err(to_http)
}

pub async fn http_add_revision(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
    Json(request): Json<AddRevisionRequest>,
) -> Result<Json<Blueprint>, (StatusCode, String)> {
    blueprints::add_revision(&dir(&state)?, &project_id, &id, request)
        .map(Json)
        .map_err(to_http)
}

/// A revision arriving from elsewhere: stored as pending, never current. See `blueprints`.
pub async fn http_push_revision(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
    Json(request): Json<PushRequest>,
) -> Result<(StatusCode, Json<Blueprint>), (StatusCode, String)> {
    blueprints::push(&dir(&state)?, &project_id, &id, request)
        .map(|b| (StatusCode::ACCEPTED, Json(b)))
        .map_err(to_http)
}

pub async fn http_accept_pending(
    State(state): State<Shared>,
    UrlPath((project_id, id, pending_id)): UrlPath<(String, String, u32)>,
    Json(request): Json<DecideRequest>,
) -> Result<Json<Blueprint>, (StatusCode, String)> {
    blueprints::accept(&dir(&state)?, &project_id, &id, pending_id, request)
        .map(Json)
        .map_err(to_http)
}

pub async fn http_reject_pending(
    State(state): State<Shared>,
    UrlPath((project_id, id, pending_id)): UrlPath<(String, String, u32)>,
    Json(request): Json<DecideRequest>,
) -> Result<Json<Blueprint>, (StatusCode, String)> {
    blueprints::reject(&dir(&state)?, &project_id, &id, pending_id, request)
        .map(Json)
        .map_err(to_http)
}

pub async fn http_update_blueprint(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
    Json(request): Json<UpdateRequest>,
) -> Result<Json<Blueprint>, (StatusCode, String)> {
    blueprints::update(&dir(&state)?, &project_id, &id, request).map(Json).map_err(to_http)
}

#[derive(Deserialize)]
pub struct DeleteParams {
    /// The `taken_at` of the blueprint the caller read.
    pub base: String,
}

pub async fn http_delete_blueprint(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
    Query(params): Query<DeleteParams>,
) -> Result<StatusCode, (StatusCode, String)> {
    blueprints::delete(&dir(&state)?, &project_id, &id, &params.base).map_err(to_http)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct CaptureParams {
    pub model_id: String,
    pub room_id: String,
}

/// What capturing a room would produce, and nothing stored.
#[derive(Serialize)]
pub struct CaptureResponse {
    pub room_name: String,
    #[serde(flatten)]
    pub content: RevisionContent,
    /// What capture left out, as numbers.
    pub skipped: Skipped,
}

pub async fn http_capture_blueprint(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Query(params): Query<CaptureParams>,
) -> Result<Json<CaptureResponse>, (StatusCode, String)> {
    let room = RoomRef { model_id: params.model_id, room_id: params.room_id };
    // Reads the project's rooms and FF&E, which can take seconds on a large one; the other
    // service reads run inline in their handlers too.
    let result = capture_room(&state, &project_id, &room);
    match result {
        Ok(Some((captured, room_name))) => {
            Ok(Json(CaptureResponse { room_name, content: captured.content, skipped: captured.skipped }))
        }
        Ok(None) => Err((StatusCode::NOT_FOUND, "no such room in this project".to_string())),
        Err(crate::service::ServiceError::Invalid(msg)) => Err((StatusCode::BAD_REQUEST, msg)),
        Err(crate::service::ServiceError::Internal(e)) => {
            tracing::error!("capture error: {e:#}");
            Err((StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string()))
        }
    }
}
