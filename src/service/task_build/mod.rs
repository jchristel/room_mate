//! Making a task: the resolver's answer and the sheets, frozen as what a Revit session is told
//! (`docs/PLAN-blueprints.md`, decisions 21 and 26; the record is `crate::tasks`).
//!
//! **Placements leave the server in the target MODEL's own frame.** The resolver answers in the
//! project's plan frame (so a result draws straight onto the plan), and the Revit session works
//! in the model it has open, so each position goes back through the inverse of that model's
//! placement (`service::placement`) -- the one place that frame is undone. A model that needs no
//! placement (the anchor, or one exported from the project's origin) is left as it is.
//!
//! **A refusal is a reason, not an error**: a blueprint that was deleted, a revision that is not
//! there, a room that has gone, a layout that would place nothing, or a blueprint with no sheets
//! when sheets were asked for. The caller shows it against the room and carries on with the rest.

use serde_json::{json, Value};

use super::blueprint_capture::{properties_of, FT_TO_MM};
use super::blueprint_geometry::Pt;
use super::blueprint_resolve::{resolve_in_room_changes, EdgeMap};
use super::elevation_facing::{model_room_ring_in, template_ring};
use super::placement;
use super::rls_plan::{check_size, has_elevations, plan_crop, size_token, transfer, MarkerPlan, RlsPlan, RlsRequest};
use super::rls_reads::RlsReads;
use super::rls_tag_place::room_tag_head;
use super::rls_tags::{plan_tags, type_code_of, RoomItem};
use super::rls_three_d::{has_three_d, three_ds};
use super::room_locator::RoomRef;
use super::rooms::{assemble_rooms, RoomScope, RoomsResult};
use super::sheet_suggest::{distance, room_summary_in, Signature};
use super::ServiceError;
use crate::assignments::{Assignment, Mapping};
use crate::blueprints::{self, BlueprintError};
use crate::contract::Point2D;
use crate::service::blueprint_model::BlueprintSheet;
use crate::sheets::{self, SheetError, SheetRoom};
use crate::state::AppState;
use crate::tag_rules::{self, RuleSummary, TaskRule};
use crate::tasks::{BlueprintRef, NewTask, TaskItem, TaskKind, TaskRoom, UpdateMode};

mod populate;
mod rls;
pub use populate::*;
pub use rls::*;

/// A sheet whose room differs from the target by more than this (the mean of the three
/// log-ratios in `sheet_suggest::distance`; 0.3 is about 35% off on each figure) is a warning.
pub const RLS_ROOM_WARN_DISTANCE: f64 = 0.3;

fn refuse<T>(reason: impl Into<String>) -> Result<T, ServiceError> {
    Err(ServiceError::Invalid(reason.into()))
}

pub(super) fn blueprint_error(e: BlueprintError) -> ServiceError {
    match e {
        BlueprintError::Internal(e) => ServiceError::Internal(e),
        BlueprintError::NotFileBacked => {
            ServiceError::Invalid("this server was not started from a settings directory".to_string())
        }
        BlueprintError::Invalid(m) | BlueprintError::NotFound(m) | BlueprintError::Conflict(m) => {
            ServiceError::Invalid(m)
        }
    }
}

/// The room's name and its `Number` property, read from the model as it stands. Absent when the
/// project has no such room.
fn room_facts(state: &AppState, project: &str, room: &RoomRef) -> Result<Option<TaskRoom>, ServiceError> {
    let Some(rooms) = assemble_rooms(state, &RoomScope { project: Some(project), ..Default::default() })? else {
        return Ok(None);
    };
    Ok(room_facts_in(&rooms, room))
}

/// [`room_facts`] over the project's rooms already read.
pub(super) fn room_facts_in(rooms: &RoomsResult, room: &RoomRef) -> Option<TaskRoom> {
    rooms
        .rooms
        .iter()
        .find(|r| r.model_id == room.model_id && r.room.id == room.room_id)
        .map(|r| TaskRoom {
            room_id: room.room_id.clone(),
            number: r
                .room
                .properties
                .get("Number")
                .map(|v| v.value.trim().to_string())
                .filter(|n| !n.is_empty()),
            name: r.room.name.clone(),
        })
}

