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
use super::blueprint_resolve::{resolve_in_room_with, EdgeMap};
use super::elevation_facing::model_room_ring;
use super::items::{assemble_items, ItemScope};
use super::placement;
use super::rls_plan::{check_size, has_elevations, transfer, MarkerPlan, RlsPlan, RlsRequest};
use super::rls_tags::{plan_tags, RoomItem};
use super::room_locator::RoomRef;
use super::rooms::{assemble_rooms, RoomScope};
use super::sheet_suggest::{distance, room_summary, Signature};
use super::ServiceError;
use crate::assignments::{Assignment, Mapping};
use crate::blueprints::{self, BlueprintError};
use crate::contract::Point2D;
use crate::service::blueprint_model::BlueprintSheet;
use crate::sheets::{self, SheetError, SheetRoom};
use crate::state::AppState;
use crate::tag_rules::{self, RuleSummary};
use crate::tasks::{BlueprintRef, NewTask, TaskItem, TaskKind, TaskRoom, UpdateMode};

/// A sheet whose room differs from the target by more than this (the mean of the three
/// log-ratios in `sheet_suggest::distance`; 0.3 is about 35% off on each figure) is a warning.
pub const RLS_ROOM_WARN_DISTANCE: f64 = 0.3;

/// What a person sets on a populate task.
#[derive(Debug, Clone)]
pub struct PopulateParams {
    pub phase: String,
    pub overwrite_layout: bool,
    pub create_rls: bool,
    pub overwrite_rls: bool,
}

/// What a person sets on an update task.
#[derive(Debug, Clone)]
pub struct UpdateParams {
    pub phase: String,
    pub mode: UpdateMode,
    /// Report the difference and write nothing: the default, for the reason given in the plan
    /// (decision 13).
    pub dry_run: bool,
}

fn refuse<T>(reason: impl Into<String>) -> Result<T, ServiceError> {
    Err(ServiceError::Invalid(reason.into()))
}

fn blueprint_error(e: BlueprintError) -> ServiceError {
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
    Ok(rooms
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
        }))
}

/// What a stored mapping asks of the resolver.
fn edge_map_of(mapping: &Mapping) -> EdgeMap {
    match mapping {
        Mapping { frame: Some(f), .. } => EdgeMap::Frame(*f),
        Mapping { edge_map: Some(map), .. } => EdgeMap::Explicit(map.clone()),
        Mapping { shift: Some(s), .. } => EdgeMap::Shift(*s),
        _ => EdgeMap::Auto,
    }
}

fn finding(kind: &str, message: String) -> Value {
    json!({ "kind": kind, "message": message })
}

