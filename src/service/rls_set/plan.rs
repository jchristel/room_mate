//! Working a set of adjacent rooms into a plan: qualifying the rooms, describing the set as one room for matching, ranking sheets,
//! the views of each room and the fallback crop. The request and response types, and the preview itself, are in `mod.rs`.

use super::*;

/// Each asked-for room qualified to its one model, once each; an unknown or ambiguous id, and rooms in several models,
/// are findings. `None` when no room is left.
pub(super) fn qualify_all(
    rooms_read: &crate::service::rooms::RoomsResult,
    ids: &[String],
    findings: &mut Vec<Value>,
) -> Result<Option<Vec<RoomRef>>, ServiceError> {
    let mut refs: Vec<RoomRef> = Vec::new();
    for id in ids {
        match qualify_room_in(rooms_read, id) {
            Ok(Some(r)) if !refs.contains(&r) => refs.push(r),
            Ok(Some(_)) => {}
            Ok(None) => findings.push(finding("room_unknown", format!("room {id:?} is not in this project"))),
            Err(ServiceError::Invalid(reason)) => findings.push(finding("room_ambiguous", reason)),
            Err(e) => return Err(e),
        }
    }
    if refs.is_empty() {
        findings.push(finding("no_rooms", "a set needs at least one room".to_string()));
        return Ok(None);
    }
    let mut models: Vec<&str> = refs.iter().map(|r| r.model_id.as_str()).collect();
    models.sort_unstable();
    models.dedup();
    if models.len() > 1 {
        findings.push(finding(
            "several_models",
            format!(
                "the rooms are in {} models ({}); a set is one model's rooms",
                models.len(),
                models.join(", ")
            ),
        ));
    }
    Ok(Some(refs))
}

/// Each room's outline in its model's frame and what the answer says of it (name, number, area, clear height).
pub(super) fn describe(
    state: &AppState,
    project: &str,
    rooms_read: &crate::service::rooms::RoomsResult,
    refs: &[RoomRef],
    findings: &mut Vec<Value>,
) -> Result<(Vec<Vec<Pt>>, Vec<SetRoom>), ServiceError> {
    let found: Vec<_> = refs
        .iter()
        .map(|r| rooms_read.rooms.iter().find(|x| x.model_id == r.model_id && x.room.id == r.room_id))
        .collect();
    let mut rings: Vec<Vec<Pt>> = Vec::new();
    for r in refs {
        match model_room_ring_in(state, project, rooms_read, &SheetRoom::from(r))? {
            Some(ring) => rings.push(ring),
            None => {
                findings.push(finding("room_has_no_outline", format!("room {:?} has no outline", r.room_id)));
                rings.push(Vec::new());
            }
        }
    }
    let heights = clear_heights(state, project, rooms_read, refs)?;
    let set_rooms: Vec<SetRoom> = refs
        .iter()
        .zip(&found)
        .zip(&rings)
        .map(|((r, f), ring)| {
            let (height_mm, height_from) = match heights.get(r) {
                Some(&(h, HeightFrom::Ceiling)) => (h, "ceiling"),
                Some(&(h, HeightFrom::RoomProperty)) => (h, "room_property"),
                None => (DEFAULT_HEIGHT_MM, "default"),
            };
            SetRoom {
                room_id: r.room_id.clone(),
                model_id: r.model_id.clone(),
                name: f.map(|f| f.room.name.clone()).unwrap_or_default(),
                number: f
                    .and_then(|f| f.room.properties.get("Number"))
                    .map(|v| v.value.trim().to_string())
                    .filter(|n| !n.is_empty()),
                area_m2: signed_area(ring).abs() * M2_PER_MM2,
                height_mm,
                height_from,
            }
        })
        .collect();
    let defaulted: Vec<&str> = set_rooms
        .iter()
        .filter(|r| r.height_from == "default")
        .map(|r| r.room_id.as_str())
        .collect();
    if !defaulted.is_empty() {
        findings.push(finding(
            "height_defaulted",
            format!(
                "no ceiling covers and no height is stated for room(s) {}: their elevations are {DEFAULT_HEIGHT_MM} mm tall",
                defaulted.join(", ")
            ),
        ));
    }
    Ok((rings, set_rooms))
}

/// A finding when the rooms are not one group of adjacent rooms by the server's adjacency (`service::adjacency`, the
/// project's default wall tolerance, as the viewer's adjacency band reads it).
pub(super) fn adjacency_finding(
    state: &AppState,
    project: &str,
    rooms_read: &crate::service::rooms::RoomsResult,
    refs: &[RoomRef],
    findings: &mut Vec<Value>,
) {
    let found: Vec<_> = refs
        .iter()
        .filter_map(|r| rooms_read.rooms.iter().find(|x| x.model_id == r.model_id && x.room.id == r.room_id))
        .collect();
    let contract_rooms: Vec<_> = found.iter().map(|f| &f.room).collect();
    if contract_rooms.len() == refs.len() && refs.len() > 1 {
        let policy = state.settings().settings_for(project).map(|b| b.areas.clone()).unwrap_or_default();
        let wall_max = default_wall_max(&policy, rooms_read);
        let edges: Vec<(usize, usize)> = compute_adjacency_indexed(&contract_rooms, wall_max)
            .into_iter()
            .map(|e| (e.ia, e.ib))
            .collect();
        let parts = groups(refs.len(), &edges);
        if parts.len() > 1 {
            let named: Vec<String> = parts
                .iter()
                .map(|g| g.iter().map(|&i| refs[i].room_id.as_str()).collect::<Vec<_>>().join(" + "))
                .collect();
            findings.push(finding(
                "not_adjacent",
                format!(
                    "the rooms are {} groups that share no wall ({}); a set is one group of adjacent rooms",
                    parts.len(),
                    named.join("; ")
                ),
            ));
        }
    }
}

