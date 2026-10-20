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
use crate::service::room_locator::RoomRef;
use crate::service::task_build::{populate_from_assignment, rls_from_sheet, PopulateParams};
use crate::service::ServiceError;
use crate::state::Shared;
use crate::tasks::{self, Report, ReportOutcome, Task, TaskError, TaskState, TaskSummary};

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

/// What to make. `populate_room` names assignments (one task per assignment); `create_rls` names a
/// library sheet and the rooms to make sheets for.
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
        phase: String,
        #[serde(default)]
        overwrite_rls: bool,
    },
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
        CreateBody::CreateRls { sheet_id, room_ids, phase, overwrite_rls } => {
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
                match rls_from_sheet(&state, &projects, &project_id, &sheet_id, &room, &phase, overwrite_rls) {
                    Ok(new) => out.push(queue(&projects, &project_id, &room_id, new)),
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
}

/// A Revit session asks for its model's tasks. Every queued one is handed over, in full, and
/// marked in progress, so another session is not given it.
pub async fn http_claim_tasks(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(body): Json<ClaimBody>,
) -> Result<Json<Vec<Task>>, (StatusCode, String)> {
    tasks::claim(&dir(&state)?, &project_id, &body.model_id).map(Json).map_err(to_http)
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
