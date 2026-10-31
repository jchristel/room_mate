//! The sheets and tags that go with a room blueprint, and what accepting them does (plan decision 27).
//!
//! A blueprint holds only its room; its sheets live in the sheet library bound to it and its tags in
//! the tag rules bound to it. They arrive together with a push of the blueprint (`Bundle`), wait with
//! the revision while it is pending, and are applied when it is accepted (or at once for a new
//! blueprint, which has nothing to wait for). **Applying reconciles**:
//!
//! - the sheets the push carries are bound to the blueprint; a sheet bound to it that the push no
//!   longer carries is **unbound** and stays in the library as a generic sheet;
//! - the tags (observations from Revit, or the rules of an exported bundle) are bound to it; a rule
//!   bound to it that the push no longer writes is **unlinked**: its samples are merged into the
//!   generic rule with the same key and it is removed (`tag_rules::unlink_blueprint`).
//!
//! Tags are only reconciled when the bundle says it carries them (`observations` or `rules` present),
//! so a push that did not read tags never turns a blueprint's tags generic.
//!
//! The bundle is also what Export and Import move between projects: the bound sheets and rules, read
//! by `GET .../blueprints/{id}/bundle`, ride the file beside the room and go through the same apply.

use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::service::blueprint_model::BlueprintSheet;
use crate::sheets::{self, PushRequest as SheetsPush, PushedSheet, SheetRoom};
use crate::state::Shared;
use crate::tag_rules::{self, Observation, ObservationOutcome, ObservationsRequest, TagRule};

/// What travels with a blueprint push. `observations` and `rules` are kept as JSON until they are applied.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Bundle {
    #[serde(default)]
    pub sheets: Vec<BlueprintSheet>,
    /// The type parameter the tags' codes were read from.
    #[serde(default)]
    pub type_code_parameter: String,
    /// Tags as read from Revit (`tag_rules::Observation`). Present, even when empty, means "the push read tags".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observations: Option<Value>,
    /// Tag rules of an exported bundle (`tag_rules::TagRule`, samples included).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Value>,
}

/// What applying a bundle did.
#[derive(Debug, Default, Serialize)]
pub struct Applied {
    /// Sheets the library took, bound to the blueprint.
    pub sheets: usize,
    /// Sheets that were bound and are no longer carried: generic now.
    pub unlinked_sheets: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<ObservationOutcome>,
    /// Rules that were bound and are no longer written: merged into generic.
    pub unlinked_tag_rules: usize,
}

fn dir(state: &Shared) -> Result<std::path::PathBuf, (StatusCode, String)> {
    state.projects_dir().cloned().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "this server was not started from a settings directory".to_string(),
    ))
}

fn invalid(what: &str, e: serde_json::Error) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, format!("the bundle's {what} cannot be read: {e}"))
}

/// The sheets go to the sheet library, **bound to the blueprint** and made for its room. The library
/// matches by sheet identity, so a re-push updates a sheet and a changed number waits for a person.
/// They carry no tags (the extractor takes those out before a sheet leaves Revit).
pub fn bind_to_library(
    state: &Shared,
    project: &str,
    blueprint: &str,
    room: Option<&SheetRoom>,
    sheets: &[BlueprintSheet],
) -> Result<usize, (StatusCode, String)> {
    let projects = dir(state)?;
    let summary = room.and_then(|r| crate::sheets_api::summary_of(state, project, r));
    let pushed: Vec<PushedSheet> = sheets
        .iter()
        .map(|s| PushedSheet {
            revit_sheet_id: s.revit_sheet_id.clone(),
            document_title: s.document_title.clone(),
            sheet_number: s.sheet_number.clone(),
            sheet_name: s.sheet_name.clone(),
            room: room.cloned(),
            blueprint: Some(blueprint.to_string()),
            phase: None,
            room_summary: summary,
            size_mm: s.size_mm,
            rooms: Vec::new(),
            sheet: s.sheet.clone(),
        })
        .collect();
    sheets::push(&projects, project, SheetsPush { sheets: pushed })
        .map(|outcomes| outcomes.len())
        .map_err(crate::sheets_api::to_http)
}

