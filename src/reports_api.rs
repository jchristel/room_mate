//! Saved reports: one JSON document per report, read and written on disk.
//!
//! **Beside the project settings, never inside them**, and the reasoning is in
//! `roommate_shared::reports::SavedReport`. What that costs here is this small
//! module; what it buys is that a malformed report cannot fail a project's
//! boot-validated settings, and that saving one touches one file rather than
//! rewriting the settings object every milestone pin lives in.
//!
//! Layout, under the `--project-settings` directory:
//!
//! ```text
//! <projects_dir>/
//!   <project-id>.toml        the settings, untouched by any of this
//!   reports/
//!     <project-id>/
//!       <report-id>.json     one saved report
//! ```
//!
//! **`reports/` is a reserved name inside that directory**, the same trick the
//! store uses for `reference/` and `pending/`: `bootstrap::load_project_settings_dir`
//! only reads `*.toml` files directly inside it, so a subdirectory is invisible
//! to the loader and costs it nothing.
//!
//! Layout mirrors `settings_api`: a transport-agnostic core with typed errors,
//! and thin Axum adapters at the bottom. Writes are HTTP-only for the reason
//! settings writes are — the MCP server is a separate process, and its reads
//! come off the same files.

use std::path::{Path, PathBuf};

use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    Json,
};

use crate::reports::{SavedReport, SavedReportSummary};
use crate::state::{is_path_safe_component, Shared};

/// Typed failure; each transport maps it itself, as `SettingsError` is.
#[derive(Debug)]
pub enum ReportsError {
    /// The state wasn't built from a settings directory (in-memory tests).
    NotFileBacked,
    NotFound(String),
    /// An id that cannot be a file name, or a body whose id disagrees with the
    /// path — both caller faults, with the message saying which.
    Invalid(String),
    Internal(anyhow::Error),
}

/// Where one project's saved reports live.
fn project_dir(projects_dir: &Path, project_id: &str) -> Result<PathBuf, ReportsError> {
    if !is_path_safe_component(project_id) {
        return Err(ReportsError::Invalid(format!("project id {project_id:?} cannot be a directory name")));
    }
    Ok(projects_dir.join("reports").join(project_id))
}

fn report_path(projects_dir: &Path, project_id: &str, report_id: &str) -> Result<PathBuf, ReportsError> {
    if !is_path_safe_component(report_id) {
        return Err(ReportsError::Invalid(format!("report id {report_id:?} cannot be a file name")));
    }
    Ok(project_dir(projects_dir, project_id)?.join(format!("{report_id}.json")))
}

/// Every saved report for one project, newest name order.
///
/// **A rotten file is skipped rather than failing the list**, which is the
/// settings list's rule and the same reasoning: this list is exactly what
/// someone opens to notice a broken report, so it has to stay usable while one
/// exists. The broken file keeps its place on disk and says so when opened.
pub fn list_reports(projects_dir: &Path, project_id: &str) -> Result<Vec<SavedReportSummary>, ReportsError> {
    let dir = project_dir(projects_dir, project_id)?;
    let Ok(entries) = std::fs::read_dir(&dir) else {
        // No directory yet simply means nobody has saved one.
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match read_file(&path) {
            Ok(report) => out.push(SavedReportSummary::from(&report)),
            Err(e) => tracing::warn!("skipping unreadable saved report {}: {e:#}", path.display()),
        }
    }
    // By name, case-insensitively: the rail is read, not sorted by a machine.
    out.sort_by_key(|row| row.name.to_lowercase());
    Ok(out)
}

pub fn get_report(projects_dir: &Path, project_id: &str, report_id: &str) -> Result<SavedReport, ReportsError> {
    let path = report_path(projects_dir, project_id, report_id)?;
    if !path.exists() {
        return Err(ReportsError::NotFound(format!(
            "no saved report {report_id:?} for project {project_id:?}"
        )));
    }
    read_file(&path).map_err(ReportsError::Internal)
}

fn read_file(path: &Path) -> anyhow::Result<SavedReport> {
    let bytes = std::fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Write one report, creating the project's directory if this is its first.
///
/// **Temp then rename**, so a reader never sees half a document: the same rule
/// the settings save and every snapshot write follow. The id in the path wins
/// over the id in the body — or rather, a disagreement is refused, because
/// silently renaming someone's report to match a URL is a surprise nobody can
/// see.
pub fn save_report(
    projects_dir: &Path,
    project_id: &str,
    report_id: &str,
    report: &SavedReport,
) -> Result<(), ReportsError> {
    if report.id != report_id {
        return Err(ReportsError::Invalid(format!(
            "report id {:?} in the body does not match {report_id:?} in the path",
            report.id
        )));
    }
    if report.name.trim().is_empty() {
        return Err(ReportsError::Invalid("a saved report needs a name".to_string()));
    }
    let path = report_path(projects_dir, project_id, report_id)?;
    let dir = path.parent().expect("a report path always has a parent");
    std::fs::create_dir_all(dir).map_err(|e| ReportsError::Internal(e.into()))?;

    let json = serde_json::to_vec_pretty(report).map_err(|e| ReportsError::Internal(e.into()))?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &json).map_err(|e| ReportsError::Internal(e.into()))?;
    std::fs::rename(&temp, &path).map_err(|e| ReportsError::Internal(e.into()))?;
    tracing::info!("saved report {report_id} for project {project_id}");
    Ok(())
}

