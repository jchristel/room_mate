//! `/ceilings` and `/floors` fetch-side derive logic: scoping, placement, and
//! the geometric room attribution that is these entities' whole reason for
//! existing. The geometry itself is `service::surface_attribution`.
//!
//! One assembly for both, on the `service::openings` pattern: the things that
//! vary between a ceilings read and a floors read -- the storage kind, the
//! milestone pin map and the schema version -- are lookups on
//! [`SurfaceKind`], so nothing here says `if floors`. What would change a serde
//! key (the list name on the response) is the only thing split, into
//! [`CeilingsResult`] and [`FloorsResult`].
//!
//! The room polygons come from [`entity_scope::build_candidates`] rather than a
//! second read of the store, which is what keeps them **exactly the rooms
//! `/rooms` is serving** under the same milestone. A separate reader would
//! answer a different question about one building the moment a milestone was
//! pinned, and nothing would say so.
//!
//! ## What is derived and what is stored
//!
//! Nothing here is stored. Attribution is recomputed on every read, exactly as
//! `[doors] room_attribution` is, and for the same reason: changing the rule
//! changes every answer and rewrites nothing.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Serialize;

use crate::contract::PropertyPresence;
use crate::contract::{CeilingPayload, FloorPayload, Level, PropertyMap, Surface, SurfaceEnvelope};
use crate::settings::{BuiltinPropertyDef, RoomResolution};
use crate::state::{AppState, ModelKey};
use crate::storage::SnapshotKind;

use super::room_locator::RoomRef;
use super::rooms::FilterTarget;
use super::surface_attribution::{attribute, SurfaceRoom};
use super::{entity_scope, ServiceError};

/// Which surface entity a read or a push addresses.
///
/// The assembly is generic over the payload type and takes this separately,
/// so in principle `Floors` could be passed with `CeilingPayload`. The pairing
/// is made once, in [`SurfaceKind::of`], from the payload type -- so a caller
/// never names both, and there is nowhere to write the mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceKind {
    Ceilings,
    Floors,
}

/// The payload type's own kind -- the one place a payload and a
/// [`SurfaceKind`] are paired.
pub trait SurfacePayloadKind: SurfaceEnvelope {
    const KIND: SurfaceKind;
}

impl SurfacePayloadKind for CeilingPayload {
    const KIND: SurfaceKind = SurfaceKind::Ceilings;
}

impl SurfacePayloadKind for FloorPayload {
    const KIND: SurfaceKind = SurfaceKind::Floors;
}

impl SurfaceKind {
    pub fn of<P: SurfacePayloadKind>() -> Self {
        P::KIND
    }

    pub fn snapshot_kind(self) -> SnapshotKind {
        match self {
            SurfaceKind::Ceilings => SnapshotKind::Ceilings,
            SurfaceKind::Floors => SnapshotKind::Floors,
        }
    }

    /// The schema version a push of this entity must carry. Each entity
    /// versions independently, sharing a record or not.
    pub fn supported_schema(self) -> u32 {
        match self {
            SurfaceKind::Ceilings => crate::contract::SUPPORTED_CEILING_SCHEMA,
            SurfaceKind::Floors => crate::contract::SUPPORTED_FLOOR_SCHEMA,
        }
    }

    /// Singular, for a log line or an error message.
    pub fn element_label(self) -> &'static str {
        match self {
            SurfaceKind::Ceilings => "ceiling",
            SurfaceKind::Floors => "floor",
        }
    }
}

/// One ceiling or floor as served.
#[derive(Debug, Clone, Serialize)]
pub struct SurfaceResponse {
    #[serde(flatten)]
    pub surface: Surface,
    /// This surface's row in the result's `type_property_sets`. See
    /// `OpeningResponse::type_properties_ref`, which this mirrors exactly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_properties_ref: Option<u32>,
    pub project_id: String,
    /// The model that owns the surface — which is not necessarily the model
    /// that owns the rooms in `rooms` below, though on every document measured
    /// so far it is.
    pub model_id: String,
    pub source: String,

    /// Every room this surface lies over, largest overlap first.
    ///
    /// **Empty means unattributed, and that is a reported state.** A ceiling
    /// over a stairwell, an external soffit, a slab on a level that carries no
    /// rooms at all legitimately belongs to nothing — 8 of House A's 30
    /// ceilings, 17 of RHH's 1,833. Never an error, and not on its own a
    /// finding.
    pub rooms: Vec<SurfaceRoom>,
}