/// Apply a bundle to a blueprint that exists and whose revision is current: bind its sheets and tags
/// and unlink what it no longer carries. `room` is the blueprint's source room, when it has one.
pub fn apply(
    state: &Shared,
    project: &str,
    blueprint: &str,
    room: Option<&SheetRoom>,
    bundle: &Bundle,
) -> Result<Applied, (StatusCode, String)> {
    let projects = dir(state)?;
    let mut applied = Applied::default();
    if !bundle.sheets.is_empty() {
        applied.sheets = bind_to_library(state, project, blueprint, room, &bundle.sheets)?;
        let keep: Vec<(String, String)> = bundle
            .sheets
            .iter()
            .map(|s| (s.revit_sheet_id.clone(), s.document_title.clone()))
            .collect();
        applied.unlinked_sheets =
            sheets::unbind_missing(&projects, project, blueprint, &keep).map_err(crate::sheets_api::to_http)?;
    }
    let mut written: Option<std::collections::HashSet<String>> = None;
    if let Some(observations) = &bundle.observations {
        let observations: Vec<Observation> =
            serde_json::from_value(observations.clone()).map_err(|e| invalid("tag observations", e))?;
        let request = ObservationsRequest {
            blueprint: Some(blueprint.to_string()),
            type_code_parameter: bundle.type_code_parameter.clone(),
            observations,
        };
        let outcome = crate::tag_rules_api::observe_for(state, project, request)?;
        written.get_or_insert_with(Default::default).extend(outcome.rule_ids.iter().cloned());
        applied.tags = Some(outcome);
    }
    if let Some(rules) = &bundle.rules {
        let rules: Vec<TagRule> = serde_json::from_value(rules.clone()).map_err(|e| invalid("tag rules", e))?;
        let ids =
            tag_rules::import_bound(&projects, project, blueprint, rules).map_err(crate::tag_rules_api::to_http)?;
        written.get_or_insert_with(Default::default).extend(ids);
    }
    if let Some(keep) = written {
        applied.unlinked_tag_rules =
            tag_rules::unlink_blueprint(&projects, project, blueprint, &keep).map_err(crate::tag_rules_api::to_http)?;
    }
    Ok(applied)
}

/// A blueprint's bound sheets and rules, as a bundle to export.
pub fn export(state: &Shared, project: &str, blueprint: &str) -> Result<Bundle, (StatusCode, String)> {
    let projects = dir(state)?;
    let records = sheets::bound_to(&projects, project, blueprint).map_err(crate::sheets_api::to_http)?;
    let rules = tag_rules::bound_to(&projects, project, blueprint).map_err(crate::tag_rules_api::to_http)?;
    let type_code_parameter = rules
        .iter()
        .map(|r| r.key.code_parameter.clone())
        .find(|p| !p.is_empty())
        .unwrap_or_default();
    Ok(Bundle {
        sheets: records
            .iter()
            .map(|r| BlueprintSheet {
                revit_sheet_id: r.revit_sheet_id.clone(),
                document_title: r.document_title.clone(),
                sheet_number: r.sheet_number.clone(),
                sheet_name: r.sheet_name.clone(),
                size_mm: r.size_mm,
                sheet: r.sheet.clone(),
            })
            .collect(),
        type_code_parameter,
        observations: None,
        rules: Some(serde_json::to_value(&rules).map_err(|e| invalid("tag rules", e))?),
    })
}

/// `GET /projects/{id}/blueprints/{blueprint}/bundle`: what Export puts in the file beside the room.
pub async fn http_get_bundle(
    State(state): State<Shared>,
    UrlPath((project_id, blueprint)): UrlPath<(String, String)>,
) -> Result<Json<Bundle>, (StatusCode, String)> {
    export(&state, &project_id, &blueprint).map(Json)
}