pub fn delete_report(projects_dir: &Path, project_id: &str, report_id: &str) -> Result<(), ReportsError> {
    let path = report_path(projects_dir, project_id, report_id)?;
    if !path.exists() {
        return Err(ReportsError::NotFound(format!(
            "no saved report {report_id:?} for project {project_id:?}"
        )));
    }
    std::fs::remove_file(&path).map_err(|e| ReportsError::Internal(e.into()))?;
    tracing::info!("deleted report {report_id} for project {project_id}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Axum adapters
// ---------------------------------------------------------------------------

fn projects_dir(state: &Shared) -> Result<PathBuf, ReportsError> {
    state.projects_dir().cloned().ok_or(ReportsError::NotFileBacked)
}

fn to_http(err: ReportsError) -> (StatusCode, String) {
    match err {
        ReportsError::NotFileBacked => (
            StatusCode::SERVICE_UNAVAILABLE,
            "this server was not started from a settings directory".to_string(),
        ),
        ReportsError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
        ReportsError::Invalid(msg) => (StatusCode::BAD_REQUEST, msg),
        ReportsError::Internal(e) => {
            tracing::error!("saved reports error: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
        }
    }
}

pub async fn http_list_reports(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
) -> Result<Json<Vec<SavedReportSummary>>, (StatusCode, String)> {
    let dir = projects_dir(&state).map_err(to_http)?;
    list_reports(&dir, &project_id).map(Json).map_err(to_http)
}

pub async fn http_get_report(
    State(state): State<Shared>,
    UrlPath((project_id, report_id)): UrlPath<(String, String)>,
) -> Result<Json<SavedReport>, (StatusCode, String)> {
    let dir = projects_dir(&state).map_err(to_http)?;
    get_report(&dir, &project_id, &report_id).map(Json).map_err(to_http)
}

pub async fn http_save_report(
    State(state): State<Shared>,
    UrlPath((project_id, report_id)): UrlPath<(String, String)>,
    Json(report): Json<SavedReport>,
) -> Result<Json<SavedReport>, (StatusCode, String)> {
    let dir = projects_dir(&state).map_err(to_http)?;
    save_report(&dir, &project_id, &report_id, &report).map_err(to_http)?;
    Ok(Json(report))
}

pub async fn http_delete_report(
    State(state): State<Shared>,
    UrlPath((project_id, report_id)): UrlPath<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let dir = projects_dir(&state).map_err(to_http)?;
    delete_report(&dir, &project_id, &report_id).map_err(to_http)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roommate-reports-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn report(id: &str, name: &str) -> SavedReport {
        SavedReport {
            id: id.to_string(),
            name: name.to_string(),
            entity: "ceilings".to_string(),
            by_room: true,
            columns: vec!["$type_name".to_string()],
            room_columns: vec!["Number".to_string()],
            measures: vec!["overlap_area".to_string()],
            shape: Some("per_match".to_string()),
            include_rooms_without: false,
            include_unattributed: true,
            building: None,
            milestone: None,
            limit: Some(500),
            filter: None,
        }
    }

    #[test]
    fn test_save_then_read_round_trips_the_whole_definition() {
        let dir = temp_dir("round-trip");
        save_report(&dir, "House A", "ceilings-by-room", &report("ceilings-by-room", "Ceilings by room")).unwrap();

        let read = get_report(&dir, "House A", "ceilings-by-room").unwrap();

        assert_eq!(read.name, "Ceilings by room");
        assert_eq!(read.measures, ["overlap_area"]);
        assert_eq!(read.limit, Some(500));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A project with nothing saved lists nothing, rather than failing: the
    /// rail has to render before anyone has saved their first report.
    #[test]
    fn test_a_project_with_no_saved_reports_lists_none() {
        let dir = temp_dir("empty");
        assert!(list_reports(&dir, "House A").unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// One unreadable file does not take the list down with it -- this list is
    /// exactly what someone opens to find the broken one.
    #[test]
    fn test_a_rotten_file_is_skipped_rather_than_failing_the_list() {
        let dir = temp_dir("rotten");
        save_report(&dir, "House A", "good", &report("good", "Good one")).unwrap();
        std::fs::write(dir.join("reports").join("House A").join("bad.json"), b"{ not json").unwrap();

        let listed = list_reports(&dir, "House A").unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "good");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// An id that cannot be a file name is refused by name, never sanitised:
    /// silently rewriting it would make two ids the same file.
    #[test]
    fn test_an_unsafe_id_is_refused() {
        let dir = temp_dir("unsafe");
        let err = save_report(&dir, "House A", "../escape", &report("../escape", "Escape")).unwrap_err();
        assert!(matches!(err, ReportsError::Invalid(msg) if msg.contains("cannot be a file name")));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The body and the path must agree. Renaming someone's report to match a
    /// URL is the kind of helpfulness nobody can see.
    #[test]
    fn test_a_body_that_disagrees_with_the_path_is_refused() {
        let dir = temp_dir("mismatch");
        let err = save_report(&dir, "House A", "one", &report("two", "Two")).unwrap_err();
        assert!(matches!(err, ReportsError::Invalid(msg) if msg.contains("does not match")));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_delete_removes_it_and_says_so_when_it_was_not_there() {
        let dir = temp_dir("delete");
        save_report(&dir, "House A", "gone", &report("gone", "Gone")).unwrap();
        delete_report(&dir, "House A", "gone").unwrap();

        assert!(list_reports(&dir, "House A").unwrap().is_empty());
        assert!(matches!(delete_report(&dir, "House A", "gone"), Err(ReportsError::NotFound(_))));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Saving leaves no temp file behind — a reader listing the directory sees
    /// documents only.
    #[test]
    fn test_a_save_leaves_no_temp_file() {
        let dir = temp_dir("temp");
        save_report(&dir, "House A", "clean", &report("clean", "Clean")).unwrap();

        let files: Vec<String> = std::fs::read_dir(dir.join("reports").join("House A"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();

        assert_eq!(files, ["clean.json"]);

        std::fs::remove_dir_all(&dir).ok();
    }
}
