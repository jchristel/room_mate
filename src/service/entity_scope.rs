//! The scoping, pinning and geometry every entity's read shares.
//!
//! **Extracted when FF&E arrived, and the trigger is worth stating because it
//! is not the obvious one.** Windows cost none of this: they reuse `Opening`,
//! so they reuse the whole opening assembly by construction. An `Item` is a
//! different record, so the choice was between a second copy of this pipeline
//! and naming what the pipeline actually touches. It touches
//! [`SnapshotEnvelope`](crate::contract::SnapshotEnvelope) -- six field reads --
//! and never the element list, which is why everything here is generic over
//! that trait and nothing here knows what an opening or an item is.
//!
//! What stayed behind in [`openings`](super::openings) is exactly the part that
//! could not come: how many room references an element has, and how geometry
//! resolves them. That is the split
//! [`room_locator`](super::room_locator)'s own header predicted -- "the next
//! category needs the glue, not the geometry" -- reached from the other
//! direction.
//!
//! The two locator entry points are the shape of that split. An opening sits
//! *in* a wall, so its point must be stepped off the wall before it is tested
//! (`locate_sides`); an item sits *in* a room, so its point is tested where it
//! is (`locate_within`). Same candidates, same elevation axis, same probe --
//! different question.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::contract::{Loop, ModelToShared, Point2D, SnapshotEnvelope};
use crate::settings::{Milestone, RoomResolution};
use crate::state::{AppState, ModelKey};
use crate::storage::SnapshotKind;

use super::room_locator::{self, RoomRef, Unresolved};
use super::ServiceError;

