//! HTTP adapters for tag rules -- the thin half of `crate::tag_rules`.
//!
//! The extractor `POST`s observations (the tags it read from a sheet's viewports); everyone else
//! reads. Reads are also MCP tools; writes are HTTP-only, as for blueprints and sheets.
//!
//! `observe_for` is the one way tags are taken in: the Sheets button's route calls it directly, and a
//! blueprint's bundle calls it when its revision is accepted (`blueprint_bundle::apply`), so both
//! see the same blueprint items and the same room boundaries.

use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    Json,
};

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::blueprints;
use crate::service::blueprint_geometry::Pt;
use crate::service::elevation_facing::model_room_ring;
use crate::service::room_tag_place::boundary_of;
use crate::sheets::SheetRoom;
use crate::state::Shared;
use crate::tag_rules::{
    self, Boundary, ObservationOutcome, ObservationsRequest, ObserveContext, RuleSummary, TagRule, TagRuleError, P3,
};

pub fn to_http(err: TagRuleError) -> (StatusCode, String) {
    match err {
        TagRuleError::NotFileBacked => (
            StatusCode::SERVICE_UNAVAILABLE,
            "this server was not started from a settings directory".to_string(),
        ),
        TagRuleError::Invalid(msg) => (StatusCode::BAD_REQUEST, msg),
        TagRuleError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
        TagRuleError::Internal(e) => {
            tracing::error!("tag rules error: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
        }
    }
}

/// Rooms read for one push, by `(model id, room id)`; `None` is a room the server does not hold.
type RoomRings = RefCell<HashMap<(String, i64), Option<Vec<Pt>>>>;

fn dir(state: &Shared) -> Result<std::path::PathBuf, (StatusCode, String)> {
    state.projects_dir().cloned().ok_or(TagRuleError::NotFileBacked).map_err(to_http)
}

pub async fn http_list_tag_rules(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
) -> Result<Json<Vec<RuleSummary>>, (StatusCode, String)> {
    tag_rules::list(&dir(&state)?, &project_id).map(Json).map_err(to_http)
}

pub async fn http_get_tag_rule(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
) -> Result<Json<TagRule>, (StatusCode, String)> {
    tag_rules::load(&dir(&state)?, &project_id, &id).map(Json).map_err(to_http)
}

/// The extractor's push: observations in, rules out. 200 with how many samples were taken in and
/// which tags were skipped and why.
pub async fn http_observe_tags(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(request): Json<ObservationsRequest>,
) -> Result<Json<ObservationOutcome>, (StatusCode, String)> {
    observe_for(&state, &project_id, request).map(Json)
}

/// Take observations in with what the server knows around them (plan decision 27): the blueprint the
/// push is for and its items, and the rooms the room tags were placed in. Also the way a blueprint's
/// bundle is applied (`blueprint_bundle::apply`).
pub fn observe_for(
    state: &Shared,
    project_id: &str,
    request: ObservationsRequest,
) -> Result<ObservationOutcome, (StatusCode, String)> {
    let projects = dir(state)?;
    let blueprint = match request.blueprint.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        Some(id) => Some((id.to_string(), blueprint_item_ids(&projects, project_id, id)?)),
        None => None,
    };
    // A room is read from the server once per push, however many tags are on it.
    let rooms: RoomRings = RefCell::default();
    let boundary = |model: &str, room: i64, head: P3, bound: bool, scale: f64| -> Option<Boundary> {
        let mut cache = rooms.borrow_mut();
        let ring = cache
            .entry((model.to_string(), room))
            .or_insert_with(|| {
                let at = SheetRoom { model_id: model.to_string(), room_id: room.to_string() };
                model_room_ring(state, project_id, &at).ok().flatten()
            })
            .as_deref()?;
        boundary_of(ring, (head.x, head.y), bound, scale)
    };
    let ctx = ObserveContext { blueprint, boundary: Some(&boundary) };
    tag_rules::observe(&projects, project_id, request, &ctx).map_err(to_http)
}

/// The ids of the items a room blueprint holds in its current revision: a tag on one of them is bound
/// to the blueprint. (A revision waiting to be accepted does not count: its tags wait with it.)
fn blueprint_item_ids(
    projects: &std::path::Path,
    project: &str,
    blueprint: &str,
) -> Result<HashSet<String>, (StatusCode, String)> {
    let b = blueprints::load(projects, project, blueprint).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("the tags name blueprint {blueprint:?}, which cannot be read: {e:?}"),
        )
    })?;
    Ok(b.current()
        .into_iter()
        .flat_map(|r| r.content.items.iter())
        .map(|i| i.id.clone())
        .collect())
}

pub async fn http_delete_tag_rule(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    tag_rules::delete(&dir(&state)?, &project_id, &id).map_err(to_http)?;
    Ok(StatusCode::NO_CONTENT)
}
