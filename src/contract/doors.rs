//! The door half of the Revit contract — the pipeline's *second* primary
//! entity, and the first one that is not rooms.
//!
//! Split into its own file rather than added to `mod.rs` because the door types
//! are exactly the trigger [Conventions](../../docs/CODING-CONVENTIONS.md)'
//! measured-module note named for splitting `contract.rs` into `contract/`. The
//! shared envelope (`Project`, `Model`, `Snapshot`, `ModelToShared`), the
//! geometry primitives (`Loop`, `Point2D`), and the property machinery
//! (`CustomValue`, `PropertyTiers`, `lookup_property`) all stay in `mod.rs` and
//! are used from here — a door does not get its own copy of any of them, which
//! is the whole point of [Entities](../../docs/STRATEGY-ENTITIES.md) Decision 1's
//! "what generalizes" list.
//!
//! What does *not* generalize, and so lives here: the room references, the
//! two-tier property split, and the doors schema version.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{CustomValue, Loop, Model, ModelToShared, Project, PropertyTiers, Snapshot};

/// One door instance, as extracted from Revit.
///
/// **A door is not a `Room` with different fields.** The two differences that
/// drive everything else here are the second property tier (below) and the room
/// references (below that); everything they share — geometry convention,
/// identity discipline, the upload envelope — is deliberately identical, so one
/// renderer and one `model_to_shared` transform serve both entities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Door {
    /// The Revit `ElementId`, as a string on the wire like every other id here.
    ///
    /// Sourced from the export's `instance_properties.id` — the same place
    /// `translate_room` reads a room's id, so the two entities' ids come from
    /// one convention rather than two. [Entities](../../docs/STRATEGY-ENTITIES.md)
    /// lists "no door id" as the first blocker in the export; a later duHast
    /// carries it, and the sample file has 26 unique values that collide with no
    /// room id. Nothing else in the record identifies the element: `Mark` is not
    /// unique (one door in the sample is `"None"`) and `IfcGUID` is an instance
    /// property, not identity.
    pub id: String,

    /// The level the door sits on, keyed the same way `Room.level_id` is —
    /// and, like it, unique only *within* one model.
    ///
    /// No `levels` array rides the door payload. A door push targets a model
    /// that already has rooms (ingest refuses otherwise), and that model's rooms
    /// snapshot already carries the level set these ids point into. Sending a
    /// second copy would create two level lists per model that could disagree,
    /// for no reader that needs the duplicate.
    pub level_id: String,

    /// The door's footprint, in the **room convention verbatim**: `loops[0]` is
    /// the outer loop, `loops[1..]` are holes, points are decimal feet in model
    /// space, Y up.
    ///
    /// Identical on purpose ([Entities](../../docs/STRATEGY-ENTITIES.md)
    /// Decision 4): a second convention would fork every consumer that draws or
    /// transforms geometry, and there is nothing about a swing footprint that
    /// needs one.
    ///
    /// **Empty is a real state — do not make this field required.** A door
    /// family with no 3D geometry has no footprint to extract, and two of the 26
    /// doors in the sample export are exactly that (both family type
    /// `2040x620x40`). They still carry properties and both room references, so
    /// they are real doors that QA and comparison must see; only their geometry
    /// is unknown.
    ///
    /// The trap this replaced is worth naming, because the bad value looks
    /// plausible rather than absent: duHast hands back Revit's *uninitialized*
    /// `BoundingBoxXYZ`, whose min is `+1e30` and max is `-1e30`, and its own
    /// "did we get a box" and "is the loop non-empty" checks both pass. The
    /// producer therefore has to recognise the sentinel and send no loops, or
    /// every consumer downstream inherits a footprint 1e30 feet across — the
    /// same class of failure as the million-foot spike in
    /// [Area calculation](../../docs/STRATEGY-AREA-CALCULATION.md).
    #[serde(default)]
    pub loops: Vec<Loop>,

    /// The room on the door's "from" side, or `None`.
    ///
    /// **`None` is a normal state, not a missing value** — an external door has
    /// no room on one side, and 6 of the 26 doors in the sample export are
    /// one-sided. QA reports a door with *neither* side as a finding; it does
    /// not report a one-sided door at all.
    ///
    /// **One room, not a list.** The raw export carries an array only because it
    /// holds one entry per phase. RoomMate pushes exactly one phase
    /// ([Phasing](../../docs/PLAN-phasing.md)), so the array collapses to at most
    /// one entry and the producer resolves which — from the Revit API's
    /// `FamilyInstance.FromRoom[phase]`, which takes the phase and answers
    /// exactly one room, rather than from the export's `phase_id`, which is not
    /// resolvable against anything on the wire.
    ///
    /// The value is a `Room.id` **in the same model**. Room ids are unique only
    /// within a model, so this reference is only meaningful against its own
    /// model's rooms — which is why ingest requires them and QA resolves against
    /// them.
    #[serde(default)]
    pub from_room: Option<String>,

    /// The room on the door's "to" side, or `None`. Same contract as
    /// `from_room` in every respect.
    #[serde(default)]
    pub to_room: Option<String>,

    /// The `ElementId` of the door's family type, as a string.
    ///
    /// Carried because `type_properties` below is a *shared* tier — every
    /// instance of one type repeats it — and this is the identity of the thing
    /// being shared. It is what a future type-property deduplication would key a
    /// shared table on ([Entities](../../docs/STRATEGY-ENTITIES.md) "Deferred"),
    /// and what a hardware schedule would join per type rather than per leaf.
    pub type_id: String,

    /// The family type's display name (e.g. `"D120a - 820x2100 SLD"`).
    ///
    /// A display label, never a key — the same split `Project.name` and
    /// `Model.name` have from their ids, and for the same reason: a type rename
    /// in Revit must not fork anything.
    pub type_name: String,

    /// This door instance's own properties, keyed by the *source's own* property
    /// name, exactly as `Room.properties` is.
    #[serde(default)]
    pub properties: BTreeMap<String, CustomValue>,

    /// The family **type's** properties — shared by every instance of
    /// `type_id`, and kept as a separate map rather than merged into
    /// `properties`.
    ///
    /// **This is the door's defining structural difference from a room.**
    /// "This leaf is 820 wide" and "every door of this type is 820 wide" are
    /// different claims, and a hardware schedule joins against the second
    /// ([Entities](../../docs/STRATEGY-ENTITIES.md) Decision 4). Flattening the
    /// two would lose that distinction at the contract level, permanently and
    /// irrecoverably.
    ///
    /// A *lookup* across the two is a different question from how they are
    /// stored, and is answered by `PropertyTiers` below.
    #[serde(default)]
    pub type_properties: BTreeMap<String, CustomValue>,
}