/// A stable content revision for a `DoorsResult`. Duplicated from
/// `rooms::scoped_revision` rather than shared: that one takes room-scoped
/// tuples, and the shared part is three lines of hashing whose meaning ("which
/// snapshot did each model contribute") is per entity.
pub fn revision<P: SnapshotEnvelope>(scoped: &[(ModelKey, P)]) -> String {
    use std::hash::{Hash, Hasher};

    let mut parts: Vec<(&str, &str, &str)> = scoped
        .iter()
        .map(|(key, payload)| (key.project_id.as_str(), key.model_id.as_str(), payload.taken_at()))
        .collect();
    parts.sort_unstable();

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Where one side's room reference came from.
///
/// **On the wire because a consumer must be able to tell a stated answer from a
/// computed one.** The same rule `through_wall_normal` states for direction: a
/// guessed value nothing can distinguish from a measured one is worse than an
/// absent one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "origin", content = "value")]
pub enum SideOrigin {
    /// The model stated it. Always wins — see `RoomResolution`.
    Authored(RoomRef),
    /// The model stated nothing and the geometry found a room.
    Derived(RoomRef),
    /// The model stated nothing and the geometry did not resolve one, carrying
    /// why. `no_candidate` on one side of an otherwise two-sided door is an
    /// **external door**, which is the correct answer rather than a gap.
    Unresolved(Unresolved),
}

impl SideOrigin {
    /// The room this side resolves to, whatever it came from.
    pub fn room(&self) -> Option<&RoomRef> {
        match self {
            SideOrigin::Authored(r) | SideOrigin::Derived(r) => Some(r),
            SideOrigin::Unresolved(_) => None,
        }
    }
}

/// The smallest step that still leaves the wall, in feet (~15 mm).
///
/// A **centreline** model has a wall gap of zero — neighbouring rooms already
/// tile, so their boundaries are one shared line. Probing by zero would test
/// that line itself, where containment is undefined and a room may or may not
/// claim its own edge. Any positive step lands cleanly in one room or the other,
/// so this is a floor rather than a tolerance to tune.
pub const MIN_PROBE_FT: f64 = 0.05;

/// Every room a door could be resolved against, prepared once per read.
///
/// Built once rather than per door: the rooms come from storage, and reading
/// them inside the door loop would be one storage read per door.
pub struct Candidates {
    /// Candidates grouped by the model that owns them. `SameModel` probes one
    /// group; `Project` probes the union, which is the entire difference between
    /// the two modes — the same probe, a different set of rooms allowed to
    /// answer it.
    by_model: BTreeMap<String, Vec<room_locator::Candidate>>,
    /// Every candidate, already placed in the shared frame. Empty outside
    /// `Project` mode, where models are never mixed.
    shared: Vec<room_locator::Candidate>,
    /// `(model id, level id)` → elevation, so a *door* — which carries a level
    /// id, not an elevation — can be put on the same axis as the rooms.
    elevation: BTreeMap<(String, String), f64>,
    /// Model id → the wall gap its rooms were drawn to, so a probe is sized by
    /// the regime of the rooms it is reaching for rather than by a constant.
    gap_by_model: BTreeMap<String, f64>,
    /// Model id → its `model_to_shared`, needed in `Project` mode to lift the
    /// door's own point into the frame the candidates are already in.
    transform_by_model: BTreeMap<String, ModelToShared>,
    shared_frame: bool,
}

/// Apply a 2D affine to a **position**: `shared_x = a*x + c*y + e`.
fn place(m: &ModelToShared, p: Point2D) -> Point2D {
    let [a, b, c, d, e, f] = m.matrix;
    Point2D { x: a * p.x + c * p.y + e, y: b * p.x + d * p.y + f }
}

/// Apply only the **linear** part to a direction, then renormalise.
///
/// A normal is a direction, not a position, so the translation must not reach
/// it — a door's facing does not move when the model is placed somewhere else on
/// the survey grid. The transform is a rigid-body rotation
/// (`ModelToShared::is_rigid`), so renormalising is defensive rather than
/// corrective: it costs one square root and means a scaled matrix that slipped
/// past ingest's warning cannot silently lengthen every probe.
fn place_direction(m: &ModelToShared, p: Point2D) -> Option<Point2D> {
    let [a, b, c, d, _e, _f] = m.matrix;
    let (x, y) = (a * p.x + c * p.y, b * p.x + d * p.y);
    let len = (x * x + y * y).sqrt();
    if len < 1e-9 {
        return None;
    }
    Some(Point2D { x: x / len, y: y / len })
}

/// Collect the project's rooms as probe candidates, under the same milestone
/// scope the doors read is using.
///
/// **Scoped through `rooms::scope_payloads`, not by re-reading the store.** A
/// door has to be resolved against exactly the rooms `/rooms` is serving, or a
/// milestone read would answer two different questions about one building.
pub fn build_candidates<P: SnapshotEnvelope>(
    state: &AppState,
    project: Option<&str>,
    milestone: Option<&str>,
    mode: RoomResolution,
    element_payloads: &[(ModelKey, P)],
) -> Result<Candidates, ServiceError> {
    let registry = state.settings();
    let stored = state.all_snapshots(project).map_err(ServiceError::Internal)?;
    let (scoped, _) = super::rooms::scope_payloads(state, &registry, stored, project, milestone)?;

    let shared_frame = mode == RoomResolution::Project;
    let mut out = Candidates {
        by_model: BTreeMap::new(),
        shared: Vec::new(),
        elevation: BTreeMap::new(),
        gap_by_model: BTreeMap::new(),
        transform_by_model: BTreeMap::new(),
        shared_frame,
    };

    for (key, payload, bundle) in scoped {
        let boundary = bundle.areas.resolve_boundary(payload.room_boundary);
        out.gap_by_model.insert(key.model_id.clone(), bundle.areas.wall_gap_ft(boundary));
        if let Some(transform) = payload.model_to_shared {
            out.transform_by_model.insert(key.model_id.clone(), transform);
        }

        // Elevations come from this model's own `levels`, never from a merged
        // list: a level id is per-document, and two linked models name the same
        // floor with different ids. The elevation is what crosses.
        for level in &payload.levels {
            out.elevation.insert((key.model_id.clone(), level.id.clone()), level.elevation);
        }

        for room in &payload.rooms {
            let Some(outline) = room_locator::outline_of(room) else {
                continue; // unplaced room: nothing to probe against
            };
            let Some(&elevation) = out.elevation.get(&(key.model_id.clone(), room.level_id.clone())) else {
                continue; // a room on a level this model does not declare
            };
            let reference = RoomRef { model_id: key.model_id.clone(), room_id: room.id.clone() };
            if shared_frame {
                let Some(transform) = payload.model_to_shared else {
                    // Un-placed in a mode where everything else has been placed.
                    // Including it would probe it in the wrong frame, which is
                    // worse than leaving it out — a wrong room resolves and
                    // looks right.
                    continue;
                };
                let placed = geo::MapCoords::map_coords(&outline, |c| {
                    let p = place(&transform, Point2D { x: c.x, y: c.y });
                    geo::Coord { x: p.x, y: p.y }
                });
                out.shared.push(room_locator::Candidate { reference, outline: placed, elevation });
            } else {
                out.by_model.entry(key.model_id.clone()).or_default().push(room_locator::Candidate {
                    reference,
                    outline,
                    elevation,
                });
            }
        }
    }

    // A doors-only model declares its own levels and placement on the doors
    // envelope, because it has no rooms snapshot to declare them. Filled in
    // *after* the rooms pass and only where the key is absent, so a model that
    // pushes both is answered by its rooms — which keeps the duplicate a
    // redundancy rather than something that could disagree.
    //
    // This is what makes such a model's doors reachable at all: `locate` gives
    // up before probing when the elevation lookup misses, so without this every
    // door in a facade or envelope file reports `NoCandidate` however good its
    // geometry is. It only pays off under `Project` — a model with no rooms has
    // no same-model candidates to be probed against, whatever its elevations.
    for (key, payload) in element_payloads {
        if project.is_some_and(|p| payload.project().id != p) {
            continue;
        }
        for level in payload.levels() {
            out.elevation.entry((key.model_id.clone(), level.id.clone())).or_insert(level.elevation);
        }
        if let Some(transform) = payload.model_to_shared().cloned() {
            out.transform_by_model.entry(key.model_id.clone()).or_insert(transform);
        }
    }
    Ok(out)
}

/// No rooms to probe against at all — the answer when resolution is off, which
/// is every read that did not ask for it.
const NO_CANDIDATES: &[room_locator::Candidate] = &[];

impl Candidates {
    /// Everything the probe needs, in the frame the candidates are in -- or the
    /// reason there is nothing to probe.
    ///
    /// **Shared by both entry points below, and it is the half that is genuinely
    /// identical.** Whether an element is stepped off a wall or tested where it
    /// stands, it first has to have a position, an elevation to compare rooms
    /// on, a frame the candidates are already in, and a probe sized by the
    /// regime of the rooms being reached for. Writing that twice is how the two
    /// entities would drift on what `UnknownLevel` means.
    fn prepare(
        &self,
        probe: &Probe<'_>,
        model_id: &str,
    ) -> Result<(room_locator::Placement, &[room_locator::Candidate], f64), Unresolved> {
        let Some(mut point) = room_locator::position_of(probe.insertion_point, probe.loops) else {
            return Err(Unresolved::NoPosition);
        };
        let Some(&elevation) = self.elevation.get(&(model_id.to_string(), probe.level_id.to_string())) else {
            // The opening names a level nothing in scope has an elevation for,
            // so there is no axis to compare on and nothing is probed.
            //
            // Reported as its own state rather than as NoCandidate, which would
            // read as "the probe found open air" -- the ordinary answer for an
            // external opening. This is not that: an unhosted element gets an
            // invalid LevelId from Revit and the export carries -1, so the cause
            // is upstream of any geometry a reader would go looking at.
            return Err(Unresolved::UnknownLevel);
        };
        let mut normal = probe.normal;

        let candidates: &[room_locator::Candidate] = if self.shared_frame {
            // The candidates have been placed, so the door has to be too. A
            // model with no transform cannot be compared against ones that were
            // placed — it would be probed in the wrong frame.
            let Some(transform) = self.transform_by_model.get(model_id) else {
                return Err(Unresolved::NoPosition);
            };
            point = place(transform, point);
            normal = normal.and_then(|n| place_direction(transform, n));
            &self.shared
        } else {
            self.by_model.get(model_id).map_or(NO_CANDIDATES, Vec::as_slice)
        };

        // Sized by the regime of the rooms being reached for. `SameModel` only
        // ever reaches its own model's rooms; `Project` may reach any, so the
        // widest gap in scope is the honest step — a shorter one would resolve
        // some models and silently not others.
        let gap = if self.shared_frame {
            self.gap_by_model.values().copied().fold(0.0_f64, |a, b| a.max(b))
        } else {
            self.gap_by_model.get(model_id).copied().unwrap_or_default()
        };

        Ok((room_locator::Placement { point, normal, elevation }, candidates, gap.max(MIN_PROBE_FT)))
    }

    /// Resolve both sides of a wall-hosted element -- a door or a window.
    pub fn locate_sides(&self, probe: &Probe<'_>, model_id: &str) -> room_locator::Sides {
        match self.prepare(probe, model_id) {
            Ok((placement, candidates, step)) => room_locator::locate(&placement, candidates, step),
            Err(why) => room_locator::Sides {
                from: room_locator::Located::Unresolved(why),
                to: room_locator::Located::Unresolved(why),
            },
        }
    }

    /// Resolve the one room an element stands in -- an FF&E instance.
    ///
    /// **Not `locate_sides` with the normal dropped.** That would answer
    /// `NoDirection` for every item, which is a true statement about a question
    /// nobody asked: an item has no wall to pass through, so having no
    /// through-wall direction is its ordinary condition rather than a failure to
    /// resolve. This tests the point where it stands, and the only unresolved
    /// states reachable are the ones that mean something for an item --
    /// `NoPosition`, `UnknownLevel`, `NoCandidate` and `Ambiguous`.
    pub fn locate_within(&self, probe: &Probe<'_>, model_id: &str) -> room_locator::Located {
        match self.prepare(probe, model_id) {
            Ok((placement, candidates, _)) => room_locator::locate_within(&placement, candidates),
            Err(why) => room_locator::Located::Unresolved(why),
        }
    }
}

/// Fold one side's authored reference and one side's derived answer into the
/// single origin the response carries.
///
/// **Authored always wins.** A door's `to_room` is the modeller's assignment —
/// what the door *serves*, which is not always what it opens into — so geometry
/// replacing it would be the reconciliation `CLAUDE.md` forbids. Geometry fills
/// what the model left absent, and disagrees audibly with what it did not
/// (`OpeningReport::room_geometry_mismatches`).
pub fn side_origin(authored: Option<&str>, model_id: &str, derived: &room_locator::Located) -> SideOrigin {
    if let Some(room_id) = authored {
        return SideOrigin::Authored(RoomRef { model_id: model_id.to_string(), room_id: room_id.to_string() });
    }
    match derived {
        room_locator::Located::Found(reference) => SideOrigin::Derived(reference.clone()),
        room_locator::Located::Unresolved(why) => SideOrigin::Unresolved(*why),
    }
}

/// `(model id, room id)` → that room's building key, for the rooms in scope.
///
/// **Built by calling `assemble_rooms`, not by re-deriving classification.** A
/// door's building has to mean exactly what a room's building means, or a
/// building-scoped doors read and a building-scoped rooms read would disagree
/// about the same building — so this asks the same function `/rooms` does, with
/// the same project and milestone scope and deliberately *no* building filter
/// (the filtering happens per door, against the door's owner).
///
/// Keyed on the pair because room ids are unique only within a model.
pub fn building_by_room(
    state: &AppState,
    project: Option<&str>,
    milestone: Option<&str>,
) -> Result<BTreeMap<(String, String), String>, ServiceError> {
    let rooms =
        super::rooms::assemble_rooms(state, &super::rooms::RoomScope { project, milestone, ..Default::default() })?;
    let Some(rooms) = rooms else {
        return Ok(BTreeMap::new());
    };

    let registry = state.settings();
    let mut out = BTreeMap::new();
    for room in &rooms.rooms {
        let Some(tier) = registry
            .settings_for(&room.project_id)
            .and_then(|b| super::rooms::building_tier_index(&b.hierarchy))
        else {
            continue; // a project with no "Building" tier answers no building
        };
        if let Some(value) = room.classification.get(tier) {
            out.insert(
                (room.model_id.clone(), room.room.id.clone()),
                super::rooms::building_key(&value.code, &value.name),
            );
        }
    }
    Ok(out)
}

/// Scope one entity's stored snapshots to the request, substituting a
/// milestone's pinned snapshot for a model's latest where one is pinned.
///
/// **One implementation for four entities, and the milestone pin map is the
/// only thing that varies** -- which is why it arrives as a closure rather than
/// as a `SnapshotKind` this function would have to match on. `Milestone` carries
/// four independent maps because the entities are pushed independently and their
/// snapshot ids do not correspond; the caller knows which of the four it means,
/// and a match here would be a second place that has to be kept in step.
///
/// The discipline is `rooms::scope_payloads`' verbatim: a project without the
/// named milestone contributes nothing, a model that milestone does not pin
/// contributes nothing, and a pin whose snapshot no longer exists is skipped
/// with a warning rather than failing the read -- "signal, not error".
pub fn scope_snapshots<P: SnapshotEnvelope + serde::de::DeserializeOwned>(
    state: &AppState,
    kind: SnapshotKind,
    project: Option<&str>,
    milestone: Option<&str>,
    pins: impl Fn(&Milestone) -> &BTreeMap<String, String>,
) -> Result<Vec<(ModelKey, P)>, ServiceError> {
    let stored: Vec<(ModelKey, P)> = state.all_opening_snapshots(kind, project).map_err(ServiceError::Internal)?;
    let registry = state.settings();

    let mut scoped: Vec<(ModelKey, P)> = Vec::new();
    for (key, payload) in stored {
        if project.is_some_and(|p| payload.project().id != p) {
            continue;
        }
        let Some(bundle) = registry.settings_for(&payload.project().id) else {
            continue;
        };
        match milestone {
            None => scoped.push((key, payload)),
            Some(wanted) => {
                let Some(ms) = bundle.milestones.iter().find(|m| m.name == wanted) else {
                    continue;
                };
                let Some(pinned_id) = pins(ms).get(&key.model_id) else {
                    continue;
                };
                match state.get_opening_snapshot::<P>(kind, &key, pinned_id).map_err(ServiceError::Internal)? {
                    Some(pinned) => scoped.push((key, pinned)),
                    None => tracing::warn!(
                        "milestone '{}' pins {} snapshot {:?} for {}/{}, but no such snapshot exists -- skipping the model",
                        wanted,
                        kind.label(),
                        pinned_id,
                        key.project_id,
                        key.model_id
                    ),
                }
            }
        }
    }
    Ok(scoped)
}

/// The phase each contributing model was filtered to, keyed by project then
/// model.
///
/// Read off each **snapshot**, never off the lineage's current phase: a snapshot
/// written before phasing existed reports itself unphased forever, and that
/// stays true after a later push phases the lineage (PLAN-phasing D8).
pub fn phase_by_model<P: SnapshotEnvelope>(
    scoped: &[(ModelKey, P)],
) -> BTreeMap<String, BTreeMap<String, Option<String>>> {
    let mut out: BTreeMap<String, BTreeMap<String, Option<String>>> = BTreeMap::new();
    for (key, payload) in scoped {
        out.entry(key.project_id.clone())
            .or_default()
            .insert(key.model_id.clone(), payload.phase().map(str::to_string));
    }
    out
}

/// What an element gives the locator, whatever entity it is.
///
/// **`normal` is the whole difference between the two entities' geometry.** An
/// opening carries one and is probed on both sides of it; an item does not and
/// is tested where it stands. Carrying it as `Option` rather than splitting the
/// struct keeps one preparation path -- position, elevation, frame, probe size
/// -- which is the part that is genuinely identical and the part most likely to
/// drift if it were written twice.
pub struct Probe<'a> {
    pub insertion_point: Option<Point2D>,
    pub loops: &'a [Loop],
    pub level_id: &'a str,
    /// The plan direction to step along. `None` for an element that sits in a
    /// room rather than between two, where there is nothing to step off.
    pub normal: Option<Point2D>,
}
