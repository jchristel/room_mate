//! The scoping, pinning and geometry every entity's read shares.
//!
//! **Extracted when FF&E arrived, and the trigger is worth stating because it
//! is not the obvious one.** Windows cost none of this: they reuse `Opening`,
//! so they reuse the whole opening assembly by construction. An `Item` is a
//! different record, so the choice was between a second copy of this pipeline
//! and naming what the pipeline actually touches. It touches
//! [`SnapshotEnvelope`](crate::contract::SnapshotEnvelope) -- six field reads --
//! and never the element list, which is why everything here is generic over
//! that trait and nothing here knows what an opening or an item is.
//!
//! What stayed behind in [`openings`](super::openings) is exactly the part that
//! could not come: how many room references an element has, and how geometry
//! resolves them. That is the split
//! [`room_locator`](super::room_locator)'s own header predicted -- "the next
//! category needs the glue, not the geometry" -- reached from the other
//! direction.
//!
//! The two locator entry points are the shape of that split. An opening sits
//! *in* a wall, so its point must be stepped off the wall before it is tested
//! (`locate_sides`); an item sits *in* a room, so its point is tested where it
//! is (`locate_within`). Same candidates, same elevation axis, same probe --
//! different question.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::contract::{Loop, ModelToShared, Point2D, RoomPayload, SnapshotEnvelope};
use crate::settings::{Milestone, RoomResolution};
use crate::state::{AppState, ModelKey, ProjectSettings, SettingsRegistry};
use crate::storage::{ModelIndexRow, SnapshotKind};

use super::room_locator::{self, RoomRef, Unresolved};
use super::ServiceError;

mod candidates;
mod storey;
pub use candidates::*;
pub use storey::*;

/// A stable content revision for a `DoorsResult`. Duplicated from
/// `rooms::scoped_revision` rather than shared: that one takes room-scoped
/// tuples, and the shared part is three lines of hashing whose meaning ("which
/// snapshot did each model contribute") is per entity.
pub fn revision<P: SnapshotEnvelope>(scoped: &[(ModelKey, P)]) -> String {
    use std::hash::{Hash, Hasher};

    let mut parts: Vec<(&str, &str, &str)> = scoped
        .iter()
        .map(|(key, payload)| (key.project_id.as_str(), key.model_id.as_str(), payload.taken_at()))
        .collect();
    parts.sort_unstable();

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// This kind's snapshot pins on one milestone: model id → snapshot id.
///
/// **The one place a kind is paired with its pin map**, and an exhaustive match
/// so a new kind cannot compile without choosing one. `Milestone` carries a map
/// per entity because the entities are pushed independently and their snapshot
/// ids do not correspond.
///
/// The pairing used to be written three ways: a closure at every
/// `scope_snapshots` call, a `pins` method on each entity family, and -- in
/// `scope_cursor`, which is handed only a kind -- `attachments`, the *rooms*
/// map, for every kind. So the ETag of `/doors?milestone=` hashed a rooms
/// snapshot id the doors read never served. Two copies of one rule, and the copy
/// without the closure guessed.
pub fn milestone_pins(milestone: &Milestone, kind: SnapshotKind) -> &BTreeMap<String, String> {
    match kind {
        SnapshotKind::Rooms => &milestone.attachments,
        SnapshotKind::Doors => &milestone.door_attachments,
        SnapshotKind::Windows => &milestone.window_attachments,
        SnapshotKind::Ffe => &milestone.ffe_attachments,
        SnapshotKind::Spaces => &milestone.space_attachments,
        SnapshotKind::Ceilings => &milestone.ceiling_attachments,
        SnapshotKind::Floors => &milestone.floor_attachments,
    }
}

/// One model's contribution to a scoped read, decided before any snapshot is
/// opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRead {
    pub key: ModelKey,
    /// The snapshot id (`taken_at`) to serve: the model's latest, or the id a
    /// milestone pins.
    pub id: String,
}

