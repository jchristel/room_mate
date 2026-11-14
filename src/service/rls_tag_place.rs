//! What a create-RLS task needs to know about tags in THIS room (plan decision 27): where its room
//! tag goes.
//!
//! It is handed the two rooms' outlines `task_build` has already read (once per request, for every room of it), and
//! keeps apart from the pure piece it uses (`room_tag_place`) because it chooses the rule and turns a refusal into
//! a finding. Used by `task_build`. Which blueprint item an item stands for is no longer asked: an item rule applies to every
//! instance of its type code (`tag_rules::rls_item_rules`).

use serde_json::Value;

use super::blueprint_geometry::Pt;
use super::rls_plan::RoomTagPlan;
use super::room_tag_place::head_in;
use super::task_build::finding;
use crate::sheets::SheetRecord;
use crate::tag_rules::RuleSummary;

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
///
/// `template` is the outline of the room the sheet was made for and `target` this room's, each in its own model's
/// frame (`elevation_facing::model_room_ring_in`); either absent when that room is not on the server.
pub fn room_tag_head(
    template: Option<&[Pt]>,
    target: Option<&[Pt]>,
    record: &SheetRecord,
    rules: &[RuleSummary],
    findings: &mut Vec<Value>,
) -> Option<RoomTagPlan> {
    let of_room = |r: &&RuleSummary| r.tag_kind == "room" && r.key.viewport_type.eq_ignore_ascii_case("floor plan");
    let rule = rules
        .iter()
        .filter(of_room)
        .find(|r| !r.key.blueprint.is_empty())
        .or_else(|| rules.iter().find(of_room));
    let (Some(rule), Some(scale)) = (rule, plan_scale(&record.sheet)) else {
        return None;
    };
    let boundary = rule.placement.boundary?;
    let target_ring = target?;
    match head_in(&boundary, template, target_ring, scale) {
        Ok(p) => Some(RoomTagPlan { head: [p.0, p.1], rule_id: rule.id.clone() }),
        Err(reason) => {
            findings.push(finding("room_tag_not_placed", reason));
            None
        }
    }
}
