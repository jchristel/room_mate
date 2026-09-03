//! The **doors envelope**: how a push, a stored snapshot and a stream frame of
//! `Opening`s are shaped when the entity is doors.
//!
//! The record itself is no longer here. It moved to
//! [`openings`](super::openings) once measurement showed a window record and a
//! door record are structurally identical, and this file kept only the parts
//! that are genuinely per entity:
//!
//! - **the schema version.** Doors are at v2; windows start at v1, because the
//!   versions are independent by design and a shared constant would tie two
//!   contracts that have no reason to move together.
//! - **the on-disk and on-the-wire key.** A stored doors snapshot names its
//!   element list `doors`, and every snapshot already written says so. A shared
//!   payload type could only have one such name, so windows get their own
//!   envelope beside this one rather than a rename that would strand history.
//!
//! That split is the answer to a question worth recording, because the obvious
//! move is the wrong one: generalising the *envelope* as well would have been
//! tidier to read and would have changed serde keys, which is not a refactor at
//! all — it is a migration. Sharing the record costs nothing and buys the whole
//! service layer; sharing the envelope buys a shorter file and costs the store.
//!
//! The upload envelope's own pieces (`Project`, `Model`, `Snapshot`,
//! `ModelToShared`) stay in `mod.rs` and are used from here, so neither entity
//! carries a private copy — the "what generalizes" list in
//! [Entities](../../docs/STRATEGY-ENTITIES.md).

use serde::{Deserialize, Serialize};

use super::openings::Opening;
use super::{Level, Model, ModelToShared, Project, Snapshot};

/// One timestamped push of one model's doors.
///
/// Carries the **same upload envelope** as `RoomPayload` — `project`, `model`,
/// `snapshot`, `phase` — resolved through the same `ensure_taken_at` /
/// `validate_snapshot_id` / `normalize_phase` functions rather than
/// reimplementations. That is the first item on [Entities](../../docs/STRATEGY-ENTITIES.md)'s
/// "what generalizes" list, and the reason a doors push needs
/// no new identity concepts at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoorPayload {
    /// **Doors version independently of rooms** — see `SUPPORTED_DOOR_SCHEMA`.
    pub schema_version: u32,

    pub project: Project,
    pub model: Model,
    #[serde(default)]
    pub snapshot: Snapshot,

    /// The Revit phase this push was filtered to, in lockstep with
    /// `RoomPayload::phase` and with the same `Option`-on-the-type,
    /// required-at-ingest split: the type stays permissive because every stored
    /// snapshot re-parses through it at boot, and the handler is strict.
    ///
    /// A doors push is stricter than a rooms push in one way, and it is worth
    /// knowing why. A rooms push whose phase disagrees with the lineage is
    /// *quarantined* and can be promoted, because promotion is how a model
    /// re-phases. A doors push that disagrees is **refused**: promoting it would
    /// move the lineage while every room snapshot stayed on the old phase,
    /// stranding the very rooms `from_room`/`to_room` resolve against.
    #[serde(default)]
    pub phase: Option<String>,

    /// Model→shared placement, in lockstep with `RoomPayload`.
    ///
    /// Carried even though no reader consumes it yet, which is a deliberate
    /// exception to this codebase's "an unread field drifts" rule. A door
    /// footprint is geometry in the same model space as the rooms', and every
    /// geometry payload here carries its own placement; a doors payload that
    /// omitted it would be the odd one out, and the omission would surface as a
    /// contract change on the day a viewer draws doors rather than as a field
    /// that was there all along.
    ///
    /// **Per push, never per door.** The raw export repeats `rotation_coord` and
    /// `translation_coord` (survey millimetres) on every polygon — a model-level
    /// fact duplicated 26 times. Dropped at translation, exactly as
    /// `translate_room` already drops the room equivalent.
    #[serde(default)]
    pub model_to_shared: Option<ModelToShared>,

    /// The level set `Opening.level_id` points into, for a model that has no rooms
    /// snapshot to supply one.
    ///
    /// **Empty is the ordinary case and stays the ordinary case.** A model that
    /// pushes rooms as well already declares its levels there, and this list is
    /// then redundant — `service::doors` prefers the rooms' set, so sending it
    /// twice changes nothing. What this exists for is the model that pushes
    /// doors and *no rooms at all*: a facade or envelope file, whose doors are
    /// real and whose rooms live in the interior models it is linked against.
    ///
    /// Without it those doors are unreachable rather than merely unresolved.
    /// `service::doors::Candidates::locate` looks an elevation up by
    /// `(model_id, level_id)` and gives up before probing when it misses, so a
    /// doors-only model's every door returned `NoCandidate` no matter what the
    /// geometry said — 191 of them on the job this was built for, of which ~93
    /// are real doors once nested leaves are excluded.
    ///
    /// **Optional and additive, so no schema bump** (STRATEGY.md's rule): every
    /// v2 payload already on disk stays valid and means exactly what it meant.
    ///
    /// The *elevation* is what this is for, not the id. Level ids are
    /// per-document and never match across models; elevations are what cross,
    /// which is why `room_locator` compares on them. See `LEVEL_EPS_FT`.
    #[serde(default)]
    pub levels: Vec<Level>,

    pub doors: Vec<Opening>,
}