/// Which snapshot of `kind` each in-scope model contributes, decided from the
/// store index and the settings alone.
///
/// **Parsing is what a read costs, so everything that can be decided without it
/// is decided here, first.** The loop this replaced parsed every model's latest
/// snapshot and only then asked whether the request wanted it -- so under a
/// milestone each one was parsed, discarded, and its pinned snapshot parsed in
/// its place: two parses for one answer, the larger of them thrown away.
///
/// **Shared with `scope_cursor`, and that is the correctness half.** The cursor
/// is the ETag. A cursor and a read that each decide "which snapshot" can
/// disagree, and the unsafe direction of that disagreement is a 304 for a body
/// that changed -- which is exactly how the cursor came to hash rooms pins for
/// every kind (see `milestone_pins`). One function cannot disagree with itself.
///
/// The rules are the ones the two loops already applied: an unregistered
/// project contributes nothing; without a milestone a model contributes its
/// latest; with one, a project lacking that milestone contributes nothing and a
/// model it does not pin contributes nothing. A model holding no snapshot of
/// this kind contributes nothing even when pinned -- nothing was ever pushed for
/// the pin to name.
///
/// **Whether a pinned snapshot still exists is not decided here.** The index
/// knows each model's latest and nothing older, so a dangling pin is planned and
/// the read drops it with a warning. For the cursor that errs the safe way: it
/// counts a model the body will not contain, which can only over-report change.
///
/// Takes the index and registry rather than the state so a caller holding one
/// settings snapshot for the whole request plans against *that* snapshot.
pub fn plan_reads(
    index: &[ModelIndexRow],
    registry: &SettingsRegistry,
    kind: SnapshotKind,
    project: Option<&str>,
    milestone: Option<&str>,
) -> Vec<PlannedRead> {
    let mut plan = Vec::new();
    for row in index {
        if project.is_some_and(|p| row.key.project_id != p) {
            continue;
        }
        let Some(bundle) = registry.settings_for(&row.key.project_id) else {
            continue;
        };
        let Some(latest) = row.latest.get(&kind) else {
            continue;
        };
        let id = match milestone {
            None => latest,
            Some(wanted) => {
                let Some(ms) = bundle.milestones.iter().find(|m| m.name == wanted) else {
                    continue;
                };
                let Some(pinned) = milestone_pins(ms, kind).get(&row.key.model_id) else {
                    continue;
                };
                pinned
            }
        };
        plan.push(PlannedRead { key: row.key.clone(), id: id.clone() });
    }
    plan
}

/// Scope one entity's stored snapshots to the request, substituting a
/// milestone's pinned snapshot for a model's latest where one is pinned.
///
/// **One implementation for every entity**: plan from the index
/// (`plan_reads`), then open exactly the snapshots the plan names. A pin whose
/// snapshot no longer exists is skipped with a warning rather than failing the
/// read -- "signal, not error".
pub fn scope_snapshots<P: SnapshotEnvelope + serde::de::DeserializeOwned>(
    state: &AppState,
    kind: SnapshotKind,
    project: Option<&str>,
    milestone: Option<&str>,
) -> Result<Vec<(ModelKey, P)>, ServiceError> {
    let index = state.model_index().map_err(ServiceError::Internal)?;
    let plan = plan_reads(&index, &state.settings(), kind, project, milestone);
    read_planned(state, kind, plan, milestone)
}

/// Open and parse the snapshots a plan names, in plan order.
///
/// Split from `scope_snapshots` for the rooms read, which plans against the
/// settings snapshot it holds for the whole request rather than taking a fresh
/// one here.
pub(crate) fn read_planned<P: serde::de::DeserializeOwned>(
    state: &AppState,
    kind: SnapshotKind,
    plan: Vec<PlannedRead>,
    milestone: Option<&str>,
) -> Result<Vec<(ModelKey, P)>, ServiceError> {
    let mut scoped = Vec::with_capacity(plan.len());
    for PlannedRead { key, id } in plan {
        match state.get_opening_snapshot::<P>(kind, &key, &id).map_err(ServiceError::Internal)? {
            Some(payload) => scoped.push((key, payload)),
            None => match milestone {
                Some(wanted) => tracing::warn!(
                    "milestone '{}' pins {} snapshot {:?} for {}/{}, but no such snapshot exists -- skipping the model",
                    wanted,
                    kind.label(),
                    id,
                    key.project_id,
                    key.model_id
                ),
                // The index named it a moment ago, so the file went between the
                // two reads -- a hand deletion, since nothing in the server
                // deletes a snapshot.
                None => tracing::warn!(
                    "{} snapshot {:?} for {}/{} is indexed as the latest but could not be read -- skipping the model",
                    kind.label(),
                    id,
                    key.project_id,
                    key.model_id
                ),
            },
        }
    }
    Ok(scoped)
}

