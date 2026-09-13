//! The **floors envelope**: how a push and a stored snapshot of floors are
//! shaped. The seventh entity, and the second slab: the record is
//! `surfaces::Surface`, shared with ceilings, and the reasons that is safe are
//! in that module.
//!
//! What is floor-specific stays here, and there is less of it than a new entity
//! usually brings, because duHast's `to_data_floor` is `to_data_ceiling` with
//! the category and the offset parameter swapped:
//!
//! - **`height_offset` is to the TOP of the floor** (`FLOOR_HEIGHTABOVELEVEL_PARAM`),
//!   where a ceiling's is to its underside. See `Surface::height_offset`.
//! - **`OST_Floors` is not "finish floors".** A structural slab, a finish
//!   floor, a balcony and a hardstanding are all this category; foundation
//!   slabs are not (`OST_StructuralFoundation`). Nothing here filters, because
//!   which of those a question wants is the reader's call -- the `Structural`
//!   instance property says which is which.
//! - **Phase is the range test**, as for doors and ceilings: a floor is built
//!   in one phase and may be demolished in a later one.
//!
//! The envelope is per entity for the reason `windows.rs` states: `floors` is
//! baked into every stored file.

use serde::{Deserialize, Serialize};

use super::surfaces::{Surface, SurfaceEnvelope, SurfaceModelEnvelope, SurfaceStreamEnvelope, SurfaceUploadParts};
use super::{Level, Model, ModelToShared, Project, Snapshot};

/// One timestamped push of one model's floors. Field for field the
/// `CeilingPayload` envelope, and every field means what it means there.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FloorPayload {
    /// Versions independently of every other entity — see
    /// `SUPPORTED_FLOOR_SCHEMA`.
    pub schema_version: u32,

    pub project: Project,
    pub model: Model,
    #[serde(default)]
    pub snapshot: Snapshot,

    /// The Revit phase this push was filtered to. The range test, on the
    /// `CeilingPayload::phase` terms.
    #[serde(default)]
    pub phase: Option<String>,

    /// Model→shared placement, per push. Load-bearing for the reason
    /// `CeilingPayload::model_to_shared` gives: the room join is geometric, so
    /// a misplaced model does not produce a wrong room, it produces none.
    #[serde(default)]
    pub model_to_shared: Option<ModelToShared>,

    /// This document's levels, on `SpacePayload::levels`' optional terms.
    #[serde(default)]
    pub levels: Vec<Level>,

    /// **An empty list is legal**, on the ceilings argument rather than the
    /// spaces one: a model with rooms and no floors -- a fit-out package whose
    /// slabs live in the base build -- is ordinary, and the server cannot tell
    /// that from a broken export. The producer refuses an empty RUN instead.
    pub floors: Vec<Surface>,
}

/// The buffered multi-model floors upload (`POST /floors`).
#[derive(Debug, Clone, Deserialize)]
pub struct FloorsUpload {
    pub schema_version: u32,
    pub project: Project,
    #[serde(default)]
    pub snapshot: Snapshot,
    #[serde(default)]
    pub phase: Option<String>,
    pub models: Vec<FloorModelUpload>,
}

impl FloorsUpload {
    /// This upload as the streamed route would have received it -- see
    /// `SurfaceUploadParts`.
    pub fn into_parts(self) -> SurfaceUploadParts {
        let mut models = Vec::with_capacity(self.models.len());
        let mut elements = Vec::with_capacity(self.models.len());
        for m in self.models {
            elements.push((m.envelope.model.id.clone(), m.floors));
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

/// One model's block on a buffered floors upload.
#[derive(Debug, Clone, Deserialize)]
pub struct FloorModelUpload {
    #[serde(flatten)]
    pub envelope: SurfaceModelEnvelope,
    pub floors: Vec<Surface>,
}

/// The floors schema this build accepts. **Starts at 1**, for the reason every
/// entity since doors has given: a version records a contract's own history,
/// and sharing a record with ceilings does not give floors ceilings' history.
pub const SUPPORTED_FLOOR_SCHEMA: u32 = 1;

impl super::SnapshotEnvelope for FloorPayload {
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

impl SurfaceEnvelope for FloorPayload {
    fn surfaces(&self) -> &[Surface] {
        &self.floors
    }
    fn surfaces_mut(&mut self) -> &mut Vec<Surface> {
        &mut self.floors
    }
    fn from_model_envelope(
        schema_version: u32,
        project: Project,
        snapshot: Snapshot,
        phase: Option<String>,
        envelope: SurfaceModelEnvelope,
    ) -> Self {
        FloorPayload {
            schema_version,
            project,
            model: envelope.model,
            snapshot,
            phase,
            model_to_shared: envelope.model_to_shared,
            levels: envelope.levels,
            floors: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire shape end to end, and the one key that is this entity's own:
    /// the list is `floors`. A floors file that parsed as ceilings, or the
    /// reverse, would be read by the wrong assembly with nothing to say so.
    #[test]
    fn test_floor_payload_round_trips_under_its_own_key() {
        let json = serde_json::json!({
            "schema_version": 1,
            "project":  { "id": "House A", "name": "House A" },
            "model":    { "id": "arch", "name": "Arch", "source": "revit" },
            "snapshot": { "taken_at": "2026-09-13T00:00:00Z" },
            "phase": "New Construction",
            "floors": [{
                "id": "f1",
                "level_id": "lvl1",
                "height_offset": 0.0,
                "polygons": [{ "loops": [{ "points": [
                    { "x": 0.0, "y": 0.0 }, { "x": 10.0, "y": 0.0 }, { "x": 10.0, "y": 10.0 }
                ] }] }],
                "properties": { "Structural": { "value": "1", "storage_type": "Integer" } }
            }]
        });
        let payload: FloorPayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.surfaces().len(), 1);
        assert_eq!(payload.floors[0].polygons[0].loops[0].points.len(), 3);

        let back = serde_json::to_value(&payload).unwrap();
        assert!(back.get("floors").is_some(), "stored under `floors`");
        assert!(back.get("ceilings").is_none(), "and under nothing else");
        assert!(
            serde_json::from_value::<super::super::CeilingPayload>(back).is_err(),
            "a floors file must not parse as a ceilings one"
        );
    }
}
