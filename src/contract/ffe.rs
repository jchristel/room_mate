//! The **FF&E envelope**: how a push, a stored snapshot and a stream frame of
//! `Item`s are shaped.
//!
//! Structurally the same file `doors.rs` and `windows.rs` are, and for once the
//! resemblance is the *cheap* part rather than the interesting one. What made
//! `windows.rs` a near-copy of `doors.rs` was a deliberate refusal to share the
//! envelope — the element key and the schema version are baked into bytes
//! already on disk, so merging them would be a migration rather than a refactor.
//! The same argument applies here without needing to be re-made, and this file
//! is where it stops being an argument and becomes a pattern: **an entity's
//! envelope is per entity, its record is shared only where measurement says the
//! records are the same, and everything between them is written once.**
//!
//! For FF&E the record is *not* shared (see [`items`](super::items)), so this
//! envelope and that record are both new, and the whole rest of the pipeline is
//! not. That is the fourth-entity cost stated exactly:
//! `ensure_taken_at`/`validate_snapshot_id`, the phase rules, `Project`,
//! `Model`, `Snapshot`, `ModelToShared`, `Level`, the store, the manifest and
//! the property machinery are all used from where they already live.

use serde::{Deserialize, Serialize};

use super::items::Item;
use super::{Level, Model, ModelToShared, Project, Snapshot};

/// One timestamped push of one model's FF&E.
///
/// Carries the **same upload envelope** as `RoomPayload`, `DoorPayload` and
/// `WindowPayload` — `project`, `model`, `snapshot`, `phase` — resolved through
/// the same functions rather than reimplementations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FfePayload {
    /// **FF&E versions independently of the other three** — see
    /// `SUPPORTED_FFE_SCHEMA`.
    pub schema_version: u32,

    pub project: Project,
    pub model: Model,
    #[serde(default)]
    pub snapshot: Snapshot,

    /// The Revit phase this push was filtered to, with the same
    /// `Option`-on-the-type, required-at-ingest split every payload here uses:
    /// the type stays permissive because every stored snapshot re-parses through
    /// it at boot, and the handler is strict.
    ///
    /// **An FF&E push that disagrees with the lineage is refused, not
    /// quarantined** — the doors rule, for the doors reason. Promoting it would
    /// move the lineage while every room snapshot stayed on the old phase,
    /// stranding the rooms `Item::room` resolves against. An *unphased* lineage
    /// is still phased by whichever push reaches it first.
    ///
    /// Items are filtered by the **range** test doors and windows use
    /// (`elements_in_phase`), not the equality test rooms use: an item is placed
    /// in one phase and may be demolished in a later one, so it exists across a
    /// span. Running items through the room predicate returns nothing, silently
    /// — the failure that cost five empty pushes to find, and the reason this is
    /// written down for a fourth time rather than assumed to be common
    /// knowledge.
    #[serde(default)]
    pub phase: Option<String>,

    /// Model→shared placement, in lockstep with the other three payloads.
    ///
    /// Carried on the same terms and for the same reason: an item footprint is
    /// geometry in the same model space as the rooms', every geometry payload
    /// here carries its own placement, and a payload that omitted it would be
    /// the odd one out. **Per push, never per item.**
    #[serde(default)]
    pub model_to_shared: Option<ModelToShared>,

    /// The level set `Item::level_id` points into, for a model that has no rooms
    /// snapshot to supply one.
    ///
    /// **Deliberately less load-bearing here than it is for windows, and the
    /// difference is the premise of the entity.** A facade model holds windows
    /// and no rooms, so without this list every window in it is unreachable
    /// rather than merely unresolved. FF&E lives in the same document as the
    /// rooms it serves — that is *why* RoomMate performs this join, since Revit
    /// will not schedule it — so the rooms snapshot normally supplies the
    /// levels and this list is redundant.
    ///
    /// It is carried anyway, because "normally" is not "always": the buckets are
    /// independent and an FF&E push may legitimately arrive before its rooms.
    /// Empty stays legal and stays ordinary; the rooms snapshot's copy wins
    /// wherever both exist, so sending it twice is a redundancy rather than a
    /// disagreement.
    #[serde(default)]
    pub levels: Vec<Level>,

    pub ffe: Vec<Item>,
}

/// One model's block on a multi-model FF&E upload.
///
/// No `room_boundary`: that is a rooms fact this contract has no key for.
/// `levels` IS carried, on the terms `FfePayload::levels` states.
#[derive(Debug, Clone, Deserialize)]
pub struct FfeModelEnvelope {
    #[serde(flatten)]
    pub model: Model,
    #[serde(default)]
    pub model_to_shared: Option<ModelToShared>,
    #[serde(default)]
    pub levels: Vec<Level>,
}