/// The phase each contributing model was filtered to, keyed by project then
/// model.
///
/// Read off each **snapshot**, never off the lineage's current phase: a snapshot
/// written before phasing existed reports itself unphased forever, and that
/// stays true after a later push phases the lineage (PLAN-phasing D8).
pub fn phase_by_model<P: SnapshotEnvelope>(
    scoped: &[(ModelKey, P)],
) -> BTreeMap<String, BTreeMap<String, Option<String>>> {
    let mut out: BTreeMap<String, BTreeMap<String, Option<String>>> = BTreeMap::new();
    for (key, payload) in scoped {
        out.entry(key.project_id.clone())
            .or_default()
            .insert(key.model_id.clone(), payload.phase().map(str::to_string));
    }
    out
}

/// The levels each contributing model declares, keyed by model id.
///
/// **Why every element entity has to send this.** A `Level.id` is unique only
/// within its own document, and the level picker is built from `/rooms`, which
/// deduped its ids across the project's linked models (`rooms::dedup_levels`).
/// An element's `level_id` is neither: it is the raw id its own model used. So
/// matching an element to the displayed storey by id is wrong twice over -- a
/// model that lost the dedup race has every element silently dropped, and a
/// model with no rooms at all (a facade package) contributes no canonical id
/// for its elements to match in the first place. A facade model is exactly
/// that case: its external doors were invisible on every level.
///
/// **Elevation is what crosses documents**, so the consumer resolves
/// `(model_id, level_id) -> elevation` through this map and matches on the
/// number. `service::spaces` reached the same conclusion first and this is that
/// map, hoisted so all four element entities share one answer.
///
/// **Empty for a model that pushed no levels**, which is legal -- they are
/// optional on the envelope -- and which a consumer must tell apart from "no
/// level matches".
pub fn levels_by_model<P: SnapshotEnvelope>(scoped: &[(ModelKey, P)]) -> BTreeMap<String, Vec<crate::contract::Level>> {
    scoped
        .iter()
        .map(|(key, payload)| (key.model_id.clone(), payload.levels().to_vec()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use super::*;
    use crate::contract::{DoorPayload, Model, Project, Snapshot, SUPPORTED_DOOR_SCHEMA};
    use crate::storage::{FsStore, SnapshotMeta, SnapshotStore};

    fn bundle(milestones: Vec<Milestone>) -> ProjectSettings {
        ProjectSettings {
            spaces: Default::default(),
            reference: BTreeMap::new(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec![],
            milestones,
            anchor_model: None,
            tag_type_code: None,
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            windows: Default::default(),
            ffe: Default::default(),
            hierarchy_exclusions: vec![],
            routing: Default::default(),
        }
    }

    fn milestone(name: &str, rooms: &[(&str, &str)], doors: &[(&str, &str)]) -> Milestone {
        let pins = |p: &[(&str, &str)]| p.iter().map(|(m, id)| (m.to_string(), id.to_string())).collect();
        Milestone {
            name: name.to_string(),
            date: "2026-01-01".to_string(),
            reference_snapshots: BTreeMap::new(),
            attachments: pins(rooms),
            door_attachments: pins(doors),
            window_attachments: BTreeMap::new(),
            ffe_attachments: BTreeMap::new(),
            space_attachments: BTreeMap::new(),
            ceiling_attachments: BTreeMap::new(),
            floor_attachments: BTreeMap::new(),
        }
    }

    fn row(project: &str, model: &str, latest: &[(SnapshotKind, &str)]) -> ModelIndexRow {
        ModelIndexRow {
            key: ModelKey { project_id: project.to_string(), model_id: model.to_string() },
            project_name: project.to_string(),
            model_name: model.to_string(),
            latest: latest.iter().map(|(k, id)| (*k, id.to_string())).collect(),
            placement: None,
        }
    }

    fn planned(plan: &[PlannedRead]) -> Vec<(&str, &str)> {
        plan.iter().map(|p| (p.key.model_id.as_str(), p.id.as_str())).collect()
    }

    fn door_meta<'a>(key: &'a ModelKey, taken_at: &'a str) -> SnapshotMeta<'a> {
        SnapshotMeta {
            kind: SnapshotKind::Doors,
            key,
            project_name: "P",
            model_name: "M",
            taken_at,
            phase: None,
            model_to_shared: None,
        }
    }

    fn doors(taken_at: &str) -> DoorPayload {
        DoorPayload {
            schema_version: SUPPORTED_DOOR_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: taken_at.to_string() },
            phase: None,
            model_to_shared: None,
            levels: vec![],
            doors: vec![],
        }
    }

    /// The rules the two loops this replaced applied, stated once: registered
    /// projects only, the latest without a milestone, and under one a model
    /// must be both pinned and hold a snapshot of the kind.
    #[test]
    fn test_the_plan_admits_exactly_what_the_read_serves() {
        use SnapshotKind::{Doors, Rooms};
        let index = vec![
            row("p1", "m1", &[(Rooms, "r-new"), (Doors, "d-new")]),
            row("p1", "m2", &[(Rooms, "r-new")]),
            row("unregistered", "m3", &[(Rooms, "r-new")]),
        ];
        let registry = SettingsRegistry::from_bundles(
            HashMap::from([(
                "p1".to_string(),
                bundle(vec![milestone(
                    "M",
                    &[("m1", "r-old")],
                    &[("m1", "d-old"), ("m2", "d-dangling")],
                )]),
            )]),
            None,
        );
        let plan = |kind, project, ms| plan_reads(&index, &registry, kind, project, ms);

        assert_eq!(
            planned(&plan(Rooms, None, None)),
            [("m1", "r-new"), ("m2", "r-new")],
            "no bundle, no read"
        );
        assert_eq!(planned(&plan(Doors, None, None)), [("m1", "d-new")], "m2 holds no doors");
        assert!(plan(Rooms, Some("unregistered"), None).is_empty());

        assert_eq!(planned(&plan(Rooms, None, Some("M"))), [("m1", "r-old")], "m2 is not pinned for rooms");
        assert_eq!(
            planned(&plan(Doors, None, Some("M"))),
            [("m1", "d-old")],
            "the m2 doors pin names nothing that was ever pushed"
        );
        assert!(plan(Rooms, None, Some("no such milestone")).is_empty());
    }

    /// The ETag of a milestone read follows that entity's OWN pin.
    ///
    /// Written against the bug it fixes: `scope_cursor` used to read the rooms
    /// map (`attachments`) for every kind, so two milestones pinning the same
    /// rooms snapshot and different doors snapshots produced one doors cursor
    /// for two different doors bodies. The store is identical in both states
    /// below; only the doors snapshot the milestone names differs.
    #[test]
    fn test_a_milestone_cursor_hashes_each_kinds_own_pin() {
        let dir = std::env::temp_dir().join(format!("roommate-plan-cursor-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let key = ModelKey { project_id: "p1".to_string(), model_id: "m1".to_string() };
        let store = FsStore::new(dir.clone()).unwrap();
        for ts in ["2026-02-01T00:00:00Z", "2026-03-01T00:00:00Z"] {
            store.put_raw(&door_meta(&key, ts), &serde_json::to_vec(&doors(ts)).unwrap()).unwrap();
        }

        let cursor_pinning = |doors_pin: &str| {
            let ms = milestone("M", &[("m1", "2026-01-01T00:00:00Z")], &[("m1", doors_pin)]);
            let state = AppState::new(
                Box::new(FsStore::new(dir.clone()).unwrap()),
                HashMap::from([("p1".to_string(), bundle(vec![ms]))]),
                None,
            );
            super::super::scope_cursor(&state, Some("p1"), Some("M"), &[SnapshotKind::Doors]).unwrap()
        };
        assert_ne!(cursor_pinning("2026-02-01T00:00:00Z"), cursor_pinning("2026-03-01T00:00:00Z"));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A milestone read opens the pinned snapshot and never the latest.
    ///
    /// Proven by making the latest unreadable: the old loop parsed every
    /// model's latest before looking at the milestone, so a malformed latest
    /// failed a read that was never going to serve it. Counting results could
    /// not show this -- both versions return the pinned snapshot when every
    /// file parses.
    #[test]
    fn test_a_milestone_read_never_parses_the_latest() {
        let dir = std::env::temp_dir().join(format!("roommate-plan-pinned-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let key = ModelKey { project_id: "p1".to_string(), model_id: "m1".to_string() };
        let store = FsStore::new(dir.clone()).unwrap();
        let pinned = "2026-02-01T00:00:00Z";
        store
            .put_raw(&door_meta(&key, pinned), &serde_json::to_vec(&doors(pinned)).unwrap())
            .unwrap();
        store.put_raw(&door_meta(&key, "2026-06-01T00:00:00Z"), b"not a snapshot").unwrap();

        let state = AppState::new(
            Box::new(store),
            HashMap::from([("p1".to_string(), bundle(vec![milestone("M", &[], &[("m1", pinned)])]))]),
            None,
        );

        let read = scope_snapshots::<DoorPayload>(&state, SnapshotKind::Doors, Some("p1"), Some("M")).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].1.snapshot.taken_at, pinned);
        assert!(
            scope_snapshots::<DoorPayload>(&state, SnapshotKind::Doors, Some("p1"), None).is_err(),
            "the latest really is malformed -- the milestone read simply never opened it"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    // ---------- storey scope ----------

    fn level(id: &str, name: &str, elevation: f64) -> crate::contract::Level {
        crate::contract::Level { id: id.into(), name: name.into(), elevation }
    }

    /// Absent parameters are an unscoped read; a bad elevation is refused with
    /// the value named, never silently read as every storey.
    #[test]
    fn test_storey_scope_parses_or_refuses() {
        assert_eq!(StoreyScope::parse(None, None), Ok(None));
        let scope = StoreyScope::parse(Some("0, 3000.5,"), Some("a,b")).unwrap().unwrap();
        assert_eq!(scope.elevations, [0.0, 3000.5]);
        assert_eq!(scope.level_ids.len(), 2);
        let err = StoreyScope::parse(Some("0,ground"), None).unwrap_err();
        assert!(err.contains("ground"), "{err}");
        assert!(StoreyScope::parse(Some("NaN"), None).is_err());
    }

    /// The three cases `StoreyScope` documents, each a superset of what the
    /// viewer's `onStorey` keeps.
    #[test]
    fn test_storey_admission_is_a_superset_of_the_viewer_rule() {
        let levels = BTreeMap::from([
            // Declares levels. "L1 ref" shares LEVEL 1's elevation under another
            // name: onStorey's by-elevation fallback may need it, so it is kept.
            (
                "arch".to_string(),
                vec![
                    level("10", "LEVEL 0", 0.0),
                    level("11", "LEVEL 1", 3000.0),
                    level("12", "L1 ref", 3020.0),
                ],
            ),
            // Declares none: only its level ids can place its elements.
            ("facade".to_string(), vec![]),
        ]);
        let scope = StoreyScope::parse(Some("3000"), Some("11")).unwrap().unwrap();
        let admit = scope.admitter(&levels);

        assert!(admit.admits("arch", "11"), "on the requested storey");
        assert!(admit.admits("arch", "12"), "within LEVEL_EPS_MM of it, whatever its name");
        assert!(!admit.admits("arch", "10"), "another storey");
        assert!(!admit.admits("arch", "-1"), "a level its own model does not declare places it nowhere");
        assert!(admit.admits("facade", "11"), "no declared levels: the id is all there is");
        assert!(!admit.admits("facade", "10"));

        // No model declares any level: the viewer shows everything, so the body
        // must hold everything.
        let bare = BTreeMap::from([("facade".to_string(), vec![])]);
        assert!(scope.admitter(&bare).admits("facade", "anything"));
    }
}