/// A surface resolves its own two property tiers plus a small set of
/// intrinsics — the same vocabulary a door or an item answers, so a column
/// name that projects on one entity projects on every other.
///
/// **Reference sources are absent rather than unsupported**: no source declares
/// `entity = "ceilings"` today, and answering `Absent` is what a room gives for
/// a source it did not match, so the day one is declared the same name starts
/// resolving instead of changing meaning.
impl FilterTarget for SurfaceResponse {
    fn presence(&self, source: Option<&str>, property: &str, builtin_defs: &[BuiltinPropertyDef]) -> PropertyPresence {
        /// A surface's own struct fields always exist, so blank collapses to
        /// `Empty`, never `Absent`.
        fn intrinsic(value: Option<&str>) -> PropertyPresence {
            match value {
                None => PropertyPresence::Absent,
                Some("") => PropertyPresence::Empty,
                Some(v) => PropertyPresence::Present(v.to_string()),
            }
        }

        match source {
            Some(_) => PropertyPresence::Absent,
            None => match property {
                "$id" => intrinsic(Some(&self.surface.id)),
                "$type_name" => intrinsic(self.surface.type_name.as_deref()),
                "$type_id" => intrinsic(self.surface.type_id.as_deref()),
                "$level_id" => intrinsic(Some(&self.surface.level_id)),
                "$model_id" => intrinsic(Some(&self.model_id)),
                // Read from Revit, never re-derived, and the field a floors
                // report filters on to tell a finish from a roof build-up.
                "$height_offset" => match self.surface.height_offset {
                    Some(v) => PropertyPresence::Present(format!("{v:.3}")),
                    None => PropertyPresence::Absent,
                },
                canonical => crate::contract::property_presence(&self.surface, canonical, &self.source, builtin_defs),
            },
        }
    }
}

/// What a surface read is scoped to.
pub struct SurfaceScope<'a> {
    pub project: Option<&'a str>,
    pub milestone: Option<&'a str>,
    /// The storeys the viewer is showing; `None` reads every storey. Narrows
    /// which surfaces are served, never which rooms they are attributed
    /// against. See `entity_scope::StoreyScope`.
    pub storeys: Option<&'a entity_scope::StoreyScope>,
}

/// A surface read before it is named for its entity. See `openings::Assembled`,
/// which this mirrors: share everything except what would change a serde key.
pub struct Assembled {
    pub revision: String,
    pub phase_by_model: BTreeMap<String, BTreeMap<String, Option<String>>>,
    pub levels_by_model: BTreeMap<String, Vec<Level>>,
    pub surfaces: Vec<SurfaceResponse>,
}

/// The ceilings and floors use of `type_table::tabulate`: the one place a
/// surface's bag field and row field are named.
fn tabulate_surfaces(surfaces: &mut [SurfaceResponse]) -> Vec<Arc<PropertyMap>> {
    super::type_table::tabulate(surfaces, |s| (&mut s.surface.type_properties, &mut s.type_properties_ref))
}

/// The whole `/ceilings` body.
#[derive(Debug, Clone, Serialize)]
pub struct CeilingsResult {
    pub revision: String,
    pub phase_by_model: BTreeMap<String, BTreeMap<String, Option<String>>>,
    pub levels_by_model: BTreeMap<String, Vec<Level>>,
    pub ceilings: Vec<SurfaceResponse>,
    /// Each distinct type-property bag once; see `service::type_table`.
    pub type_property_sets: Vec<Arc<PropertyMap>>,
}

impl From<Assembled> for CeilingsResult {
    fn from(mut a: Assembled) -> Self {
        Self {
            type_property_sets: tabulate_surfaces(&mut a.surfaces),
            revision: a.revision,
            phase_by_model: a.phase_by_model,
            levels_by_model: a.levels_by_model,
            ceilings: a.surfaces,
        }
    }
}

/// The whole `/floors` body. A separate type from `CeilingsResult` for its one
/// differing key and nothing else.
#[derive(Debug, Clone, Serialize)]
pub struct FloorsResult {
    pub revision: String,
    pub phase_by_model: BTreeMap<String, BTreeMap<String, Option<String>>>,
    pub levels_by_model: BTreeMap<String, Vec<Level>>,
    pub floors: Vec<SurfaceResponse>,
    /// Each distinct type-property bag once; see `service::type_table`.
    pub type_property_sets: Vec<Arc<PropertyMap>>,
}

impl From<Assembled> for FloorsResult {
    fn from(mut a: Assembled) -> Self {
        Self {
            type_property_sets: tabulate_surfaces(&mut a.surfaces),
            revision: a.revision,
            phase_by_model: a.phase_by_model,
            levels_by_model: a.levels_by_model,
            floors: a.surfaces,
        }
    }
}