impl FfeModelEnvelope {
    /// Rebuild the single-model `FfePayload` this block plus the run's shared
    /// envelope describes — see `RoomModelEnvelope::into_payload`, which this
    /// mirrors and which explains why a push decomposes at all.
    pub fn into_payload(
        self,
        schema_version: u32,
        project: Project,
        snapshot: Snapshot,
        phase: Option<String>,
        ffe: Vec<Item>,
    ) -> FfePayload {
        FfePayload {
            schema_version,
            project,
            model: self.model,
            snapshot,
            phase,
            model_to_shared: self.model_to_shared,
            levels: self.levels,
            ffe,
        }
    }
}

/// The first NDJSON line of a streamed FF&E push (`POST /ffe/stream`): the run's
/// shared identity plus one `FfeModelEnvelope` per model, with every item
/// arriving on a following line as a `StreamItem`.
///
/// Its own type rather than making `ffe` optional on `FfePayload`, for the
/// reason `StreamEnvelope` gives: the envelope must deserialize alone with no
/// items present, and `FfePayload` must keep `ffe` guaranteed for every other
/// consumer.
///
/// **Streaming matters more for this entity than for any other except rooms.**
/// An opening push is tens of elements per model; House A alone holds 644 items
/// across nine categories, and that is a house.
#[derive(Debug, Clone, Deserialize)]
pub struct FfeStreamEnvelope {
    pub schema_version: u32,
    pub project: Project,
    #[serde(default)]
    pub snapshot: Snapshot,
    #[serde(default)]
    pub phase: Option<String>,
    pub models: Vec<FfeModelEnvelope>,
}

/// One item line of a streamed push: the item, plus the id of the model it
/// belongs to. See `StreamRoom` for why the id rides every element rather than
/// a grouping marker.
#[derive(Debug, Clone, Deserialize)]
pub struct StreamItem {
    pub model_id: String,
    #[serde(flatten)]
    pub item: Item,
}

/// The buffered multi-model FF&E upload (`POST /ffe`).
#[derive(Debug, Clone, Deserialize)]
pub struct FfeUpload {
    pub schema_version: u32,
    pub project: Project,
    #[serde(default)]
    pub snapshot: Snapshot,
    #[serde(default)]
    pub phase: Option<String>,
    pub models: Vec<FfeModelUpload>,
}

/// One model's block on a buffered FF&E upload: its `FfeModelEnvelope` plus its
/// items.
#[derive(Debug, Clone, Deserialize)]
pub struct FfeModelUpload {
    #[serde(flatten)]
    pub envelope: FfeModelEnvelope,
    pub ffe: Vec<Item>,
}

/// FF&E schema version this server accepts. **Starts at 1, and moves
/// independently of the other three.**
///
/// The same argument windows made against starting at 2 to match doors: a
/// version number records *a contract's own* history, and this contract has
/// none. Numbering it to signal "same generation as the others" would claim a
/// predecessor that never existed and leave a permanent gap for the next reader
/// to hunt. Four entities, four version lines.
pub const SUPPORTED_FFE_SCHEMA: u32 = 1;

impl super::items::ItemEnvelope for FfePayload {
    fn project(&self) -> &Project {
        &self.project
    }
    fn model(&self) -> &Model {
        &self.model
    }
    fn taken_at(&self) -> &str {
        &self.snapshot.taken_at
    }
    fn phase(&self) -> Option<&str> {
        self.phase.as_deref()
    }
    fn model_to_shared(&self) -> Option<&ModelToShared> {
        self.model_to_shared.as_ref()
    }
    fn levels(&self) -> &[Level] {
        &self.levels
    }
    fn items(&self) -> &[Item] {
        &self.ffe
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire shape, end to end: a v1 FF&E payload round-trips with the room,
    /// the category, both property tiers and the orientation intact.
    #[test]
    fn test_ffe_payload_round_trips() {
        let json = serde_json::json!({
            "schema_version": 1,
            "project":  { "id": "House A", "name": "House A" },
            "model":    { "id": "bf", "name": "Building BF", "source": "revit" },
            "snapshot": { "taken_at": "2026-09-05T00:00:00Z" },
            "phase": "New Construction",
            "ffe": [{
                "id": "2618113",
                "level_id": "290501",
                "category": "OST_Furniture",
                "room": "2621156",
                "insertion_point": { "x": 33.91, "y": 43.04 },
                "facing": { "x": 0.0, "y": -1.0 },
                "type_id": "t1",
                "type_name": "Desk 1600x800",
                "properties": { "Mark": { "value": "F-101", "storage_type": "String" } },
                "type_properties": { "Depth": { "value": "800" } }
            }]
        });

        let payload: FfePayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.schema_version, SUPPORTED_FFE_SCHEMA);
        assert_eq!(payload.ffe.len(), 1);
        assert_eq!(payload.ffe[0].category, "OST_Furniture");
        assert_eq!(payload.ffe[0].room.as_deref(), Some("2621156"));
        assert_eq!(payload.phase.as_deref(), Some("New Construction"));
        assert!(payload.ffe[0].loops.is_empty(), "no footprint until upstream change U1");
    }

