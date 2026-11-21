//! Reading an entity's elements for a report: rooms, openings, surfaces, FF&E and spaces each read once into `Element`s with a field reader,
//! so the rows are built over one shape whatever the entity.

use super::*;

/// One element, reduced to what a report needs: a way to read its fields, its
/// owning model, and the rooms it is attributed to with their measures.
///
/// **Assembled per entity and then never branched on again.** Every difference
/// between a door, a ceiling and an item is resolved here, which is what keeps
/// the row building below one implementation rather than seven.
pub(super) struct Element {
    /// `(source, field)` resolution, boxed so the row builder holds elements of
    /// several concrete types without knowing which.
    pub(super) read: FieldReader,
    /// The rooms this element is attributed to, in the entity's own order, with
    /// whatever the join measured. Empty means unattributed — a reported state.
    pub(super) rooms: Vec<Attribution>,
}

/// One element's field lookup, in the `?filter=` vocabulary: `None` source for
/// the entity's own properties and intrinsics, `Some(name)` for a joined
/// reference record.
pub(super) type FieldReader = Box<dyn Fn(Option<&str>, &str, &[BuiltinPropertyDef]) -> PropertyPresence>;

/// One element-to-room attribution, as the entity read reported it.
pub(super) struct Attribution {
    pub(super) model_id: String,
    pub(super) room_id: String,
    pub(super) measures: BTreeMap<&'static str, String>,
}

/// One model's level names by id, shared by every element of that model.
pub(super) type LevelNames = Arc<BTreeMap<String, String>>;

/// Each model's level names, so an element reads the name of the level ITS document declares.
///
/// **The model's own name for a level, not a canonical storey.** A `level_id` is per document, so the
/// id alone names nothing across models; the name the element's own model gives it is a fact that
/// needs no judgement ("LEVEL 2" in every fit-out model). Grouping differently-named levels into one
/// storey is the viewer's name-plus-elevation rule (`renderer/storey.ts`) and is not repeated here.
pub(super) fn level_names_by_model(
    levels: &BTreeMap<String, Vec<crate::contract::Level>>,
) -> BTreeMap<String, LevelNames> {
    levels
        .iter()
        .map(|(model, levels)| {
            (model.clone(), Arc::new(levels.iter().map(|l| (l.id.clone(), l.name.clone())).collect()))
        })
        .collect()
}

pub(super) fn names_of(by_model: &BTreeMap<String, LevelNames>, model: &str) -> LevelNames {
    by_model.get(model).cloned().unwrap_or_default()
}

/// Add the `$level` intrinsic to a reader: the name of the level `$level_id` points at. An id the model
/// does not declare reads as absent, not as the id, so a cell never passes an id off as a name.
pub(super) fn with_level_name(read: FieldReader, names: LevelNames) -> FieldReader {
    Box::new(move |source, property, defs| {
        if source.is_none() && property == "$level" {
            return match read(None, "$level_id", defs) {
                PropertyPresence::Present(id) => {
                    names.get(&id).map_or(PropertyPresence::Absent, |n| PropertyPresence::Present(n.clone()))
                }
                other => other,
            };
        }
        read(source, property, defs)
    })
}

pub(super) fn presence_reader<T>(element: T) -> FieldReader
where
    T: FilterTarget + 'static,
{
    Box::new(move |source, property, defs| element.presence(source, property, defs))
}

pub(super) fn number(value: f64) -> String {
    format!("{value:.2}")
}

/// Read one entity, in the report's scope, and flatten it to `Element`s.
///
/// One arm per entity and nothing else: everything that differs between a door
/// and a ceiling is resolved in these readers, so the row building below never
/// asks what it is holding.
pub(super) fn read_elements(
    state: &AppState,
    scope: &ReportScope<'_>,
    entity: Entity,
) -> Result<Option<Vec<Element>>, ServiceError> {
    match entity {
        Entity::Rooms => read_rooms(state, scope),
        Entity::Doors => read_openings::<DoorPayload>(state, scope, OpeningKind::Doors),
        Entity::Windows => read_openings::<WindowPayload>(state, scope, OpeningKind::Windows),
        Entity::Ceilings => read_surfaces::<CeilingPayload>(state, scope),
        Entity::Floors => read_surfaces::<FloorPayload>(state, scope),
        Entity::Ffe => read_items(state, scope),
        Entity::Spaces => read_spaces(state, scope),
    }
}