/// One model's block on a multi-model doors upload — the doors counterpart of
/// `RoomModelEnvelope`, and still shorter than it.
///
/// No `room_boundary`: that is a rooms fact the doors contract has no key for.
/// `levels` IS carried, on the optional terms `DoorPayload::levels` states — a
/// doors-only model has no rooms snapshot to declare the level set its doors
/// point into, and without one its doors cannot be placed on an elevation axis
/// at all. A model that also pushes rooms may leave it empty and usually does.
#[derive(Debug, Clone, Deserialize)]
pub struct DoorModelEnvelope {
    #[serde(flatten)]
    pub model: Model,
    #[serde(default)]
    pub model_to_shared: Option<ModelToShared>,
    #[serde(default)]
    pub levels: Vec<Level>,
}

impl DoorModelEnvelope {
    /// Rebuild the single-model `DoorPayload` this block plus the run's shared
    /// envelope describes — see `RoomModelEnvelope::into_payload`, which this
    /// mirrors and which explains why a push decomposes at all.
    pub fn into_payload(
        self,
        schema_version: u32,
        project: Project,
        snapshot: Snapshot,
        phase: Option<String>,
        doors: Vec<Opening>,
    ) -> DoorPayload {
        DoorPayload {
            schema_version,
            project,
            model: self.model,
            snapshot,
            phase,
            model_to_shared: self.model_to_shared,
            levels: self.levels,
            doors,
        }
    }
}

/// The first NDJSON line of a streamed doors push (`POST /doors/stream`): the
/// run's shared identity plus one `DoorModelEnvelope` per model, with every
/// door arriving on a following line as a `StreamDoor`.
///
/// Its own type rather than making `doors` optional on `DoorPayload`, for the
/// reason `StreamEnvelope` gives: the envelope must deserialize alone with no
/// doors present, and `DoorPayload` must keep `doors` guaranteed for every other
/// consumer.
#[derive(Debug, Clone, Deserialize)]
pub struct DoorStreamEnvelope {
    pub schema_version: u32,
    pub project: Project,
    #[serde(default)]
    pub snapshot: Snapshot,
    #[serde(default)]
    pub phase: Option<String>,
    pub models: Vec<DoorModelEnvelope>,
}

/// One door line of a streamed push: the door, plus the id of the model it
/// belongs to. See `StreamRoom` for why the id rides every element rather than
/// a grouping marker.
#[derive(Debug, Clone, Deserialize)]
pub struct StreamDoor {
    pub model_id: String,
    #[serde(flatten)]
    pub door: Opening,
}

/// The buffered multi-model doors upload (`POST /doors`), the counterpart of
/// `RoomsUpload`.
#[derive(Debug, Clone, Deserialize)]
pub struct DoorsUpload {
    pub schema_version: u32,
    pub project: Project,
    #[serde(default)]
    pub snapshot: Snapshot,
    #[serde(default)]
    pub phase: Option<String>,
    pub models: Vec<DoorModelUpload>,
}

/// One model's block on a buffered doors upload: its `DoorModelEnvelope` plus
/// its doors.
#[derive(Debug, Clone, Deserialize)]
pub struct DoorModelUpload {
    #[serde(flatten)]
    pub envelope: DoorModelEnvelope,
    pub doors: Vec<Opening>,
}