    /// An item naming no room parses. Unlike the facade-window case, this is a
    /// minority state rather than the whole model -- 75 of 647 on House A -- but
    /// it is ordinary: an item in a corridor void or outside the envelope has no
    /// room, and a contract that required one would reject a legitimate model.
    #[test]
    fn test_an_item_with_no_room_is_valid() {
        let json = serde_json::json!({
            "schema_version": 1,
            "project":  { "id": "p", "name": "P" },
            "model":    { "id": "m", "name": "M", "source": "revit" },
            "snapshot": { "taken_at": "2026-09-05T00:00:00Z" },
            "ffe": [{
                "id": "1", "level_id": "-1", "category": "OST_GenericModel",
                "type_id": "t", "type_name": "Bollard"
            }]
        });

        let payload: FfePayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.ffe[0].room, None);
        assert_eq!(payload.ffe[0].level_id, "-1", "an unhosted item keeps its sentinel level");
    }

    /// `levels` is optional and additive here exactly as it is on doors and
    /// windows, so a producer that does not send it still parses -- which for
    /// FF&E is the ordinary case, since its rooms live in the same document.
    #[test]
    fn test_levels_are_optional_on_an_ffe_payload() {
        let json = serde_json::json!({
            "schema_version": 1,
            "project":  { "id": "p", "name": "P" },
            "model":    { "id": "m", "name": "M", "source": "revit" },
            "snapshot": { "taken_at": "2026-09-05T00:00:00Z" },
            "ffe": []
        });

        let payload: FfePayload = serde_json::from_value(json).unwrap();
        assert!(payload.levels.is_empty());
        assert!(payload.ffe.is_empty(), "an empty item list is legal here; the producer refuses it");
    }

    /// The stream envelope must deserialize with **no items present** -- that is
    /// the whole reason it is a separate type from `FfePayload`.
    #[test]
    fn test_ffe_stream_envelope_deserializes_without_items() {
        let json = serde_json::json!({
            "schema_version": 1,
            "project":  { "id": "p", "name": "P" },
            "snapshot": { "taken_at": "2026-09-05T00:00:00Z" },
            "phase": "New Construction",
            "models": [{ "id": "bf", "name": "Building BF", "source": "revit" }]
        });

        let envelope: FfeStreamEnvelope = serde_json::from_value(json).unwrap();
        assert_eq!(envelope.models.len(), 1);
        assert_eq!(envelope.models[0].model.id, "bf");
        assert!(envelope.models[0].levels.is_empty());
    }

    /// Every streamed item names its own model, so a multi-model push needs no
    /// grouping marker in the stream.
    #[test]
    fn test_stream_item_carries_its_model_id() {
        let json = serde_json::json!({
            "model_id": "bf",
            "id": "1", "level_id": "290501", "category": "OST_PlumbingFixtures",
            "type_id": "t", "type_name": "WHB"
        });

        let line: StreamItem = serde_json::from_value(json).unwrap();
        assert_eq!(line.model_id, "bf");
        assert_eq!(line.item.id, "1");
        assert_eq!(line.item.category, "OST_PlumbingFixtures");
    }

    /// A component round-trips with its parent id, which is what makes
    /// `[ffe] nested_components` a read-time policy instead of a producer-side
    /// filter that nothing downstream could see or change.
    #[test]
    fn test_a_component_reaches_the_server() {
        let json = serde_json::json!({
            "schema_version": 1,
            "project":  { "id": "p", "name": "P" },
            "model":    { "id": "m", "name": "M", "source": "revit" },
            "snapshot": { "taken_at": "2026-09-05T00:00:00Z" },
            "ffe": [{
                "id": "2", "level_id": "1483", "category": "OST_Furniture",
                "room": "r1", "super_component_id": "3729614",
                "type_id": "t", "type_name": "Handle_Joinery_FIJO_900"
            }]
        });

        let payload: FfePayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.ffe[0].super_component_id.as_deref(), Some("3729614"));
    }

    /// The four version lines are independent, asserted rather than argued so
    /// that a future bump to one cannot quietly drag the others.
    #[test]
    fn test_ffe_schema_is_independent_of_the_other_three() {
        assert_eq!(SUPPORTED_FFE_SCHEMA, 1);
        assert_ne!(SUPPORTED_FFE_SCHEMA, super::super::SUPPORTED_SCHEMA);
        assert_ne!(SUPPORTED_FFE_SCHEMA, super::super::SUPPORTED_DOOR_SCHEMA);
    }
}
