//! The create-RLS task: a library sheet copied for one room (`rls_from_sheet_with`), the room's items and doors the sheet is
//! drawn against, and a queued task's 3D views aimed again (`aim_three_ds`). Reads go through `RlsReads`, once per request.

use super::*;

/// The FF&E attributed to a room, in the room's own model frame: what an RLS can tag. Nested components come too,
/// marked `nested`, whatever the project's `[ffe] nested_components` says: `rls_tags::plan_tags` keeps one only when
/// the library has a rule for its type code, because a person tagged one of those (a shower rail nested in its
/// shower) and a part nobody tagged (a handle) must never be offered.
pub(super) fn room_items(reads: &RlsReads, room: &RoomRef, parameter: &str) -> Result<Vec<RoomItem>, ServiceError> {
    let to_model = project_to_model(reads.state, reads.project, &room.model_id)?;
    let Some(ffe) = reads.ffe()? else {
        return Ok(Vec::new());
    };
    let no_tier = crate::contract::PropertyMap::new();
    Ok(ffe
        .ffe
        .iter()
        .filter(|it| it.owner_rooms_qualified.contains(room))
        .map(|it| {
            let type_props = it
                .type_properties_ref
                .and_then(|r| ffe.type_property_sets.get(r as usize))
                .map_or(&no_tier, |a| a);
            let properties = properties_of(&it.item.properties);
            let type_properties = properties_of(type_props);
            RoomItem {
                id: it.item.id.clone(),
                family: it.item.family_name.clone().unwrap_or_default(),
                type_name: it.item.type_name.clone(),
                category: it.item.category.clone(),
                type_code: type_code_of(parameter, &type_properties, &properties),
                footprint: it
                    .item
                    .loops
                    .first()
                    .map(|l| l.points.iter().map(|p| to_model.point((p.x * FT_TO_MM, p.y * FT_TO_MM))).collect())
                    .unwrap_or_default(),
                facing: it.item.facing.as_ref().map(|f| to_model.direction((f.x, f.y))),
                nested: it.item.super_component_id.is_some(),
                properties,
                type_properties,
            }
        })
        .collect())
}

/// The footprints of the doors that open onto or out of `room` (either side names it, as a blueprint's capture reads
/// them), in the room's own model's frame, mm.
pub(super) fn door_marks(reads: &RlsReads, room: &RoomRef) -> Result<Vec<Vec<[f64; 2]>>, ServiceError> {
    let to_model = project_to_model(reads.state, reads.project, &room.model_id)?;
    Ok(reads
        .doors()?
        .iter()
        .filter(|d| d.opens_onto(room))
        .map(|d| {
            d.door
                .footprint
                .iter()
                .map(|&p| {
                    let q = to_model.point(p);
                    [q.0, q.1]
                })
                .collect()
        })
        .collect())
}

/// A queued create-RLS task's 3D views looked along the directions a person chose in plan (`aims`: the view's index
/// from 1 and a heading; each view not named keeps the template's direction): the cameras, the walls to hide and the
/// items to see through worked out again from the room's outline the task stored. The task's FF&E is read for the
/// last of those, which is why this is not a plain edit of the stored plan.
pub fn aim_three_ds(
    state: &AppState,
    project: &str,
    task: &crate::tasks::Task,
    aims: &[(usize, [f64; 2])],
) -> Result<Vec<crate::service::rls_three_d::ThreeDPlan>, ServiceError> {
    let invalid = |m: &str| ServiceError::Invalid(m.to_string());
    let rls = task.rls.as_ref().ok_or_else(|| invalid("the task holds no sheet plan"))?;
    if rls.three_ds.is_empty() {
        return Err(invalid("the task has no 3D view to aim"));
    }
    if rls.room_outline.len() < 3 {
        return Err(invalid(
            "the task was made before it kept the room's outline: make it again to aim its 3D views",
        ));
    }
    for (index, d) in aims {
        if !(1..=rls.three_ds.len()).contains(index) {
            return Err(ServiceError::Invalid(format!("the task has no 3D view {index}")));
        }
        if !d[0].is_finite() || !d[1].is_finite() || d[0].hypot(d[1]) < 1e-9 {
            return Err(invalid("a direction needs a length"));
        }
    }
    let sheet = &task.sheets.first().ok_or_else(|| invalid("the task holds no sheet"))?.sheet;
    let ring: Vec<Pt> = rls.room_outline.iter().map(|p| (p[0], p[1])).collect();
    let reads = RlsReads::new(state, project);
    let room = RoomRef { model_id: task.model_id.clone(), room_id: task.room.room_id.clone() };
    let items = room_items(&reads, &room, &type_code_parameter(state, project))?;
    Ok(three_ds(sheet, &ring, rls.plan_crop.as_ref(), &items, aims))
}

pub(super) fn sheet_for_rules(record: &sheets::SheetRecord) -> BlueprintSheet {
    BlueprintSheet {
        revit_sheet_id: record.revit_sheet_id.clone(),
        document_title: record.document_title.clone(),
        sheet_number: record.sheet_number.clone(),
        sheet_name: record.sheet_name.clone(),
        size_mm: record.size_mm,
        room_summary: None,
        room_outline: Vec::new(),
        sheet: record.sheet.clone(),
    }
}

