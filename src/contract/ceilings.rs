//! The **ceilings envelope**: how a push, a stored snapshot and a stream frame
//! of `Ceiling`s are shaped.
//!
//! The sixth entity, and the first whose room association is **purely
//! geometric**. Doors, windows and FF&E each carry an authored room reference
//! and use geometry only as the opt-in fallback `[doors] room_resolution`
//! names. A Revit ceiling has no room parameter and a room has no ceiling
//! parameter, so there is nothing here to prefer an authored answer over and
//! nothing for geometry to overwrite. That inverts the precedence rule applied
//! everywhere else in this codebase, and it is this entity's own rule in the
//! way spaces have three.
//!
//! Two consequences follow, and both are in `service::ceilings` rather than
//! here, because **attribution is derived at read time and never stored** —
//! the rule `[doors] room_attribution` already follows, for the same reason:
//! changing the policy changes every answer and rewrites nothing.
//!
//! ## The record is the room shape, and that was measured
//!
//! `id`, `level_id`, `loops` and `properties` mean for a ceiling what they mean
//! for a room, so this reuses `Loop` verbatim. The tempting objection is that a
//! ceiling exports *several* polygons — 7 of 30 on House A, up to 6 of them —
//! and that a single outer-ring-plus-holes field therefore cannot carry one.
//! **That objection is wrong, and the probe is what settled it**
//! (`scripts/analyse_ceilings_probe.py`, House A, 2026-09-10).
//!
//! `convert_solid_to_flattened_2d_points` walks the *horizontal faces* of a
//! solid, and a slab has two: its top and its bottom, near-identical in plan.
//! Of the 7 multi-polygon ceilings, 4 were the same face twice — IoU above 0.98
//! between the two largest pieces, and their areas summing to exactly twice
//! their union — and the other 3 were the largest face plus sub-1-sqft noise
//! off the side faces. **The largest polygon equalled the union of all of them
//! on every ceiling measured.** So `polygon[0]` is the whole ceiling, which is
//! what `loops_from_polygon` already takes, and a producer that unioned or
//! summed the polygons would double-count area on 4 of 30.
//!
//! If a document ever exports a ceiling in genuinely disjoint pieces, the
//! analyser reports it under Q6 as "carries genuinely ADDITIONAL geometry", and
//! *that* is the signal to widen this field — not the polygon count, which
//! reads as evidence and is not.
//!
//! ## Phase is the doors range test, not the rooms equality test
//!
//! A room BELONGS to a phase (`ROOM_PHASE`); a ceiling is **built** in one and
//! may be demolished in a later one, so it takes `PHASE_CREATED` /
//! `PHASE_DEMOLISHED` and the range test, exactly as a door does. No ceiling on
//! House A is demolished, so the two tests happen to agree there — which is
//! precisely why this is written down rather than inferred from a passing run.
//! `CLAUDE.md` records that running rooms through the range test returns
//! nothing, silently, at a cost of five empty pushes; this is the same trap
//! facing the other way.
//!
//! ## The envelope is per entity, for the reasons `windows.rs` states
//!
//! The element key (`ceilings`) and the schema version are both baked into
//! bytes on disk, so a merged payload type would be a migration rather than a
//! refactor.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{CustomValue, Level, Loop, Model, ModelToShared, Project, Snapshot};

/// One timestamped push of one model's ceilings.
///
/// Carries the **same upload envelope** as every other entity — `project`,
/// `model`, `snapshot`, `phase` — resolved through the same `ensure_taken_at` /
/// `validate_snapshot_id` / `normalize_phase` functions rather than
/// reimplementations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CeilingPayload {
    /// **Ceilings version independently of the other five** — see
    /// `SUPPORTED_CEILING_SCHEMA`.
    pub schema_version: u32,

    pub project: Project,
    pub model: Model,
    #[serde(default)]
    pub snapshot: Snapshot,

    /// The Revit phase this push was filtered to, with the same
    /// `Option`-on-the-type, required-at-ingest split every other payload has:
    /// the type stays permissive because every stored snapshot re-parses
    /// through it at boot, and the handler is strict.
    ///
    /// **The range test, not the equality test** — see the module header.
    #[serde(default)]
    pub phase: Option<String>,

    /// Model→shared placement, in lockstep with every other payload. **Per
    /// push, never per ceiling.**
    ///
    /// It carries more weight here than for any previous entity, because the
    /// room join is geometric: two models whose placements disagree produce
    /// ceilings that miss their rooms entirely, where a door with an authored
    /// reference would still resolve. `service::placement` composes
    /// `anchor⁻¹ ∘ model` for every read, so ceilings and rooms are compared in
    /// one project-local frame or the answer is meaningless.
    #[serde(default)]
    pub model_to_shared: Option<ModelToShared>,

    /// This document's levels, on the same optional terms
    /// `SpacePayload::levels` states: a model that pushes both rooms and
    /// ceilings sends them twice, and a redundant list is not a disagreement.
    #[serde(default)]
    pub levels: Vec<Level>,

    /// **An empty list is legal, and it is not the spaces argument.**
    ///
    /// `reject_empty_rooms` refuses an empty rooms push because a rooms push
    /// happens when someone exported a document that has rooms in it. A model
    /// with rooms and no ceilings is ordinary — a shell, a base-build package,
    /// an external works file — and the server cannot tell that from a broken
    /// export, exactly as it cannot for doors. So the server accepts zero here
    /// and the producer refuses to send zero, which is the asymmetry
    /// `post_doors.empty_push_refusal` already documents: the two are answering
    /// different questions rather than disagreeing.
    pub ceilings: Vec<Ceiling>,
}

