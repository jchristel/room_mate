//! `/doors` read assembly: merge every stored model's latest doors snapshot
//! into one flat payload, scoped by project, milestone and property filter.
//!
//! **Deliberately thinner than `service::rooms`, and the differences are the
//! point.** Doors reuse the filter grammar, the milestone pinning discipline and
//! the phase reporting verbatim; they have no reference-source join (that is R4,
//! which lands with doors' first reference source), no classification hierarchy,
//! and no level dedup — a door's `level_id` points into the level set its
//! model's *rooms* snapshot already carries, so there is nothing here to merge.
//!
//! `?building=` **is** supported, and was the last thing to arrive: a door's
//! building is its *owning* room's building, so the scope only became
//! answerable once `[doors] room_attribution` settled which room owns a door
//! (the attribution rule is in `CLAUDE.md`). Before that, any answer would have
//! settled that question by accident.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::contract::{Door, DoorPayload, PropertyPresence};
use crate::reference::{ReferenceData, ReferenceRecord};
use crate::settings::{BuiltinPropertyDef, ReferenceEntity};
use crate::state::{AppState, ModelKey};

use super::rooms::{FilterTarget, RoomFilter};
use super::ServiceError;

/// A door as sent to a consumer: the stored door plus the identity of the model
/// it came from.
///
/// The model ids are on the wire, unlike `RoomResponse`'s (which skips its
/// `source`), because a door's `from_room`/`to_room` are only meaningful
/// alongside them: room ids are unique within a model, so a consumer resolving a
/// door to its rooms needs to know which model's rooms to look in. Without it,
/// a merged multi-model response would be ambiguous in exactly the way the QA
/// report has to work around.
#[derive(Serialize)]
pub struct DoorResponse {
    #[serde(flatten)]
    pub door: Door,

    /// The project this door's model belongs to.
    pub project_id: String,
    /// The model this door came from — the scope its room references resolve in.
    pub model_id: String,

    /// The room(s) this door is attributed to under the project's
    /// `[doors] room_attribution` policy, in policy order.
    ///
    /// **A list, and empty means homeless.** A list because the `both` policy
    /// attributes a door between two rooms twice — which is the point of that
    /// policy for area rollups — and one shape that covers all five policies
    /// beats an `Option` plus a special case. Empty is a *reported state*, not a
    /// missing value: it means either the door names no room at all, or the
    /// policy declined to use the reference it has (`to_room` against a door
    /// that only opens *from* somewhere). QA distinguishes the two.
    ///
    /// Derived at read time from the stored references, never stored: changing
    /// the policy changes every answer immediately and rewrites nothing.
    pub owner_rooms: Vec<String>,

    /// Joined reference-source records, keyed by source name — the same shape
    /// `RoomResponse::reference` carries, and flattened the same way so
    /// `schedule.FireRating` reads identically on either entity.
    ///
    /// Only sources declaring `entity = "doors"` land here. A source with no
    /// match for this door contributes no entry: an unmatched key is a signal,
    /// not an error, exactly as for rooms.
    #[serde(flatten)]
    pub reference: BTreeMap<String, ReferenceRecord>,

    /// The owning model's `Model.source` (e.g. "revit"), for canonical property
    /// resolution. Not wire shape, same as `RoomResponse::source`.
    #[serde(skip)]
    pub source: String,
}

/// Doors resolve the entity's own two property tiers plus a small set of
/// intrinsics, and nothing else.
///
/// **Source-qualified fields resolve against this door's joined sources**, and
/// `Absent` when it joined none — which is exactly the answer a *room* gets for
/// a source it did not match. That equivalence was designed in before the join
/// existed: the previous implementation returned `Absent` for every qualified
/// field precisely so that the day R4 landed, the same predicate would start
/// matching rather than change status. It has, and it did.
impl FilterTarget for DoorResponse {
    fn presence(&self, source: Option<&str>, property: &str, builtin_defs: &[BuiltinPropertyDef]) -> PropertyPresence {
        /// A door's own struct fields always exist, so blank collapses to
        /// `Empty`, never `Absent` — the same rule `RoomResponse`'s intrinsics
        /// follow.
        fn intrinsic(value: Option<&str>) -> PropertyPresence {
            match value {
                None => PropertyPresence::Absent,
                Some("") => PropertyPresence::Empty,
                Some(v) => PropertyPresence::Present(v.to_string()),
            }
        }

        match source {
            // Same three-way answer `RoomResponse` gives, deliberately: source
            // not joined and field not in the row are both `Absent`, an empty
            // cell is `Empty`.
            Some(name) => match self.reference.get(name) {
                None => PropertyPresence::Absent,
                Some(record) => match record.fields.get(property) {
                    None => PropertyPresence::Absent,
                    Some(v) if v.is_empty() => PropertyPresence::Empty,
                    Some(v) => PropertyPresence::Present(v.clone()),
                },
            },
            None => match property {
                "$id" => intrinsic(Some(&self.door.id)),
                "$type_name" => intrinsic(Some(&self.door.type_name)),
                "$type_id" => intrinsic(Some(&self.door.type_id)),
                "$level_id" => intrinsic(Some(&self.door.level_id)),
                // The room references, as filterable fields. `$from_room=` with
                // no match is how a caller asks "which doors point at this
                // room"; an external door is `Absent` on its missing side, so it
                // fails every operator rather than matching a negative one.
                "$from_room" => intrinsic(self.door.from_room.as_deref()),
                "$to_room" => intrinsic(self.door.to_room.as_deref()),
                // Anything else walks the two property tiers, instance first —
                // the R2 rule, unchanged and not reimplemented here.
                canonical => crate::contract::property_presence(&self.door, canonical, &self.source, builtin_defs),
            },
        }
    }
}

