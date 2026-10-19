//! HTTP adapters for assignments -- the thin half of `crate::assignments`.
//!
//! The list is a read; assigning and removing are HTTP-only writes, for the reason every
//! authored write is (the MCP server is a separate process reading the same files).
//! The room arrives as an id with the model optional, because the plan's room rows name
//! no model: it is qualified the way capture and resolve qualify it, and refused naming the
//! models when the id is ambiguous.

use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use crate::assignments::{self, AssignRequest, Assignment, AssignmentRow, Mapping, RoomKey};
use crate::blueprints_api::{service_error, to_http};
use crate::service::blueprint_capture::{qualify_room, room_ring};
use crate::state::Shared;

fn dir(state: &Shared) -> Result<std::path::PathBuf, (StatusCode, String)> {
    state
        .projects_dir()
        .cloned()
        .ok_or(crate::blueprints::BlueprintError::NotFileBacked)
        .map_err(to_http)
}

pub async fn http_list_assignments(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
) -> Result<Json<Vec<AssignmentRow>>, (StatusCode, String)> {
    assignments::list(&dir(&state)?, &project_id).map(Json).map_err(to_http)
}

#[derive(Deserialize)]
pub struct AssignBody {
    pub blueprint_id: String,
    #[serde(default)]
    pub revision: Option<u32>,
    pub room_id: String,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub mapping: Mapping,
    #[serde(default)]
    pub note: Option<String>,
}

/// Put a blueprint in a room, replacing the room's existing assignment.
pub async fn http_assign(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(body): Json<AssignBody>,
) -> Result<Json<Assignment>, (StatusCode, String)> {
    let room = match qualify_room(&state, &project_id, &body.room_id, body.model_id.as_deref()) {
        Ok(Some(room)) => room,
        Ok(None) => return Err((StatusCode::NOT_FOUND, "no such room in this project".to_string())),
        Err(e) => return Err(service_error(e)),
    };
    let room_name = match room_ring(&state, &project_id, &room) {
        Ok(Some((name, _))) => name,
        Ok(None) => return Err((StatusCode::NOT_FOUND, "no such room in this project".to_string())),
        Err(e) => return Err(service_error(e)),
    };
    let request = AssignRequest {
        blueprint_id: body.blueprint_id,
        revision: body.revision,
        room: RoomKey { model_id: room.model_id, room_id: room.room_id },
        room_name,
        mapping: body.mapping,
        note: body.note,
    };
    assignments::assign(&dir(&state)?, &project_id, request).map(Json).map_err(to_http)
}

pub async fn http_delete_assignment(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    assignments::delete(&dir(&state)?, &project_id, &id).map_err(to_http)?;
    Ok(StatusCode::NO_CONTENT)
}
