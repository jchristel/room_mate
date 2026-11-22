//! Capturing a room from what the server already holds: the room found among the project's rooms (`qualify_room`), its outline, the doors
//! that open onto or out of it and its FF&E, handed to the pure capture in `mod.rs`. Nothing is read from Revit here.

use super::*;

/// A room reference from a request that may leave the model out, because the plan's room rows
/// do not name theirs. With a model it is taken as given; without one the room id is looked up
/// among the project's rooms: `None` when no room has it, an error naming the models when
/// several do, because a silent guess would act on the wrong room (a room id is unique only
/// within its model). The rule `connections::Resolver` applies to a saved link.
pub fn qualify_room(
    state: &AppState,
    project: &str,
    room_id: &str,
    model_id: Option<&str>,
) -> Result<Option<RoomRef>, ServiceError> {
    if let Some(model_id) = model_id.map(str::trim).filter(|m| !m.is_empty()) {
        return Ok(Some(RoomRef { model_id: model_id.to_string(), room_id: room_id.to_string() }));
    }
    let Some(rooms) = assemble_rooms(state, &RoomScope { project: Some(project), ..Default::default() })? else {
        return Ok(None);
    };
    qualify_room_in(&rooms, room_id)
}

/// [`qualify_room`] with no model named, against the project's rooms already read: a request that qualifies many
/// rooms reads them once.
pub fn qualify_room_in(rooms: &RoomsResult, room_id: &str) -> Result<Option<RoomRef>, ServiceError> {
    let mut models: Vec<&str> =
        rooms.rooms.iter().filter(|r| r.room.id == room_id).map(|r| r.model_id.as_str()).collect();
    models.sort_unstable();
    models.dedup();
    match models.as_slice() {
        [] => Ok(None),
        [only] => Ok(Some(RoomRef { model_id: (*only).to_string(), room_id: room_id.to_string() })),
        many => Err(ServiceError::Invalid(format!(
            "room id {room_id:?} exists in {} models ({}); name the model",
            many.len(),
            many.join(", ")
        ))),
    }
}

/// The room's name and outer loop in millimetres, in the project's plan frame, or `None` when the
/// project has no such room. Shared by capture and the resolver so both read the same outline.
pub fn room_ring(state: &AppState, project: &str, room: &RoomRef) -> Result<Option<(String, Vec<Pt>)>, ServiceError> {
    let Some(rooms) = assemble_rooms(state, &RoomScope { project: Some(project), ..Default::default() })? else {
        return Ok(None);
    };
    let Some(found) = rooms.rooms.iter().find(|r| r.model_id == room.model_id && r.room.id == room.room_id) else {
        return Ok(None);
    };
    let Some(outer) = found.room.loops.first() else {
        return Err(ServiceError::Invalid(format!("room {:?} has no outline", room.room_id)));
    };
    let ring = loop_mm(outer);
    Ok(Some((found.room.name.clone(), ring)))
}

/// The doors that open onto or out of `room` (either side names it), as footprints in
/// millimetres. **Not the doors attributed to it**: a door belongs to the room it opens into
/// (`room_attribution`), but a door to the corridor is still a door of this room's wall, and
/// that is the question orientation asks. A door with no footprint is left out.
pub fn room_doors(state: &AppState, project: &str, room: &RoomRef) -> Result<Vec<SourceDoor>, ServiceError> {
    Ok(project_doors(state, project)?
        .into_iter()
        .filter(|d| d.opens_onto(room))
        .map(|d| d.door)
        .collect())
}

/// A door with the rooms it names, for a caller asking about several rooms from one read.
pub struct ProjectDoor {
    pub model_id: String,
    pub from_room: Option<String>,
    pub to_room: Option<String>,
    pub door: SourceDoor,
}

impl ProjectDoor {
    /// Either side names `room`, in the room's own model (a room id is unique only within one).
    pub fn opens_onto(&self, room: &RoomRef) -> bool {
        let names = |side: &Option<String>| side.as_deref() == Some(room.room_id.as_str());
        self.model_id == room.model_id && (names(&self.from_room) || names(&self.to_room))
    }
}

/// Every door of the project that has a footprint, in millimetres. One read, so assigning many
/// rooms costs one assembly of the doors and not one per room.
pub fn project_doors(state: &AppState, project: &str) -> Result<Vec<ProjectDoor>, ServiceError> {
    let scope = OpeningScope { project: Some(project), ..Default::default() };
    let Some(assembled) = assemble_openings::<DoorPayload>(state, OpeningKind::Doors, &scope)? else {
        return Ok(Vec::new());
    };
    Ok(assembled
        .openings
        .iter()
        .filter_map(|d| {
            let outer = d.door.loops.first()?;
            let footprint: Vec<Pt> = loop_mm(outer);
            (footprint.len() >= 3).then(|| ProjectDoor {
                model_id: d.model_id.clone(),
                from_room: d.door.from_room.clone(),
                to_room: d.door.to_room.clone(),
                door: SourceDoor { id: d.door.id.clone(), footprint },
            })
        })
        .collect())
}

/// Read one room and the FF&E attributed to it, and capture them.
///
/// Returns `Ok(None)` when the project has no room by that reference. The FF&E is the
/// project's, scoped by the room it is attributed to (`owner_rooms_qualified`), so the
/// attribution policy the project runs on is the one a blueprint is captured under.
pub fn capture_room(
    state: &AppState,
    project: &str,
    room: &RoomRef,
) -> Result<Option<(Captured, String)>, ServiceError> {
    let Some((room_name, ring)) = room_ring(state, project, room)? else {
        return Ok(None);
    };

    let mut sources = Vec::new();
    if let Some(ffe) = assemble_items(state, &ItemScope { project: Some(project), ..Default::default() })? {
        let no_tier = PropertyMap::new();
        for it in ffe.ffe.iter().filter(|it| it.owner_rooms_qualified.contains(room)) {
            let type_props: &PropertyMap = it
                .type_properties_ref
                .and_then(|r| ffe.type_property_sets.get(r as usize))
                .map_or(&no_tier, |a| a);
            sources.push(SourceItem {
                id: it.item.id.clone(),
                family_name: it.item.family_name.clone(),
                type_name: it.item.type_name.clone(),
                category: it.item.category.clone(),
                footprint: it.item.loops.first().map(loop_mm).unwrap_or_default(),
                facing: it.item.facing.as_ref().map(|f| (f.x, f.y)),
                nested: it.item.super_component_id.is_some(),
                properties: properties_of(&it.item.properties),
                type_properties: properties_of(type_props),
            });
        }
    }
    let captured = capture_shaped(&ring, sources, room_doors(state, project, room)?, Reference::Box)?;
    Ok(Some((captured, room_name)))
}
