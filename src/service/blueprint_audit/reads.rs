//! The audit of ONE assignment: the layout the resolver gives for the room today, set against what the room
//! holds today. The pure comparison is `super::audit`; this reads the assignment, its pinned revision, the
//! room and the project's FF&E and makes the two lists it compares.

use serde::Serialize;

use super::super::blueprint_capture::{kit_of, loop_mm, properties_of, FT_TO_MM};
use super::super::blueprint_geometry::{centroid, Pt};
use super::super::blueprint_resolve::{resolve_in_room_with, FitKind};
use super::super::items::{assemble_items, ItemScope};
use super::super::rls_tags::type_code_of;
use super::super::room_locator::RoomRef;
use super::super::task_build::{blueprint_error, edge_map_of};
use super::super::ServiceError;
use super::{audit, Audit, Element, Layout};
use crate::assignments::{self, RoomKey};
use crate::blueprints::{self, BlueprintError};
use crate::service::blueprint_model::Property;
use crate::state::AppState;

/// One assignment's audit, with what it was measured against.
#[derive(Debug, Clone, Serialize)]
pub struct AssignmentAudit {
    pub assignment_id: String,
    pub blueprint_id: String,
    pub blueprint_name: String,
    /// The revision the assignment pins, which is the layout that was judged.
    pub revision: u32,
    pub room: RoomKey,
    pub room_name: String,
    /// How the blueprint fits the room; `edge_count_differs`, `mirror_image` and `no_fit` leave no positions to
    /// judge and the audit compares by code only.
    pub fit: FitKind,
    /// The project's type-code parameter the codes were read from.
    pub type_code_parameter: String,
    #[serde(flatten)]
    pub audit: Audit,
}

fn refuse<T>(reason: impl Into<String>) -> Result<T, ServiceError> {
    Err(ServiceError::Invalid(reason.into()))
}

/// Audit one assignment. `Ok(None)` when the project has no such assignment or its room has gone.
pub fn audit_assignment(
    state: &AppState,
    projects_dir: &std::path::Path,
    project: &str,
    assignment_id: &str,
) -> Result<Option<AssignmentAudit>, ServiceError> {
    let assignment = match assignments::list_raw(projects_dir, project) {
        Ok(all) => all.into_iter().find(|a| a.id == assignment_id),
        Err(e) => return Err(blueprint_error(e)),
    };
    let Some(assignment) = assignment else {
        return Ok(None);
    };

    // The codes are the project's type-code parameter, the same identity the tag rules use. Without it there is
    // no code to match on, and flagging every item `no_code` would be a wrong answer dressed as a finding.
    let bundle = state.settings();
    let settings = bundle.settings_for(project);
    let parameter = settings.and_then(|b| b.tag_type_code.clone()).filter(|p| !p.trim().is_empty());
    let Some(parameter) = parameter else {
        return refuse(
            "the project names no type-code parameter (tag_type_code in its settings), so there is no code to match elements on",
        );
    };
    let policy = settings.map(|b| b.blueprint_audit.clone()).unwrap_or_default();

    let blueprint = match blueprints::load(projects_dir, project, &assignment.blueprint_id) {
        Ok(b) => b,
        Err(BlueprintError::NotFound(_)) => return refuse("the assignment's blueprint has been deleted"),
        Err(e) => return Err(blueprint_error(e)),
    };
    let Some(revision) = blueprint.revision(assignment.revision) else {
        return refuse(format!("the blueprint has no revision {}", assignment.revision));
    };
    let room = RoomRef {
        model_id: assignment.room.model_id.clone(),
        room_id: assignment.room.room_id.clone(),
    };
    let mapping = edge_map_of(&assignment.mapping);
    // The resolver applies the assignment's overrides, so an item the room excludes is not in the layout and a
    // nudged one is where the room wants it.
    let Some((_, resolution)) =
        resolve_in_room_with(state, project, &revision.content, &room, &mapping, &assignment.overrides)?
    else {
        return Ok(None);
    };

    let layout: Vec<Layout> = resolution
        .placements
        .iter()
        .filter_map(|p| {
            let source = revision.content.items.iter().find(|i| i.id == p.item_id)?;
            Some(Layout {
                id: p.item_id.clone(),
                code: type_code_of(&parameter, &source.type_properties, &source.properties),
                type_name: p.type_name.clone(),
                family_name: source.family_name.clone(),
                centre: (p.centre.x, p.centre.y),
                facing: (p.facing.x, p.facing.y),
                hosting: source.hosting.clone(),
                class: source.class,
                kit: source.kit.clone(),
            })
        })
        .collect();

    let elements = room_elements(state, project, &room, &parameter)?;
    let judge_location = matches!(resolution.fit.kind, FitKind::Plain | FitKind::Manual | FitKind::Framed);
    Ok(Some(AssignmentAudit {
        assignment_id: assignment.id.clone(),
        blueprint_id: blueprint.id.clone(),
        blueprint_name: blueprint.name.clone(),
        revision: revision.number,
        room: assignment.room.clone(),
        room_name: assignment.room_name.clone(),
        fit: resolution.fit.kind,
        type_code_parameter: parameter,
        audit: audit(&layout, &elements, judge_location, &policy),
    }))
}

/// The room's FF&E as the audit reads it: hosts only. Read with components whatever the project's
/// `[ffe] nested_components` says, and the components dropped here, so that setting never changes a verdict (a
/// guard with a nested bracket is one guard).
fn room_elements(
    state: &AppState,
    project: &str,
    room: &RoomRef,
    parameter: &str,
) -> Result<Vec<Element>, ServiceError> {
    let Some(ffe) =
        assemble_items(state, &ItemScope { project: Some(project), with_components: true, ..Default::default() })?
    else {
        return Ok(Vec::new());
    };
    let no_tier = crate::contract::PropertyMap::new();
    let mut out = Vec::new();
    for it in ffe.ffe.iter().filter(|it| !it.is_component && it.owner_rooms_qualified.contains(room)) {
        let type_props = it
            .type_properties_ref
            .and_then(|r| ffe.type_property_sets.get(r as usize))
            .map_or(&no_tier, |a| a);
        let instance: Vec<Property> = properties_of(&it.item.properties);
        let type_tier: Vec<Property> = properties_of(type_props);
        out.push(Element {
            id: it.item.id.clone(),
            unique_id: it.item.unique_id.clone(),
            code: type_code_of(parameter, &type_tier, &instance),
            type_name: it.item.type_name.clone(),
            family_name: it.item.family_name.clone(),
            centre: element_centre(&it.item),
            facing: it.item.facing.as_ref().map(|f| (f.x, f.y)),
            hosting: super::super::blueprint_capture::hosting_of(&it.item),
            kit: kit_of(&it.item),
        });
    }
    Ok(out)
}

/// Where an element is, in millimetres: the middle of its footprint (the quantity a blueprint item's place was
/// captured from), else its insertion point. Both are in the project's plan frame by the time they are read.
fn element_centre(item: &crate::contract::Item) -> Option<Pt> {
    if let Some(outer) = item.loops.first() {
        let ring = loop_mm(outer);
        if ring.len() >= 3 {
            return Some(centroid(&ring));
        }
    }
    item.insertion_point.as_ref().map(|p| (p.x * FT_TO_MM, p.y * FT_TO_MM))
}
