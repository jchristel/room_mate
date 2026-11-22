//! The settings routes: each handler is an adapter over the functions in `mod.rs`, and the one place a settings error becomes a status
//! (it keeps its own mapping on purpose: 404 / 422 and an internal error that shows its text).

use super::*;

pub(super) fn to_http(err: SettingsError) -> (StatusCode, String) {
    match err {
        SettingsError::NotFileBacked => (
            StatusCode::NOT_FOUND,
            "this server has no --project-settings directory (settings editing unavailable)".to_string(),
        ),
        SettingsError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
        SettingsError::Conflict(msg) => (StatusCode::CONFLICT, msg),
        SettingsError::Invalid(msg) => (StatusCode::UNPROCESSABLE_ENTITY, msg),
        SettingsError::Internal(e) => {
            tracing::error!("settings API internal error: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}

pub(super) fn require_dir(state: &AppState) -> Result<PathBuf, (StatusCode, String)> {
    state.projects_dir().cloned().ok_or_else(|| to_http(SettingsError::NotFileBacked))
}

/// `GET /api/settings/projects`
pub async fn http_list_projects(
    State(state): State<Shared>,
) -> Result<Json<Vec<ProjectFileSummary>>, (StatusCode, String)> {
    let dir = require_dir(&state)?;
    let mut files = list_project_files(&dir).map_err(to_http)?;
    fill_model_phases(&state, &mut files);
    Ok(Json(files))
}

/// Each project's models and the phase the server holds for them, from the model index.
///
/// **Best effort, and only here**: the list is the exporter's project picker, which reads the phase to preselect
/// beside it so the window needs no second request per project. A store that cannot answer leaves the maps empty
/// (the picker then starts on the first phase), and the MCP list tool, which shares `list_project_files`, is
/// unchanged.
pub(crate) fn fill_model_phases(state: &AppState, files: &mut [ProjectFileSummary]) {
    let Ok(rows) = state.model_index() else { return };
    for row in rows {
        let Ok(Some(phase)) = state.model_phase(&row.key) else {
            continue;
        };
        for file in files.iter_mut().filter(|f| f.project_id.as_deref() == Some(row.key.project_id.as_str())) {
            file.model_phases.insert(row.key.model_id.clone(), phase.clone());
        }
    }
}

/// `GET /api/settings/projects/{id}`
pub async fn http_get_project(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
) -> Result<Json<ProjectSettingsResponse>, (StatusCode, String)> {
    let dir = require_dir(&state)?;
    let (file, settings) = get_project_file(&dir, &project_id).map_err(to_http)?;
    // Checked against the data as it is NOW, on every read of the editor: a push since
    // startup can change the answer, and the page is where it is acted on.
    let findings = crate::settings_check::check(&state, &settings);
    Ok(Json(ProjectSettingsResponse { file, settings, findings }))
}

/// `GET /api/settings/resolve/{id}` — the viewer's read-only, default-falling-
/// back variant of `http_get_project`. Same response shape; only the lookup
/// differs (`resolve_project_file`). The viewer, which holds a payload id that
/// isn't a settings `project_id`, is the sole intended caller; editors keep the
/// strict `/api/settings/projects/{id}` route for GET and PUT.
pub async fn http_get_project_resolved(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
) -> Result<Json<ProjectSettingsResponse>, (StatusCode, String)> {
    let dir = require_dir(&state)?;
    let (file, settings) = resolve_project_file(&dir, &project_id).map_err(to_http)?;
    Ok(Json(ProjectSettingsResponse { file, settings, findings: Vec::new() }))
}

/// `POST /api/settings/projects` (create)
pub async fn http_create_project(
    State(state): State<Shared>,
    Json(settings): Json<Settings>,
) -> Result<Json<SaveResponse>, (StatusCode, String)> {
    let settings = save_project(&state, None, settings).map_err(to_http)?;
    Ok(Json(SaveResponse { applied: true, settings }))
}

/// `PUT /api/settings/projects/{id}` (update)
///
/// Body is taken as raw JSON, not as `Settings`, so `merge_over_stored` can
/// still see which keys the client actually sent — typing it here would
/// collapse "not sent" into "default" before anything could preserve it.
pub async fn http_update_project(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(incoming): Json<serde_json::Value>,
) -> Result<Json<SaveResponse>, (StatusCode, String)> {
    let dir = require_dir(&state)?;
    let settings = merge_over_stored(&dir, &project_id, incoming).map_err(to_http)?;
    let settings = save_project(&state, Some(&project_id), settings).map_err(to_http)?;
    Ok(Json(SaveResponse { applied: true, settings }))
}

/// Optional `?taken_at=` on a dRofus upload — the snapshot-id half of the
/// upload envelope, carried as a query param because a raw CSV body has no
/// JSON envelope to put it in.
#[derive(Deserialize)]
pub struct ReferenceUploadQuery {
    #[serde(default)]
    pub taken_at: Option<String>,
}

/// `POST /projects/{id}/reference/{source}` — raw `text/csv` body (buffered
/// `Bytes`: real reference exports are a few MB of CSV, not the >100 MB FFE
/// case that forced `/rooms/stream` to stream), for any reference source the
/// project configures with an `upload` origin.
pub async fn http_upload_reference(
    State(state): State<Shared>,
    UrlPath((project_id, source)): UrlPath<(String, String)>,
    Query(query): Query<ReferenceUploadQuery>,
    body: axum::body::Bytes,
) -> Result<Json<ReferenceUploadResult>, (StatusCode, String)> {
    upload_reference(&state, &project_id, &source, query.taken_at.as_deref(), &body)
        .map(Json)
        .map_err(to_http)
}
