//! The **surface record**: one ceiling or one floor, and everything about a
//! push of them that does not name the entity.
//!
//! Ceilings and floors are the two entities whose room association is
//! **purely geometric**. Doors, windows and FF&E each carry an authored room
//! reference and use geometry only as the opt-in fallback `[doors]
//! room_resolution` names. A Revit ceiling has no room parameter, a floor has
//! none either, and a room names neither, so there is nothing to prefer an
//! authored answer over and nothing for geometry to overwrite. The attribution
//! is derived at read time in `service::surfaces` and never stored.
//!
//! ## Why one record serves both
//!
//! The two lines STRATEGY-ENTITIES.md draws decide it. **No serde key here
//! names an entity** -- `id`, `level_id`, `height_offset`, `polygons` and the
//! two property tiers mean the same thing on a floor as on a ceiling -- so
//! sharing changes nothing on disk. **And no field means nothing on either**:
//! both are slabs duHast measures by walking the horizontal faces of a solid
//! (`to_data_ceiling` and `to_data_floor` are the same function with the
//! category and the offset parameter swapped), so the footprint arrives in the
//! same shape with the same faults, and both have a height above their level.
//! What that height *is* differs -- a ceiling's is its underside, a floor's its
//! top -- and it is documented on the field rather than split into two.
//!
//! The ENVELOPES stay per entity, in `ceilings.rs` and `floors.rs`, for the
//! reason `windows.rs` states: the element list's key is baked into every
//! stored file, so a merged payload would be a migration rather than a
//! refactor. That is the `Opening` split exactly -- one record, two envelopes.

use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::{Level, Loop, Model, ModelToShared, Project, PropertyMap, Snapshot};

/// One ceiling or one floor, as stored and as served.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Surface {
    pub id: String,

    /// The level the MODELLER assigned, via `CEILING_HEIGHTABOVELEVEL_PARAM` or
    /// `FLOOR_HEIGHTABOVELEVEL_PARAM`.
    ///
    /// **Never re-derived from geometry**, and the FF&E level trap is why: a
    /// derived level looks correct, so `duHast.Revit.Family.Export.to_data_item`
    /// put 9,186 of 15,070 items on the wrong storey and nothing noticed until
    /// somebody compared a chair against its own `Level` parameter. duHast's
    /// `get_level_data` reads the authored value for both entities; the
    /// extractor must not compete with it.
    pub level_id: String,

    /// Height above `level_id`, in decimal feet.
    ///
    /// **Not the same edge on the two entities**: a ceiling's offset is to its
    /// underside, a floor's to its TOP. Both are what Revit's own "Height
    /// Offset From Level" parameter answers for that category, which is why the
    /// field is shared rather than split -- it is the value a modeller sees in
    /// the properties palette either way.
    ///
    /// The only field that separates two surfaces stacked in one plan position
    /// -- a bulkhead over a flat soffit, a finish floor over a structural slab
    /// -- since their plan polygons may be nested. Carried rather than derived
    /// because the attribution works in plan and would otherwise report both
    /// with no way to say which is which.
    #[serde(default)]
    pub height_offset: Option<f64>,

    /// Plan footprint: **every** piece duHast measured, each an outer ring plus
    /// its holes, in decimal feet, model space, Y up.
    ///
    /// **A list of pieces rather than one ring, and RHH's ceilings are what
    /// forced that.** House A's 30 ceilings made a single `loops` look
    /// sufficient: 7 exported more than one polygon and every extra was the same
    /// horizontal face again (IoU above 0.98), so the largest piece *was* the
    /// whole ceiling and `polygon[0]` happened to be it. RHH is not like that.
    /// Of its 48 multi-polygon ceilings the pieces are genuinely DISJOINT, and
    /// the two shortcuts cost real area — measured against the union, taking
    /// `polygon[0]` loses 44.0% on average and 99.2% at worst (a 1,457 sqft
    /// ceiling whose first piece is 11 sqft), and even taking the LARGEST piece
    /// loses 25.7% on average, with 31 of the 48 losing more than 5%.
    ///
    /// So the only operation that is correct on both documents is the **union**
    /// of the pieces, which needs the pieces to be on the wire. That is what
    /// this field is. It also collapses House A's duplicate faces back to one
    /// face for free, which is why this is not a special case for one project.
    ///
    /// **A piece is an island OR a separate part; a hole is always an inner
    /// ring.** duHast classifies a face's edge loops by containment rather than
    /// by their order (which the Revit API documents as arbitrary): the largest
    /// remaining loop is an outer ring, a loop inside it is a hole, and a loop
    /// inside one of those holes is an island that becomes its own piece. The
    /// union then fills an island back into the hole around it, which is what
    /// a floor sketched as a ring with a platform in its opening actually is.
    ///
    /// **Empty is a reported state, not an absence.** A surface duHast cannot
    /// measure is exported with no pieces rather than dropped (fixed upstream
    /// 2026-09-10), because it still carries a good id, level, phase and both
    /// property maps — and because a dropped element is indistinguishable
    /// downstream from one nobody modelled. It is attributed to no room and
    /// says so.
    pub polygons: Vec<SurfacePolygon>,

    /// Instance properties, source-native and flat, like every other entity.
    /// For a floor this is where `Structural` rides: a structural slab and a
    /// finish floor are both `OST_Floors`, and the flag is a Revit parameter
    /// rather than a category.
    #[serde(default, with = "super::property_codec::map")]
    pub properties: PropertyMap,

    /// Type properties, same tiering rule as doors: a tier wins only when it is
    /// `Present`, and a blank instance value does not shadow a real type one.
    /// Shared behind an `Arc`, one copy per distinct bag in a snapshot.
    #[serde(default, with = "super::property_codec::shared_map")]
    pub type_properties: Arc<PropertyMap>,

    /// The type id, carried for the same reason a door's is — it is ready to
    /// key a shared type table on the *wire* if the payload-size optimization
    /// the entities doc defers is ever taken. Storage no longer needs it: the
    /// stored table is keyed by content (`property_codec`).
    #[serde(default)]
    pub type_id: Option<String>,

    /// The type name, when the export carried one.
    #[serde(default)]
    pub type_name: Option<String>,
}

