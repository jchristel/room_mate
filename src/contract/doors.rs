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
//! is the whole point of [Entities](../../docs/STRATEGY-ENTITIES.md)'s
//! "what generalizes" list under "What makes something a primary entity".
//!
//! What does *not* generalize, and so lives here: the room references, the
//! two-tier property split, and the doors schema version.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{CustomValue, Loop, Model, ModelToShared, Point2D, Project, PropertyTiers, Snapshot};

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
    /// No `levels` array rides the door payload. The model's rooms snapshot
    /// carries the level set these ids point into, and sending a second copy
    /// would create two level lists per model that could disagree, for no reader
    /// that needs the duplicate.
    ///
    /// Note the reason is no longer "ingest refuses a doors push to a model with
    /// no rooms" — it does not, and doors may arrive first. The level set may
    /// therefore be absent for a while, which is a state the server reports (see
    /// `service::validation::PendingRoomReference`) rather than a reason to
    /// duplicate it here.
    pub level_id: String,

    /// The door's footprint, in the **room convention verbatim**: `loops[0]` is
    /// the outer loop, `loops[1..]` are holes, points are decimal feet in model
    /// space, Y up.
    ///
    /// Identical on purpose: a second convention would fork every consumer that draws or
    /// transforms geometry, and there is nothing about a swing footprint that
    /// needs one.
    ///
    /// **Empty is a real state — do not make this field required.** A door with
    /// no measurable footprint still carries properties and both room
    /// references, so it is a real door QA and comparison must see; only its
    /// geometry is unknown.
    ///
    /// Worth knowing how this was *mis*-read: two `2040x620x40` doors in the
    /// House A export arrived empty, and that was taken as "these families have
    /// no 3D geometry". They do. duHast was mis-measuring them, and with that
    /// fixed (2026-08-07) they come through at 5.10 × 0.13 ft. No door in the
    /// current export is empty — but snapshots pushed before the fix are still
    /// on disk and still parse through this type, which is the reason the field
    /// stays permissive rather than a hypothetical about families.
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
    /// ([Phasing](../../docs/Superseded/PLAN-phasing.md)), so the array collapses to at most
    /// one entry and the producer resolves which — from the Revit API's
    /// `FamilyInstance.FromRoom[phase]`, which takes the phase and answers
    /// exactly one room, rather than from the export's `phase_id`, which is not
    /// resolvable against anything on the wire.
    ///
    /// The value is a `Room.id` **in the same model**. Room ids are unique only
    /// within a model, so this reference is only meaningful against its own
    /// model's rooms — which is why QA resolves against them per model and never
    /// project-wide.
    ///
    /// Ingest used to require those rooms to be there already. It no longer
    /// does: "not yet" is a legitimate answer, and refusing the push meant
    /// refusing data that becomes resolvable the moment the rooms arrive. The
    /// question moved to `door_report`, which can re-answer it every time the
    /// data changes.
    #[serde(default)]
    pub from_room: Option<String>,

    /// The room on the door's "to" side, or `None`. Same contract as
    /// `from_room` in every respect.
    #[serde(default)]
    pub to_room: Option<String>,

    /// Where the door instance sits, in the same space as `loops` — decimal
    /// feet, model space, Y up.
    ///
    /// **The one thing every door has, whatever else is missing.** `loops` is
    /// allowed to be empty, and a consumer that can only place a door by its
    /// footprint cannot place such a door at all — it exists in QA and in
    /// `/doors` but nowhere a reader looks at a plan. The insertion point is
    /// what a door-with-no-geometry can still be drawn at, which is the
    /// difference between "we know nothing about its shape" and "it silently is
    /// not there".
    ///
    /// No door in the current House A export is empty (the two that once were
    /// turned out to be a duHast measuring bug, not families without geometry),
    /// so this now earns its place on the older snapshots still on disk rather
    /// than on the live one.
    ///
    /// The producer emits this for every door — it is Revit's `LocationPoint`,
    /// which a placed `FamilyInstance` always has. `Option` here is not the
    /// producer hedging: it is because every stored snapshot re-parses through
    /// this type at boot, and snapshots pushed before this field existed are
    /// still on disk. Same permissive-type / strict-producer split
    /// `DoorPayload::phase` documents.
    #[serde(default)]
    pub insertion_point: Option<Point2D>,

    /// A unit vector pointing **through the wall, along the direction the door
    /// faces** — `FamilyInstance.FacingOrientation`, projected to plan.
    ///
    /// **The normal, never the tangent.** The tangent (along the wall run) would
    /// leave every consumer to rotate it 90° and then decide the sign of that
    /// rotation, which is precisely the ambiguity this field exists to remove. A
    /// consumer points an arrow *along this vector directly*; there is no trig
    /// to get wrong, and nothing to re-derive.
    ///
    /// **This is where the door faces, NOT "toward `to_room`".** The two usually
    /// coincide — measured against the House A export, all 20 doors carrying
    /// both references put `to_room` on the `+normal` side and `from_room`
    /// behind — but they are different claims and a reader must not collapse
    /// them.
    ///
    /// A door is attributed to the room it *serves*, which the modeller decides,
    /// and that is not always the room it opens into. A cupboard off a long
    /// corridor is the standard case: the door swings into the corridor and
    /// belongs to the cupboard. 2 of the 26 House A doors are deliberately this
    /// shape. So the arrow drawn from this vector can legitimately point away
    /// from the room the door is attributed to, and neither value is wrong —
    /// they answer different questions.
    ///
    /// Which is exactly why this field is exported rather than derived. The
    /// facing cannot be recovered from the room references (they may be
    /// deliberately opposite it), and the references cannot be recovered from
    /// the facing (they carry an intent geometry does not hold). A consumer
    /// needing both needs both sent.
    ///
    /// Deriving the direction from the host wall instead would have added a
    /// *third* answer to a question that already has two legitimate ones.
    ///
    /// Absent is a real state and consumers must degrade rather than guess: draw
    /// the footprint, omit the arrow. A guessed direction is worse than none,
    /// because nothing downstream can tell it from a measured one. It arrives
    /// absent for a snapshot pushed before this field existed, and for a door
    /// whose facing has no plan component at all (a hatch in a floor).
    ///
    /// Normalised by the producer. Not re-validated here — a consumer that
    /// cares normalises defensively, and rejecting a push over a vector length
    /// would fail a whole model's doors for something with no bearing on any
    /// other reader.
    #[serde(default)]
    pub through_wall_normal: Option<Point2D>,

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
    /// Flattening the
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

    pub doors: Vec<Door>,
}