/// Resolve one comparable/filterable field name against an assembled door, in
/// the same `source.property` vocabulary `/doors`' filter parses.
///
/// The door counterpart of `rooms::resolve_presence`, and it exists for the
/// identical reason: "what can I write before the dot" must have one answer
/// across filtering and comparison, or a name that filters correctly would
/// silently diff as nothing. It returns only the presence, not the resolved
/// namespace — a door has no joined sources to react to per source, which is
/// the one thing that genuinely differs from the rooms version.
pub fn resolve_presence(
    door: &DoorResponse,
    field: &str,
    known: &std::collections::BTreeSet<String>,
    builtin: &[BuiltinPropertyDef],
) -> PropertyPresence {
    match super::rooms::split_namespace(field, known) {
        super::rooms::NamespaceSplit::Joined { source, property } => door.presence(Some(&source), property, builtin),
        super::rooms::NamespaceSplit::Unqualified(name) => door.presence(None, name, builtin),
        // Rejected at settings load and filter parse, so unreachable through
        // either configured path — degrades to "nothing to compare" rather than
        // panicking, the same discipline `rooms::resolve_presence` follows.
        super::rooms::NamespaceSplit::UnknownSource(_) => PropertyPresence::Absent,
    }
}

/// Everything that narrows a doors read.
#[derive(Default)]
pub struct DoorScope<'a> {
    pub project: Option<&'a str>,
    /// Opaque building key, as for rooms — **a door's building is its owning
    /// room's building.** This became answerable only once
    /// `[doors] room_attribution` decided which room owns a door; before that,
    /// any answer would have settled the ownership question by accident.
    ///
    /// A **homeless door matches no building**, which is the honest reading and
    /// worth stating: it is not evidence the door belongs elsewhere, only that
    /// nothing attributes it, so it drops out of a building-scoped view exactly
    /// as a room with no classification does.
    pub building: Option<&'a str>,
    pub milestone: Option<&'a str>,
    pub filter: Option<&'a RoomFilter>,
}

/// The merged doors payload.
#[derive(Serialize)]
pub struct DoorsResult {
    pub schema_version: u32,
    /// Stable content revision over the contributing `(model, snapshot)` pairs,
    /// same role and same construction as `RoomsResult::revision`: a consumer
    /// compares this one field instead of re-hashing the payload.
    pub revision: String,
    pub doors: Vec<DoorResponse>,
    /// The Revit phase each contributing model's doors were filtered to, keyed
    /// by project id then model id — the same shape and the same reason as
    /// `RoomsResult::phase_by_model`. Read off each snapshot, never off the
    /// lineage's current phase.
    pub phase_by_model: BTreeMap<String, BTreeMap<String, Option<String>>>,
}