/// A door is two-tier: its own properties first, its family type's second.
///
/// The precedence rule itself — that a tier wins only when it is `Present`, so
/// a *blank* instance parameter does not shadow a real type value — lives on
/// `property_presence` in `mod.rs`, measured against this very export. This impl
/// only declares the order.
impl PropertyTiers for Door {
    fn tiers(&self) -> Vec<&BTreeMap<String, CustomValue>> {
        vec![&self.properties, &self.type_properties]
    }
}

/// One timestamped push of one model's doors.
///
/// Carries the **same upload envelope** as `RoomPayload` — `project`, `model`,
/// `snapshot`, `phase` — resolved through the same `ensure_taken_at` /
/// `validate_snapshot_id` / `normalize_phase` functions rather than
/// reimplementations. That is [Entities](../../docs/STRATEGY-ENTITIES.md)
/// Decision 1's first "what generalizes" item, and the reason a doors push needs
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

    pub doors: Vec<Door>,
}

/// The first NDJSON line of a streamed doors push (`POST /doors/stream`):
/// everything in `DoorPayload` EXCEPT `doors`, which arrive as subsequent
/// lines, one door per line.
///
/// Its own type rather than making `doors` optional on `DoorPayload`, for the
/// reason `StreamEnvelope` gives: the envelope must deserialize alone with no
/// doors present, and `DoorPayload` must keep `doors` guaranteed for every other
/// consumer. Every envelope field here is in lockstep with `DoorPayload` —
/// which ingest route a producer picks must never change what the stored
/// snapshot claims.
#[derive(Debug, Clone, Deserialize)]
pub struct DoorStreamEnvelope {
    pub schema_version: u32,
    pub project: Project,
    pub model: Model,
    #[serde(default)]
    pub snapshot: Snapshot,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub model_to_shared: Option<ModelToShared>,
}

