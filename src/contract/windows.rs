//! The **windows envelope**: how a push, a stored snapshot and a stream frame
//! of `Opening`s are shaped when the entity is windows.
//!
//! Deliberately a sibling of [`doors`](super::doors) rather than a
//! generalisation of it, and the reason is narrow enough to state exactly. The
//! *record* is shared — see [`openings`](super::openings), where measurement
//! showed a window record and a door record are structurally identical. The
//! envelope is not, because two of its facts are per entity and both are baked
//! into bytes already on disk:
//!
//! - **the element key.** A stored doors snapshot names its list `doors`; this
//!   one names it `windows`. One shared payload type could carry only one such
//!   name, so sharing it would mean renaming a key every existing snapshot
//!   already uses — a migration, not a refactor.
//! - **the schema version.** Doors are at v2, windows start at v1. Independent
//!   by design: a change to the doors contract has nothing to say about windows,
//!   and a shared constant would force every producer of one to re-release over
//!   a change to the other.
//!
//! So this file is close to a copy of `doors.rs`, and that is the smaller cost.
//! The duplication is ~90 lines of field declarations whose shape the compiler
//! checks; the alternative was one type with a serde key that lies about half
//! the snapshots it parses.
//!
//! **What is NOT duplicated is the part that matters**: the record, the upload
//! envelope's own pieces (`Project`, `Model`, `Snapshot`, `ModelToShared`),
//! identity resolution, the phase rules, the store, and the whole read side.

use serde::{Deserialize, Serialize};

use super::openings::Opening;
use super::{Level, Model, ModelToShared, Project, Snapshot};

/// One timestamped push of one model's windows.
///
/// Carries the **same upload envelope** as `RoomPayload` and `DoorPayload` —
/// `project`, `model`, `snapshot`, `phase` — resolved through the same
/// `ensure_taken_at` / `validate_snapshot_id` / `normalize_phase` functions
/// rather than reimplementations. A third entity needs no new identity concepts
/// at all, which is the claim [Entities](../../docs/STRATEGY-ENTITIES.md) made
/// and this is the test of it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowPayload {
    /// **Windows version independently of rooms and doors** — see
    /// `SUPPORTED_WINDOW_SCHEMA`.
    pub schema_version: u32,

    pub project: Project,
    pub model: Model,
    #[serde(default)]
    pub snapshot: Snapshot,

    /// The Revit phase this push was filtered to, in lockstep with
    /// `RoomPayload::phase` and `DoorPayload::phase`, with the same
    /// `Option`-on-the-type, required-at-ingest split: the type stays permissive
    /// because every stored snapshot re-parses through it at boot, and the
    /// handler is strict.
    ///
    /// **A windows push that disagrees with the lineage is refused, not
    /// quarantined** — the doors rule, for the doors reason. Promoting it would
    /// move the lineage while every room snapshot stayed on the old phase,
    /// stranding the rooms `from_room`/`to_room` resolve against. An *unphased*
    /// lineage is still phased by whichever push reaches it first, which may now
    /// be a windows one.
    ///
    /// Worth noting that windows are filtered by the same **range** test doors
    /// use (`elements_in_phase`), not the equality test rooms use: a window, like
    /// a door, is built in one phase and may be demolished in a later one, so it
    /// exists across a span. Running windows through the room predicate returns
    /// nothing, silently — the failure that cost five empty pushes to find.
    #[serde(default)]
    pub phase: Option<String>,

    /// Model→shared placement, in lockstep with `RoomPayload` and `DoorPayload`.
    ///
    /// Carried on the same terms and for the same reason: a window footprint is
    /// geometry in the same model space as the rooms', every geometry payload
    /// here carries its own placement, and a payload that omitted it would be
    /// the odd one out. **Per push, never per window.**
    #[serde(default)]
    pub model_to_shared: Option<ModelToShared>,

    /// The level set `Opening.level_id` points into, for a model that has no
    /// rooms snapshot to supply one.
    ///
    /// **This matters more for windows than it did for doors, and the facade
    /// file is why.** Measured 2026-09-03: a facade model held 158 windows and
    /// 191 doors and **not one room**, because it links its interiors rather
    /// than containing them. `service` looks an elevation up by
    /// `(model_id, level_id)` before it will probe an opening's surroundings, so
    /// without this list every window in such a model is unreachable rather than
    /// merely unresolved — and unreachable is the state that makes
    /// `room_resolution` useless exactly where it is needed most.
    ///
    /// Empty stays legal and stays ordinary: a model that pushes rooms declares
    /// its levels there, and the rooms snapshot's copy wins, so sending it twice
    /// is a redundancy rather than a disagreement.
    ///
    /// The *elevation* is what this is for, not the id. Level ids are
    /// per-document and never match across models; elevations are what cross.
    #[serde(default)]
    pub levels: Vec<Level>,

    pub windows: Vec<Opening>,
}

