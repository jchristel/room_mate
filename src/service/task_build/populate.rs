//! The populate and update tasks: a blueprint's layout as the resolver places it now, for one assignment (`populate_from_assignment`,
//! `update_from_assignment`). The shared vocabulary (findings, room facts, tag rules) is in `mod.rs`.

use super::*;

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

/// The blueprint's sheets: the library sheets bound to it (plan decision 27), as they are now.
/// Empty when the task makes no sheets; a task that does and finds none is refused.
pub(super) fn bound_sheets(
    projects_dir: &std::path::Path,
    project: &str,
    blueprint: &str,
    create_rls: bool,
) -> Result<Vec<BlueprintSheet>, ServiceError> {
    if !create_rls {
        return Ok(Vec::new());
    }
    let bound: Vec<BlueprintSheet> = match sheets::bound_to(projects_dir, project, blueprint) {
        Ok(records) => records.iter().map(sheet_for_rules).collect(),
        Err(SheetError::Internal(e)) => return Err(ServiceError::Internal(e)),
        Err(e) => return refuse(format!("the sheet library cannot be read: {e:?}")),
    };
    if bound.is_empty() {
        return refuse(
            "no room layout sheet is bound to the blueprint, so sheets cannot be created from it (turn off create RLS, or push the blueprint from Revit with its sheets)",
        );
    }
    Ok(bound)
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
    let bound = bound_sheets(projects_dir, project, &blueprint.id, params.create_rls)?;
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
        item_tag_rules(state, projects_dir, project, &bound, &items, &blueprint.id, &mut findings)
    } else {
        Vec::new()
    };
    Ok(NewTask {
        rls: None,
        rls_set: None,
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
        sheets: bound,
        tag_rules: tag_rules.iter().map(TaskRule::from).collect(),
        findings,
    })
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
