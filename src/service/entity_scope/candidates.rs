//! How an element finds its room when the model names none: the candidate rooms placed into the project frame, the origin of each side of an
//! opening (authored or derived) and the building a room belongs to.

use super::*;

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
    pub(super) by_model: BTreeMap<String, Vec<room_locator::Candidate>>,
    /// Every candidate, already placed in the shared frame. Empty outside
    /// `Project` mode, where models are never mixed.
    pub(super) shared: Vec<room_locator::Candidate>,
    /// `(model id, level id)` → elevation, so a *door* — which carries a level
    /// id, not an elevation — can be put on the same axis as the rooms.
    pub(super) elevation: BTreeMap<(String, String), f64>,
    /// Model id → the wall gap its rooms were drawn to, so a probe is sized by
    /// the regime of the rooms it is reaching for rather than by a constant.
    pub(super) gap_by_model: BTreeMap<String, f64>,
    /// The step for an element whose OWN model holds no rooms -- a facade or
    /// envelope file -- which is the project's `max_wall_thickness` whatever
    /// regime the rooms were drawn to.
    ///
    /// **A centreline gap of zero is a claim about walls the rooms share**, and
    /// the facade's wall is not one of them: it lives in another document, so
    /// the interior rooms stop at their own lining or at a link-bounded line
    /// short of it, never at the facade window's insertion point. Measured on
    /// a real project 2026-09-21: the median facade window sat well over half a foot from the nearest
    /// room edge, so the 15 mm centreline step resolved almost none and a wall
    /// thickness reaches most. Only the room-less model gets it, because an
    /// opening in a model that has rooms IS on a shared line, and a 1.5 ft step
    /// there would jump a cupboard into the room beyond it.
    pub(super) cross_model_gap: f64,
    /// Model id → its `model_to_shared`, needed in `Project` mode to lift the
    /// door's own point into the frame the candidates are already in.
    pub(super) transform_by_model: BTreeMap<String, ModelToShared>,
    pub(super) shared_frame: bool,
}

/// Apply a 2D affine to a **position**: `shared_x = a*x + c*y + e`.
pub(super) fn place(m: &ModelToShared, p: Point2D) -> Point2D {
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
pub(super) fn place_direction(m: &ModelToShared, p: Point2D) -> Option<Point2D> {
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
pub(super) const NO_CANDIDATES: &[room_locator::Candidate] = &[];

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
    pub(super) fn prepare(
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
    rooms: &crate::service::rooms::ScopedRooms<'_>,
    project: Option<&str>,
    milestone: Option<&str>,
) -> Result<BTreeMap<(String, String), String>, ServiceError> {
    let rooms = crate::service::rooms::rooms_result(
        state,
        rooms,
        &crate::service::rooms::RoomScope { project, milestone, ..Default::default() },
    )?;

    let mut out = BTreeMap::new();
    for room in &rooms.rooms {
        let Some(tier) = registry
            .settings_for(&room.project_id)
            .and_then(|b| crate::service::rooms::building_tier_index(&b.hierarchy))
        else {
            continue; // a project with no "Building" tier answers no building
        };
        if let Some(value) = room.classification.get(tier) {
            out.insert(
                (room.model_id.clone(), room.room.id.clone()),
                crate::service::rooms::building_key(&value.code, &value.name),
            );
        }
    }
    Ok(out)
}
