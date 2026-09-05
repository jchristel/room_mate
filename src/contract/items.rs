//! The **item** record: one placed FF&E family instance.
//!
//! A sibling of [`Opening`](super::openings::Opening), not a widening of it, and
//! the distinction is the whole reason this file exists. Windows shared the door
//! record because measurement showed the two are structurally identical. An item
//! is not: measured against a real duHast export on 2026-09-05, a door carries
//! `polygon`, `from_room`/`to_room`, `room_calculation_point` and Z extents; an
//! item carries `location_point` and a phase-unioned `rooms` list, and neither
//! record is a subset of the other.
//!
//! The line that settled every windows question was *share it unless sharing
//! would change a serde key*. It does not reach here, and the honest extension
//! is **share it unless sharing would make a field mean nothing**. Modelling an
//! item as a one-sided `Opening` was the tempting move: it would carry a
//! `through_wall_normal` describing nothing, a `from_room` permanently `None`,
//! and — because `OpeningReport::external` counts "a room on exactly one side" —
//! it would report every item in the model as an external opening. A field that
//! is right in shape and wrong in meaning is worse than a new type.
//!
//! **What is NOT duplicated is the part that matters.** The upload envelope
//! (`Project`, `Model`, `Snapshot`, `ModelToShared`), the geometry primitives
//! (`Loop`, `Point2D`), the property machinery (`CustomValue`, `PropertyTiers`,
//! `lookup_property`) and every identity rule are used from `mod.rs`. A fourth
//! entity needs no new identity concepts at all, which is the claim
//! [Entities](../../docs/STRATEGY-ENTITIES.md) makes and this is the second test
//! of it.
//!
//! Figures throughout cite the House A probe (`docs/PLAN-ffe.md`,
//! "As measured"): 647 instances collected across nine categories, 644 exported.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{CustomValue, Loop, Point2D, PropertyTiers};

