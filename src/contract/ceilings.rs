//! The **ceilings envelope**: how a push and a stored snapshot of ceilings are
//! shaped. The record itself is `surfaces::Surface`, shared with floors -- see
//! that module for why sharing it changes no key.
//!
//! The sixth entity, and the first whose room association is **purely
//! geometric**. What was specific to ceilings when they were the only such
//! entity -- the footprint as a list of pieces, the union, the tolerance --
//! turned out to be specific to *slabs*, and moved to `surfaces` and
//! `service::surface_attribution` when floors arrived.
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

use serde::{Deserialize, Serialize};

use super::surfaces::{Surface, SurfaceEnvelope, SurfaceModelEnvelope, SurfaceStreamEnvelope, SurfaceUploadParts};
use super::{Level, Model, ModelToShared, Project, Snapshot};

/// One timestamped push of one model's ceilings.
///
/// Carries the **same upload envelope** as every other entity — `project`,
/// `model`, `snapshot`, `phase` — resolved through the same `ensure_taken_at` /
/// `validate_snapshot_id` / `normalize_phase` functions rather than
/// reimplementations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CeilingPayload {
    /// **Ceilings version independently of the other entities** — see
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
    /// It carries more weight here than for an entity with an authored
    /// reference, because the room join is geometric: two models whose
    /// placements disagree produce ceilings that miss their rooms entirely,
    /// where a door would still resolve. `service::placement` composes
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
    pub ceilings: Vec<Surface>,
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

impl CeilingsUpload {
    /// This upload as the streamed route would have received it -- see
    /// `SurfaceUploadParts`.
    pub fn into_parts(self) -> SurfaceUploadParts {
        let mut models = Vec::with_capacity(self.models.len());
        let mut elements = Vec::with_capacity(self.models.len());
        for m in self.models {
            elements.push((m.envelope.model.id.clone(), m.ceilings));
            models.push(m.envelope);
        }
        let envelope = SurfaceStreamEnvelope {
            schema_version: self.schema_version,
            project: self.project,
            snapshot: self.snapshot,
            phase: self.phase,
            models,
        };
        (envelope, elements)
    }
}

/// One model's block on a buffered ceilings upload: its
/// `SurfaceModelEnvelope` plus its ceilings.
#[derive(Debug, Clone, Deserialize)]
pub struct CeilingModelUpload {
    #[serde(flatten)]
    pub envelope: SurfaceModelEnvelope,
    pub ceilings: Vec<Surface>,
}

/// The ceilings schema this build accepts. **Starts at 1, and moves
/// independently of the others.**
///
/// Same argument doors made against starting at 6 to match rooms, and windows
/// against starting at 2 to match doors: a version number records *a contract's
/// own* history, and this contract has none.
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

impl SurfaceEnvelope for CeilingPayload {
    fn surfaces(&self) -> &[Surface] {
        &self.ceilings
    }
    fn surfaces_mut(&mut self) -> &mut Vec<Surface> {
        &mut self.ceilings
    }
    fn from_model_envelope(
        schema_version: u32,
        project: Project,
        snapshot: Snapshot,
        phase: Option<String>,
        envelope: SurfaceModelEnvelope,
    ) -> Self {
        CeilingPayload {
            schema_version,
            project,
            model: envelope.model,
            snapshot,
            phase,
            model_to_shared: envelope.model_to_shared,
            levels: envelope.levels,
            ceilings: Vec::new(),
        }
    }
}