pub(super) fn read_rooms(state: &AppState, scope: &ReportScope<'_>) -> Result<Option<Vec<Element>>, ServiceError> {
    let room_scope = RoomScope {
        project: scope.project,
        building: scope.building,
        milestone: scope.milestone,
        filter: None,
    };
    Ok(rooms::assemble_rooms(state, &room_scope)?.map(|result| {
        let names: LevelNames = Arc::new(result.levels.iter().map(|l| (l.id.clone(), l.name.clone())).collect());
        result
            .rooms
            .into_iter()
            .map(|room| {
                let model_id = room.model_id.clone();
                let room_id = room.room.id.clone();
                Element {
                    read: with_level_name(presence_reader(room), names.clone()),
                    // A room is its own attribution, so a room schedule can
                    // carry room columns without a special case.
                    rooms: vec![Attribution { model_id, room_id, measures: BTreeMap::new() }],
                }
            })
            .collect()
    }))
}

/// Doors and windows, whose attribution is `owner_rooms_qualified` — the
/// policy's answer, model-qualified, which is the only form safe to index
/// rooms with across linked models.
pub(super) fn read_openings<P: crate::contract::OpeningEnvelope + serde::de::DeserializeOwned>(
    state: &AppState,
    scope: &ReportScope<'_>,
    kind: OpeningKind,
) -> Result<Option<Vec<Element>>, ServiceError> {
    let opening_scope = OpeningScope {
        project: scope.project,
        building: scope.building,
        milestone: scope.milestone,
        filter: None,
        storeys: None,
    };
    Ok(openings::assemble_openings::<P>(state, kind, &opening_scope)?.map(|assembled| {
        let names = level_names_by_model(&assembled.levels_by_model);
        assembled
            .openings
            .into_iter()
            .map(|opening| {
                let rooms = opening
                    .owner_rooms_qualified
                    .iter()
                    .map(|owner| Attribution {
                        model_id: owner.model_id.clone(),
                        room_id: owner.room_id.clone(),
                        measures: BTreeMap::new(),
                    })
                    .collect();
                let level = names_of(&names, &opening.model_id);
                Element { read: with_level_name(presence_reader(opening), level), rooms }
            })
            .collect()
    }))
}

/// Ceilings and floors, whose attribution is geometric and carries the three
/// fractions plus the width the tolerance is expressed in. All four ride every
/// row, because which of them matters is the consumer's policy.
pub(super) fn read_surfaces<P: surfaces::SurfacePayloadKind>(
    state: &AppState,
    scope: &ReportScope<'_>,
) -> Result<Option<Vec<Element>>, ServiceError> {
    let surface_scope = SurfaceScope { project: scope.project, milestone: scope.milestone, storeys: None };
    Ok(surfaces::assemble_surfaces::<P>(state, &surface_scope)?.map(|assembled| {
        let names = level_names_by_model(&assembled.levels_by_model);
        assembled
            .surfaces
            .into_iter()
            .map(|surface| {
                let rooms = surface
                    .rooms
                    .iter()
                    .map(|room| Attribution {
                        model_id: room.model_id.clone(),
                        room_id: room.room_id.clone(),
                        measures: BTreeMap::from([
                            ("overlap_area", number(room.overlap_area)),
                            ("fraction_of_element", number(room.fraction_of_element)),
                            ("fraction_of_room", number(room.fraction_of_room)),
                            ("mean_width", number(room.mean_width)),
                        ]),
                    })
                    .collect();
                let level = names_of(&names, &surface.model_id);
                Element { read: with_level_name(presence_reader(surface), level), rooms }
            })
            .collect()
    }))
}

pub(super) fn read_items(state: &AppState, scope: &ReportScope<'_>) -> Result<Option<Vec<Element>>, ServiceError> {
    let item_scope = ItemScope {
        project: scope.project,
        building: scope.building,
        milestone: scope.milestone,
        filter: None,
        storeys: None,
        with_components: false,
    };
    Ok(items::assemble_items(state, &item_scope)?.map(|result| {
        let names = level_names_by_model(&result.levels_by_model);
        result
            .ffe
            .into_iter()
            .map(|item| {
                // Where the room came from — stated by the modeller, or filled
                // by geometry. A column, because the two are different claims.
                let origin = format!("{:?}", item.room_origin).to_lowercase();
                let rooms = item
                    .owner_rooms_qualified
                    .iter()
                    .map(|owner| Attribution {
                        model_id: owner.model_id.clone(),
                        room_id: owner.room_id.clone(),
                        measures: BTreeMap::from([("room_origin", origin.clone())]),
                    })
                    .collect();
                let level = names_of(&names, &item.model_id);
                Element { read: with_level_name(presence_reader(item), level), rooms }
            })
            .collect()
    }))
}