/// One ceiling, as stored and as served.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ceiling {
    pub id: String,

    /// The level the MODELLER assigned, via `CEILING_HEIGHTABOVELEVEL_PARAM`.
    ///
    /// **Never re-derived from geometry**, and the FF&E level trap is why: a
    /// derived level looks correct, so `duHast.Revit.Family.Export.to_data_item`
    /// put 9,186 of 15,070 items on the wrong storey and nothing noticed until
    /// somebody compared a chair against its own `Level` parameter. duHast's
    /// `get_level_data` reads the authored value for a ceiling; the extractor
    /// must not compete with it.
    pub level_id: String,

    /// Height above `level_id`, in decimal feet.
    ///
    /// The only field that separates two ceilings stacked over one room — a
    /// bulkhead over a flat soffit — since their plan polygons may be nested.
    /// Carried rather than derived because the geometric attribution in
    /// `service::ceilings` works in plan and would otherwise report both as
    /// covering the room with no way to say which is which.
    #[serde(default)]
    pub height_offset: Option<f64>,

    /// Plan footprint: `[0]` outer, `[1..]` holes, decimal feet, model space,
    /// Y up — the room convention verbatim, so one renderer and one placement
    /// transform serve both.
    ///
    /// **Empty is a reported state, not an absence.** A ceiling duHast cannot
    /// measure is exported with an empty polygon rather than dropped (fixed
    /// upstream 2026-09-10), because it still carries a good id, level, phase
    /// and both property maps — and because a dropped ceiling is
    /// indistinguishable downstream from one nobody modelled. Such a ceiling
    /// is attributed to no room and says so.
    pub loops: Vec<Loop>,

    /// Instance properties, source-native and flat, like every other entity.
    #[serde(default)]
    pub properties: BTreeMap<String, CustomValue>,

    /// Type properties, same tiering rule as doors: a tier wins only when it is
    /// `Present`, and a blank instance value does not shadow a real type one.
    #[serde(default)]
    pub type_properties: BTreeMap<String, CustomValue>,

    /// The ceiling's type id, carried for the same reason a door's is — it is
    /// ready to key a shared type table if the payload-size optimization the
    /// entities doc defers is ever taken.
    #[serde(default)]
    pub type_id: Option<String>,

    /// The ceiling's type name, when the export carried one.
    #[serde(default)]
    pub type_name: Option<String>,
}

/// One model's block on a multi-model ceilings upload.
///
/// No `room_boundary`, unlike `SpaceModelEnvelope`: a boundary regime describes
/// how a *room* was drawn, and it is what `service::areas` needs to know how
/// much wall sits between two rooms. A ceiling is a slab with its own edges and
/// nothing about it is measured to a finish face or a centreline.
#[derive(Debug, Clone, Deserialize)]
pub struct CeilingModelEnvelope {
    #[serde(flatten)]
    pub model: Model,
    #[serde(default)]
    pub model_to_shared: Option<ModelToShared>,
    #[serde(default)]
    pub levels: Vec<Level>,
}

impl CeilingModelEnvelope {
    /// Rebuild the single-model `CeilingPayload` this block plus the run's
    /// shared envelope describes — see `RoomModelEnvelope::into_payload`, which
    /// this mirrors and which explains why a push decomposes at all.
    pub fn into_payload(
        self,
        schema_version: u32,
        project: Project,
        snapshot: Snapshot,
        phase: Option<String>,
        ceilings: Vec<Ceiling>,
    ) -> CeilingPayload {
        CeilingPayload {
            schema_version,
            project,
            model: self.model,
            snapshot,
            phase,
            model_to_shared: self.model_to_shared,
            levels: self.levels,
            ceilings,
        }
    }
}

/// The first NDJSON line of a streamed ceilings push (`POST /ceilings/stream`):
/// the run's shared identity plus one `CeilingModelEnvelope` per model, with
/// every ceiling arriving on a following line as a `StreamCeiling`.
#[derive(Debug, Clone, Deserialize)]
pub struct CeilingStreamEnvelope {
    pub schema_version: u32,
    pub project: Project,
    #[serde(default)]
    pub snapshot: Snapshot,
    #[serde(default)]
    pub phase: Option<String>,
    pub models: Vec<CeilingModelEnvelope>,
}

/// One ceiling line of a streamed push: the ceiling, plus the id of the model
/// it belongs to. See `StreamRoom` for why the id rides every element rather
/// than a grouping marker.
#[derive(Debug, Clone, Deserialize)]
pub struct StreamCeiling {
    pub model_id: String,
    #[serde(flatten)]
    pub ceiling: Ceiling,
}

/// The buffered multi-model ceilings upload (`POST /ceilings`).
#[derive(Debug, Clone, Deserialize)]
pub struct CeilingsUpload {
    pub schema_version: u32,
    pub project: Project,
    #[serde(default)]
    pub snapshot: Snapshot,
    #[serde(default)]
    pub phase: Option<String>,
    pub models: Vec<CeilingModelUpload>,
}

/// One model's block on a buffered ceilings upload: its
/// `CeilingModelEnvelope` plus its ceilings.
#[derive(Debug, Clone, Deserialize)]
pub struct CeilingModelUpload {
    #[serde(flatten)]
    pub envelope: CeilingModelEnvelope,
    pub ceilings: Vec<Ceiling>,
}

/// The ceilings schema this build accepts. **Starts at 1, and moves
/// independently of the other five.**
///
/// Same argument doors made against starting at 6 to match rooms, and windows
/// against starting at 2 to match doors: a version number records *a contract's
/// own* history, and this contract has none. Six entities, six version lines.
pub const SUPPORTED_CEILING_SCHEMA: u32 = 1;

impl super::SnapshotEnvelope for CeilingPayload {
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