/// One FF&E instance, as extracted from Revit.
///
/// **An item sits IN one room; an opening sits BETWEEN two.** That single
/// difference produces the three that matter here — one `room` rather than a
/// pair, `facing` rather than `through_wall_normal`, and no attribution policy
/// to choose between sides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    /// The Revit `ElementId`, as a string on the wire like every other id here.
    ///
    /// Sourced from the export's `instance_properties.id`, the same place
    /// `translate_room` and `translate_opening` read theirs, so all four
    /// entities' ids come from one convention rather than four. Nothing else in
    /// the record identifies the instance: `Mark` is carried by only 52% of
    /// top-level items and 5.6% of components, so it is a label, not identity.
    pub id: String,

    /// The level this item sits on, keyed the same way `Room.level_id` is —
    /// and, like it, unique only *within* one model.
    ///
    /// **`"-1"` is a real value and must not be filtered out.** Revit gives an
    /// unhosted element an invalid `LevelId`, and duHast's item level is
    /// *derived* rather than read — the last level at or below the instance's
    /// solid bounding box, with no level at all when the family holds no solids.
    /// 71 of 644 items in the House A export carry `-1` on exactly those terms.
    /// The read side answers `Unresolved::UnknownLevel` for them, which says the
    /// probe never ran rather than that it ran and found open air.
    ///
    /// The level set these ids point into normally comes from the model's
    /// **rooms** snapshot, which for FF&E is the ordinary case rather than the
    /// lucky one: an item lives in the same document as the rooms it serves,
    /// which is the premise of the whole entity. `FfePayload::levels` is the
    /// fallback, and is far less load-bearing here than it is for windows.
    pub level_id: String,

    /// The Revit category this instance was collected under, e.g.
    /// `"OST_Furniture"`.
    ///
    /// **On the record even though it is not in the export.** `DataItem` has no
    /// category field, despite `get_all_item_data` walking nine of them and
    /// discarding which one each instance came from — so this is the one field
    /// the extractor supplies from its own collector pass rather than reading.
    /// That is a third rule beside "read the geometry from the export" and "read
    /// the room from Revit", and it is not an exception to either: the export
    /// does not carry it at all, so this fills a gap rather than choosing
    /// between two answers.
    ///
    /// Carried because a consumer's first question about FF&E is almost always
    /// "which kind" — `?category=` filters on it through the existing grammar,
    /// and QA breaks findings down by it. The *selection* of which categories to
    /// push stays the exporter's business; this is the wire being able to say
    /// what arrived. On House A `OST_GenericModel` was the largest of the nine
    /// at 201 of 647, which is precisely the number a reader needs and could not
    /// otherwise get.
    pub category: String,

    /// The room this item is in, or `None`.
    ///
    /// **One room, not two sides and not a list.** An item sits in one room, so
    /// there is nothing for an attribution policy to choose between — which is
    /// why `[ffe]` carries no `room_attribution` while `[doors]` and
    /// `[windows]` must.
    ///
    /// Read by the extractor from `FamilyInstance.get_Room(phase)`, **never**
    /// from the export's own `rooms` field. That field is a list unioned across
    /// every phase with nothing saying which entry belongs to which — the
    /// `phase_id` trap for the third consecutive entity — and its third fallback
    /// is `doc.GetRoomAtPoint`, a geometric answer computed producer-side where
    /// the server can no longer report it as derived. An authored reference and
    /// a geometric one must stay distinguishable, which is what `SideOrigin`
    /// exists for.
    ///
    /// `None` is an ordinary state, not a defect: an item in a corridor void, in
    /// a shaft, or outside the building envelope has no room. Measured on House
    /// A, 572 of 647 named a room in the pushed phase and 0 did in the phase
    /// beside it — which is what a phase-agnostic union would have blurred.
    ///
    /// The value is a `Room.id` **in the same model**. Room ids are unique only
    /// within a model, so QA resolves it per model and never project-wide.
    #[serde(default)]
    pub room: Option<String>,

    /// Where the item sits, in the same space as `loops` — decimal feet, model
    /// space, Y up.
    ///
    /// **Converted at the producer boundary, because the export states it in
    /// millimetres.** `DataItem.location_point.translation_coord` goes through
    /// duHast's `convert_XYZ_to_point3`, while the polygon path does not
    /// convert at all — so one exported record holds a footprint in feet and a
    /// position in mm. Measured rather than assumed: 643 House A instances
    /// measured twice give a median ratio of 304.8 exactly.
    ///
    /// **The one thing every placed item has, whatever else is missing.**
    /// `loops` may be empty; the insertion point is what an item with no
    /// measurable footprint can still be drawn at, which is the difference
    /// between "we do not know its shape" and "it silently is not there".
    ///
    /// `Option` is not the producer hedging — duHast refuses any instance whose
    /// `Location` is not a `LocationPoint`, so a pushed item always has one. It
    /// is because every stored snapshot re-parses through this type at boot, the
    /// same permissive-type / strict-producer split every payload here uses.
    #[serde(default)]
    pub insertion_point: Option<Point2D>,

    /// A unit vector along the family's own X axis, projected to plan — which
    /// way the item faces.
    ///
    /// **Read from the export, not re-derived.** `location_point.rotation_coord`
    /// is a 3×3 matrix whose rows are the instance's basis vectors in world
    /// space; this is row 0 (BasisX), flattened and normalised. The extractor
    /// could ask Revit for `FacingOrientation` instead — and must not, because
    /// the export carries the answer and an extractor that computes its own
    /// silently discards what the producer sent. That rule cost two failed
    /// attempts to learn on the door footprint.
    ///
    /// **Not the same claim `Opening::through_wall_normal` makes.** That vector
    /// is a *through-wall* direction with a from-side and a to-side, and the
    /// whole subtlety about a cupboard door serving one room while opening into
    /// another. An item has no wall to pass through and one room; this is simply
    /// its orientation, which a symbol renderer needs because a rectangle does
    /// not say which end is the front.
    ///
    /// Absent when the facing has no plan component (an item whose local X
    /// points along Z), and for a snapshot pushed before this field existed. A
    /// consumer degrades rather than guessing: draw the footprint, omit the
    /// orientation. A guessed direction is worse than none, because nothing
    /// downstream can tell it from a measured one.
    #[serde(default)]
    pub facing: Option<Point2D>,

    /// The item's footprint, in the **room convention verbatim**: `loops[0]` is
    /// the outer loop, `loops[1..]` are holes, points are decimal feet in model
    /// space, Y up.
    ///
    /// Identical to `Room::loops` and `Opening::loops` on purpose. A second
    /// geometry convention would fork every consumer that draws or transforms
    /// geometry, and there is nothing about a desk that needs one — which is why
    /// duHast flattens the instance's *oriented* bounding box through the same
    /// three calls `to_data_door` makes, rather than sending local-frame extents
    /// for the consumer to compose against `facing`.
    ///
    /// **Empty is a real state and was the ONLY state until upstream change U1.**
    /// Every one of the 644 House A items arrived with no polygon, because
    /// `DataItem` had no geometry field at all; the record was designed against
    /// that and the read side must never assume otherwise. An item with no
    /// footprint still carries its room, its properties and its insertion point,
    /// so it is a real item QA and comparison must see — only its shape is
    /// unknown.
    #[serde(default)]
    pub loops: Vec<Loop>,

    /// The `ElementId` of the instance this one is a component of, or `None`.
    ///
    /// **On the wire so the exclusion can be a policy rather than a silence**,
    /// which is the one place FF&E deliberately does not follow doors.
    /// `nested_opening_ids` drops a door leaf at the producer, correctly: "is
    /// this door leaf a door" has one answer everywhere. "Is this component an
    /// item" does not — a joinery handle is not, a chair nested in a workstation
    /// group might be — so it is a project convention, and conventions live in
    /// settings and are applied at read time. See `[ffe] nested_components`.
    ///
    /// Measured on House A: 179 of 647 instances have one (27.7%), and the
    /// same-category test doors use would catch **10** of them, because an
    /// item's parent is usually in a *different* category — 87 furniture
    /// components of casework (one family, all joinery handles), 70 generic
    /// models inside electrical fixtures, 3 generic models inside doors.
    ///
    /// The doors-era discriminator does not transfer either, and that is the
    /// trap this field exists to avoid: a nested door component carried neither
    /// a room reference nor a `Mark`, but a nested item sits physically in a
    /// room and Revit says so — 97.8% of components name a room against 84.8%
    /// of top-level items. Only `Mark` still separates them (5.6% against
    /// 52.4%), and suggestively rather than safely. This id is the one reliable
    /// discriminator.
    ///
    /// duHast writes `-1` for "no super component"; the extractor maps that to
    /// `None` so the sentinel does not reach any consumer.
    #[serde(default)]
    pub super_component_id: Option<String>,

    /// The `ElementId` of the item's family type, as a string.
    ///
    /// Carried because `type_properties` below is a *shared* tier — every
    /// instance of one type repeats it — and this is the identity of the thing
    /// being shared. It is what a future type-property deduplication would key a
    /// shared table on, and FF&E is what makes that optimisation worth
    /// measuring: House A's doors snapshot is 414 KB for 26 doors, and this
    /// entity ships hundreds per model.
    pub type_id: String,

    /// The family type's display name (e.g. `"Desk 1600x800"`).
    ///
    /// A display label, never a key — the same split `Project.name` and
    /// `Model.name` have from their ids, and for the same reason: a type rename
    /// in Revit must not fork anything.
    pub type_name: String,

    /// This instance's own properties, keyed by the *source's own* property
    /// name, exactly as `Room.properties` and `Opening::properties` are.
    #[serde(default)]
    pub properties: BTreeMap<String, CustomValue>,

    /// The family **type's** properties — shared by every instance of
    /// `type_id`, and kept as a separate map rather than merged into
    /// `properties`.
    ///
    /// The same two-tier split an opening has, and it survives unchanged here
    /// because the export produces both through the very same
    /// `get_instance_properties` / `get_type_properties` helpers. That is why
    /// `post_common.properties_to_map` works on an item with no modification,
    /// and it is the single largest thing this entity got for free.
    #[serde(default)]
    pub type_properties: BTreeMap<String, CustomValue>,
}