/// Assemble a ceilings or floors read, or `None` when no snapshot of that kind
/// exists at all — the same "nothing pushed" signal every other entity returns.
pub fn assemble_surfaces<P: SurfacePayloadKind>(
    state: &AppState,
    scope: &SurfaceScope<'_>,
) -> Result<Option<Assembled>, ServiceError> {
    let kind = SurfaceKind::of::<P>();
    if !state.has_any_snapshot(kind.snapshot_kind()).map_err(ServiceError::Internal)? {
        return Ok(None);
    }

    // Phase 1 -- scope, through the same function every entity uses.
    let scoped: Vec<(ModelKey, P)> =
        entity_scope::scope_snapshots(state, kind.snapshot_kind(), scope.project, scope.milestone)?;

    let revision = entity_scope::revision(&scoped);
    let phase_by_model = entity_scope::phase_by_model(&scoped);
    let levels_by_model = entity_scope::levels_by_model(&scoped);
    // Resolved once per read, asked once per surface below.
    let storeys = scope.storeys.map(|s| s.admitter(&levels_by_model));
    let placement = super::placement::from_index(state)?;

    // Phase 2 -- the room candidates, once per project.
    //
    // `SameModel` always, never the project's `room_resolution` setting: that
    // setting exists to decide whether geometry may *override an absent
    // authored reference*, and a surface has no authored reference to be
    // absent. Reading it here would let a project switch this entity off
    // entirely and report every ceiling or floor as homeless, which is a wrong
    // answer rather than a disabled feature.
    //
    // The rooms are read once for every project in scope, and not at all when
    // no surface is.
    let registry = state.settings();
    let rooms = if scoped.is_empty() {
        None
    } else {
        Some(super::rooms::scope_payloads(state, &registry, scope.project, scope.milestone)?)
    };
    let mut candidates_by_project: BTreeMap<String, entity_scope::Candidates> = BTreeMap::new();
    for (_, payload) in &scoped {
        if candidates_by_project.contains_key(&payload.project().id) {
            continue;
        }
        let rooms = rooms.iter().flat_map(super::rooms::ScopedRooms::models);
        candidates_by_project.insert(
            payload.project().id.clone(),
            entity_scope::build_candidates(rooms, Some(&payload.project().id), RoomResolution::SameModel, &scoped),
        );
    }

    // Phase 3 -- derive.
    let mut surfaces: Vec<SurfaceResponse> = Vec::new();
    for (_key, payload) in &scoped {
        let project_id = &payload.project().id;
        let model = payload.model();
        let model_frame = placement.for_model(project_id, &model.id);
        let candidates = candidates_by_project.get(project_id);

        for surface in payload.surfaces() {
            // Before attribution, which is the expensive half of this read.
            if storeys.as_ref().is_some_and(|s| !s.admits(&model.id, &surface.level_id)) {
                continue;
            }
            // Attribution runs in the MODEL's own frame, before placement --
            // the room candidates were built in that frame too under
            // `SameModel`, so placing the surface first would compare a placed
            // surface against unplaced rooms and miss every time.
            //
            // The storey is found by the elevation the surface's OWN model
            // states for its own level id -- a `Level.id` is per document, so
            // the elevation is what crosses. A surface naming a level its model
            // does not declare is attributed to nothing rather than to every
            // room at every height.
            let rooms = candidates
                .and_then(|c| {
                    let elevation = c.elevation_of(&model.id, &surface.level_id)?;
                    Some(attribute(surface, elevation, c.rooms_in_model(&model.id)))
                })
                .unwrap_or_default();

            let mut surface = surface.clone();
            if let Some(transform) = model_frame {
                // Every piece, not just the first: placing one of them would put
                // the rest in the wrong frame.
                for piece in &mut surface.polygons {
                    super::placement::place_loops(transform, &mut piece.loops);
                }
            }
            surfaces.push(SurfaceResponse {
                surface,
                type_properties_ref: None,
                project_id: project_id.clone(),
                model_id: model.id.clone(),
                source: model.source.clone(),
                rooms,
            });
        }
    }

    Ok(Some(Assembled { revision, phase_by_model, levels_by_model, surfaces }))
}

/// Every surface attributed to one room, for the room-centric question these
/// entities were actually asked: "list ceilings by room", "list floors by room".
///
/// A thin inversion rather than a second assembly, so both answers are derived
/// from one attribution pass and cannot disagree.
pub fn by_room(surfaces: &[SurfaceResponse]) -> BTreeMap<RoomRef, Vec<&SurfaceResponse>> {
    let mut out: BTreeMap<RoomRef, Vec<&SurfaceResponse>> = BTreeMap::new();
    for response in surfaces {
        for room in &response.rooms {
            out.entry(RoomRef { model_id: room.model_id.clone(), room_id: room.room_id.clone() })
                .or_default()
                .push(response);
        }
    }
    out
}