/// One piece of a surface's plan footprint: an outer ring and its holes.
///
/// The room convention verbatim (`[0]` outer, `[1..]` holes), so one renderer
/// and one placement transform serve a surface piece and a room alike. What a
/// surface adds is that there may be SEVERAL of these — see
/// `Surface::polygons`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfacePolygon {
    pub loops: Vec<Loop>,
}

/// One model's block on a multi-model ceilings or floors upload.
///
/// No `room_boundary`, unlike `SpaceModelEnvelope`: a boundary regime describes
/// how a *room* was drawn, and it is what `service::areas` needs to know how
/// much wall sits between two rooms. A slab has its own edges and nothing about
/// it is measured to a finish face or a centreline.
#[derive(Debug, Clone, Deserialize)]
pub struct SurfaceModelEnvelope {
    #[serde(flatten)]
    pub model: Model,
    #[serde(default)]
    pub model_to_shared: Option<ModelToShared>,
    #[serde(default)]
    pub levels: Vec<Level>,
}

/// The first NDJSON line of a streamed ceilings or floors push: the run's
/// shared identity plus one `SurfaceModelEnvelope` per model, with every
/// element arriving on a following line as a `StreamSurface`.
///
/// Shared because it names no entity -- the route does that.
#[derive(Debug, Clone, Deserialize)]
pub struct SurfaceStreamEnvelope {
    pub schema_version: u32,
    pub project: Project,
    #[serde(default)]
    pub snapshot: Snapshot,
    #[serde(default)]
    pub phase: Option<String>,
    pub models: Vec<SurfaceModelEnvelope>,
}

/// One element line of a streamed push: the surface, plus the id of the model
/// it belongs to. See `StreamRoom` for why the id rides every element rather
/// than a grouping marker -- and for a surface it matters more, because a
/// surface filed under the wrong model is attributed against the wrong model's
/// rooms.
#[derive(Debug, Clone, Deserialize)]
pub struct StreamSurface {
    pub model_id: String,
    #[serde(flatten)]
    pub surface: Surface,
}

/// A buffered upload taken apart into what the streamed route receives: the
/// envelope line, and each model's elements.
///
/// This is what lets both routes of both entities feed ONE ingest path, so the
/// buffered and streamed pushes cannot store different things.
pub type SurfaceUploadParts = (SurfaceStreamEnvelope, Vec<(String, Vec<Surface>)>);

/// A stored ceilings or floors snapshot, whichever it is.
///
/// The `OpeningEnvelope` pattern: every method is a field read or a
/// constructor, and the trait exists to hide *which list key* the elements
/// live under. The pairing of a payload type with its `SnapshotKind` is made
/// once, in `service::surfaces::SurfaceKind`.
pub trait SurfaceEnvelope: super::SnapshotEnvelope + Serialize + DeserializeOwned {
    fn surfaces(&self) -> &[Surface];

    /// The list, for a quarantined push that buffers instead of streaming.
    fn surfaces_mut(&mut self) -> &mut Vec<Surface>;

    /// Rebuild the single-model payload one model block plus the run's shared
    /// envelope describes, with no elements yet -- see
    /// `RoomModelEnvelope::into_payload`, which explains why a push decomposes
    /// at all.
    fn from_model_envelope(
        schema_version: u32,
        project: Project,
        snapshot: Snapshot,
        phase: Option<String>,
        envelope: SurfaceModelEnvelope,
    ) -> Self;
}
