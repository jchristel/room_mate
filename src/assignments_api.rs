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
use serde::{Deserialize, Serialize};

use crate::assignments::{self, AssignRequest, Assignment, AssignmentRow, ItemOverride, Mapping, RoomKey};
use crate::blueprints;
use crate::blueprints_api::{service_error, to_http};
use crate::service::blueprint_capture::{qualify_room, room_ring};
use crate::service::blueprint_solve::{solve_rooms, RoomSolve};
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
    /// This room's changes to single items. Left out, the room's existing ones are KEPT (a client
    /// written before they existed must not clear them by assigning again); `[]` clears them.
    #[serde(default)]
    pub overrides: Option<Vec<ItemOverride>>,
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
        overrides: body.overrides,
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

#[derive(Deserialize)]
pub struct SolveBody {
    #[serde(default)]
    pub revision: Option<u32>,
    pub room_ids: Vec<String>,
}

const MAX_ROOMS_PER_BATCH: usize = 500;

/// How one blueprint revision does in each of several rooms, best fit by shape and doors. A read:
/// nothing is stored. A POST only because the list of rooms can be long.
pub async fn http_solve_blueprint(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
    Json(body): Json<SolveBody>,
) -> Result<Json<Vec<RoomSolve>>, (StatusCode, String)> {
    if body.room_ids.len() > MAX_ROOMS_PER_BATCH {
        return Err((StatusCode::BAD_REQUEST, format!("at most {MAX_ROOMS_PER_BATCH} rooms at a time")));
    }
    let blueprint = blueprints::load(&dir(&state)?, &project_id, &id).map_err(to_http)?;
    let number = body.revision.unwrap_or(blueprint.current_revision);
    let Some(revision) = blueprint.revision(number) else {
        return Err((StatusCode::NOT_FOUND, format!("the blueprint has no revision {number}")));
    };
    solve_rooms(&state, &project_id, &revision.content, &body.room_ids)
        .map(Json)
        .map_err(service_error)
}

#[derive(Deserialize)]
pub struct AssignManyBody {
    pub blueprint_id: String,
    #[serde(default)]
    pub revision: Option<u32>,
    pub room_ids: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// What happened to one room of a batch.
#[derive(Serialize)]
pub struct AssignOutcome {
    pub room_id: String,
    pub assigned: bool,
    /// Why not, when not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Put one blueprint revision in each of several rooms at the best fit, replacing each room's
/// assignment. **Best fit only**: a mapping by hand is a decision about one room and is made in
/// the single-room view. Each room stands alone, so one that cannot be assigned is a row saying
/// so and the rest still are.
pub async fn http_assign_many(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(body): Json<AssignManyBody>,
) -> Result<Json<Vec<AssignOutcome>>, (StatusCode, String)> {
    if body.room_ids.len() > MAX_ROOMS_PER_BATCH {
        return Err((StatusCode::BAD_REQUEST, format!("at most {MAX_ROOMS_PER_BATCH} rooms at a time")));
    }
    let projects = dir(&state)?;
    let mut out = Vec::new();
    for room_id in body.room_ids {
        let one = async {
            let room = match qualify_room(&state, &project_id, &room_id, None) {
                Ok(Some(room)) => room,
                Ok(None) => return Err("no such room in this project".to_string()),
                Err(e) => return Err(service_error(e).1),
            };
            let room_name = match room_ring(&state, &project_id, &room) {
                Ok(Some((name, _))) => name,
                Ok(None) => return Err("no such room in this project".to_string()),
                Err(e) => return Err(service_error(e).1),
            };
            let request = AssignRequest {
                blueprint_id: body.blueprint_id.clone(),
                revision: body.revision,
                room: RoomKey { model_id: room.model_id, room_id: room.room_id },
                room_name,
                mapping: Mapping::default(),
                overrides: None,
                note: body.note.clone(),
            };
            assignments::assign(&projects, &project_id, request).map(|_| ()).map_err(|e| to_http(e).1)
        };
        let result = one.await;
        out.push(AssignOutcome { room_id, assigned: result.is_ok(), error: result.err() });
    }
    Ok(Json(out))
}