/// Doors schema version this server accepts. **Starts at 1, and moves
/// independently of `SUPPORTED_SCHEMA`.**
///
/// Versioning doors against the room contract's v6 would couple two things that
/// will move separately: a change to the room property tiers has nothing to say
/// about doors, and bumping both would force every room producer to re-release
/// over a doors-only change (and vice versa). Two entities, two version lines
/// ([Entities](../../docs/STRATEGY-ENTITIES.md) Decision 4).
///
/// Starting at 1 rather than at 6 is the other half of that: a shared starting
/// number would imply a shared history these two do not have.
pub const SUPPORTED_DOOR_SCHEMA: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire shape, end to end: a v1 doors payload round-trips with both
    /// property tiers, both room references, and the footprint intact.
    #[test]
    fn test_door_payload_round_trips() {
        let json = serde_json::json!({
            "schema_version": 1,
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
                "properties": { "Mark": { "value": "29", "storage_type": "String" } },
                "type_properties": { "Leaf 1 Width": { "value": "1100.0", "storage_type": "Double" } }
            }]
        });

        let payload: DoorPayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.schema_version, SUPPORTED_DOOR_SCHEMA);
        assert_eq!(payload.phase.as_deref(), Some("New Construction"));

        let door = &payload.doors[0];
        assert_eq!(door.id, "2628382");
        assert_eq!(door.from_room.as_deref(), Some("2621499"));
        assert_eq!(door.to_room.as_deref(), Some("2620294"));
        assert_eq!(door.type_name, "D199a - 1080x2945 SLD");
        assert_eq!(door.properties["Mark"].value, "29");
        assert_eq!(door.type_properties["Leaf 1 Width"].value, "1100.0");
        assert_eq!(door.loops[0].points.len(), 3);

        let reparsed: DoorPayload = serde_json::from_str(&serde_json::to_string(&payload).unwrap()).unwrap();
        assert_eq!(reparsed.doors[0].to_room.as_deref(), Some("2620294"));
        assert_eq!(reparsed.doors[0].type_properties["Leaf 1 Width"].value, "1100.0");
    }

    /// **An external door is a normal door.** One side absent must deserialize
    /// as `None` rather than failing, and both sides absent must too — the
    /// contract does not decide whether that is a problem, QA does. 6 of the 26
    /// doors in the sample export are one-sided.
    #[test]
    fn test_room_references_are_optional_on_both_sides() {
        let base = serde_json::json!({
            "id": "d1",
            "level_id": "lvl1",
            "loops": [],
            "type_id": "t1",
            "type_name": "Single"
        });

        let neither: Door = serde_json::from_value(base.clone()).unwrap();
        assert!(neither.from_room.is_none() && neither.to_room.is_none());

        let mut external = base.clone();
        external["to_room"] = serde_json::json!("r1");
        let external: Door = serde_json::from_value(external).unwrap();
        assert!(external.from_room.is_none());
        assert_eq!(external.to_room.as_deref(), Some("r1"));

        // Both property maps default to empty, like `Room.properties` — a door
        // with no properties still deserializes rather than failing.
        assert!(neither.properties.is_empty() && neither.type_properties.is_empty());
    }

    /// A door with no footprint parses, because a door family with no 3D
    /// geometry is a real thing — two of the 26 sample doors are one, and both
    /// carry room references that QA has to see. Making `loops` required would
    /// have forced the producer to either drop those doors or invent a
    /// footprint, and the sentinel it would have invented from is `1e30`.
    #[test]
    fn test_a_door_with_no_footprint_is_valid() {
        let door: Door = serde_json::from_value(serde_json::json!({
            "id": "3475937",
            "level_id": "290501",
            "from_room": "2621499",
            "to_room": "2621156",
            "type_id": "t1",
            "type_name": "2040x620x40"
        }))
        .unwrap();

        assert!(door.loops.is_empty());
        assert_eq!(door.from_room.as_deref(), Some("2621499"), "still a real door→room link");
        assert_eq!(door.to_room.as_deref(), Some("2621156"));
    }

    /// A door's tiers are instance-first, type-second. Asserted with the real
    /// collision from the sample export: `Door Leaf Thickness` is blank on the
    /// instance and real on the type, so the *order* alone would give the wrong
    /// answer — it is the `Present`-only rule in `mod.rs` that saves it, and
    /// this test is what would notice if that rule were ever relaxed to plain
    /// shadowing.
    #[test]
    fn test_door_tiers_are_instance_then_type() {
        let door: Door = serde_json::from_value(serde_json::json!({
            "id": "d1",
            "level_id": "lvl1",
            "loops": [],
            "type_id": "t1",
            "type_name": "Single",
            "properties": {
                "Door Leaf Thickness": { "value": "" },
                "Mark": { "value": "29" }
            },
            "type_properties": {
                "Door Leaf Thickness": { "value": "40.0" },
                "Fire Rating": { "value": "FD30" }
            }
        }))
        .unwrap();

        assert_eq!(door.tiers().len(), 2, "instance and type");

        // Instance wins where it has a real value.
        assert_eq!(super::super::lookup_property(&door, "Mark", "revit", &[]), Some("29".to_string()));
        // Type answers what the instance leaves blank.
        assert_eq!(
            super::super::lookup_property(&door, "Door Leaf Thickness", "revit", &[]),
            Some("40.0".to_string()),
            "a blank instance parameter must not shadow the type's value"
        );
        // Type-only properties resolve through the fall-through.
        assert_eq!(
            super::super::lookup_property(&door, "Fire Rating", "revit", &[]),
            Some("FD30".to_string())
        );
    }

    /// The streamed envelope deserializes with no `doors` key present — proves
    /// it doesn't accidentally require one — and carries every envelope field in
    /// lockstep with the buffered payload.
    #[test]
    fn test_door_stream_envelope_deserializes_without_doors() {
        let mut json = serde_json::json!({
            "schema_version": 1,
            "project":  { "id": "House A", "name": "House A" },
            "model":    { "id": "m1", "name": "ARCH", "source": "revit" }
        });

        let envelope: DoorStreamEnvelope = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(envelope.schema_version, SUPPORTED_DOOR_SCHEMA);
        assert_eq!(envelope.snapshot.taken_at, "", "for `ensure_taken_at` to resolve");
        assert!(envelope.phase.is_none());
        assert!(envelope.model_to_shared.is_none());

        json["phase"] = serde_json::json!("New Construction");
        json["model_to_shared"] = serde_json::json!({ "matrix": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] });
        let envelope: DoorStreamEnvelope = serde_json::from_value(json).unwrap();
        assert_eq!(envelope.phase.as_deref(), Some("New Construction"));
        assert!(envelope.model_to_shared.is_some());
    }

    /// Doors version independently of rooms. If this ever reads as equal, the
    /// two version lines have been coupled — which is the thing Decision 4 says
    /// not to do.
    #[test]
    fn test_door_schema_is_independent_of_the_room_schema() {
        assert_eq!(SUPPORTED_DOOR_SCHEMA, 1);
        assert_ne!(SUPPORTED_DOOR_SCHEMA, super::super::SUPPORTED_SCHEMA);
    }
}
