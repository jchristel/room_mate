//! HTTP adapters for room tasks -- the thin half of `crate::tasks`.
//!
//! **Two audiences.** A person makes tasks from assignments (or a library sheet), reads the list
//! and dismisses or resends. A Revit session `POST`s `roomtasks` naming its model to be handed
//! that model's queued tasks (a POST, because asking changes them to in progress) and `POST`s
//! `roomtasks/report` with a status per task. Reads are also MCP tools; every write is HTTP-only.

use axum::{
    extract::{Path as UrlPath, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::assignments;
use crate::blueprints_api::service_error;
use crate::service::blueprint_capture::qualify_room;
use crate::service::rls_plan::{DepthOverride, RlsRequest};
use crate::service::rls_tags::TagRequest;
use crate::service::room_locator::RoomRef;
use crate::service::task_build::{
    populate_from_assignment, rls_from_sheet, update_from_assignment, PopulateParams, UpdateParams,
};
use crate::service::ServiceError;
use crate::state::Shared;
use crate::tasks::{self, Report, ReportOutcome, Task, TaskError, TaskState, TaskSummary, UpdateMode};

const MAX_ROOMS_PER_REQUEST: usize = 200;

fn to_http(err: TaskError) -> (StatusCode, String) {
    match err {
        TaskError::NotFileBacked => (
            StatusCode::SERVICE_UNAVAILABLE,
            "this server was not started from a settings directory".to_string(),
        ),
        TaskError::Invalid(m) => (StatusCode::BAD_REQUEST, m),
        TaskError::NotFound(m) => (StatusCode::NOT_FOUND, m),
        TaskError::Conflict(m) => (StatusCode::CONFLICT, m),
        TaskError::Internal(e) => {
            tracing::error!("tasks error: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
        }
    }
}

fn dir(state: &Shared) -> Result<std::path::PathBuf, (StatusCode, String)> {
    state.projects_dir().cloned().ok_or(TaskError::NotFileBacked).map_err(to_http)
}

#[derive(Deserialize)]
pub struct ListParams {
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub state: Option<TaskState>,
}

pub async fn http_list_tasks(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<TaskSummary>>, (StatusCode, String)> {
    tasks::list(&dir(&state)?, &project_id, params.model_id.as_deref(), params.state)
        .map(Json)
        .map_err(to_http)
}

pub async fn http_get_task(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
) -> Result<Json<Task>, (StatusCode, String)> {
    tasks::load(&dir(&state)?, &project_id, &id).map(Json).map_err(to_http)
}

/// A create-RLS request for ONE room, to look at before it is made.
#[derive(Deserialize)]
pub struct RlsPreviewBody {
    pub sheet_id: String,
    pub room_id: String,
    #[serde(default)]
    pub sheet_size: Option<String>,
    #[serde(default)]
    pub depths: Vec<DepthOverride>,
    #[serde(default)]
    pub tags: Option<TagRequest>,
}

/// What the task for this room would tell Revit about the sheet size and the elevations, and what
/// it would warn about. A read of a draft: nothing is stored, and the answer is built by the same
/// function the task is made from, so a preview is the task and not a second opinion.
pub async fn http_preview_rls(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(body): Json<RlsPreviewBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let projects = dir(&state)?;
    let room: RoomRef = match qualify_room(&state, &project_id, &body.room_id, None) {
        Ok(Some(room)) => room,
        Ok(None) => return Err((StatusCode::NOT_FOUND, "no such room in this project".to_string())),
        Err(e) => return Err(service_error(e)),
    };
    let request = RlsRequest {
        sheet_size: body.sheet_size,
        depths: body.depths,
        overwrite_rls: false,
        tags: Some(body.tags.unwrap_or_default()),
    };
    match rls_from_sheet(&state, &projects, &project_id, &body.sheet_id, &room, "preview", &request) {
        Ok(new) => Ok(Json(serde_json::json!({ "room": new.room, "rls": new.rls, "findings": new.findings }))),
        Err(e) => Err(service_error(e)),
    }
}

/// What to make. `populate_room` and `update_room` name assignments (one task per assignment);
/// `create_rls` names a library sheet and the rooms to make sheets for.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CreateBody {
    PopulateRoom {
        assignment_ids: Vec<String>,
        phase: String,
        #[serde(default)]
        overwrite_layout: bool,
        #[serde(default)]
        create_rls: bool,
        #[serde(default)]
        overwrite_rls: bool,
    },
    CreateRls {
        sheet_id: String,
        room_ids: Vec<String>,
        /// Ignored: the views take the phase of their room, read in Revit. Still accepted so a client
        /// written before that does not fail.
        #[serde(default)]
        #[allow(dead_code)]
        phase: Option<String>,
        #[serde(default)]
        overwrite_rls: bool,
        /// Carry the project's tag rules for the sheet's viewport types. On by default, which is
        /// what a request written before the field meant; the library page turns it off, because a
        /// sheet made on its own is the layout and nothing placed on it.
        #[serde(default = "yes")]
        with_tags: bool,
        /// `a0` to `a3`: the size of the sheets made. Absent: the template's own.
        #[serde(default)]
        sheet_size: Option<String>,
        /// Changes to how far an elevation looks, by the template's view id.
        #[serde(default)]
        depths: Vec<DepthOverride>,
        /// What a person decided about untagged items (`rls_tags`). Ignored without `with_tags`.
        #[serde(default)]
        tags: Option<TagRequest>,
    },
    /// Bring rooms populated earlier back in line with their assignments' blueprints.
    UpdateRoom {
        assignment_ids: Vec<String>,
        phase: String,
        mode: UpdateMode,
        /// Defaults to true: report the difference and write nothing.
        #[serde(default = "yes")]
        dry_run: bool,
    },
}

fn yes() -> bool {
    true
}

/// What happened to one room of a request: the task made, or why not.
#[derive(Serialize)]
pub struct Made {
    /// The assignment id or room id the request named.
    pub subject: String,
    pub room_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
}

fn refusal(subject: &str, room_name: &str, reason: String) -> Made {
    Made {
        subject: subject.to_string(),
        room_name: room_name.to_string(),
        task: None,
        refused: Some(reason),
    }
}

fn queue(projects: &std::path::Path, project: &str, subject: &str, new: crate::tasks::NewTask) -> Made {
    let room_name = new.room.name.clone();
    match tasks::create(projects, project, new) {
        Ok(task) => Made {
            subject: subject.to_string(),
            room_name,
            task: Some(TaskSummary::from(&task)),
            refused: None,
        },
        Err(e) => refusal(subject, &room_name, e.to_string()),
    }
}

/// Make tasks. **Each room stands alone**: one that cannot be made into a task is a row saying
/// why, and the rest are still queued.
pub async fn http_create_tasks(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(body): Json<CreateBody>,
) -> Result<(StatusCode, Json<Vec<Made>>), (StatusCode, String)> {
    let projects = dir(&state)?;
    let mut out = Vec::new();
    match body {
        CreateBody::PopulateRoom { assignment_ids, phase, overwrite_layout, create_rls, overwrite_rls } => {
            if assignment_ids.len() > MAX_ROOMS_PER_REQUEST {
                return Err((StatusCode::BAD_REQUEST, format!("at most {MAX_ROOMS_PER_REQUEST} rooms at a time")));
            }
            let all = assignments::list_raw(&projects, &project_id).map_err(crate::blueprints_api::to_http)?;
            let params = PopulateParams { phase, overwrite_layout, create_rls, overwrite_rls };
            for id in assignment_ids {
                let Some(assignment) = all.iter().find(|a| a.id == id) else {
                    out.push(refusal(&id, "", "no such assignment".to_string()));
                    continue;
                };
                match populate_from_assignment(&state, &projects, &project_id, assignment, &params) {
                    Ok(new) => out.push(queue(&projects, &project_id, &id, new)),
                    Err(ServiceError::Invalid(reason)) => out.push(refusal(&id, &assignment.room_name, reason)),
                    Err(e) => return Err(service_error(e)),
                }
            }
        }
        CreateBody::UpdateRoom { assignment_ids, phase, mode, dry_run } => {
            if assignment_ids.len() > MAX_ROOMS_PER_REQUEST {
                return Err((StatusCode::BAD_REQUEST, format!("at most {MAX_ROOMS_PER_REQUEST} rooms at a time")));
            }
            let all = assignments::list_raw(&projects, &project_id).map_err(crate::blueprints_api::to_http)?;
            let params = UpdateParams { phase, mode, dry_run };
            for id in assignment_ids {
                let Some(assignment) = all.iter().find(|a| a.id == id) else {
                    out.push(refusal(&id, "", "no such assignment".to_string()));
                    continue;
                };
                match update_from_assignment(&state, &projects, &project_id, assignment, &params) {
                    Ok(new) => out.push(queue(&projects, &project_id, &id, new)),
                    Err(ServiceError::Invalid(reason)) => out.push(refusal(&id, &assignment.room_name, reason)),
                    Err(e) => return Err(service_error(e)),
                }
            }
        }
        CreateBody::CreateRls { sheet_id, room_ids, phase: _, overwrite_rls, with_tags, sheet_size, depths, tags } => {
            let request = RlsRequest {
                sheet_size,
                depths,
                overwrite_rls,
                tags: with_tags.then(|| tags.unwrap_or_default()),
            };
            if room_ids.len() > MAX_ROOMS_PER_REQUEST {
                return Err((StatusCode::BAD_REQUEST, format!("at most {MAX_ROOMS_PER_REQUEST} rooms at a time")));
            }
            for room_id in room_ids {
                let room: RoomRef = match qualify_room(&state, &project_id, &room_id, None) {
                    Ok(Some(room)) => room,
                    Ok(None) => {
                        out.push(refusal(&room_id, "", "no such room in this project".to_string()));
                        continue;
                    }
                    Err(ServiceError::Invalid(reason)) => {
                        out.push(refusal(&room_id, "", reason));
                        continue;
                    }
                    Err(e) => return Err(service_error(e)),
                };
                match rls_from_sheet(
                    &state,
                    &projects,
                    &project_id,
                    &sheet_id,
                    &room,
                    crate::tasks::ROOM_OWN_PHASE,
                    &request,
                ) {
                    Ok(mut new) => {
                        if !with_tags {
                            new.tag_rules.clear();
                        }
                        out.push(queue(&projects, &project_id, &room_id, new));
                    }
                    Err(ServiceError::Invalid(reason)) => out.push(refusal(&room_id, "", reason)),
                    Err(e) => return Err(service_error(e)),
                }
            }
        }
    }
    Ok((StatusCode::CREATED, Json(out)))
}

#[derive(Deserialize)]
pub struct ClaimBody {
    /// The model the session has open: tasks are asked for by it.
    pub model_id: String,
    /// Only tasks of these kinds; absent or empty, every kind.
    #[serde(default)]
    pub kinds: Vec<crate::tasks::TaskKind>,
}

/// A Revit session asks for its model's tasks. Every queued one is handed over, in full, and
/// marked in progress, so another session is not given it.
pub async fn http_claim_tasks(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(body): Json<ClaimBody>,
) -> Result<Json<Vec<Task>>, (StatusCode, String)> {
    tasks::claim(&dir(&state)?, &project_id, &body.model_id, &body.kinds)
        .map(Json)
        .map_err(to_http)
}

#[derive(Deserialize)]
pub struct ReportBody {
    pub reports: Vec<Report>,
}

/// A Revit session reports the tasks it was handed: a status each, with what it found.
pub async fn http_report_tasks(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(body): Json<ReportBody>,
) -> Result<Json<Vec<ReportOutcome>>, (StatusCode, String)> {
    tasks::report(&dir(&state)?, &project_id, body.reports).map(Json).map_err(to_http)
}

pub async fn http_dismiss_task(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
) -> Result<Json<Task>, (StatusCode, String)> {
    tasks::dismiss(&dir(&state)?, &project_id, &id).map(Json).map_err(to_http)
}

pub async fn http_resend_task(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
) -> Result<Json<Task>, (StatusCode, String)> {
    tasks::resend(&dir(&state)?, &project_id, &id).map(Json).map_err(to_http)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A create-RLS request names no phase: the views take their room's. One that still sends a
    /// phase (an older client) is accepted and the phase is ignored.
    #[test]
    fn test_a_create_rls_request_needs_no_phase_and_ignores_one() {
        let bare: CreateBody =
            serde_json::from_str(r#"{"kind":"create_rls","sheet_id":"s","room_ids":["1"]}"#).unwrap();
        assert!(matches!(bare, CreateBody::CreateRls { phase: None, .. }));
        let old: CreateBody =
            serde_json::from_str(r#"{"kind":"create_rls","sheet_id":"s","room_ids":["1"],"phase":"New"}"#).unwrap();
        assert!(matches!(old, CreateBody::CreateRls { phase: Some(_), .. }));
    }
}
