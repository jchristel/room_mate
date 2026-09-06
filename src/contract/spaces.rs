//! The **spaces envelope**: how a push, a stored snapshot and a stream frame of
//! `Room`s are shaped when the entity is spaces.
//!
//! The fifth entity, and the first whose *record* is shared with rooms rather
//! than with openings. A Revit Space is a `SpatialElement`, the same base a Room
//! is: duHast's `DataSpace` is `DataRoom` minus the ceiling and floor lists, its
//! export runs the same `get_2d_points_from_revit_room` over the element, and it
//! reads the same `ROOM_PHASE`. So `id`, `name`, `level_id`, `loops` and
//! `properties` mean for a space exactly what they mean for a room, and a space
//! has no type tier to leave permanently empty. That is the windows split
//! (`Opening` shared, envelopes separate), not the FF&E one.
//!
//! The envelope is still per entity, for the two reasons `windows.rs` states and
//! which do not need re-making: the element key (`spaces`, not `rooms`) and the
//! schema version are both baked into bytes on disk.
//!
//! **Two things here are NOT the rooms envelope, and both were measured rather
//! than assumed** (RHH, 2026-09-06; see `docs/Superseded/PLAN-spaces.md`):
//!
//! - **`levels` is optional.** A rooms payload requires it. A services model
//!   pushes spaces and usually nothing else, so its levels arrive here or not at
//!   all — but a model that pushes both sends them twice, and a redundant list
//!   is not a disagreement. Optional matches what a producer can promise.
//! - **An empty `spaces` list is legal, and that is the point of the entity.**
//!   `reject_empty_rooms` exists because a rooms push happens when someone
//!   exported a document that has rooms in it, so an empty one is a producer
//!   fault. Here the empty push is the *answer*: "this services model was
//!   audited and holds no spaces" is the finding the whole entity was built to
//!   report, and it is a different fact from "this model was never pushed".

use serde::{Deserialize, Serialize};

use super::{Level, Model, ModelToShared, Project, Room, RoomBoundary, Snapshot};

/// One timestamped push of one model's spaces.
///
/// Carries the **same upload envelope** as every other entity — `project`,
/// `model`, `snapshot`, `phase` — resolved through the same `ensure_taken_at` /
/// `validate_snapshot_id` / `normalize_phase` functions rather than
/// reimplementations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpacePayload {
    /// **Spaces version independently of the other four** — see
    /// `SUPPORTED_SPACE_SCHEMA`.
    pub schema_version: u32,

    pub project: Project,
    pub model: Model,
    #[serde(default)]
    pub snapshot: Snapshot,

    /// The Revit phase this push was filtered to, with the same
    /// `Option`-on-the-type, required-at-ingest split every other payload has:
    /// the type stays permissive because every stored snapshot re-parses through
    /// it at boot, and the handler is strict.
    ///
    /// **Spaces use the rooms EQUALITY test, not the openings range test.** A
    /// space belongs to one phase (`ROOM_PHASE`) exactly as a room does; it is
    /// not built in one phase and demolished in a later one the way a door is.
    /// Running spaces through `elements_in_phase` returns nothing, silently —
    /// the failure that cost five empty pushes to find, and which this entity is
    /// close enough to doors to re-earn.
    ///
    /// **A phase name is per document, and on a real project the disciplines do
    /// not agree on one.** Measured on RHH: the mechanical model keeps 1,532 of
    /// its 1,533 spaces in a phase called `Future`, while every other services
    /// model uses `New Construction` and the architects use `New Construction`
    /// and `Future Expansion`. Since the push phase is one per *run*
    /// (`choose_phase` offers only names common to every selected document), a
    /// run over all four services models under `New Construction` pushes three
    /// full models and **one** space from the fourth. Nothing here can detect
    /// that — it is a correctly filtered push of a phase that model barely uses —
    /// which is why the producer owes a per-model count in its summary.
    #[serde(default)]
    pub phase: Option<String>,

    /// Model→shared placement, in lockstep with every other payload. **Per push,
    /// never per space.**
    ///
    /// It matters more here than the field's own header suggests. A space and
    /// the room it names live in different documents by construction, so any
    /// future geometric confirmation between the two has to cross models — and
    /// that is the one thing `RoomResolution::Project` warns has never been
    /// checked against a real survey. Carried now so the data is there when that
    /// check is built; nothing in this entity reads it yet.
    #[serde(default)]
    pub model_to_shared: Option<ModelToShared>,

    /// Which boundary regime these spaces were drawn to (see `RoomBoundary`), on
    /// the same optional terms `RoomPayload::room_boundary` states.
    ///
    /// **Carried because it is the documented cause of the area differences this
    /// entity exists to report.** An MEP space computed to wall centre and an
    /// architectural room computed to finish face differ by half a wall
    /// thickness, which is not a data error — it is a difference in what was
    /// measured. On RHH that showed up as a median relative difference falling
    /// from 14.0% on rooms under 2 m2 to 0.7% on rooms over 100, because the
    /// delta scales with perimeter while the value scales with area. A reader
    /// tuning a tolerance needs to know which regime each side used.
    #[serde(default)]
    pub room_boundary: Option<RoomBoundary>,

    /// The level set `Room.level_id` points into, for a services model that has
    /// no rooms snapshot to supply one.
    ///
    /// Optional, unlike the rooms payload's — see this module's header. The
    /// *elevation* is what this is for, not the id: level ids are per-document
    /// and never match across models, elevations are what cross.
    #[serde(default)]
    pub levels: Vec<Level>,

    /// This model's spaces. **May legitimately be empty** — see the module
    /// header.
    pub spaces: Vec<Room>,
}