/// What a stored mapping asks of the resolver.
pub(super) fn edge_map_of(mapping: &Mapping) -> EdgeMap {
    match mapping {
        Mapping { frame: Some(f), .. } => EdgeMap::Frame(*f),
        Mapping { edge_map: Some(map), .. } => EdgeMap::Explicit(map.clone()),
        Mapping { shift: Some(s), .. } => EdgeMap::Shift(*s),
        _ => EdgeMap::Auto,
    }
}

/// A finding on a task: what the plug-in and the tasks page show beside it.
pub(super) fn finding(kind: &str, message: String) -> Value {
    json!({ "kind": kind, "message": message })
}

/// The viewport types the sheets show: a tag rule is for one of them.
fn viewport_types(sheets: &[BlueprintSheet]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for sheet in sheets {
        for port in sheet.sheet.get("view_ports").and_then(Value::as_array).into_iter().flatten() {
            if let Some(t) = port.get("vp_type").and_then(Value::as_str)
                && !out.iter().any(|o| o.eq_ignore_ascii_case(t))
            {
                out.push(t.to_string());
            }
        }
    }
    out
}

/// The project's type-code parameter (`tag_type_code`), blank when it has none.
fn type_code_parameter(state: &AppState, project: &str) -> String {
    state
        .settings()
        .settings_for(project)
        .and_then(|b| b.tag_type_code.clone())
        .map(|p| p.trim().to_string())
        .unwrap_or_default()
}

/// The tag rules a populate task carries: those for the type codes of the items it places, generic and
/// bound to the blueprint (plan decision 27).
fn item_tag_rules(
    state: &AppState,
    projects_dir: &std::path::Path,
    project: &str,
    sheets: &[BlueprintSheet],
    items: &[TaskItem],
    blueprint: &str,
    findings: &mut Vec<Value>,
) -> Vec<RuleSummary> {
    let parameter = type_code_parameter(state, project);
    let codes: Vec<String> = items
        .iter()
        .filter_map(|i| type_code_of(&parameter, &i.type_properties, &i.properties))
        .collect();
    tag_rules_for(projects_dir, project, sheets, &parameter, Some(&codes), Some(blueprint), findings)
}

/// The tag rules a task carries (plan decision 24). A rules store that cannot be read is a finding
/// and no rules, never a refusal: the layout does not depend on them.
fn tag_rules_for(
    projects_dir: &std::path::Path,
    project: &str,
    sheets: &[BlueprintSheet],
    parameter: &str,
    codes: Option<&[String]>,
    blueprint: Option<&str>,
    findings: &mut Vec<Value>,
) -> Vec<RuleSummary> {
    let types = viewport_types(sheets);
    if types.is_empty() {
        return Vec::new();
    }
    if parameter.is_empty() {
        findings.push(finding(
            "tag_type_code_unset",
            "the project's settings name no tag type code, so items get no tags (room tags still do)".to_string(),
        ));
    }
    match tag_rules::rules_for(projects_dir, project, &types, parameter, codes, blueprint) {
        Ok(rules) => rules,
        Err(_) => {
            findings.push(finding(
                "tag_rules_unreadable",
                "the tag rules could not be read, so no tags will be placed".to_string(),
            ));
            Vec::new()
        }
    }
}