/// An item is two-tier: its own properties first, its family type's second.
///
/// The precedence rule itself — that a tier wins only when it is `Present`, so
/// a *blank* instance parameter does not shadow a real type value — lives on
/// `property_presence` in `mod.rs`. This impl only declares the order, and
/// declares the same one an `Opening` does, because the question is the same
/// question.
impl PropertyTiers for Item {
    fn tiers(&self) -> Vec<&BTreeMap<String, CustomValue>> {
        vec![&self.properties, &self.type_properties]
    }
}

/// What the read side needs from a stored FF&E snapshot.
///
/// The `Opening` counterpart is `OpeningEnvelope`, and this is deliberately a
/// separate trait rather than a shared one over both records. They answer the
/// same five questions, but the sixth — what the element list holds — has a
/// different element type, and a trait generic over that would push the entity
/// back into every signature the split exists to keep clean.
///
/// Kept deliberately narrow, on the terms `OpeningEnvelope` states: every method
/// here is a field read the assembly already did, and the trait exists to hide
/// *which struct* the field came from, not to grow behaviour.
pub trait ItemEnvelope: super::SnapshotEnvelope {
    /// The elements themselves, and the only method this trait adds. The five
    /// facts an FF&E snapshot shares with every other entity's live on
    /// [`SnapshotEnvelope`](super::SnapshotEnvelope), which is what lets one
    /// scoping pipeline serve all four.
    fn items(&self) -> &[Item];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{lookup_property, PropertyPresence};

    fn value(v: &str) -> CustomValue {
        CustomValue { value: v.to_string(), storage_type: None }
    }