/// One model's block on a multi-model spaces upload.
///
/// Carries `room_boundary` where `WindowModelEnvelope` does not, because a space
/// has a boundary regime and an opening does not.
#[derive(Debug, Clone, Deserialize)]
pub struct SpaceModelEnvelope {
    #[serde(flatten)]
    pub model: Model,
    #[serde(default)]
    pub model_to_shared: Option<ModelToShared>,
    #[serde(default)]
    pub room_boundary: Option<RoomBoundary>,
    #[serde(default)]
    pub levels: Vec<Level>,
}

impl SpaceModelEnvelope {
    /// Rebuild the single-model `SpacePayload` this block plus the run's shared
    /// envelope describes — see `RoomModelEnvelope::into_payload`, which this
    /// mirrors and which explains why a push decomposes at all.
    pub fn into_payload(
        self,
        schema_version: u32,
        project: Project,
        snapshot: Snapshot,
        phase: Option<String>,
        spaces: Vec<Room>,
    ) -> SpacePayload {
        SpacePayload {
            schema_version,
            project,
            model: self.model,
            snapshot,
            phase,
            model_to_shared: self.model_to_shared,
            room_boundary: self.room_boundary,
            levels: self.levels,
            spaces,
        }
    }
}

/// The first NDJSON line of a streamed spaces push (`POST /spaces/stream`): the
/// run's shared identity plus one `SpaceModelEnvelope` per model, with every
/// space arriving on a following line as a `StreamSpace`.
///
/// Its own type rather than making `spaces` optional on `SpacePayload`, for the
/// reason `StreamEnvelope` gives: the envelope must deserialize alone with no
/// spaces present, and `SpacePayload` must keep `spaces` guaranteed for every
/// other consumer.
#[derive(Debug, Clone, Deserialize)]
pub struct SpaceStreamEnvelope {
    pub schema_version: u32,
    pub project: Project,
    #[serde(default)]
    pub snapshot: Snapshot,
    #[serde(default)]
    pub phase: Option<String>,
    pub models: Vec<SpaceModelEnvelope>,
}

/// One space line of a streamed push: the space, plus the id of the model it
/// belongs to. See `StreamRoom` for why the id rides every element rather than a
/// grouping marker.
#[derive(Debug, Clone, Deserialize)]
pub struct StreamSpace {
    pub model_id: String,
    #[serde(flatten)]
    pub space: Room,
}