/// The set as one room: the rectangle round all its rooms, their summed area and item count.
pub(super) fn set_signature(
    ctx: &Context,
    refs: &[RoomRef],
    rings: &[Vec<Pt>],
    set_rooms: &[SetRoom],
) -> Option<Signature> {
    set_rectangle(rings).map(|(long, short)| Signature {
        area_m2: set_rooms.iter().map(|r| r.area_m2).sum(),
        proportion: if short > 0.0 { long / short } else { 1.0 },
        items: refs.iter().map(|r| ctx.item_count(r)).sum(),
    })
}

/// The library sheets ranked against the set, best first, and a finding for those that cannot be ranked.
pub(super) fn rank_set(
    ctx: &Context,
    sheets: &[SheetRecord],
    signature: Option<&Signature>,
    findings: &mut Vec<Value>,
) -> Vec<crate::service::sheet_suggest::Suggestion> {
    let (ranked, unresolved, unassigned) = match signature {
        Some(sig) => ctx.rank_sheets(sig, sheets),
        None => (Vec::new(), Vec::new(), 0),
    };
    if unassigned + unresolved.len() > 0 {
        findings.push(finding(
            "sheets_not_ranked",
            format!(
                "{} of {} library sheets cannot be ranked: they carry no room figures and name no room on the server \
                 (push them again from the Sheets button)",
                unassigned + unresolved.len(),
                sheets.len()
            ),
        ));
    }
    ranked
}

/// The views of the set on `sheet`'s scales. `None` (and a finding) when the sheet has no plan viewport to take a
/// scale from.
#[allow(clippy::too_many_arguments)]
pub(super) fn set_views(
    state: &AppState,
    project: &str,
    rooms_read: &crate::service::rooms::RoomsResult,
    sheet: &SheetRecord,
    refs: &[RoomRef],
    rings: &[Vec<Pt>],
    set_rooms: &[SetRoom],
    findings: &mut Vec<Value>,
) -> Result<Option<SetViews>, ServiceError> {
    let body = &sheet.sheet;
    let Some(plan_scale) = ports_of(body, "floor plan").first().and_then(|p| scale_of(p)) else {
        findings.push(finding(
            "sheet_has_no_plan",
            format!("library sheet {} has no plan viewport", sheet.sheet_number),
        ));
        return Ok(None);
    };
    let template = match &sheet.room {
        Some(r) => model_room_ring_in(state, project, rooms_read, r)?,
        None => None,
    };
    let all: Vec<Pt> = rings.iter().flatten().copied().collect();
    let turn = set_turn(template.as_deref().unwrap_or(&[]), rings);
    let crop = template.as_deref().and_then(|t| plan_crop_turned(body, t, &all, turn)).or_else(|| {
        findings.push(finding(
            "plan_framing_unknown",
            format!(
                "the library sheet's room or plan crop cannot be read: the plan is the set plus {FALLBACK_CROP_MARGIN_MM} mm"
            ),
        ));
        fallback_crop(&all, turn)
    });
    let Some(crop) = crop else {
        return Ok(None);
    };
    let plan = SetPlanView {
        paper_mm: [
            (crop.max[0] - crop.min[0]) / plan_scale,
            (crop.max[1] - crop.min[1]) / plan_scale,
        ],
        crop,
        scale: plan_scale,
    };
    let elevation_scale = ports_of(body, "elevation").first().and_then(|p| scale_of(p)).unwrap_or(plan_scale);
    let elevations = refs
        .iter()
        .zip(rings)
        .zip(set_rooms)
        .flat_map(|((r, ring), room)| room_elevations(&r.room_id, ring, room.height_mm, elevation_scale))
        .collect();
    let schedule_paper = ports_of(body, "schedule").first().and_then(|p| paper_of(p));
    let schedules = match schedule_paper {
        Some(paper_mm) => refs
            .iter()
            .map(|r| SetSchedule { room_id: r.room_id.clone(), paper_mm, estimate: true })
            .collect(),
        None => Vec::new(),
    };
    let three_d = ports_of(body, "three d")
        .first()
        .and_then(|p| paper_of(p))
        .map(|paper_mm| SetThreeD { paper_mm, estimate: true });
    Ok(Some(SetViews { plan, elevations, schedules, three_d }))
}

/// The set's box in the turned frame plus [`FALLBACK_CROP_MARGIN_MM`], when the template's framing is unknown.
pub(super) fn fallback_crop(all: &[Pt], turn: Option<crate::service::rls_turn::ViewTurn>) -> Option<PlanCrop> {
    let deg = turn.map_or(0.0, |t| t.angle_deg);
    let seen: Vec<Pt> = all.iter().map(|&p| rotate(p, -deg)).collect();
    let (x0, x1) = seen.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.0), b.max(p.0)));
    let (y0, y1) = seen.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.1), b.max(p.1)));
    if !(x1 > x0 && y1 > y0) {
        return None;
    }
    let m = FALLBACK_CROP_MARGIN_MM;
    let half = ((x1 - x0) / 2.0 + m, (y1 - y0) / 2.0 + m);
    let centre = rotate(((x0 + x1) / 2.0, (y0 + y1) / 2.0), deg);
    Some(PlanCrop {
        min: [centre.0 - half.0, centre.1 - half.1],
        max: [centre.0 + half.0, centre.1 + half.1],
        offsets_mm: crate::service::rls_plan::CropOffsets { left: m, right: m, bottom: m, top: m },
        turn,
    })
}