    /// The record round-trips with a room, an orientation and both property
    /// tiers -- the wire shape `/ffe` answers with.
    #[test]
    fn test_item_round_trips() {
        let json = serde_json::json!({
            "id": "2618113",
            "level_id": "290501",
            "category": "OST_Furniture",
            "room": "2621156",
            "insertion_point": { "x": 33.91, "y": 43.04 },
            "facing": { "x": 1.0, "y": 0.0 },
            "loops": [{ "points": [{ "x": 0.0, "y": 0.0 }, { "x": 5.2, "y": 0.0 }] }],
            "super_component_id": null,
            "type_id": "t1",
            "type_name": "Desk 1600x800",
            "properties": { "Mark": { "value": "F-101", "storage_type": "String" } },
            "type_properties": { "Depth": { "value": "800" } }
        });

        let item: Item = serde_json::from_value(json).unwrap();
        assert_eq!(item.id, "2618113");
        assert_eq!(item.category, "OST_Furniture");
        assert_eq!(item.room.as_deref(), Some("2621156"));
        assert_eq!(item.super_component_id, None);
        assert_eq!(item.loops[0].points.len(), 2);
    }

    /// **An item with no footprint parses, because that was every item.** All
    /// 644 House A items arrived with no polygon: `DataItem` had no geometry
    /// field until upstream change U1, so a contract that required one would
    /// have rejected the only export that existed when it was written.
    #[test]
    fn test_an_item_with_no_footprint_is_valid() {
        let json = serde_json::json!({
            "id": "1", "level_id": "290501", "category": "OST_GenericModel",
            "type_id": "t", "type_name": "Bollard",
            "insertion_point": { "x": 1.0, "y": 2.0 }
        });

        let item: Item = serde_json::from_value(json).unwrap();
        assert!(item.loops.is_empty(), "an unmeasured footprint is empty, not absent");
        assert!(item.facing.is_none());
        assert_eq!(item.room, None, "an item outside every room is ordinary, not broken");
    }

    /// `"-1"` is a level id like any other on the wire. Revit gives an unhosted
    /// element an invalid `LevelId` and duHast derives the item's level from
    /// solid geometry a family may not have -- 71 of 644 on House A. Filtering
    /// it here would turn a reportable state into a missing field.
    #[test]
    fn test_an_unhosted_item_keeps_its_minus_one_level() {
        let json = serde_json::json!({
            "id": "1", "level_id": "-1", "category": "OST_SpecialityEquipment",
            "type_id": "t", "type_name": "Hoist"
        });

        let item: Item = serde_json::from_value(json).unwrap();
        assert_eq!(item.level_id, "-1");
    }

    /// A component names its parent, which is the only reliable way to tell one
    /// from a first-class item -- see the field doc for why the doors tests
    /// (same category, no room, no Mark) all fail here.
    #[test]
    fn test_a_component_names_its_parent() {
        let json = serde_json::json!({
            "id": "2", "level_id": "1483", "category": "OST_Furniture",
            "room": "2621156",
            "super_component_id": "3729614",
            "type_id": "t", "type_name": "Handle_Joinery_FIJO_900"
        });

        let item: Item = serde_json::from_value(json).unwrap();
        assert_eq!(item.super_component_id.as_deref(), Some("3729614"));
        assert!(
            item.room.is_some(),
            "a component sits in a room and Revit says so -- 97.8% of them do, \
             which is why 'no room reference' cannot be the filter"
        );
    }

    /// The two tiers resolve with instance-then-type precedence, and a BLANK
    /// instance value does not shadow a real type value -- the same rule an
    /// opening obeys, asserted here so the shared `lookup_property` cannot
    /// silently stop applying to this record.
    #[test]
    fn test_item_property_tiers_resolve_instance_then_type() {
        let item = Item {
            id: "1".into(),
            level_id: "1483".into(),
            category: "OST_Furniture".into(),
            room: None,
            insertion_point: None,
            facing: None,
            loops: vec![],
            super_component_id: None,
            type_id: "t".into(),
            type_name: "Desk".into(),
            properties: BTreeMap::from([("Mark".to_string(), value("F-101")), ("Depth".to_string(), value(""))]),
            type_properties: BTreeMap::from([
                ("Depth".to_string(), value("800")),
                ("Manufacturer".to_string(), value("Acme")),
            ]),
        };

        assert_eq!(lookup_property(&item, "Mark", "revit", &[]), Some("F-101".to_string()));
        assert_eq!(
            lookup_property(&item, "Depth", "revit", &[]),
            Some("800".to_string()),
            "a blank instance value must not shadow a real type value"
        );
        assert_eq!(lookup_property(&item, "Manufacturer", "revit", &[]), Some("Acme".to_string()));
        assert!(matches!(
            crate::contract::property_presence(&item, "Nothing", "revit", &[]),
            PropertyPresence::Absent
        ));
    }
}