/// A stable content revision for a `DoorsResult`. Duplicated from
/// `rooms::scoped_revision` rather than shared: that one takes room-scoped
/// tuples, and the shared part is three lines of hashing whose meaning ("which
/// snapshot did each model contribute") is per entity.
fn doors_revision(scoped: &[(ModelKey, DoorPayload)]) -> String {
    use std::hash::{Hash, Hasher};

    let mut parts: Vec<(&str, &str, &str)> = scoped
        .iter()
        .map(|(key, payload)| (key.project_id.as_str(), key.model_id.as_str(), payload.snapshot.taken_at.as_str()))
        .collect();
    parts.sort_unstable();

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// `(model id, room id)` → that room's building key, for the rooms in scope.
///
/// **Built by calling `assemble_rooms`, not by re-deriving classification.** A
/// door's building has to mean exactly what a room's building means, or a
/// building-scoped doors read and a building-scoped rooms read would disagree
/// about the same building — so this asks the same function `/rooms` does, with
/// the same project and milestone scope and deliberately *no* building filter
/// (the filtering happens per door, against the door's owner).
///
/// Keyed on the pair because room ids are unique only within a model.
fn building_by_room(
    state: &AppState,
    scope: &DoorScope<'_>,
) -> Result<BTreeMap<(String, String), String>, ServiceError> {
    let rooms = super::rooms::assemble_rooms(
        state,
        &super::rooms::RoomScope { project: scope.project, milestone: scope.milestone, ..Default::default() },
    )?;
    let Some(rooms) = rooms else {
        return Ok(BTreeMap::new());
    };

    let registry = state.settings();
    let mut out = BTreeMap::new();
    for room in &rooms.rooms {
        let Some(tier) = registry
            .settings_for(&room.project_id)
            .and_then(|b| super::rooms::building_tier_index(&b.hierarchy))
        else {
            continue; // a project with no "Building" tier answers no building
        };
        if let Some(value) = room.classification.get(tier) {
            out.insert(
                (room.model_id.clone(), room.room.id.clone()),
                super::rooms::building_key(&value.code, &value.name),
            );
        }
    }
    Ok(out)
}

/// Merge every stored model's doors into one payload, scoped by `DoorScope`.
///
/// Returns `Ok(None)` when nothing has ever been pushed — the adapter's "204 No
/// Content" case, same contract as `assemble_rooms`. A filter or scope that
/// merely matches nothing is `Ok(Some)` with an empty list: the store has data,
/// the question just has an empty answer.
///
/// A project with no registered settings contributes nothing, matching
/// `assemble_rooms`' skip-on-read policy — a model with nothing to resolve
/// canonical property names against has no home in the response.
pub fn assemble_doors(state: &AppState, scope: &DoorScope<'_>) -> Result<Option<DoorsResult>, ServiceError> {
    let stored = state.all_door_snapshots().map_err(ServiceError::Internal)?;
    if stored.is_empty() {
        return Ok(None);
    }
    let registry = state.settings();

    // Phase 1 — scope to the request, substituting a milestone's pinned doors
    // snapshot for the model's latest where one is pinned. Same discipline as
    // `rooms::scope_payloads`: a project without the named milestone, or a model
    // that milestone does not pin, contributes nothing, and a pin whose snapshot
    // no longer exists is skipped with a warning rather than failing the read
    // ("signal, not error").
    let mut scoped: Vec<(ModelKey, DoorPayload)> = Vec::new();
    for (key, payload) in stored {
        if scope.project.is_some_and(|p| payload.project.id != p) {
            continue;
        }
        let Some(bundle) = registry.settings_for(&payload.project.id) else {
            continue;
        };
        match scope.milestone {
            None => scoped.push((key, payload)),
            Some(wanted) => {
                let Some(ms) = bundle.milestones.iter().find(|m| m.name == wanted) else {
                    continue;
                };
                let Some(pinned_id) = ms.door_attachments.get(&key.model_id) else {
                    continue;
                };
                match state.get_door_snapshot(&key, pinned_id).map_err(ServiceError::Internal)? {
                    Some(pinned) => scoped.push((key, pinned)),
                    None => tracing::warn!(
                        "milestone '{}' pins doors snapshot {:?} for {}/{}, but no such snapshot exists — skipping the model",
                        wanted,
                        pinned_id,
                        key.project_id,
                        key.model_id
                    ),
                }
            }
        }
    }

    let revision = doors_revision(&scoped);
    let mut phase_by_model: BTreeMap<String, BTreeMap<String, Option<String>>> = BTreeMap::new();
    let mut doors: Vec<DoorResponse> = Vec::new();

    // A door's building is its owning room's building, so a building scope needs
    // the rooms classified. Resolved **only when a building filter is actually
    // given** — it is a second storage read plus a classification pass, and the
    // overwhelmingly common doors read does not need it.
    let building_of_room = match scope.building {
        Some(_) => building_by_room(state, scope)?,
        None => BTreeMap::new(),
    };

    // Phase 2 — derive the response doors, applying the property filter *after*
    // assembly so a predicate sees the same resolved vocabulary a consumer does.
    for (key, payload) in &scoped {
        phase_by_model
            .entry(key.project_id.clone())
            .or_default()
            .insert(key.model_id.clone(), payload.phase.clone());

        let bundle = registry.settings_for(&payload.project.id);
        let builtin: &[BuiltinPropertyDef] = bundle.map(|b| b.builtin_properties.as_slice()).unwrap_or_default();
        let attribution = bundle.map(|b| b.doors.room_attribution).unwrap_or_default();

        // The doors half of R4: this project's sources declaring
        // `entity = "doors"`, resolved once per model rather than per door.
        // Rooms scope theirs the same way in `assemble_scoped_rooms`.
        let door_sources: BTreeMap<&str, &ReferenceData> = bundle
            .map(|b| {
                b.reference
                    .iter()
                    .filter(|(_, cfg)| cfg.entity == ReferenceEntity::Doors)
                    .filter_map(|(name, cfg)| Some((name.as_str(), cfg.data.as_ref()?)))
                    .collect()
            })
            .unwrap_or_default();

        for door in &payload.doors {
            // One join per configured source: read its link property off the
            // DOOR -- instance tier then type tier, the R2 rule -- and look up
            // the record. `lookup_property` is the same function rooms use; a
            // door is simply another `PropertyTiers`.
            let reference: BTreeMap<String, ReferenceRecord> = door_sources
                .iter()
                .filter_map(|(name, data)| {
                    let record =
                        crate::contract::lookup_property(door, &data.link_property, &payload.model.source, builtin)
                            .and_then(|key| data.by_id.get(&key).cloned())?;
                    Some(((*name).to_string(), record))
                })
                .collect();

            let response = DoorResponse {
                owner_rooms: attribution
                    .owners(door.from_room.as_deref(), door.to_room.as_deref())
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                door: door.clone(),
                project_id: payload.project.id.clone(),
                model_id: payload.model.id.clone(),
                reference,
                source: payload.model.source.clone(),
            };
            // A homeless door matches no building — see `DoorScope::building`.
            // Room ids are unique only within a model, so the lookup is keyed on
            // the pair, never the bare room id.
            if let Some(wanted) = scope.building
                && !response.owner_rooms.iter().any(|room| {
                    building_of_room.get(&(key.model_id.clone(), room.clone())).is_some_and(|b| b == wanted)
                })
            {
                continue;
            }
            if scope.filter.is_none_or(|f| f.matches(&response, builtin)) {
                doors.push(response);
            }
        }
    }

    Ok(Some(DoorsResult {
        schema_version: crate::contract::SUPPORTED_DOOR_SCHEMA,
        revision,
        doors,
        phase_by_model,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{CustomValue, Model, Project, Snapshot, SUPPORTED_DOOR_SCHEMA};
    use crate::state::ProjectSettings;
    use crate::storage::MemStore;
    use std::collections::{BTreeSet, HashMap};

    fn make_door(id: &str, from_room: Option<&str>, to_room: Option<&str>, props: &[(&str, &str)]) -> Door {
        let mut properties = BTreeMap::new();
        for (k, v) in props {
            properties.insert(k.to_string(), CustomValue { value: v.to_string(), storage_type: None });
        }
        Door {
            id: id.to_string(),
            level_id: "1".to_string(),
            loops: vec![],
            from_room: from_room.map(str::to_string),
            to_room: to_room.map(str::to_string),
            // These helpers exercise references and properties, never placement,
            // so `None` here is the honest input rather than a stub: a door with
            // no position and no direction is a state the contract carries.
            insertion_point: None,
            through_wall_normal: None,
            type_id: "t1".to_string(),
            type_name: "Single".to_string(),
            properties,
            type_properties: BTreeMap::new(),
        }
    }

    fn bundle() -> ProjectSettings {
        ProjectSettings {
            reference: BTreeMap::new(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec![],
            milestones: vec![],
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            hierarchy_exclusions: vec![],
        }
    }

    /// **R4 end to end: a source declaring `entity = "doors"` joins onto doors.**
    ///
    /// Before this, `[sources.reference.<name>]` meant "for rooms" with nothing
    /// saying so, and a source configured for anything else parsed, loaded and
    /// joined nowhere. The filter grammar was already written for this day --
    /// `DoorResponse::presence` answered `Absent` for every source-qualified
    /// field specifically so the same predicate would start *matching* rather
    /// than change status once the join existed.
    #[tokio::test]
    async fn test_a_doors_scoped_reference_source_joins_onto_doors() {
        // Row 1 is labels, row 2 col 0 names the door property holding the key.
        let csv = b"DoorRef,FireRating
Door Mark,FireRating
D-101,60
";
        let data = crate::reference::load_reference_from_bytes(csv).expect("csv loads");

        let mut settings = bundle();
        settings.reference.insert(
            "schedule".to_string(),
            crate::state::ProjectReferenceSource { entity: ReferenceEntity::Doors, data: Some(data), fields: vec![] },
        );

        let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), settings)]), None);
        state
            .set_door_snapshot(DoorPayload {
                schema_version: SUPPORTED_DOOR_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
                phase: Some("New Construction".to_string()),
                model_to_shared: None,
                doors: vec![
                    make_door("d1", None, Some("r1"), &[("Door Mark", "D-101")]),
                    // No matching key: an unmatched row is a signal, not an
                    // error, so this door simply joins nothing.
                    make_door("d2", None, Some("r1"), &[("Door Mark", "D-999")]),
                ],
            })
            .unwrap();

        let result = assemble_doors(&state, &DoorScope::default()).unwrap().expect("data");
        let joined = result.doors.iter().find(|d| d.door.id == "d1").expect("d1");
        assert_eq!(
            joined.reference.get("schedule").and_then(|r| r.fields.get("FireRating")),
            Some(&"60".to_string()),
            "the door schedule joined onto the door"
        );

        let unmatched = result.doors.iter().find(|d| d.door.id == "d2").expect("d2");
        assert!(unmatched.reference.is_empty(), "an unmatched key joins nothing, and is not an error");

        // And the filter grammar reaches it -- the same `source.property` shape
        // a room filter writes, which is the whole reason the namespace stayed
        // flat rather than nesting the entity.
        let builtin: &[BuiltinPropertyDef] = &[];
        assert_eq!(
            joined.presence(Some("schedule"), "FireRating", builtin),
            PropertyPresence::Present("60".to_string())
        );
        assert_eq!(
            unmatched.presence(Some("schedule"), "FireRating", builtin),
            PropertyPresence::Absent,
            "a door that joined nothing answers Absent, exactly as a room does"
        );
    }

    /// The other half of the scoping: a **rooms** source must not join onto
    /// doors. Without the entity check a door whose link property happened to
    /// collide would pick up a room's columns.
    #[tokio::test]
    async fn test_a_rooms_scoped_reference_source_does_not_join_onto_doors() {
        let csv = b"DoorRef,FireRating
Door Mark,FireRating
D-101,60
";
        let data = crate::reference::load_reference_from_bytes(csv).expect("csv loads");

        let mut settings = bundle();
        settings.reference.insert(
            "schedule".to_string(),
            crate::state::ProjectReferenceSource { entity: ReferenceEntity::Rooms, data: Some(data), fields: vec![] },
        );

        let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), settings)]), None);
        state
            .set_door_snapshot(DoorPayload {
                schema_version: SUPPORTED_DOOR_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
                phase: Some("New Construction".to_string()),
                model_to_shared: None,
                doors: vec![make_door("d1", None, Some("r1"), &[("Door Mark", "D-101")])],
            })
            .unwrap();

        let result = assemble_doors(&state, &DoorScope::default()).unwrap().expect("data");
        assert!(
            result.doors[0].reference.is_empty(),
            "a rooms-scoped source is not this entity's source, even when the key matches"
        );
    }

    fn state_with(doors: Vec<(&str, &str, Vec<Door>)>) -> AppState {
        let mut projects = HashMap::new();
        for (project, _, _) in &doors {
            projects.insert(project.to_string(), bundle());
        }
        let state = AppState::new(Box::new(MemStore::new()), projects, None);
        for (project, model, list) in doors {
            state
                .set_door_snapshot(DoorPayload {
                    schema_version: SUPPORTED_DOOR_SCHEMA,
                    project: Project { id: project.to_string(), name: "P".to_string() },
                    model: Model { id: model.to_string(), name: "M".to_string(), source: "revit".to_string() },
                    snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
                    phase: Some("New Construction".to_string()),
                    model_to_shared: None,
                    doors: list,
                })
                .unwrap();
        }
        state
    }

    fn filter(expr: &str) -> RoomFilter {
        RoomFilter::parse_query(expr, &BTreeSet::new()).expect("parses")
    }

    /// Nothing pushed at all is `None` — the adapter's 204, distinct from a
    /// scope that matched nothing.
    #[test]
    fn test_no_doors_pushed_is_none() {
        let state = AppState::new(Box::new(MemStore::new()), HashMap::new(), None);
        assert!(assemble_doors(&state, &DoorScope::default()).unwrap().is_none());
    }

    /// A scope that matches nothing is an empty list, not `None`: the store has
    /// data, the question just has an empty answer.
    #[test]
    fn test_a_scope_matching_nothing_is_an_empty_list() {
        let state = state_with(vec![("p1", "m1", vec![make_door("d1", Some("r1"), None, &[])])]);
        let result = assemble_doors(&state, &DoorScope { project: Some("nope"), ..Default::default() })
            .unwrap()
            .expect("the store has data");
        assert!(result.doors.is_empty());
    }

    /// Doors from every model merge, each carrying the model it came from —
    /// which is what makes its room references resolvable.
    #[test]
    fn test_doors_merge_carrying_their_model_identity() {
        let state = state_with(vec![
            ("p1", "m1", vec![make_door("d1", Some("r1"), None, &[])]),
            ("p1", "m2", vec![make_door("d1", Some("r1"), None, &[])]),
        ]);
        let result = assemble_doors(&state, &DoorScope::default()).unwrap().expect("data");
        assert_eq!(result.doors.len(), 2);
        let models: BTreeSet<&str> = result.doors.iter().map(|d| d.model_id.as_str()).collect();
        assert_eq!(
            models,
            BTreeSet::from(["m1", "m2"]),
            "the same door id in two models stays distinguishable"
        );
        assert_eq!(result.phase_by_model["p1"]["m1"].as_deref(), Some("New Construction"));
    }

    /// The property filter reaches a door's own properties, using the same
    /// grammar `/rooms` uses.
    #[test]
    fn test_filter_matches_a_door_property() {
        let state = state_with(vec![(
            "p1",
            "m1",
            vec![
                make_door("d1", Some("r1"), None, &[("Mark", "29")]),
                make_door("d2", Some("r2"), None, &[("Mark", "33")]),
            ],
        )]);
        let f = filter("Mark=29");
        let result = assemble_doors(&state, &DoorScope { filter: Some(&f), ..Default::default() })
            .unwrap()
            .expect("data");
        assert_eq!(result.doors.len(), 1);
        assert_eq!(result.doors[0].door.id, "d1");
    }

    /// The room references are filterable intrinsics — `$to_room=` is how a
    /// caller asks "which doors open into this room", which is the read the
    /// whole door→room link exists for.
    #[test]
    fn test_filter_matches_a_room_reference_intrinsic() {
        let state = state_with(vec![(
            "p1",
            "m1",
            vec![
                make_door("d1", Some("r1"), Some("r2"), &[]),
                make_door("d2", Some("r3"), None, &[]),
            ],
        )]);
        let f = filter("$to_room=r2");
        let result = assemble_doors(&state, &DoorScope { filter: Some(&f), ..Default::default() })
            .unwrap()
            .expect("data");
        assert_eq!(result.doors.len(), 1);
        assert_eq!(result.doors[0].door.id, "d1");

        // An external door is `Absent` on its missing side, so it fails every
        // operator — including the negative one. "This door has no to_room" is
        // not evidence that its to_room differs from r2.
        let f = filter("$to_room!=r2");
        let result = assemble_doors(&state, &DoorScope { filter: Some(&f), ..Default::default() })
            .unwrap()
            .expect("data");
        assert!(
            result.doors.is_empty(),
            "d2 has no to_room, so it does not match a negative operator either"
        );
    }

    /// The two property tiers are both reachable through the filter, instance
    /// first — the R2 rule applied through the read path rather than restated.
    #[test]
    fn test_filter_reaches_the_type_tier() {
        let mut door = make_door("d1", Some("r1"), None, &[("Door Leaf Thickness", "")]);
        door.type_properties.insert(
            "Door Leaf Thickness".to_string(),
            CustomValue { value: "40.0".to_string(), storage_type: None },
        );
        let state = state_with(vec![("p1", "m1", vec![door])]);

        let f = filter("Door Leaf Thickness=40.0");
        let result = assemble_doors(&state, &DoorScope { filter: Some(&f), ..Default::default() })
            .unwrap()
            .expect("data");
        assert_eq!(result.doors.len(), 1, "a blank instance value does not shadow the type's");
    }

    /// A source-qualified predicate resolves `Absent` rather than erroring:
    /// doors carry no reference joins until R4, and `Absent` is the same answer
    /// a room gets for a source it did not join. The day R4 lands, this
    /// predicate starts matching instead of changing status.
    #[test]
    fn test_a_source_qualified_filter_matches_no_door() {
        let state = state_with(vec![("p1", "m1", vec![make_door("d1", Some("r1"), None, &[])])]);
        let known = BTreeSet::from(["drofus".to_string()]);
        let f = RoomFilter::parse_query("drofus.NetArea=25.5", &known).expect("parses against a known source");

        let result = assemble_doors(&state, &DoorScope { filter: Some(&f), ..Default::default() })
            .unwrap()
            .expect("data");
        assert!(result.doors.is_empty());
    }

    /// An unregistered project's doors are skipped on read, matching
    /// `assemble_rooms` — there is nothing to resolve canonical names against.
    #[test]
    fn test_an_unregistered_projects_doors_are_skipped() {
        let state = AppState::new(Box::new(MemStore::new()), HashMap::new(), None);
        state
            .set_door_snapshot(DoorPayload {
                schema_version: SUPPORTED_DOOR_SCHEMA,
                project: Project { id: "ghost".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
                phase: Some("New Construction".to_string()),
                model_to_shared: None,
                doors: vec![make_door("d1", Some("r1"), None, &[])],
            })
            .unwrap();

        let result = assemble_doors(&state, &DoorScope::default()).unwrap().expect("the store has data");
        assert!(result.doors.is_empty());
    }

    /// The revision is stable between two idle reads and moves when the
    /// contributing snapshot changes — the "has anything changed?" signal, same
    /// contract as `/rooms`.
    #[test]
    fn test_revision_is_stable_and_moves_on_a_push() {
        let state = state_with(vec![("p1", "m1", vec![make_door("d1", Some("r1"), None, &[])])]);
        let first = assemble_doors(&state, &DoorScope::default()).unwrap().unwrap().revision;
        let again = assemble_doors(&state, &DoorScope::default()).unwrap().unwrap().revision;
        assert_eq!(first, again, "two idle reads agree");

        state
            .set_door_snapshot(DoorPayload {
                schema_version: SUPPORTED_DOOR_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-03-01T00:00:00Z".to_string() },
                phase: Some("New Construction".to_string()),
                model_to_shared: None,
                doors: vec![make_door("d1", Some("r1"), None, &[])],
            })
            .unwrap();
        let after = assemble_doors(&state, &DoorScope::default()).unwrap().unwrap().revision;
        assert_ne!(first, after, "a new snapshot moves it");
    }

    /// `owner_rooms` on the read follows the project's policy, and an empty
    /// list is the homeless signal rather than a missing field.
    #[test]
    fn test_owner_rooms_follows_the_configured_policy() {
        let doors = vec![
            make_door("both", Some("r1"), Some("r2"), &[]),
            make_door("from-only", Some("r3"), None, &[]),
            make_door("homeless", None, None, &[]),
        ];

        let owners_for = |policy: crate::settings::RoomAttribution| {
            let mut settings = bundle();
            settings.doors.room_attribution = policy;
            let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), settings)]), None);
            state
                .set_door_snapshot(DoorPayload {
                    schema_version: SUPPORTED_DOOR_SCHEMA,
                    project: Project { id: "p1".to_string(), name: "P".to_string() },
                    model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                    snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
                    phase: Some("New Construction".to_string()),
                    model_to_shared: None,
                    doors: doors.clone(),
                })
                .unwrap();
            let r = assemble_doors(&state, &DoorScope::default()).unwrap().unwrap();
            r.doors.into_iter().map(|d| (d.door.id, d.owner_rooms)).collect::<BTreeMap<_, _>>()
        };

        // The decided default: opens-into, else opens-from, else homeless.
        let chain = owners_for(crate::settings::RoomAttribution::ToRoomThenFromRoom);
        assert_eq!(chain["both"], vec!["r2".to_string()]);
        assert_eq!(chain["from-only"], vec!["r3".to_string()]);
        assert!(chain["homeless"].is_empty());

        // A narrower policy leaves the from-only door homeless.
        let strict = owners_for(crate::settings::RoomAttribution::ToRoom);
        assert_eq!(strict["both"], vec!["r2".to_string()]);
        assert!(strict["from-only"].is_empty());

        // `both` is why this is a list: one door, two owners, to_room first.
        let both = owners_for(crate::settings::RoomAttribution::Both);
        assert_eq!(both["both"], vec!["r2".to_string(), "r1".to_string()]);
        assert_eq!(both["from-only"], vec!["r3".to_string()], "one-sided doors still yield one owner");
    }

    /// **`?building=` scopes doors by their OWNING room's building** — the
    /// capability the attribution decision unlocked.
    ///
    /// Also pins the two rules that make it correct: a homeless door matches no
    /// building (it is not evidence of belonging elsewhere), and the lookup is
    /// keyed on `(model, room)` so a same-numbered room in another model cannot
    /// resolve it — the collision this codebase guards everywhere doors touch
    /// room ids.
    #[test]
    fn test_building_scope_follows_the_owning_room() {
        let mut settings = bundle();
        settings.hierarchy = vec![crate::settings::HierarchyTier {
            name: "Building".to_string(),
            code_property: None,
            name_property: Some("Building".to_string()),
        }];
        let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), settings)]), None);

        // Two models, each with a room id "r1" — in DIFFERENT buildings.
        let room = |id: &str, building: &str| crate::contract::Room {
            id: id.to_string(),
            name: id.to_string(),
            level_id: "1".to_string(),
            loops: vec![],
            properties: BTreeMap::from([(
                "Building".to_string(),
                CustomValue { value: building.to_string(), storage_type: None },
            )]),
        };
        for (model, building) in [("m1", "North"), ("m2", "South")] {
            state
                .set_snapshot(crate::contract::RoomPayload {
                    schema_version: crate::contract::SUPPORTED_SCHEMA,
                    project: Project { id: "p1".to_string(), name: "P".to_string() },
                    model: Model { id: model.to_string(), name: "M".to_string(), source: "revit".to_string() },
                    snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
                    phase: Some("New Construction".to_string()),
                    model_to_shared: None,
                    room_boundary: None,
                    levels: vec![],
                    rooms: vec![room("r1", building)],
                })
                .unwrap();
        }
        // m1's door owns m1's r1 (North); m2's door owns m2's r1 (South).
        for model in ["m1", "m2"] {
            state
                .set_door_snapshot(DoorPayload {
                    schema_version: SUPPORTED_DOOR_SCHEMA,
                    project: Project { id: "p1".to_string(), name: "P".to_string() },
                    model: Model { id: model.to_string(), name: "M".to_string(), source: "revit".to_string() },
                    snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
                    phase: Some("New Construction".to_string()),
                    model_to_shared: None,
                    doors: vec![
                        make_door(&format!("{model}-owned"), None, Some("r1"), &[]),
                        make_door(&format!("{model}-homeless"), None, None, &[]),
                    ],
                })
                .unwrap();
        }

        let in_building = |key: &str| {
            let r = assemble_doors(&state, &DoorScope { building: Some(key), ..Default::default() })
                .unwrap()
                .unwrap();
            r.doors.into_iter().map(|d| d.door.id).collect::<BTreeSet<_>>()
        };

        assert_eq!(
            in_building("|North"),
            BTreeSet::from(["m1-owned".to_string()]),
            "m2's door owns m2's r1, which is South — a same-numbered room in another model must not resolve it"
        );
        assert_eq!(in_building("|South"), BTreeSet::from(["m2-owned".to_string()]));
        assert!(in_building("|Nowhere").is_empty());

        // Both homeless doors are absent from every building, and present when
        // no building is asked for — the filter excludes them, nothing else does.
        let all = assemble_doors(&state, &DoorScope::default()).unwrap().unwrap();
        assert_eq!(all.doors.len(), 4, "unscoped, homeless doors are served normally");
    }

    /// A milestone serves the doors snapshot it pins, not the model's latest.
    #[test]
    fn test_milestone_serves_the_pinned_doors_snapshot() {
        let mut settings = bundle();
        settings.milestones = vec![crate::settings::Milestone {
            name: "Stage 2".to_string(),
            date: "2026-02-01".to_string(),
            reference_snapshots: BTreeMap::new(),
            attachments: BTreeMap::new(),
            door_attachments: BTreeMap::from([("m1".to_string(), "2026-02-01T00:00:00Z".to_string())]),
        }];
        let state = AppState::new(
            Box::new(
                crate::storage::FsStore::new(
                    std::env::temp_dir().join(format!("roommate-doors-milestone-{}", std::process::id())),
                )
                .unwrap(),
            ),
            HashMap::from([("p1".to_string(), settings)]),
            None,
        );
        let push = |ts: &str, id: &str| {
            state
                .set_door_snapshot(DoorPayload {
                    schema_version: SUPPORTED_DOOR_SCHEMA,
                    project: Project { id: "p1".to_string(), name: "P".to_string() },
                    model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                    snapshot: Snapshot { taken_at: ts.to_string() },
                    phase: Some("New Construction".to_string()),
                    model_to_shared: None,
                    doors: vec![make_door(id, Some("r1"), None, &[])],
                })
                .unwrap()
        };
        push("2026-02-01T00:00:00Z", "pinned");
        push("2026-06-01T00:00:00Z", "latest");

        let latest = assemble_doors(&state, &DoorScope::default()).unwrap().unwrap();
        assert_eq!(latest.doors[0].door.id, "latest");

        let pinned = assemble_doors(&state, &DoorScope { milestone: Some("Stage 2"), ..Default::default() })
            .unwrap()
            .unwrap();
        assert_eq!(pinned.doors[0].door.id, "pinned");

        // A model the milestone does not pin contributes nothing, matching the
        // rooms discipline rather than silently falling back to latest.
        let unknown = assemble_doors(&state, &DoorScope { milestone: Some("nope"), ..Default::default() })
            .unwrap()
            .unwrap();
        assert!(unknown.doors.is_empty());
    }
}
