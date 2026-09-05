//! The **opening** record: the one element shape a door and a window share.
//!
//! Split out of `doors.rs` once measurement settled that they share it. A
//! full-depth key-path diff of real duHast exports from two models -- a house
//! and a facade file, 206 windows against 217 doors -- found zero structural
//! differences in either direction. Same fields, same meanings, same geometry
//! convention, so one type models both and neither entity owns it.
//!
//! **What lives here is the record and nothing else.** The upload envelope, the
//! schema version and the stored-snapshot shape stay per entity, in `doors.rs`
//! and later `windows.rs`, because those genuinely differ: a stored doors
//! snapshot keys its element list `doors` on disk, and the doors contract is at
//! v2 while windows start at v1. Sharing the record while keeping the envelopes
//! apart is the whole design -- see the "what generalizes" list in
//! [Entities](../../docs/STRATEGY-ENTITIES.md).
//!
//! The shared envelope (`Project`, `Model`, `Snapshot`, `ModelToShared`), the
//! geometry primitives (`Loop`, `Point2D`) and the property machinery
//! (`CustomValue`, `PropertyTiers`, `lookup_property`) all stay in `mod.rs`: an
//! opening does not get its own copy of any of them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{CustomValue, Loop, Point2D, PropertyTiers};

/// One door instance, as extracted from Revit.
///
/// **An opening is not a `Room` with different fields.** The two differences
/// that drive everything else here are the second property tier (below) and the
/// room references (below that); everything they share — geometry convention,
/// identity discipline, the upload envelope — is deliberately identical, so one
/// renderer and one `model_to_shared` transform serve every entity.
///
/// **A door and a window are the same record, and that is measured rather than
/// assumed.** duHast's two exporters read alike, but so did several things that
/// turned out to be wrong (the `±1e30` sentinel, the axis-aligned footprint, the
/// unresolvable `phase_id`), so the claim was checked against real exports from
/// two documents: a house and a facade file, 206 windows against 217 doors, all
/// captured in one pass with one duHast. A full-depth key-path diff found zero
/// differences in either direction, and every field below was populated for
/// both.
///
/// The examples throughout these comments still cite doors, deliberately: they
/// are the cases that *decided* each rule, and replacing a measured door figure
/// with a vaguer statement about openings in general would trade evidence for
/// symmetry. Where windows differ in DEGREE rather than in kind — far more of
/// them carry no room reference at all — it is said at the field concerned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opening {
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
    /// The set these ids point into normally comes from the model's **rooms**
    /// snapshot, and that stays the preferred source — one level list per model,
    /// no chance of two disagreeing.
    ///
    /// `DoorPayload::levels` is the fallback, and exists for the one case the
    /// rooms snapshot cannot cover: a model that pushes doors and no rooms at
    /// all. Read that field for why, and note the ordering — a rooms snapshot
    /// wins wherever both exist, so the duplicate this comment used to warn
    /// about cannot become a disagreement, only a redundancy.
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
    /// **For windows the absent case is not the exception, and in a facade model
    /// it is everything.** Measured 2026-09-03: in a facade file that links its
    /// interiors, **zero** of 158 windows and **zero** of 191 doors carried a
    /// reference on either side. That is structural rather than sloppy —
    /// `FamilyInstance.FromRoom[phase]` only sees the host document, so an
    /// opening in a facade file *cannot* name a room that lives in a link. It is
    /// why `[…] room_resolution` is not an optimisation for such a model but the
    /// only mechanism by which any opening in it is ever attributed, and why a
    /// QA report that phrased "no room on either side" as an anomaly would drown
    /// a legitimate model in findings.
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
    /// question moved to `opening_report`, which can re-answer it every time the
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
impl PropertyTiers for Opening {
    fn tiers(&self) -> Vec<&BTreeMap<String, CustomValue>> {
        vec![&self.properties, &self.type_properties]
    }
}

/// What the read side needs from a stored snapshot, whichever entity's envelope
/// carried it.
///
/// **This is the seam that lets one assembly serve doors and windows.** The
/// envelopes stay separate — `DoorPayload` names its list `doors` and
/// `WindowPayload` names its `windows`, because that key is baked into every
/// snapshot already written — but nothing downstream of ingest cares what the
/// key was called. It cares about five facts, and they are identical.
///
/// Kept deliberately narrow. Every method here is a field read the assembly
/// already did; the trait exists to hide *which struct* the field came from, not
/// to grow behaviour. Widening it would start pulling per-entity decisions back
/// into a shared abstraction, which is the direction the envelope split exists
/// to prevent — `project`/`model` are here only because scoping and the model
/// key need them, and anything an entity does differently belongs at the call
/// site with a `SnapshotKind` beside it, not behind another method.
pub trait OpeningEnvelope: super::SnapshotEnvelope {
    /// The elements themselves. The one method whose *implementation* differs
    /// between entities, and it differs only in the field name it reads.
    ///
    /// **The other five moved up to [`SnapshotEnvelope`](super::SnapshotEnvelope)**
    /// when FF&E arrived, because they turned out to be facts about a stored
    /// snapshot rather than facts about an opening -- and an `Item` snapshot
    /// answers every one of them identically. What is left here is the only
    /// thing that made this trait an *opening* trait in the first place.
    fn openings(&self) -> &[Opening];
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let old: Opening = serde_json::from_value(base.clone()).unwrap();
        assert!(old.insertion_point.is_none());
        assert!(old.through_wall_normal.is_none());

        // Placed, but no readable direction — the "rectangle/cross without an
        // arrow" case. Never a reason to synthesise one.
        let mut placed = base.clone();
        placed["insertion_point"] = serde_json::json!({ "x": 1.5, "y": -2.5 });
        let placed: Opening = serde_json::from_value(placed).unwrap();
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
        let nulled: Opening = serde_json::from_value(nulled).unwrap();
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

        let door: Opening = serde_json::from_value(json).unwrap();
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

        let neither: Opening = serde_json::from_value(base.clone()).unwrap();
        assert!(neither.from_room.is_none() && neither.to_room.is_none());

        let mut external = base.clone();
        external["to_room"] = serde_json::json!("r1");
        let external: Opening = serde_json::from_value(external).unwrap();
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
        let door: Opening = serde_json::from_value(serde_json::json!({
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
        let door: Opening = serde_json::from_value(serde_json::json!({
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
}