/// A populate task for one assignment, from the pinned revision and the room as it stands.
pub fn populate_from_assignment(
    state: &AppState,
    projects_dir: &std::path::Path,
    project: &str,
    assignment: &Assignment,
    params: &PopulateParams,
) -> Result<NewTask, ServiceError> {
    let blueprint = match blueprints::load(projects_dir, project, &assignment.blueprint_id) {
        Ok(b) => b,
        Err(BlueprintError::NotFound(_)) => return refuse("the assignment's blueprint has been deleted"),
        Err(e) => return Err(blueprint_error(e)),
    };
    let Some(revision) = blueprint.revision(assignment.revision) else {
        return refuse(format!("the blueprint has no revision {}", assignment.revision));
    };
    if params.create_rls && revision.content.sheets.is_empty() {
        return refuse(
            "the blueprint has no room layout sheets, so sheets cannot be created from it (turn off create RLS, or push the blueprint from Revit with its sheets)",
        );
    }
    let room = RoomRef {
        model_id: assignment.room.model_id.clone(),
        room_id: assignment.room.room_id.clone(),
    };
    let Some(task_room) = room_facts(state, project, &room)? else {
        return refuse("the room is not in the model any more");
    };
    let mapping = edge_map_of(&assignment.mapping);
    let Some((_, resolution)) =
        resolve_in_room_with(state, project, &revision.content, &room, &mapping, &assignment.overrides)?
    else {
        return refuse("the room is not in the model any more");
    };
    let live = revision.content.items.iter().filter(|i| !i.excluded).count();
    if resolution.placements.is_empty() && live > 0 {
        return refuse(format!(
            "nothing would be placed ({:?}): map the blueprint's edges to the room's by hand in the assign view first",
            resolution.fit.kind
        ));
    }

    let to_model = project_to_model(state, project, &room.model_id)?;
    let items: Vec<TaskItem> = resolution
        .placements
        .iter()
        .filter_map(|p| {
            let source = revision.content.items.iter().find(|i| i.id == p.item_id)?;
            let (x_mm, y_mm) = to_model.point((p.centre.x, p.centre.y));
            let (facing_x, facing_y) = to_model.direction((p.facing.x, p.facing.y));
            let host = source.properties.iter().find(|q| q.name == "Host").map(|q| q.value.clone());
            Some(TaskItem {
                item_id: p.item_id.clone(),
                family_name: source.family_name.clone(),
                type_name: p.type_name.clone(),
                category: p.category.clone(),
                placement_type: p.class,
                host,
                x_mm,
                y_mm,
                facing_x,
                facing_y,
                size_mm: p.size_mm,
                properties: source.properties.clone(),
                type_properties: source.type_properties.clone(),
            })
        })
        .collect();
    let mut findings: Vec<Value> = resolution.findings.iter().filter_map(|f| serde_json::to_value(f).ok()).collect();
    if !to_model.is_identity() && to_model.degenerate {
        findings.push(finding(
            "frame_unknown",
            "the model's placement could not be undone, so positions are in the project's plan frame".to_string(),
        ));
    }
    let tag_rules = if params.create_rls {
        let pairs: Vec<(String, String)> = items
            .iter()
            .map(|i| (i.family_name.clone().unwrap_or_default(), i.type_name.clone()))
            .collect();
        tag_rules_for(projects_dir, project, &revision.content.sheets, Some(&pairs), &mut findings)
    } else {
        Vec::new()
    };
    Ok(NewTask {
        rls: None,
        kind: TaskKind::PopulateRoom,
        update_mode: None,
        dry_run: false,
        model_id: room.model_id,
        room: task_room,
        phase: params.phase.clone(),
        overwrite_layout: params.overwrite_layout,
        create_rls: params.create_rls,
        overwrite_rls: params.overwrite_rls,
        blueprint: Some(BlueprintRef {
            id: blueprint.id.clone(),
            name: blueprint.name.clone(),
            revision: revision.number,
        }),
        library_sheet_id: None,
        items,
        sheets: if params.create_rls { revision.content.sheets.clone() } else { Vec::new() },
        tag_rules,
        findings,
    })
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

/// The tag rules a task carries (plan decision 24). A rules store that cannot be read is a finding
/// and no rules, never a refusal: the layout does not depend on them.
fn tag_rules_for(
    projects_dir: &std::path::Path,
    project: &str,
    sheets: &[BlueprintSheet],
    items: Option<&[(String, String)]>,
    findings: &mut Vec<Value>,
) -> Vec<RuleSummary> {
    let types = viewport_types(sheets);
    if types.is_empty() {
        return Vec::new();
    }
    match tag_rules::rules_for(projects_dir, project, &types, items) {
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

/// An update task for one assignment: the layout as the resolver places it now, with no sheets
/// and no tags (an update moves and changes items; it does not make a layout sheet).
pub fn update_from_assignment(
    state: &AppState,
    projects_dir: &std::path::Path,
    project: &str,
    assignment: &Assignment,
    params: &UpdateParams,
) -> Result<NewTask, ServiceError> {
    let populate = PopulateParams {
        phase: params.phase.clone(),
        overwrite_layout: false,
        create_rls: false,
        overwrite_rls: false,
    };
    let mut new = populate_from_assignment(state, projects_dir, project, assignment, &populate)?;
    new.kind = TaskKind::UpdateRoom;
    new.update_mode = Some(params.mode);
    new.dry_run = params.dry_run;
    Ok(new)
}

/// The FF&E attributed to a room, in the room's own model frame: what an RLS can tag. Nested
/// components are left out (they are parts of a family, not items of the room).
fn room_items(state: &AppState, project: &str, room: &RoomRef) -> Result<Vec<RoomItem>, ServiceError> {
    let to_model = project_to_model(state, project, &room.model_id)?;
    let Some(ffe) = assemble_items(state, &ItemScope { project: Some(project), ..Default::default() })? else {
        return Ok(Vec::new());
    };
    let no_tier = crate::contract::PropertyMap::new();
    Ok(ffe
        .ffe
        .iter()
        .filter(|it| it.owner_rooms_qualified.contains(room) && it.item.super_component_id.is_none())
        .map(|it| {
            let type_props = it
                .type_properties_ref
                .and_then(|r| ffe.type_property_sets.get(r as usize))
                .map_or(&no_tier, |a| a);
            RoomItem {
                id: it.item.id.clone(),
                family: it.item.family_name.clone().unwrap_or_default(),
                type_name: it.item.type_name.clone(),
                category: it.item.category.clone(),
                footprint: it
                    .item
                    .loops
                    .first()
                    .map(|l| l.points.iter().map(|p| to_model.point((p.x * FT_TO_MM, p.y * FT_TO_MM))).collect())
                    .unwrap_or_default(),
                facing: it.item.facing.as_ref().map(|f| to_model.direction((f.x, f.y))),
                properties: properties_of(&it.item.properties),
                type_properties: properties_of(type_props),
            }
        })
        .collect())
}

fn sheet_for_rules(record: &sheets::SheetRecord) -> BlueprintSheet {
    BlueprintSheet {
        revit_sheet_id: record.revit_sheet_id.clone(),
        document_title: record.document_title.clone(),
        sheet_number: record.sheet_number.clone(),
        sheet_name: record.sheet_name.clone(),
        sheet: record.sheet.clone(),
    }
}

/// Where the template's elevation markers and views go in this room (`service::rls_plan`), with a
/// finding for whatever could not be placed.
fn place_elevations(
    state: &AppState,
    project: &str,
    room: &RoomRef,
    record: &sheets::SheetRecord,
    request: &RlsRequest,
    findings: &mut Vec<Value>,
) -> Result<Vec<MarkerPlan>, ServiceError> {
    let mut markers = Vec::new();
    if has_elevations(&record.sheet) {
        let target = SheetRoom { model_id: room.model_id.clone(), room_id: room.room_id.clone() };
        let template = match &record.room {
            Some(r) => model_room_ring(state, project, r)?,
            None => None,
        };
        match (template, model_room_ring(state, project, &target)?) {
            (Some(t), Some(g)) => {
                let (placed, notes) = transfer(&record.sheet, &t, &g, request);
                markers = placed;
                findings.extend(notes.into_iter().map(|n| finding("elevation_not_placed", n)));
            }
            (None, _) => findings.push(finding(
                "elevation_not_placed",
                "the elevations were not placed: the room the sheet was made for is not on the server".to_string(),
            )),
            _ => {}
        }
    }
    for o in &request.depths {
        if !markers.iter().any(|m| m.views.iter().any(|v| v.view_id == o.view_id)) {
            findings.push(finding(
                "depth_override_unused",
                format!("no placed view {} to change the depth of", o.view_id),
            ));
        }
    }
    Ok(markers)
}

/// A create-RLS task: one library sheet's layout for one room. Nothing is placed. A sheet whose
/// room is far from the target's, by the figures it stored at push, is a warning and not a refusal.
pub fn rls_from_sheet(
    state: &AppState,
    projects_dir: &std::path::Path,
    project: &str,
    sheet_id: &str,
    room: &RoomRef,
    phase: &str,
    request: &RlsRequest,
) -> Result<NewTask, ServiceError> {
    let sheet_size = match request.sheet_size.as_deref().map(check_size).transpose() {
        Ok(size) => size,
        Err(reason) => return refuse(reason),
    };
    let record = match sheets::load(projects_dir, project, sheet_id) {
        Ok(r) => r,
        Err(SheetError::NotFound(m) | SheetError::Invalid(m)) => return refuse(m),
        Err(SheetError::Internal(e)) => return Err(ServiceError::Internal(e)),
        Err(e) => return refuse(format!("the sheet library cannot be read: {e:?}")),
    };
    let Some(task_room) = room_facts(state, project, room)? else {
        return refuse("the room is not in the model");
    };
    let mut findings = Vec::new();
    match (record.room_summary, room_summary(state, project, room)?) {
        (Some(stored), Some(target)) => {
            let (_, d) = distance(&Signature::from(target), &Signature::from(stored));
            if d > RLS_ROOM_WARN_DISTANCE {
                findings.push(finding(
                    "room_size_differs",
                    format!(
                        "the sheet was made for a room of {:.1} x {:.1} m, {:.1} m2 with {} items; this room is {:.1} x {:.1} m, {:.1} m2 with {} items",
                        stored.x_mm / 1000.0,
                        stored.y_mm / 1000.0,
                        stored.area_m2,
                        stored.items,
                        target.x_mm / 1000.0,
                        target.y_mm / 1000.0,
                        target.area_m2,
                        target.items
                    ),
                ));
            }
        }
        (None, _) => findings.push(finding(
            "sheet_room_unknown",
            "the sheet has no stored summary of its room, so its size was not compared with this room's".to_string(),
        )),
        (Some(_), None) => {}
    }
    let markers = place_elevations(state, project, room, &record, request, &mut findings)?;
    // No items are placed: the rules for every element are carried, for the plug-in to apply to
    // whatever it finds in the room.
    let tag_rules = tag_rules_for(
        projects_dir,
        project,
        std::slice::from_ref(&sheet_for_rules(&record)),
        None,
        &mut findings,
    );
    let tags = match &request.tags {
        Some(t) => Some(plan_tags(&record.sheet, &room_items(state, project, room)?, &markers, &tag_rules, t)),
        None => None,
    };
    let rls = (sheet_size.is_some() || !markers.is_empty() || tags.is_some()).then_some(RlsPlan {
        sheet_size,
        markers,
        tags,
    });
    Ok(NewTask {
        rls,
        kind: TaskKind::CreateRls,
        update_mode: None,
        dry_run: false,
        model_id: room.model_id.clone(),
        room: task_room,
        phase: phase.to_string(),
        overwrite_layout: false,
        create_rls: true,
        overwrite_rls: request.overwrite_rls,
        blueprint: None,
        library_sheet_id: Some(record.id.clone()),
        items: Vec::new(),
        tag_rules,
        sheets: vec![BlueprintSheet {
            revit_sheet_id: record.revit_sheet_id,
            document_title: record.document_title,
            sheet_number: record.sheet_number,
            sheet_name: record.sheet_name,
            sheet: record.sheet,
        }],
        findings,
    })
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