/// The buffered multi-model spaces upload (`POST /spaces`).
#[derive(Debug, Clone, Deserialize)]
pub struct SpacesUpload {
    pub schema_version: u32,
    pub project: Project,
    #[serde(default)]
    pub snapshot: Snapshot,
    #[serde(default)]
    pub phase: Option<String>,
    pub models: Vec<SpaceModelUpload>,
}

/// One model's block on a buffered spaces upload: its `SpaceModelEnvelope` plus
/// its spaces.
#[derive(Debug, Clone, Deserialize)]
pub struct SpaceModelUpload {
    #[serde(flatten)]
    pub envelope: SpaceModelEnvelope,
    pub spaces: Vec<Room>,
}

/// Spaces schema version this server accepts. **Starts at 1, and moves
/// independently of the other four.**
///
/// The argument doors made against starting at 6 to match rooms, and windows
/// made against starting at 2 to match doors: a version number records *a
/// contract's own* history, and this contract has none. Five entities, five
/// version lines.
pub const SUPPORTED_SPACE_SCHEMA: u32 = 1;

impl super::SnapshotEnvelope for SpacePayload {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Enclosure;

    /// The wire shape, end to end: a v1 spaces payload round-trips with its
    /// properties, its footprint and its enclosure state intact.
    #[test]
    fn test_space_payload_round_trips() {
        let json = serde_json::json!({
            "schema_version": 1,
            "project":  { "id": "RHH", "name": "RHH" },
            "model":    { "id": "RHH-JHA-ME-MDL-HOS", "name": "Mechanical", "source": "revit" },
            "snapshot": { "taken_at": "2026-09-06T00:00:00Z" },
            "phase": "Future",
            "spaces": [{
                "id": "10513046",
                "name": "HEATING & ASSEMBLY KIT011",
                "level_id": "5576711",
                "loops": [{ "points": [{ "x": 0.0, "y": 0.0 }, { "x": 4.0, "y": 0.0 }] }],
                "enclosure": "enclosed",
                "properties": { "Number": { "value": "KIT011", "storage_type": "String" } }
            }]
        });

        let payload: SpacePayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.schema_version, SUPPORTED_SPACE_SCHEMA);
        assert_eq!(payload.spaces.len(), 1);
        assert_eq!(payload.spaces[0].enclosure, Some(Enclosure::Enclosed));
        assert_eq!(payload.spaces[0].loops[0].points.len(), 2);
        assert_eq!(payload.phase.as_deref(), Some("Future"), "a services model names its own phase");
    }

    /// **An empty spaces list is legal, and it is the finding rather than a
    /// fault.** The rooms path 422s on this; here "audited, and holds none" is
    /// exactly what requirement 1 buys, and a different fact from "never
    /// pushed". Asserted so nobody copies `reject_empty_rooms` across.
    #[test]
    fn test_an_empty_spaces_push_is_valid() {
        let json = serde_json::json!({
            "schema_version": 1,
            "project":  { "id": "RHH", "name": "RHH" },
            "model":    { "id": "RHH-JHA-AV-MDL-HOS", "name": "Audio Visual", "source": "revit" },
            "snapshot": { "taken_at": "2026-09-06T00:00:00Z" },
            "phase": "New Construction",
            "spaces": []
        });

        let payload: SpacePayload = serde_json::from_value(json).unwrap();
        assert!(payload.spaces.is_empty());
        assert!(payload.levels.is_empty(), "levels are optional here, unlike a rooms payload");
    }

    /// The three enclosure states round-trip, and `unmeasured` is distinguishable
    /// from `unenclosed` on the wire — which is the entire reason the type is not
    /// a `bool`. An unenclosed space carries empty `loops` and stays a real
    /// element QA must see.
    #[test]
    fn test_every_enclosure_state_survives_the_wire() {
        for (wire, expected) in [
            ("enclosed", Enclosure::Enclosed),
            ("unenclosed", Enclosure::Unenclosed),
            ("unmeasured", Enclosure::Unmeasured),
        ] {
            let json = serde_json::json!({
                "id": "s1",
                "name": "S",
                "level_id": "l1",
                "loops": [],
                "enclosure": wire,
                "properties": {}
            });
            let space: Room = serde_json::from_value(json).unwrap();
            assert_eq!(space.enclosure, Some(expected));
            let reparsed: Room = serde_json::from_str(&serde_json::to_string(&space).unwrap()).unwrap();
            assert_eq!(reparsed.enclosure, Some(expected), "survives a round-trip");
        }
    }

    /// **A room's bytes are unchanged by the field existing.** `enclosure` is
    /// absent on every room and skipped when serializing, so a rooms snapshot
    /// written before this field cannot be told from one written after.
    #[test]
    fn test_enclosure_is_absent_and_unwritten_for_a_room() {
        let json = serde_json::json!({
            "id": "r1", "name": "Office", "level_id": "l1", "loops": [], "properties": {}
        });
        let room: Room = serde_json::from_value(json).unwrap();
        assert_eq!(room.enclosure, None);

        let written = serde_json::to_value(&room).unwrap();
        assert!(
            written.get("enclosure").is_none(),
            "a room must not gain an `enclosure` key, or every stored snapshot changes shape"
        );
    }

    /// The stream envelope must deserialize with **no spaces present** — that is
    /// the whole reason it is a separate type from `SpacePayload`.
    #[test]
    fn test_space_stream_envelope_deserializes_without_spaces() {
        let json = serde_json::json!({
            "schema_version": 1,
            "project":  { "id": "RHH", "name": "RHH" },
            "snapshot": { "taken_at": "2026-09-06T00:00:00Z" },
            "phase": "New Construction",
            "models": [{ "id": "RHH-JHA-HY-MDL-HOS", "name": "Hydraulic", "source": "revit" }]
        });

        let envelope: SpaceStreamEnvelope = serde_json::from_value(json).unwrap();
        assert_eq!(envelope.models.len(), 1);
        assert_eq!(envelope.models[0].model.id, "RHH-JHA-HY-MDL-HOS");
        assert!(envelope.models[0].levels.is_empty());
        assert!(envelope.models[0].room_boundary.is_none());
    }

    /// Every streamed space names its own model, so a multi-model push needs no
    /// grouping marker in the stream.
    #[test]
    fn test_stream_space_carries_its_model_id() {
        let json = serde_json::json!({
            "model_id": "RHH-JHA-FR-MDL-HOS",
            "id": "s1",
            "name": "S",
            "level_id": "l1",
            "loops": [],
            "enclosure": "unenclosed",
            "properties": {}
        });

        let line: StreamSpace = serde_json::from_value(json).unwrap();
        assert_eq!(line.model_id, "RHH-JHA-FR-MDL-HOS");
        assert_eq!(line.space.enclosure, Some(Enclosure::Unenclosed));
    }

    /// `room_boundary` rides the model envelope, because the boundary regime is
    /// per document and it is the documented cause of the area differences this
    /// entity reports.
    #[test]
    fn test_space_model_envelope_carries_a_boundary_regime() {
        let json = serde_json::json!({
            "id": "m1", "name": "M", "source": "revit", "room_boundary": "centreline"
        });
        let envelope: SpaceModelEnvelope = serde_json::from_value(json).unwrap();
        assert_eq!(envelope.room_boundary, Some(RoomBoundary::Centreline));
    }

    /// The five version lines are independent, asserted rather than argued so a
    /// future bump to one cannot quietly drag the others.
    #[test]
    fn test_space_schema_is_independent_of_the_other_four() {
        assert_eq!(SUPPORTED_SPACE_SCHEMA, 1);
        assert_ne!(SUPPORTED_SPACE_SCHEMA, super::super::SUPPORTED_SCHEMA);
        assert_ne!(SUPPORTED_SPACE_SCHEMA, super::super::SUPPORTED_DOOR_SCHEMA);
    }
}