/// One model's block on a multi-model windows upload — the windows counterpart
/// of `DoorModelEnvelope`.
///
/// No `room_boundary`: that is a rooms fact this contract has no key for.
/// `levels` IS carried, on the terms `WindowPayload::levels` states.
#[derive(Debug, Clone, Deserialize)]
pub struct WindowModelEnvelope {
    #[serde(flatten)]
    pub model: Model,
    #[serde(default)]
    pub model_to_shared: Option<ModelToShared>,
    #[serde(default)]
    pub levels: Vec<Level>,
}

impl WindowModelEnvelope {
    /// Rebuild the single-model `WindowPayload` this block plus the run's shared
    /// envelope describes — see `RoomModelEnvelope::into_payload`, which this
    /// mirrors and which explains why a push decomposes at all.
    pub fn into_payload(
        self,
        schema_version: u32,
        project: Project,
        snapshot: Snapshot,
        phase: Option<String>,
        windows: Vec<Opening>,
    ) -> WindowPayload {
        WindowPayload {
            schema_version,
            project,
            model: self.model,
            snapshot,
            phase,
            model_to_shared: self.model_to_shared,
            levels: self.levels,
            windows,
        }
    }
}

/// The first NDJSON line of a streamed windows push (`POST /windows/stream`):
/// the run's shared identity plus one `WindowModelEnvelope` per model, with
/// every window arriving on a following line as a `StreamWindow`.
///
/// Its own type rather than making `windows` optional on `WindowPayload`, for
/// the reason `StreamEnvelope` gives: the envelope must deserialize alone with
/// no windows present, and `WindowPayload` must keep `windows` guaranteed for
/// every other consumer.
#[derive(Debug, Clone, Deserialize)]
pub struct WindowStreamEnvelope {
    pub schema_version: u32,
    pub project: Project,
    #[serde(default)]
    pub snapshot: Snapshot,
    #[serde(default)]
    pub phase: Option<String>,
    pub models: Vec<WindowModelEnvelope>,
}

/// One window line of a streamed push: the window, plus the id of the model it
/// belongs to. See `StreamRoom` for why the id rides every element rather than
/// a grouping marker.
#[derive(Debug, Clone, Deserialize)]
pub struct StreamWindow {
    pub model_id: String,
    #[serde(flatten)]
    pub window: Opening,
}

/// The buffered multi-model windows upload (`POST /windows`), the counterpart
/// of `DoorsUpload`.
#[derive(Debug, Clone, Deserialize)]
pub struct WindowsUpload {
    pub schema_version: u32,
    pub project: Project,
    #[serde(default)]
    pub snapshot: Snapshot,
    #[serde(default)]
    pub phase: Option<String>,
    pub models: Vec<WindowModelUpload>,
}

/// One model's block on a buffered windows upload: its `WindowModelEnvelope`
/// plus its windows.
#[derive(Debug, Clone, Deserialize)]
pub struct WindowModelUpload {
    #[serde(flatten)]
    pub envelope: WindowModelEnvelope,
    pub windows: Vec<Opening>,
}