/// Spaces, matched to rooms by key.
///
/// **The match is `spaces::match_spaces`, never a second one here**, which is
/// what the by-room type waited for. Two things it reports rather than
/// resolves, both carried as measures on the row:
///
/// - **An ambiguous key attributes to the FIRST candidate and says so.**
///   Choosing between rooms would be a guess that looks like an answer;
///   dropping the match would hide a real pairing behind a key problem.
/// - **A space that matched nothing is unattributed**, which is the ordinary
///   state for a project whose rooms have not been pushed at all — a services
///   model audited against an empty room set. The report says which of "no
///   rooms to match" and "no key configured" it is (see `spaces_note`), because
///   an empty room column otherwise reads as a fault in the model.
pub(super) fn read_spaces(state: &AppState, scope: &ReportScope<'_>) -> Result<Option<Vec<Element>>, ServiceError> {
    let space_scope = SpaceScope {
        project: scope.project,
        milestone: scope.milestone,
        model: None,
        filter: None,
        storeys: None,
    };
    let Some(result) = spaces::assemble_spaces(state, &space_scope)? else {
        return Ok(None);
    };

    let matches = space_matches(state, scope)?;
    let names = level_names_by_model(&result.levels_by_model);
    Ok(Some(
        result
            .spaces
            .into_iter()
            .map(|space| {
                let id = (space.model_id.clone(), space.space.id.clone());
                let rooms = match matches.as_ref().and_then(|m| m.get(&id)) {
                    Some(spaces::SpaceMatch::Matched { key, room, ambiguous }) => vec![Attribution {
                        model_id: room.model_id.clone(),
                        room_id: room.room_id.clone(),
                        measures: BTreeMap::from([
                            ("match_key", key.clone()),
                            (
                                "ambiguous_key",
                                if *ambiguous {
                                    "yes, matched the first of several rooms".to_string()
                                } else {
                                    String::new()
                                },
                            ),
                        ]),
                    }],
                    // Unmatched, no key, or no match attempted: all three are
                    // reported states, and `spaces_note` says which.
                    _ => Vec::new(),
                };
                let level = names_of(&names, &space.model_id);
                Element { read: with_level_name(presence_reader(space), level), rooms }
            })
            .collect(),
    ))
}

/// The space-to-room pairing for this scope, or `None` when the project
/// configures no space key — in which case nothing was attempted, which is a
/// different answer from nothing matching.
pub(super) fn space_matches(
    state: &AppState,
    scope: &ReportScope<'_>,
) -> Result<Option<spaces::SpaceMatches>, ServiceError> {
    let Some(project) = scope.project else { return Ok(None) };
    let registry = state.settings();
    let Some(bundle) = registry.settings_for(project) else {
        return Ok(None);
    };

    let stored_rooms = state
        .all_opening_snapshots::<crate::contract::RoomPayload>(SnapshotKind::Rooms, Some(project))
        .map_err(ServiceError::Internal)?;
    let stored_spaces = state
        .all_opening_snapshots::<crate::contract::SpacePayload>(SnapshotKind::Spaces, Some(project))
        .map_err(ServiceError::Internal)?;

    Ok(spaces::match_spaces(
        &stored_rooms,
        &stored_spaces,
        &bundle.spaces,
        &bundle.builtin_properties,
    ))
}

/// Why a spaces-by-room report has unattributed rows, when it does.
///
/// **Three states an empty room column can mean**, and a reader cannot tell
/// them apart from the table: no key is configured so nothing was attempted,
/// the project has no rooms to match against at all, or the keys simply did not
/// line up. Only the last is a finding about the models.
pub(super) fn spaces_note(state: &AppState, scope: &ReportScope<'_>) -> Option<String> {
    let project = scope.project?;
    let registry = state.settings();
    let bundle = registry.settings_for(project)?;
    if bundle.spaces.comparison_key.is_none() {
        return Some(
            "No space key is configured for this project, so no space was matched to a room. \
             Set [spaces] comparison_key in settings."
                .to_string(),
        );
    }
    let has_rooms = state
        .model_index()
        .ok()?
        .iter()
        .any(|row| row.key.project_id == project && row.latest.contains_key(&SnapshotKind::Rooms));
    (!has_rooms).then(|| {
        "This project has no rooms pushed, so every space is unattributed. That is the room set being \
         empty, not the spaces being wrong."
            .to_string()
    })
}
