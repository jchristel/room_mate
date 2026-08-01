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
//! No `?building=` either. A door's building would have to come from the rooms
//! it connects, and *which* room owns a door is
//! [Entities](../../docs/STRATEGY-ENTITIES.md) Decision 6's open question. A
//! scope that silently picked one answer would settle that question by accident;
//! leaving it out keeps it open.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::contract::{Door, DoorPayload, PropertyPresence};
use crate::settings::BuiltinPropertyDef;
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

    /// The owning model's `Model.source` (e.g. "revit"), for canonical property
    /// resolution. Not wire shape, same as `RoomResponse::source`.
    #[serde(skip)]
    pub source: String,
}

/// Doors resolve the entity's own two property tiers plus a small set of
/// intrinsics, and nothing else.
///
/// **A source-qualified field always resolves `Absent`.** Doors carry no joined
/// reference sources yet (R4), so `hardware.FireRating` on a door is not an
/// error — it is a field this door does not have, which is exactly the answer a
/// *room* gets for a source it did not join. Keeping it `Absent` rather than an
/// error means the filter grammar means one thing across both entities, and the
/// day R4 lands the same predicate starts matching instead of changing status.
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
            Some(_) => PropertyPresence::Absent,
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

/// Everything that narrows a doors read. Mirrors `RoomScope` minus `building` —
/// see the module doc for why that one is absent rather than unimplemented.
#[derive(Default)]
pub struct DoorScope<'a> {
    pub project: Option<&'a str>,
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

    // Phase 2 — derive the response doors, applying the property filter *after*
    // assembly so a predicate sees the same resolved vocabulary a consumer does.
    for (key, payload) in &scoped {
        phase_by_model
            .entry(key.project_id.clone())
            .or_default()
            .insert(key.model_id.clone(), payload.phase.clone());

        let builtin: &[BuiltinPropertyDef] = registry
            .settings_for(&payload.project.id)
            .map(|b| b.builtin_properties.as_slice())
            .unwrap_or_default();

        for door in &payload.doors {
            let response = DoorResponse {
                door: door.clone(),
                project_id: payload.project.id.clone(),
                model_id: payload.model.id.clone(),
                source: payload.model.source.clone(),
            };
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
            hierarchy_exclusions: vec![],
        }
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