/// The template room's outline (or why there is none) and this room's (absent when it is not on the server).
pub(super) type Rings = (Result<Vec<Pt>, &'static str>, Option<Vec<Pt>>);

/// Where the template's elevation markers and views go in this room (`service::rls_plan`), with a
/// finding for whatever could not be placed.
pub(super) fn place_elevations(
    rings: &Rings,
    record: &sheets::SheetRecord,
    request: &RlsRequest,
    findings: &mut Vec<Value>,
) -> Vec<MarkerPlan> {
    let mut markers = Vec::new();
    if has_elevations(&record.sheet) {
        match rings {
            (Ok(t), Some(g)) => {
                let (placed, notes) = transfer(&record.sheet, t, g, request);
                markers = placed;
                findings.extend(notes.into_iter().map(|n| finding("elevation_not_placed", n)));
            }
            (Err(why), _) => {
                findings.push(finding("elevation_not_placed", format!("the elevations were not placed: {why}")))
            }
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
    markers
}

/// The template room's outline and this room's, each in its own model's frame (the sheet's numbers are), read once
/// for the elevations and the plan crop.
pub(super) fn template_and_target(
    reads: &RlsReads,
    room: &RoomRef,
    record: &sheets::SheetRecord,
) -> Result<Rings, ServiceError> {
    let rooms = reads.rooms()?;
    let template = template_ring(reads.state, reads.project, rooms, record)?;
    let target = match rooms {
        Some(rooms) => model_room_ring_in(reads.state, reads.project, rooms, &SheetRoom::from(room))?,
        None => None,
    };
    Ok((template, target))
}

/// A warning, never a refusal, when the room the sheet was made for (by the figures it stored at push) is far from
/// this one.
pub(super) fn room_size_finding(
    reads: &RlsReads,
    room: &RoomRef,
    record: &sheets::SheetRecord,
    findings: &mut Vec<Value>,
) -> Result<(), ServiceError> {
    // the item count reads the FF&E, so only when there is a stored summary to compare it with
    let target = match (record.room_summary, reads.rooms()?) {
        (Some(_), Some(rooms)) => match room_summary_in(rooms, room, 0) {
            Some(_) => room_summary_in(rooms, room, reads.item_count(room)?),
            None => None,
        },
        _ => None,
    };
    match (record.room_summary, target) {
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
    Ok(())
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
    rls_from_sheet_with(&RlsReads::new(state, project), projects_dir, sheet_id, room, phase, request)
}

/// [`rls_from_sheet`] over the project's rooms and FF&E read once (`rls_reads`): a request for many rooms makes
/// every task from the same reads.
pub fn rls_from_sheet_with(
    reads: &RlsReads,
    projects_dir: &std::path::Path,
    sheet_id: &str,
    room: &RoomRef,
    phase: &str,
    request: &RlsRequest,
) -> Result<NewTask, ServiceError> {
    let (state, project) = (reads.state, reads.project);
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
    // The sheet asked for none: the size it was measured at says which one it is (duHast's own
    // `sheet_size` is blank on these sheets).
    let sheet_size = sheet_size.or_else(|| {
        let size = record.size_mm.as_ref()?;
        size_token(size.width_mm, size.height_mm).map(str::to_string)
    });
    let Some(task_room) = reads.rooms()?.and_then(|rooms| room_facts_in(rooms, room)) else {
        return refuse("the room is not in the model");
    };
    let mut findings = Vec::new();
    room_size_finding(reads, room, &record, &mut findings)?;
    let rings = template_and_target(reads, room, &record)?;
    let markers = place_elevations(&rings, &record, request, &mut findings);
    let plan_crop = match &rings {
        (Ok(t), Some(g)) => plan_crop(&record.sheet, t, g),
        _ => None,
    };
    // No items are placed: the rules for every element are carried, for the plug-in to apply to
    // whatever it finds in the room.
    let parameter = type_code_parameter(state, project);
    let tag_rules = rls_tag_rules(
        projects_dir,
        project,
        std::slice::from_ref(&sheet_for_rules(&record)),
        &parameter,
        record.blueprint.as_deref(),
        &mut findings,
    );
    // The room's FF&E is read once, and only when something asks for it: the tags, or a 3D view to see past.
    let three_d_room = rings.1.as_ref().filter(|_| has_three_d(&record.sheet));
    let items = if request.tags.is_some() || three_d_room.is_some() {
        room_items(reads, room, &parameter)?
    } else {
        Vec::new()
    };
    let tags = request.tags.as_ref().map(|t| plan_tags(&record.sheet, &items, &markers, &tag_rules, t));
    let three_ds = three_d_room
        .map(|ring| three_ds(&record.sheet, ring, plan_crop.as_ref(), &items, &[]))
        .unwrap_or_default();
    // what the tasks page draws the cameras against: the room's outline and its doors, in the model's own frame
    let room_outline: Vec<[f64; 2]> = rings.1.iter().flatten().map(|p| [p.0, p.1]).collect();
    let doors = door_marks(reads, room)?;
    let room_tag = room_tag_head(rings.0.as_deref().ok(), rings.1.as_deref(), &record, &tag_rules, &mut findings);
    let rls = (sheet_size.is_some()
        || !markers.is_empty()
        || tags.is_some()
        || room_tag.is_some()
        || plan_crop.is_some()
        || !room_outline.is_empty()
        || !three_ds.is_empty())
    .then_some(RlsPlan {
        sheet_size,
        markers,
        tags,
        room_tag,
        plan_crop,
        three_ds,
        room_outline,
        doors,
        viewport_moves: Vec::new(),
    });
    Ok(NewTask {
        rls,
        rls_set: None,
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
        tag_rules: tag_rules.iter().map(TaskRule::from).collect(),
        sheets: vec![BlueprintSheet {
            revit_sheet_id: record.revit_sheet_id,
            document_title: record.document_title,
            sheet_number: record.sheet_number,
            sheet_name: record.sheet_name,
            size_mm: record.size_mm,
            room_summary: None,
            room_outline: Vec::new(),
            sheet: record.sheet,
        }],
        findings,
    })
}