/// One model's block on a multi-model doors upload — the doors counterpart of
/// `RoomModelEnvelope`, and deliberately shorter than it.
///
/// No `levels` and no `room_boundary`: a doors push targets a model whose rooms
/// snapshot already carries the level set `Door.level_id` points into, and the
/// boundary regime is a rooms fact the doors contract has no key for. What is
/// left is identity and placement, which is exactly what a door needs.
#[derive(Debug, Clone, Deserialize)]
pub struct DoorModelEnvelope {
    #[serde(flatten)]
    pub model: Model,
    #[serde(default)]
    pub model_to_shared: Option<ModelToShared>,
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
        doors: Vec<Door>,
    ) -> DoorPayload {
        DoorPayload {
            schema_version,
            project,
            model: self.model,
            snapshot,
            phase,
            model_to_shared: self.model_to_shared,
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
    pub door: Door,
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
    pub doors: Vec<Door>,
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

    /// **Placement is optional on the type and always sent by the producer**, and
    /// the two are not in tension: every snapshot on disk re-parses through this
    /// type at boot, and snapshots pushed before these fields existed are still
    /// there. A door missing both must deserialize, not fail — the alternative
    /// is a server that will not start against its own stored history.
    ///
    /// The states are independent, which is the point of testing them apart: a
    /// door can have a position and no direction (its facing has no plan
    /// component), and that is a door a consumer draws without an arrow rather
    /// than one it drops.
    #[test]
    fn test_placement_fields_are_optional_and_independent() {
        let base = serde_json::json!({
            "id": "d1",
            "level_id": "lvl1",
            "loops": [],
            "type_id": "t1",
            "type_name": "Single"
        });

        // A pre-placement snapshot: neither field, still a door.
        let old: Door = serde_json::from_value(base.clone()).unwrap();
        assert!(old.insertion_point.is_none());
        assert!(old.through_wall_normal.is_none());

        // Placed, but no readable direction — the "rectangle/cross without an
        // arrow" case. Never a reason to synthesise one.
        let mut placed = base.clone();
        placed["insertion_point"] = serde_json::json!({ "x": 1.5, "y": -2.5 });
        let placed: Door = serde_json::from_value(placed).unwrap();
        assert_eq!(placed.insertion_point.as_ref().map(|p| p.y), Some(-2.5));
        assert!(placed.through_wall_normal.is_none());

        // **Explicit `null`, not just an absent key** — which is what the
        // producer actually sends. `post_doors.translate_door` writes both keys
        // on every door whether or not Revit answered, because "Revit had no
        // plan direction for this door" and "this producer is too old to have
        // looked" are different facts and a missing key cannot tell them apart.
        // A `#[serde(default)]` covers the absent key; this covers the wire.
        let mut nulled = base.clone();
        nulled["insertion_point"] = serde_json::Value::Null;
        nulled["through_wall_normal"] = serde_json::Value::Null;
        let nulled: Door = serde_json::from_value(nulled).unwrap();
        assert!(nulled.insertion_point.is_none());
        assert!(nulled.through_wall_normal.is_none());
    }

    /// The normal is carried as authored and **not** re-normalised or rejected
    /// on the way in. The producer normalises; the contract's job is to deliver
    /// what it was given, so a consumer that needs a unit vector can normalise
    /// defensively and one that only needs a direction pays nothing.
    ///
    /// Pinned as a test because "the server quietly fixed it up" and "the
    /// producer sent it right" are indistinguishable from the outside until
    /// something depends on the difference.
    #[test]
    fn test_normal_is_carried_verbatim() {
        let json = serde_json::json!({
            "id": "d1",
            "level_id": "lvl1",
            "loops": [],
            "type_id": "t1",
            "type_name": "Single",
            "through_wall_normal": { "x": 3.0, "y": 4.0 }
        });

        let door: Door = serde_json::from_value(json).unwrap();
        let n = door.through_wall_normal.as_ref().unwrap();
        assert_eq!((n.x, n.y), (3.0, 4.0));
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
