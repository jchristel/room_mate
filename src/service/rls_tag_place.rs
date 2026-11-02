//! What a create-RLS task needs to know about tags in THIS room (plan decision 27): where its room
//! tag goes, and which blueprint item each of its items stands for.
//!
//! Both read the world (rooms, an assignment's resolution), so they live apart from the pure
//! pieces they use (`room_tag_place`, `rls_tags::match_instances`). Used by `task_build`.

use serde_json::Value;

use super::blueprint_geometry::Pt;
use super::blueprint_resolve::resolve_in_room_with;
use super::elevation_facing::model_room_ring;
use super::rls_plan::RoomTagPlan;
use super::rls_tags::type_code_of;
use super::room_locator::RoomRef;
use super::room_tag_place::head_in;
use super::task_build::{blueprint_error, edge_map_of, finding, project_to_model};
use super::ServiceError;
use crate::assignments;
use crate::blueprints::{self, BlueprintError};
use crate::sheets::{SheetRecord, SheetRoom};
use crate::state::AppState;
use crate::tag_rules::RuleSummary;

/// A blueprint item and a room item are the same when they have the same type code and lie this close.
pub const INSTANCE_MATCH_MM: f64 = 500.0;

/// The scale of the sheet's first plan viewport, which is the one a room tag's distance is written in.
fn plan_scale(sheet: &Value) -> Option<f64> {
    sheet
        .get("view_ports")?
        .as_array()?
        .iter()
        .filter(|p| p.get("vp_type").and_then(Value::as_str) == Some("floor plan"))
        .find_map(|p| p.get("view")?.get("view_scale")?.as_f64().filter(|s| *s > 0.0))
}

/// Where the room tag goes in `room`, in its model's frame, from the room tag rule the task carries.
/// A rule bound to the sheet's blueprint wins over the generic one. `None` when there is no rule, the
/// rule has no boundary (learned before the room was on the server: the plug-in applies its offset in
/// the plan's axes), or the place cannot be worked out, which is a finding.
pub fn room_tag_head(
    state: &AppState,
    project: &str,
    room: &RoomRef,
    record: &SheetRecord,
    rules: &[RuleSummary],
    findings: &mut Vec<Value>,
) -> Result<Option<RoomTagPlan>, ServiceError> {
    let of_room = |r: &&RuleSummary| r.tag_kind == "room" && r.key.viewport_type.eq_ignore_ascii_case("floor plan");
    let rule = rules
        .iter()
        .filter(of_room)
        .find(|r| !r.key.blueprint.is_empty())
        .or_else(|| rules.iter().find(of_room));
    let (Some(rule), Some(scale)) = (rule, plan_scale(&record.sheet)) else {
        return Ok(None);
    };
    let Some(boundary) = rule.placement.boundary else {
        return Ok(None);
    };
    let target = SheetRoom::from(room);
    let Some(target_ring) = model_room_ring(state, project, &target)? else {
        return Ok(None);
    };
    let template = match &record.room {
        Some(r) => model_room_ring(state, project, r)?,
        None => None,
    };
    match head_in(&boundary, template.as_deref(), &target_ring, scale) {
        Ok(p) => Ok(Some(RoomTagPlan { head: [p.0, p.1], rule_id: rule.id.clone() })),
        Err(reason) => {
            findings.push(finding("room_tag_not_placed", reason));
            Ok(None)
        }
    }
}

/// The blueprint's items as the room's assignment of it places them now: `(blueprint item id, type
/// code, centre in the room's model frame)`. Empty when the room has no assignment of the blueprint,
/// or it cannot be read: the room's items then simply take the generic rules.
pub fn blueprint_placements(
    state: &AppState,
    projects_dir: &std::path::Path,
    project: &str,
    blueprint: &str,
    room: &RoomRef,
    parameter: &str,
) -> Result<Vec<(String, String, Pt)>, ServiceError> {
    let all = assignments::list_raw(projects_dir, project).map_err(blueprint_error)?;
    let Some(assignment) = all
        .iter()
        .find(|a| a.blueprint_id == blueprint && a.room.model_id == room.model_id && a.room.room_id == room.room_id)
    else {
        return Ok(Vec::new());
    };
    let bp = match blueprints::load(projects_dir, project, blueprint) {
        Ok(b) => b,
        Err(BlueprintError::NotFound(_)) => return Ok(Vec::new()),
        Err(e) => return Err(blueprint_error(e)),
    };
    let Some(revision) = bp.revision(assignment.revision) else {
        return Ok(Vec::new());
    };
    let mapping = edge_map_of(&assignment.mapping);
    let Some((_, resolution)) =
        resolve_in_room_with(state, project, &revision.content, room, &mapping, &assignment.overrides)?
    else {
        return Ok(Vec::new());
    };
    let to_model = project_to_model(state, project, &room.model_id)?;
    Ok(resolution
        .placements
        .iter()
        .filter_map(|p| {
            let source = revision.content.items.iter().find(|i| i.id == p.item_id)?;
            let code = type_code_of(parameter, &source.type_properties, &source.properties)?;
            Some((p.item_id.clone(), code, to_model.point((p.centre.x, p.centre.y))))
        })
        .collect())
}
