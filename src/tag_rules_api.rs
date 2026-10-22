//! HTTP adapters for tag rules -- the thin half of `crate::tag_rules`.
//!
//! The extractor `POST`s observations (the tags it read from a sheet's viewports); everyone else
//! reads. Reads are also MCP tools; writes are HTTP-only, as for blueprints and sheets.

use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    Json,
};

use crate::state::Shared;
use crate::tag_rules::{self, ObservationOutcome, ObservationsRequest, RuleSummary, TagRule, TagRuleError};

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
    tag_rules::observe(&dir(&state)?, &project_id, request).map(Json).map_err(to_http)
}

pub async fn http_delete_tag_rule(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    tag_rules::delete(&dir(&state)?, &project_id, &id).map_err(to_http)?;
    Ok(StatusCode::NO_CONTENT)
}