/// The tag rules a create-RLS task carries: the room, space and area rules as for any task (generic, and those
/// bound to the sheet's blueprint), and for items the effective rules (`tag_rules::rls_item_rules`): per type
/// code, the generic pool's rule or else the rule pooled from every room blueprint, applied to any instance of
/// the type whether or not the room has an assignment. A store that cannot be read is a finding and no rules.
fn rls_tag_rules(
    projects_dir: &std::path::Path,
    project: &str,
    sheets: &[BlueprintSheet],
    parameter: &str,
    blueprint: Option<&str>,
    findings: &mut Vec<Value>,
) -> Vec<RuleSummary> {
    let mut rules: Vec<RuleSummary> =
        tag_rules_for(projects_dir, project, sheets, parameter, None, blueprint, findings)
            .into_iter()
            .filter(|r| tag_rules::is_spatial(&r.tag_kind))
            .collect();
    let types = viewport_types(sheets);
    if !types.is_empty() && !parameter.is_empty() {
        match tag_rules::load_all(projects_dir, project) {
            Ok(all) => rules.extend(tag_rules::rls_item_rules(&all, &types, parameter)),
            Err(_) if findings.iter().any(|f| f["kind"] == "tag_rules_unreadable") => {}
            Err(_) => findings.push(finding(
                "tag_rules_unreadable",
                "the tag rules could not be read, so no tags will be placed".to_string(),
            )),
        }
    }
    rules
}

/// The project frame back into one model's own frame (millimetres in and out; the placement is in feet).
pub(crate) struct ToModel {
    inverse: Option<crate::contract::ModelToShared>,
    /// The model has a placement but it could not be inverted: positions are left as they are.
    degenerate: bool,
}

impl ToModel {
    fn is_identity(&self) -> bool {
        self.inverse.is_none() && !self.degenerate
    }

    pub(crate) fn point(&self, p: (f64, f64)) -> (f64, f64) {
        match &self.inverse {
            Some(t) => {
                let q = t.place(Point2D { x: p.0 / FT_TO_MM, y: p.1 / FT_TO_MM });
                (q.x * FT_TO_MM, q.y * FT_TO_MM)
            }
            None => p,
        }
    }

    fn direction(&self, d: (f64, f64)) -> (f64, f64) {
        match &self.inverse {
            Some(t) => t.place_direction(Point2D { x: d.0, y: d.1 }).map_or(d, |q| (q.x, q.y)),
            None => d,
        }
    }
}

pub(crate) fn project_to_model(state: &AppState, project: &str, model_id: &str) -> Result<ToModel, ServiceError> {
    let placement = placement::from_index(state)?;
    Ok(match placement.for_model(project, model_id) {
        None => ToModel { inverse: None, degenerate: false },
        Some(t) => match t.inverse() {
            Some(inv) => ToModel { inverse: Some(inv), degenerate: false },
            None => ToModel { inverse: None, degenerate: true },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::ModelToShared;

    /// A model turned a quarter and moved 10 ft: a position in the project frame goes back to where
    /// the model's own frame has it, and a direction only turns.
    #[test]
    fn test_a_position_goes_back_through_the_inverse_of_the_models_placement() {
        // model -> project: x' = -y + 10, y' = x  (a quarter turn and a move, in feet)
        let forward = ModelToShared { matrix: [0.0, 1.0, -1.0, 0.0, 10.0, 0.0] };
        let to_model = ToModel { inverse: forward.inverse(), degenerate: false };
        // The model's own point (3 ft, 4 ft) is at (6, 3) ft in the project frame.
        let project_mm = (6.0 * FT_TO_MM, 3.0 * FT_TO_MM);
        let (x, y) = to_model.point(project_mm);
        assert!((x - 3.0 * FT_TO_MM).abs() < 1e-6 && (y - 4.0 * FT_TO_MM).abs() < 1e-6, "{x} {y}");
        // The project's +x direction is the model's -y... the quarter turn undone.
        let (dx, dy) = to_model.direction((1.0, 0.0));
        assert!(dx.abs() < 1e-9 && (dy + 1.0).abs() < 1e-9, "{dx} {dy}");
    }

    #[test]
    fn test_a_model_with_no_placement_is_left_as_it_is() {
        let none = ToModel { inverse: None, degenerate: false };
        assert!(none.is_identity());
        assert_eq!(none.point((12.5, -3.0)), (12.5, -3.0));
        assert_eq!(none.direction((0.0, 1.0)), (0.0, 1.0));
    }
}