/// Doors schema version this server accepts. **Starts at 1, and moves
/// independently of `SUPPORTED_SCHEMA`.**
///
/// Versioning doors against the room contract's v6 would couple two things that
/// will move separately: a change to the room property tiers has nothing to say
/// about doors, and bumping both would force every room producer to re-release
/// over a doors-only change (and vice versa). Two entities, two version lines
/// — the per-entity half of [Entities](../../docs/STRATEGY-ENTITIES.md)'s
/// "what does not generalize".
///
/// Starting at 1 rather than at 6 is the other half of that: a shared starting
/// number would imply a shared history these two do not have.
///
/// **Now 2: one push carries many models**, in lockstep with rooms' v6 → v7 and
/// for the identical reason — the envelope's single `model` block became a
/// `models` list and each door names its own. That the two bumped together is a
/// coincidence of one change touching both entities, not the version lines
/// merging: the next change to either will move one number and not the other.
pub const SUPPORTED_DOOR_SCHEMA: u32 = 2;

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire shape, end to end: a v2 doors payload round-trips with both
    /// property tiers, both room references, and the footprint intact.
    #[test]
    fn test_door_payload_round_trips() {
        let json = serde_json::json!({
            "schema_version": 2,
            "project":  { "id": "House A", "name": "House A" },
            "model":    { "id": "m1", "name": "ARCH", "source": "revit" },
            "snapshot": { "taken_at": "2026-07-29T20:03:41Z" },
            "phase": "New Construction",
            "doors": [{
                "id": "2628382",
                "level_id": "290501",
                "loops": [{ "points": [
                    { "x": 22.72, "y": -2.95 },
                    { "x": 26.62, "y": -2.95 },
                    { "x": 26.62, "y": -2.42 }
                ] }],
                "from_room": "2621499",
                "to_room": "2620294",
                "type_id": "2503229",
                "type_name": "D199a - 1080x2945 SLD",
                "insertion_point": { "x": 24.67, "y": -2.68 },
                "through_wall_normal": { "x": 0.0, "y": -1.0 },
                "properties": { "Mark": { "value": "29", "storage_type": "String" } },
                "type_properties": { "Leaf 1 Width": { "value": "1100.0", "storage_type": "Double" } }
            }]
        });

        let payload: DoorPayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.schema_version, SUPPORTED_DOOR_SCHEMA);
        assert_eq!(payload.phase.as_deref(), Some("New Construction"));

        let door = &payload.doors[0];
        assert_eq!(door.id, "2628382");
        assert_eq!(door.insertion_point.as_ref().map(|p| p.x), Some(24.67));
        assert_eq!(door.through_wall_normal.as_ref().map(|n| n.y), Some(-1.0));
        assert_eq!(door.from_room.as_deref(), Some("2621499"));
        assert_eq!(door.to_room.as_deref(), Some("2620294"));
        assert_eq!(door.type_name, "D199a - 1080x2945 SLD");
        assert_eq!(door.properties["Mark"].value, "29");
        assert_eq!(door.type_properties["Leaf 1 Width"].value, "1100.0");
        assert_eq!(door.loops[0].points.len(), 3);

        let reparsed: DoorPayload = serde_json::from_str(&serde_json::to_string(&payload).unwrap()).unwrap();
        assert_eq!(reparsed.doors[0].to_room.as_deref(), Some("2620294"));
        assert_eq!(reparsed.doors[0].type_properties["Leaf 1 Width"].value, "1100.0");
        assert_eq!(reparsed.doors[0].through_wall_normal.as_ref().map(|n| n.y), Some(-1.0));
    }

    /// `levels` is optional and additive, which is what keeps doors on v2: a
    /// payload written before the field existed still parses and still means
    /// what it meant. That is the STRATEGY.md test for "does this force a bump",
    /// asserted rather than argued.
    #[test]
    fn test_levels_are_optional_on_a_doors_payload() {
        let mut json = serde_json::json!({
            "schema_version": 2,
            "project":  { "id": "p", "name": "P" },
            "model":    { "id": "facade", "name": "F", "source": "revit" },
            "snapshot": { "taken_at": "2026-08-27T00:00:00Z" },
            "phase": "New Construction",
            "doors": []
        });

        let without: DoorPayload = serde_json::from_value(json.clone()).unwrap();
        assert!(without.levels.is_empty(), "absent reads as empty, not an error");

        json["levels"] = serde_json::json!([
            { "id": "16667035", "name": "GROUND", "elevation": 55500.0 }
        ]);
        let with: DoorPayload = serde_json::from_value(json).unwrap();
        assert_eq!(with.levels.len(), 1);
        assert_eq!(with.levels[0].elevation, 55500.0, "the elevation is the field's whole purpose");

        let reparsed: DoorPayload = serde_json::from_str(&serde_json::to_string(&with).unwrap()).unwrap();
        assert_eq!(reparsed.levels[0].id, "16667035", "survives a round-trip");
    }

    /// A streamed model block carries `levels` on the same optional terms, so
    /// the two push paths cannot disagree about whether a doors-only model can
    /// declare its own.
    #[test]
    fn test_stream_model_block_carries_levels() {
        let json = serde_json::json!({
            "schema_version": 2,
            "project":  { "id": "p", "name": "P" },
            "snapshot": { "taken_at": "2026-08-27T00:00:00Z" },
            "phase": "New Construction",
            "models": [
                { "id": "facade", "name": "F", "source": "revit",
                  "levels": [{ "id": "l1", "name": "GROUND", "elevation": 55500.0 }] },
                { "id": "interior", "name": "I", "source": "revit" }
            ]
        });

        let envelope: DoorStreamEnvelope = serde_json::from_value(json).unwrap();
        assert_eq!(envelope.models[0].levels.len(), 1);
        assert!(envelope.models[1].levels.is_empty(), "a model with rooms elsewhere may send none");

        let payload = envelope.models.into_iter().next().unwrap().into_payload(
            SUPPORTED_DOOR_SCHEMA,
            Project { id: "p".into(), name: "P".into() },
            Snapshot { taken_at: "2026-08-27T00:00:00Z".into() },
            Some("New Construction".into()),
            vec![],
        );
        assert_eq!(payload.levels[0].elevation, 55500.0, "the block's levels reach the decomposed payload");
    }

    /// The streamed envelope deserializes with no `doors` key present — proves
    /// it doesn't accidentally require one — and carries every envelope field in
    /// lockstep with the buffered payload.
    #[test]
    fn test_door_stream_envelope_deserializes_without_doors() {
        let mut json = serde_json::json!({
            "schema_version": 2,
            "project":  { "id": "House A", "name": "House A" },
            "models": [{ "id": "m1", "name": "ARCH", "source": "revit" }]
        });

        let envelope: DoorStreamEnvelope = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(envelope.schema_version, SUPPORTED_DOOR_SCHEMA);
        assert_eq!(envelope.snapshot.taken_at, "", "for `ensure_taken_at` to resolve");
        assert!(envelope.phase.is_none());
        assert!(envelope.models[0].model_to_shared.is_none());

        // The phase is a run fact and rides the envelope; the transform places
        // one document and rides that document's block.
        json["phase"] = serde_json::json!("New Construction");
        json["models"][0]["model_to_shared"] = serde_json::json!({ "matrix": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] });
        let envelope: DoorStreamEnvelope = serde_json::from_value(json).unwrap();
        assert_eq!(envelope.phase.as_deref(), Some("New Construction"));
        assert!(envelope.models[0].model_to_shared.is_some());
    }

    /// A door line names its model, and the door deserializes flat alongside
    /// that one extra key — the doors half of `StreamRoom`'s guarantee.
    #[test]
    fn test_stream_door_carries_its_model_id() {
        let json = serde_json::json!({
            "model_id": "m1",
            "id": "d1", "level_id": "lvl1", "type_id": "t1", "type_name": "D120a",
            "loops": [], "properties": {}, "type_properties": {}
        });
        let line: StreamDoor = serde_json::from_value(json).unwrap();
        assert_eq!(line.model_id, "m1");
        assert_eq!(line.door.id, "d1");
    }

    /// Doors version independently of rooms. If this ever reads as equal, the
    /// two version lines have been coupled — which is the thing this type says
    /// not to do.
    #[test]
    fn test_door_schema_is_independent_of_the_room_schema() {
        assert_eq!(SUPPORTED_DOOR_SCHEMA, 2);
        assert_ne!(SUPPORTED_DOOR_SCHEMA, super::super::SUPPORTED_SCHEMA);
    }
}
