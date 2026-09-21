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

use crate::contract::{Loop, ModelToShared, Point2D, RoomPayload, SnapshotEnvelope};
use crate::settings::{Milestone, RoomResolution};
use crate::state::{AppState, ModelKey, ProjectSettings, SettingsRegistry};
use crate::storage::{ModelIndexRow, SnapshotKind};

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
    /// The step for an element whose OWN model holds no rooms -- a facade or
    /// envelope file -- which is the project's `max_wall_thickness` whatever
    /// regime the rooms were drawn to.
    ///
    /// **A centreline gap of zero is a claim about walls the rooms share**, and
    /// the facade's wall is not one of them: it lives in another document, so
    /// the interior rooms stop at their own lining or at a link-bounded line
    /// short of it, never at the facade window's insertion point. Measured on
    /// RHH 2026-09-21: the median facade window sat 0.64 ft from the nearest
    /// room edge, so the 15 mm centreline step resolved 3 of 149 and a wall
    /// thickness reaches about 135. Only the room-less model gets it, because an
    /// opening in a model that has rooms IS on a shared line, and a 1.5 ft step
    /// there would jump a cupboard into the room beyond it.
    cross_model_gap: f64,
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

/// Collect one project's rooms as probe candidates.
///
/// **Handed the rooms, never reads them.** They must be the rooms
/// `rooms::scope_payloads` scoped for the same request, so an element is
/// resolved against exactly the rooms `/rooms` is serving — otherwise a
/// milestone read would answer two different questions about one building. It
/// used to do that scoping itself, which was right about *which* rooms and
/// costly about how often: every consumer in one request re-read and re-parsed
/// the same snapshots, up to four times in `/validation`.
///
/// `project` narrows `rooms` (and `element_payloads`) to one project, since a
/// read spanning projects builds one candidate set per project that wants one.
pub fn build_candidates<'a, P: SnapshotEnvelope>(
    rooms: impl IntoIterator<Item = (&'a ModelKey, &'a RoomPayload, &'a ProjectSettings)>,
    project: Option<&str>,
    mode: RoomResolution,
    element_payloads: &[(ModelKey, P)],
) -> Candidates {
    let scoped = rooms.into_iter().filter(|(key, _, _)| project.is_none_or(|p| key.project_id == p));

    let shared_frame = mode == RoomResolution::Project;
    let mut out = Candidates {
        by_model: BTreeMap::new(),
        shared: Vec::new(),
        elevation: BTreeMap::new(),
        gap_by_model: BTreeMap::new(),
        cross_model_gap: 0.0,
        transform_by_model: BTreeMap::new(),
        shared_frame,
    };

    for (key, payload, bundle) in scoped {
        let boundary = bundle.areas.resolve_boundary(payload.room_boundary);
        out.gap_by_model.insert(key.model_id.clone(), bundle.areas.wall_gap_ft(boundary));
        out.cross_model_gap = out.cross_model_gap.max(bundle.areas.max_wall_thickness);
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
    out
}

/// No rooms to probe against at all — the answer when resolution is off, which
/// is every read that did not ask for it.
const NO_CANDIDATES: &[room_locator::Candidate] = &[];

impl Candidates {
    /// This model's room candidates, already placed in the frame the set was
    /// built in.
    ///
    /// Exposed for `service::surfaces`, which needs the room POLYGONS rather
    /// than a point probe: a ceiling or a floor is not at a point, so it asks how much of
    /// itself overlaps each room instead of which room contains it. Reusing the
    /// candidate set rather than re-reading rooms is what keeps that answer
    /// scoped to exactly the rooms `/rooms` is serving -- the guarantee
    /// `build_candidates` exists for, and one a second reader would quietly
    /// break under a milestone.
    pub fn rooms_in_model(&self, model_id: &str) -> &[room_locator::Candidate] {
        self.by_model.get(model_id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The elevation this model states for one of its own level ids, or `None`
    /// when the model declares no such level.
    ///
    /// Per model on purpose: a `Level.id` is per document, so the same storey
    /// carries different ids in two linked models and the ELEVATION is what
    /// crosses. Same rule `src-js/renderer/storey.ts` follows.
    pub fn elevation_of(&self, model_id: &str, level_id: &str) -> Option<f64> {
        self.elevation.get(&(model_id.to_string(), level_id.to_string())).copied()
    }

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
        // some models and silently not others. An element whose own model has
        // no rooms is reaching across a wall no room shares: `cross_model_gap`.
        let gap = if self.shared_frame {
            let widest = self.gap_by_model.values().copied().fold(0.0_f64, |a, b| a.max(b));
            if self.gap_by_model.contains_key(model_id) {
                widest
            } else {
                widest.max(self.cross_model_gap)
            }
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
/// **Classified by the function `/rooms` uses, not re-derived.** A door's
/// building has to mean exactly what a room's building means, or a
/// building-scoped doors read and a building-scoped rooms read would disagree
/// about the same building — so this runs `rooms::rooms_result`, over rooms
/// scoped to the same project and milestone and deliberately *no* building
/// filter (the filtering happens per door, against the door's owner).
///
/// Takes the rooms rather than reading them: the same read resolves geometry
/// against them (`build_candidates`), and reading them here as well parsed them
/// twice. `rooms` must be scoped to `project` and `milestone`.
///
/// Keyed on the pair because room ids are unique only within a model.
pub(crate) fn building_by_room(
    state: &AppState,
    registry: &SettingsRegistry,
    rooms: &super::rooms::ScopedRooms<'_>,
    project: Option<&str>,
    milestone: Option<&str>,
) -> Result<BTreeMap<(String, String), String>, ServiceError> {
    let rooms = super::rooms::rooms_result(
        state,
        rooms,
        &super::rooms::RoomScope { project, milestone, ..Default::default() },
    )?;

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

/// This kind's snapshot pins on one milestone: model id → snapshot id.
///
/// **The one place a kind is paired with its pin map**, and an exhaustive match
/// so a new kind cannot compile without choosing one. `Milestone` carries a map
/// per entity because the entities are pushed independently and their snapshot
/// ids do not correspond.
///
/// The pairing used to be written three ways: a closure at every
/// `scope_snapshots` call, a `pins` method on each entity family, and -- in
/// `scope_cursor`, which is handed only a kind -- `attachments`, the *rooms*
/// map, for every kind. So the ETag of `/doors?milestone=` hashed a rooms
/// snapshot id the doors read never served. Two copies of one rule, and the copy
/// without the closure guessed.
pub fn milestone_pins(milestone: &Milestone, kind: SnapshotKind) -> &BTreeMap<String, String> {
    match kind {
        SnapshotKind::Rooms => &milestone.attachments,
        SnapshotKind::Doors => &milestone.door_attachments,
        SnapshotKind::Windows => &milestone.window_attachments,
        SnapshotKind::Ffe => &milestone.ffe_attachments,
        SnapshotKind::Spaces => &milestone.space_attachments,
        SnapshotKind::Ceilings => &milestone.ceiling_attachments,
        SnapshotKind::Floors => &milestone.floor_attachments,
    }
}

/// One model's contribution to a scoped read, decided before any snapshot is
/// opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRead {
    pub key: ModelKey,
    /// The snapshot id (`taken_at`) to serve: the model's latest, or the id a
    /// milestone pins.
    pub id: String,
}

/// Which snapshot of `kind` each in-scope model contributes, decided from the
/// store index and the settings alone.
///
/// **Parsing is what a read costs, so everything that can be decided without it
/// is decided here, first.** The loop this replaced parsed every model's latest
/// snapshot and only then asked whether the request wanted it -- so under a
/// milestone each one was parsed, discarded, and its pinned snapshot parsed in
/// its place: two parses for one answer, the larger of them thrown away.
///
/// **Shared with `scope_cursor`, and that is the correctness half.** The cursor
/// is the ETag. A cursor and a read that each decide "which snapshot" can
/// disagree, and the unsafe direction of that disagreement is a 304 for a body
/// that changed -- which is exactly how the cursor came to hash rooms pins for
/// every kind (see `milestone_pins`). One function cannot disagree with itself.
///
/// The rules are the ones the two loops already applied: an unregistered
/// project contributes nothing; without a milestone a model contributes its
/// latest; with one, a project lacking that milestone contributes nothing and a
/// model it does not pin contributes nothing. A model holding no snapshot of
/// this kind contributes nothing even when pinned -- nothing was ever pushed for
/// the pin to name.
///
/// **Whether a pinned snapshot still exists is not decided here.** The index
/// knows each model's latest and nothing older, so a dangling pin is planned and
/// the read drops it with a warning. For the cursor that errs the safe way: it
/// counts a model the body will not contain, which can only over-report change.
///
/// Takes the index and registry rather than the state so a caller holding one
/// settings snapshot for the whole request plans against *that* snapshot.
pub fn plan_reads(
    index: &[ModelIndexRow],
    registry: &SettingsRegistry,
    kind: SnapshotKind,
    project: Option<&str>,
    milestone: Option<&str>,
) -> Vec<PlannedRead> {
    let mut plan = Vec::new();
    for row in index {
        if project.is_some_and(|p| row.key.project_id != p) {
            continue;
        }
        let Some(bundle) = registry.settings_for(&row.key.project_id) else {
            continue;
        };
        let Some(latest) = row.latest.get(&kind) else {
            continue;
        };
        let id = match milestone {
            None => latest,
            Some(wanted) => {
                let Some(ms) = bundle.milestones.iter().find(|m| m.name == wanted) else {
                    continue;
                };
                let Some(pinned) = milestone_pins(ms, kind).get(&row.key.model_id) else {
                    continue;
                };
                pinned
            }
        };
        plan.push(PlannedRead { key: row.key.clone(), id: id.clone() });
    }
    plan
}

/// Scope one entity's stored snapshots to the request, substituting a
/// milestone's pinned snapshot for a model's latest where one is pinned.
///
/// **One implementation for every entity**: plan from the index
/// (`plan_reads`), then open exactly the snapshots the plan names. A pin whose
/// snapshot no longer exists is skipped with a warning rather than failing the
/// read -- "signal, not error".
pub fn scope_snapshots<P: SnapshotEnvelope + serde::de::DeserializeOwned>(
    state: &AppState,
    kind: SnapshotKind,
    project: Option<&str>,
    milestone: Option<&str>,
) -> Result<Vec<(ModelKey, P)>, ServiceError> {
    let index = state.model_index().map_err(ServiceError::Internal)?;
    let plan = plan_reads(&index, &state.settings(), kind, project, milestone);
    read_planned(state, kind, plan, milestone)
}

/// Open and parse the snapshots a plan names, in plan order.
///
/// Split from `scope_snapshots` for the rooms read, which plans against the
/// settings snapshot it holds for the whole request rather than taking a fresh
/// one here.
pub(crate) fn read_planned<P: serde::de::DeserializeOwned>(
    state: &AppState,
    kind: SnapshotKind,
    plan: Vec<PlannedRead>,
    milestone: Option<&str>,
) -> Result<Vec<(ModelKey, P)>, ServiceError> {
    let mut scoped = Vec::with_capacity(plan.len());
    for PlannedRead { key, id } in plan {
        match state.get_opening_snapshot::<P>(kind, &key, &id).map_err(ServiceError::Internal)? {
            Some(payload) => scoped.push((key, payload)),
            None => match milestone {
                Some(wanted) => tracing::warn!(
                    "milestone '{}' pins {} snapshot {:?} for {}/{}, but no such snapshot exists -- skipping the model",
                    wanted,
                    kind.label(),
                    id,
                    key.project_id,
                    key.model_id
                ),
                // The index named it a moment ago, so the file went between the
                // two reads -- a hand deletion, since nothing in the server
                // deletes a snapshot.
                None => tracing::warn!(
                    "{} snapshot {:?} for {}/{} is indexed as the latest but could not be read -- skipping the model",
                    kind.label(),
                    id,
                    key.project_id,
                    key.model_id
                ),
            },
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

/// The levels each contributing model declares, keyed by model id.
///
/// **Why every element entity has to send this.** A `Level.id` is unique only
/// within its own document, and the level picker is built from `/rooms`, which
/// deduped its ids across the project's linked models (`rooms::dedup_levels`).
/// An element's `level_id` is neither: it is the raw id its own model used. So
/// matching an element to the displayed storey by id is wrong twice over -- a
/// model that lost the dedup race has every element silently dropped, and a
/// model with no rooms at all (a facade package) contributes no canonical id
/// for its elements to match in the first place. RHH's facade model is exactly
/// that case: 78 external doors, invisible on every level.
///
/// **Elevation is what crosses documents**, so the consumer resolves
/// `(model_id, level_id) -> elevation` through this map and matches on the
/// number. `service::spaces` reached the same conclusion first and this is that
/// map, hoisted so all four element entities share one answer.
///
/// **Empty for a model that pushed no levels**, which is legal -- they are
/// optional on the envelope -- and which a consumer must tell apart from "no
/// level matches".
pub fn levels_by_model<P: SnapshotEnvelope>(scoped: &[(ModelKey, P)]) -> BTreeMap<String, Vec<crate::contract::Level>> {
    scoped
        .iter()
        .map(|(key, payload)| (key.model_id.clone(), payload.levels().to_vec()))
        .collect()
}

/// The storeys an element read is narrowed to -- the ones the viewer is
/// showing, which may be several (one per zone).
///
/// **Why the server narrows at all.** An element read used to ship every storey
/// and let the viewer throw most of it away: RHH's `/ffe` is 38,913 items across
/// dozens of storeys to draw the one or two a reader is looking at. Parsing what
/// is stored still reads the whole snapshot, but skipping an element here spares
/// its clone, its reference joins, its geometric probe, its serialisation, the
/// transfer and the browser's parse.
///
/// **A superset of what the viewer keeps, never a second opinion.** The rule
/// that puts an element on a storey is `onStorey` in `src-js/renderer/storey.ts`
/// -- name AND elevation, with announced fallbacks decided over the WHOLE
/// payload. The server does not re-implement that; it only guarantees that
/// everything `onStorey` could keep for any requested storey is still in the
/// body, so every one of its decisions comes out the same:
///
/// - a model declaring levels keeps an element whose own level is within
///   `LEVEL_EPS_MM` of ANY requested elevation. Elevation alone, deliberately
///   looser than `onStorey`'s name-and-elevation: the viewer's "by elevation"
///   fallback needs those elements, and it narrows the rest itself;
/// - a model declaring NO levels keeps an element whose `level_id` is one of the
///   requested storeys' ids -- `onStorey`'s id fallback for such a model;
/// - a read where no model declares any level is not narrowed at all, because
///   `onStorey` then shows everything and says so.
///
/// `levels_by_model` on the response stays whole, since those fallbacks read it.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreyScope {
    elevations: Vec<f64>,
    level_ids: std::collections::BTreeSet<String>,
}

impl StoreyScope {
    /// From the query's `storey_elevations` and `storey_level_ids`, both
    /// comma-separated. `None` when neither is given, which is an unscoped read.
    ///
    /// Two independent sets rather than pairs, because the rule uses them
    /// independently: elevations for models that declare levels, ids for models
    /// that do not.
    pub fn parse(elevations: Option<&str>, level_ids: Option<&str>) -> Result<Option<Self>, String> {
        if elevations.is_none() && level_ids.is_none() {
            return Ok(None);
        }
        let parts = |s: Option<&str>| {
            s.unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        let elevations = parts(elevations)
            .iter()
            .map(|p| {
                p.parse::<f64>()
                    .ok()
                    .filter(|e| e.is_finite())
                    .ok_or_else(|| format!("storey_elevations: {p:?} is not a number"))
            })
            .collect::<Result<_, _>>()?;
        Ok(Some(Self { elevations, level_ids: parts(level_ids).into_iter().collect() }))
    }

    /// This scope prepared against one read's own levels.
    pub fn admitter<'a>(
        &'a self,
        levels_by_model: &'a BTreeMap<String, Vec<crate::contract::Level>>,
    ) -> StoreyAdmitter<'a> {
        let by_model: BTreeMap<&str, BTreeMap<&str, f64>> = levels_by_model
            .iter()
            .map(|(model, levels)| (model.as_str(), levels.iter().map(|l| (l.id.as_str(), l.elevation)).collect()))
            .collect();
        let have_levels = by_model.values().any(|levels| !levels.is_empty());
        StoreyAdmitter { scope: self, by_model, have_levels }
    }
}

/// A `StoreyScope` resolved against one read's `levels_by_model`: built once,
/// asked once per element.
pub struct StoreyAdmitter<'a> {
    scope: &'a StoreyScope,
    by_model: BTreeMap<&'a str, BTreeMap<&'a str, f64>>,
    have_levels: bool,
}

impl StoreyAdmitter<'_> {
    /// Whether an element on `level_id` of `model_id` belongs in the body. See
    /// `StoreyScope` for the three cases and why each is a superset.
    pub fn admits(&self, model_id: &str, level_id: &str) -> bool {
        if !self.have_levels {
            return true;
        }
        match self.by_model.get(model_id).filter(|levels| !levels.is_empty()) {
            None => self.scope.level_ids.contains(level_id),
            Some(levels) => levels.get(level_id).is_some_and(|elevation| {
                self.scope
                    .elevations
                    .iter()
                    .any(|wanted| (elevation - wanted).abs() <= room_locator::LEVEL_EPS_MM)
            }),
        }
    }
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use super::*;
    use crate::contract::{DoorPayload, Model, Project, Snapshot, SUPPORTED_DOOR_SCHEMA};
    use crate::storage::{FsStore, SnapshotMeta, SnapshotStore};

    fn bundle(milestones: Vec<Milestone>) -> ProjectSettings {
        ProjectSettings {
            spaces: Default::default(),
            reference: BTreeMap::new(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec![],
            milestones,
            anchor_model: None,
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            windows: Default::default(),
            ffe: Default::default(),
            hierarchy_exclusions: vec![],
        }
    }

    fn milestone(name: &str, rooms: &[(&str, &str)], doors: &[(&str, &str)]) -> Milestone {
        let pins = |p: &[(&str, &str)]| p.iter().map(|(m, id)| (m.to_string(), id.to_string())).collect();
        Milestone {
            name: name.to_string(),
            date: "2026-01-01".to_string(),
            reference_snapshots: BTreeMap::new(),
            attachments: pins(rooms),
            door_attachments: pins(doors),
            window_attachments: BTreeMap::new(),
            ffe_attachments: BTreeMap::new(),
            space_attachments: BTreeMap::new(),
            ceiling_attachments: BTreeMap::new(),
            floor_attachments: BTreeMap::new(),
        }
    }

    fn row(project: &str, model: &str, latest: &[(SnapshotKind, &str)]) -> ModelIndexRow {
        ModelIndexRow {
            key: ModelKey { project_id: project.to_string(), model_id: model.to_string() },
            project_name: project.to_string(),
            model_name: model.to_string(),
            latest: latest.iter().map(|(k, id)| (*k, id.to_string())).collect(),
            placement: None,
        }
    }

    fn planned(plan: &[PlannedRead]) -> Vec<(&str, &str)> {
        plan.iter().map(|p| (p.key.model_id.as_str(), p.id.as_str())).collect()
    }

    fn door_meta<'a>(key: &'a ModelKey, taken_at: &'a str) -> SnapshotMeta<'a> {
        SnapshotMeta {
            kind: SnapshotKind::Doors,
            key,
            project_name: "P",
            model_name: "M",
            taken_at,
            phase: None,
            model_to_shared: None,
        }
    }

    fn doors(taken_at: &str) -> DoorPayload {
        DoorPayload {
            schema_version: SUPPORTED_DOOR_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: taken_at.to_string() },
            phase: None,
            model_to_shared: None,
            levels: vec![],
            doors: vec![],
        }
    }

    /// The rules the two loops this replaced applied, stated once: registered
    /// projects only, the latest without a milestone, and under one a model
    /// must be both pinned and hold a snapshot of the kind.
    #[test]
    fn test_the_plan_admits_exactly_what_the_read_serves() {
        use SnapshotKind::{Doors, Rooms};
        let index = vec![
            row("p1", "m1", &[(Rooms, "r-new"), (Doors, "d-new")]),
            row("p1", "m2", &[(Rooms, "r-new")]),
            row("unregistered", "m3", &[(Rooms, "r-new")]),
        ];
        let registry = SettingsRegistry::from_bundles(
            HashMap::from([(
                "p1".to_string(),
                bundle(vec![milestone(
                    "M",
                    &[("m1", "r-old")],
                    &[("m1", "d-old"), ("m2", "d-dangling")],
                )]),
            )]),
            None,
        );
        let plan = |kind, project, ms| plan_reads(&index, &registry, kind, project, ms);

        assert_eq!(
            planned(&plan(Rooms, None, None)),
            [("m1", "r-new"), ("m2", "r-new")],
            "no bundle, no read"
        );
        assert_eq!(planned(&plan(Doors, None, None)), [("m1", "d-new")], "m2 holds no doors");
        assert!(plan(Rooms, Some("unregistered"), None).is_empty());

        assert_eq!(planned(&plan(Rooms, None, Some("M"))), [("m1", "r-old")], "m2 is not pinned for rooms");
        assert_eq!(
            planned(&plan(Doors, None, Some("M"))),
            [("m1", "d-old")],
            "the m2 doors pin names nothing that was ever pushed"
        );
        assert!(plan(Rooms, None, Some("no such milestone")).is_empty());
    }

    /// The ETag of a milestone read follows that entity's OWN pin.
    ///
    /// Written against the bug it fixes: `scope_cursor` used to read the rooms
    /// map (`attachments`) for every kind, so two milestones pinning the same
    /// rooms snapshot and different doors snapshots produced one doors cursor
    /// for two different doors bodies. The store is identical in both states
    /// below; only the doors snapshot the milestone names differs.
    #[test]
    fn test_a_milestone_cursor_hashes_each_kinds_own_pin() {
        let dir = std::env::temp_dir().join(format!("roommate-plan-cursor-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let key = ModelKey { project_id: "p1".to_string(), model_id: "m1".to_string() };
        let store = FsStore::new(dir.clone()).unwrap();
        for ts in ["2026-02-01T00:00:00Z", "2026-03-01T00:00:00Z"] {
            store.put_raw(&door_meta(&key, ts), &serde_json::to_vec(&doors(ts)).unwrap()).unwrap();
        }

        let cursor_pinning = |doors_pin: &str| {
            let ms = milestone("M", &[("m1", "2026-01-01T00:00:00Z")], &[("m1", doors_pin)]);
            let state = AppState::new(
                Box::new(FsStore::new(dir.clone()).unwrap()),
                HashMap::from([("p1".to_string(), bundle(vec![ms]))]),
                None,
            );
            super::super::scope_cursor(&state, Some("p1"), Some("M"), &[SnapshotKind::Doors]).unwrap()
        };
        assert_ne!(cursor_pinning("2026-02-01T00:00:00Z"), cursor_pinning("2026-03-01T00:00:00Z"));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A milestone read opens the pinned snapshot and never the latest.
    ///
    /// Proven by making the latest unreadable: the old loop parsed every
    /// model's latest before looking at the milestone, so a malformed latest
    /// failed a read that was never going to serve it. Counting results could
    /// not show this -- both versions return the pinned snapshot when every
    /// file parses.
    #[test]
    fn test_a_milestone_read_never_parses_the_latest() {
        let dir = std::env::temp_dir().join(format!("roommate-plan-pinned-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let key = ModelKey { project_id: "p1".to_string(), model_id: "m1".to_string() };
        let store = FsStore::new(dir.clone()).unwrap();
        let pinned = "2026-02-01T00:00:00Z";
        store
            .put_raw(&door_meta(&key, pinned), &serde_json::to_vec(&doors(pinned)).unwrap())
            .unwrap();
        store.put_raw(&door_meta(&key, "2026-06-01T00:00:00Z"), b"not a snapshot").unwrap();

        let state = AppState::new(
            Box::new(store),
            HashMap::from([("p1".to_string(), bundle(vec![milestone("M", &[], &[("m1", pinned)])]))]),
            None,
        );

        let read = scope_snapshots::<DoorPayload>(&state, SnapshotKind::Doors, Some("p1"), Some("M")).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].1.snapshot.taken_at, pinned);
        assert!(
            scope_snapshots::<DoorPayload>(&state, SnapshotKind::Doors, Some("p1"), None).is_err(),
            "the latest really is malformed -- the milestone read simply never opened it"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    // ---------- storey scope ----------

    fn level(id: &str, name: &str, elevation: f64) -> crate::contract::Level {
        crate::contract::Level { id: id.into(), name: name.into(), elevation }
    }

    /// Absent parameters are an unscoped read; a bad elevation is refused with
    /// the value named, never silently read as every storey.
    #[test]
    fn test_storey_scope_parses_or_refuses() {
        assert_eq!(StoreyScope::parse(None, None), Ok(None));
        let scope = StoreyScope::parse(Some("0, 3000.5,"), Some("a,b")).unwrap().unwrap();
        assert_eq!(scope.elevations, [0.0, 3000.5]);
        assert_eq!(scope.level_ids.len(), 2);
        let err = StoreyScope::parse(Some("0,ground"), None).unwrap_err();
        assert!(err.contains("ground"), "{err}");
        assert!(StoreyScope::parse(Some("NaN"), None).is_err());
    }

    /// The three cases `StoreyScope` documents, each a superset of what the
    /// viewer's `onStorey` keeps.
    #[test]
    fn test_storey_admission_is_a_superset_of_the_viewer_rule() {
        let levels = BTreeMap::from([
            // Declares levels. "L1 ref" shares LEVEL 1's elevation under another
            // name: onStorey's by-elevation fallback may need it, so it is kept.
            (
                "arch".to_string(),
                vec![
                    level("10", "LEVEL 0", 0.0),
                    level("11", "LEVEL 1", 3000.0),
                    level("12", "L1 ref", 3020.0),
                ],
            ),
            // Declares none: only its level ids can place its elements.
            ("facade".to_string(), vec![]),
        ]);
        let scope = StoreyScope::parse(Some("3000"), Some("11")).unwrap().unwrap();
        let admit = scope.admitter(&levels);

        assert!(admit.admits("arch", "11"), "on the requested storey");
        assert!(admit.admits("arch", "12"), "within LEVEL_EPS_MM of it, whatever its name");
        assert!(!admit.admits("arch", "10"), "another storey");
        assert!(!admit.admits("arch", "-1"), "a level its own model does not declare places it nowhere");
        assert!(admit.admits("facade", "11"), "no declared levels: the id is all there is");
        assert!(!admit.admits("facade", "10"));

        // No model declares any level: the viewer shows everything, so the body
        // must hold everything.
        let bare = BTreeMap::from([("facade".to_string(), vec![])]);
        assert!(scope.admitter(&bare).admits("facade", "anything"));
    }
}