/// Windows schema version this server accepts. **Starts at 1, and moves
/// independently of both `SUPPORTED_SCHEMA` and `SUPPORTED_DOOR_SCHEMA`.**
///
/// **1, not 2, and the temptation to match doors is worth naming.** Windows
/// arrive already carrying many models per push — the change that took rooms
/// from v6 to v7 and doors from v1 to v2 — so a reader who knows the history
/// might expect this to start at 2 to signal "same generation". It does not,
/// because a version number records *a contract's own* history and this contract
/// has none: there was never a windows v1 carrying a single model, so numbering
/// this 2 would claim a predecessor that never existed and leave a permanent
/// gap for the next reader to hunt.
///
/// The same argument doors made against starting at 6 to match rooms. Two
/// entities, two version lines; three now.
pub const SUPPORTED_WINDOW_SCHEMA: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire shape, end to end: a v1 windows payload round-trips with both
    /// property tiers, both room references, and the footprint intact.
    #[test]
    fn test_window_payload_round_trips() {
        let json = serde_json::json!({
            "schema_version": 1,
            "project":  { "id": "House A", "name": "House A" },
            "model":    { "id": "facade", "name": "Facade", "source": "revit" },
            "snapshot": { "taken_at": "2026-09-03T00:00:00Z" },
            "phase": "New Construction",
            "windows": [{
                "id": "w1",
                "level_id": "lvl1",
                "loops": [{ "points": [{ "x": 0.0, "y": 0.0 }, { "x": 4.0, "y": 0.0 }] }],
                "from_room": "r1",
                "to_room": null,
                "type_id": "t1",
                "type_name": "Awning 1200",
                "properties": { "Mark": { "value": "W-101", "storage_type": "String" } },
                "type_properties": { "Height": { "value": "900" } }
            }]
        });

        let payload: WindowPayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.schema_version, SUPPORTED_WINDOW_SCHEMA);
        assert_eq!(payload.windows.len(), 1);
        assert_eq!(payload.windows[0].id, "w1");
        assert_eq!(payload.windows[0].from_room.as_deref(), Some("r1"));
        assert_eq!(payload.windows[0].to_room, None, "an external window is one-sided, not broken");
        assert_eq!(payload.windows[0].loops[0].points.len(), 2);
        assert_eq!(payload.phase.as_deref(), Some("New Construction"));
    }

    /// **A facade window naming no room at all parses**, because that is the
    /// ordinary state in the model this entity was built for: 0 of 158 windows
    /// in the measured facade file carried a reference on either side, since
    /// `FromRoom[phase]` cannot see into a linked document. A contract that
    /// required either side would reject a legitimate model outright.
    #[test]
    fn test_a_window_with_no_room_on_either_side_is_valid() {
        let json = serde_json::json!({
            "schema_version": 1,
            "project":  { "id": "p", "name": "P" },
            "model":    { "id": "facade", "name": "F", "source": "revit" },
            "snapshot": { "taken_at": "2026-09-03T00:00:00Z" },
            "windows": [{
                "id": "w1",
                "level_id": "lvl1",
                "loops": [],
                "type_id": "t1",
                "type_name": "Fixed",
                "properties": {},
                "type_properties": {}
            }]
        });

        let payload: WindowPayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.windows[0].from_room, None);
        assert_eq!(payload.windows[0].to_room, None);
        assert!(payload.windows[0].loops.is_empty(), "an unmeasurable footprint is empty, not absent");
    }

    /// `levels` is optional and additive here exactly as it is on doors, so a
    /// producer that does not send it still parses.
    #[test]
    fn test_levels_are_optional_on_a_windows_payload() {
        let json = serde_json::json!({
            "schema_version": 1,
            "project":  { "id": "p", "name": "P" },
            "model":    { "id": "facade", "name": "F", "source": "revit" },
            "snapshot": { "taken_at": "2026-09-03T00:00:00Z" },
            "windows": []
        });

        let payload: WindowPayload = serde_json::from_value(json).unwrap();
        assert!(payload.levels.is_empty());
        assert!(payload.windows.is_empty(), "an empty windows list is legal here; the producer refuses it");
    }

    /// The stream envelope must deserialize with **no windows present** — that
    /// is the whole reason it is a separate type from `WindowPayload`.
    #[test]
    fn test_window_stream_envelope_deserializes_without_windows() {
        let json = serde_json::json!({
            "schema_version": 1,
            "project":  { "id": "p", "name": "P" },
            "snapshot": { "taken_at": "2026-09-03T00:00:00Z" },
            "phase": "New Construction",
            "models": [{ "id": "facade", "name": "F", "source": "revit" }]
        });

        let envelope: WindowStreamEnvelope = serde_json::from_value(json).unwrap();
        assert_eq!(envelope.models.len(), 1);
        assert_eq!(envelope.models[0].model.id, "facade");
        assert!(envelope.models[0].levels.is_empty());
    }

    /// Every streamed window names its own model, so a multi-model push needs no
    /// grouping marker in the stream.
    #[test]
    fn test_stream_window_carries_its_model_id() {
        let json = serde_json::json!({
            "model_id": "facade",
            "id": "w1",
            "level_id": "lvl1",
            "loops": [],
            "type_id": "t1",
            "type_name": "Fixed",
            "properties": {},
            "type_properties": {}
        });

        let line: StreamWindow = serde_json::from_value(json).unwrap();
        assert_eq!(line.model_id, "facade");
        assert_eq!(line.window.id, "w1");
    }

    /// The three version lines are independent, asserted rather than argued so
    /// that a future bump to one cannot quietly drag the others.
    #[test]
    fn test_window_schema_is_independent_of_the_other_two() {
        assert_eq!(SUPPORTED_WINDOW_SCHEMA, 1);
        assert_ne!(SUPPORTED_WINDOW_SCHEMA, super::super::SUPPORTED_SCHEMA);
        assert_ne!(SUPPORTED_WINDOW_SCHEMA, super::super::SUPPORTED_DOOR_SCHEMA);
    }
}
