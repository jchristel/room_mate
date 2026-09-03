//! `/doors` read assembly: merge every stored model's latest doors snapshot
//! into one flat payload, scoped by project, milestone and property filter.
//!
//! **Deliberately thinner than `service::rooms`, and the differences are the
//! point.** Doors reuse the filter grammar, the milestone pinning discipline and
//! the phase reporting verbatim; they have no reference-source join (that is R4,
//! which lands with doors' first reference source), no classification hierarchy,
//! and no level dedup — a door's `level_id` points into the level set its
//! model's *rooms* snapshot already carries, so there is nothing here to merge.
//! A model that pushes doors and no rooms carries its own levels on the doors
//! envelope instead (`DoorPayload::levels`); `build_candidates` reads them only
//! where the rooms did not supply one, which is a fallback rather than a merge.
//!
//! `?building=` **is** supported, and was the last thing to arrive: a door's
//! building is its *owning* room's building, so the scope only became
//! answerable once `[doors] room_attribution` settled which room owns a door
//! (the attribution rule is in `CLAUDE.md`). Before that, any answer would have
//! settled that question by accident.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::contract::{ModelToShared, Opening, OpeningEnvelope, Point2D, PropertyPresence};
use crate::reference::{ReferenceData, ReferenceRecord};
use crate::settings::{BuiltinPropertyDef, ReferenceEntity, RoomResolution};
use crate::state::{AppState, ModelKey};
use crate::storage::SnapshotKind;

use super::room_locator::{self, RoomRef, Unresolved};
use super::rooms::{FilterTarget, RoomFilter};
use super::ServiceError;

/// A door as sent to a consumer: the stored door plus the identity of the model
/// it came from.
///
/// The model ids are on the wire, unlike `RoomResponse`'s (which skips its
/// `source`), because a door's `from_room`/`to_room` are only meaningful
/// alongside them: room ids are unique within a model, so a consumer resolving a
/// door to its rooms needs to know which model's rooms to look in. Without it,
/// a merged multi-model response would be ambiguous in exactly the way the QA
/// report has to work around.
#[derive(Serialize)]
pub struct OpeningResponse {
    #[serde(flatten)]
    pub door: Opening,

    /// The project this door's model belongs to.
    pub project_id: String,
    /// The model this door came from — the scope its room references resolve in.
    pub model_id: String,

    /// The room(s) this door is attributed to under the project's
    /// `[doors] room_attribution` policy, in policy order.
    ///
    /// **A list, and empty means homeless.** A list because the `both` policy
    /// attributes a door between two rooms twice — which is the point of that
    /// policy for area rollups — and one shape that covers all five policies
    /// beats an `Option` plus a special case. Empty is a *reported state*, not a
    /// missing value: it means either the door names no room at all, or the
    /// policy declined to use the reference it has (`to_room` against a door
    /// that only opens *from* somewhere). QA distinguishes the two.
    ///
    /// Derived at read time from the stored references, never stored: changing
    /// the policy changes every answer immediately and rewrites nothing.
    pub owner_rooms: Vec<String>,

    /// The same owners, **model-qualified** — and the only field that can name a
    /// room in another model.
    ///
    /// `owner_rooms` above is a list of bare room ids resolved against this
    /// door's own model, which is all it can be: a room id is unique only within
    /// a model. Once geometry can reach a room in a *linked* model, that shape
    /// has no way to say so, and putting the bare id there would resolve it
    /// against the wrong model's rooms — a wrong answer that looks right.
    ///
    /// So `owner_rooms` keeps its meaning exactly (same-model owners, empty
    /// means homeless) and this carries the whole truth beside it. A consumer
    /// that needs to be correct across linked models reads this one.
    pub owner_rooms_qualified: Vec<RoomRef>,

    /// Where each side's room reference came from — stated by the model,
    /// derived from geometry, or unresolved with a reason. See `SideOrigin`.
    pub room_origin: RoomOrigin,

    /// Joined reference-source records, keyed by source name — the same shape
    /// `RoomResponse::reference` carries, and flattened the same way so
    /// `schedule.FireRating` reads identically on either entity.
    ///
    /// Only sources declaring `entity = "doors"` land here. A source with no
    /// match for this door contributes no entry: an unmatched key is a signal,
    /// not an error, exactly as for rooms.
    #[serde(flatten)]
    pub reference: BTreeMap<String, ReferenceRecord>,

    /// The owning model's `Model.source` (e.g. "revit"), for canonical property
    /// resolution. Not wire shape, same as `RoomResponse::source`.
    #[serde(skip)]
    pub source: String,
}

/// Doors resolve the entity's own two property tiers plus a small set of
/// intrinsics, and nothing else.
///
/// **Source-qualified fields resolve against this door's joined sources**, and
/// `Absent` when it joined none — which is exactly the answer a *room* gets for
/// a source it did not match. That equivalence was designed in before the join
/// existed: the previous implementation returned `Absent` for every qualified
/// field precisely so that the day R4 landed, the same predicate would start
/// matching rather than change status. It has, and it did.
impl FilterTarget for OpeningResponse {
    fn presence(&self, source: Option<&str>, property: &str, builtin_defs: &[BuiltinPropertyDef]) -> PropertyPresence {
        /// A door's own struct fields always exist, so blank collapses to
        /// `Empty`, never `Absent` — the same rule `RoomResponse`'s intrinsics
        /// follow.
        fn intrinsic(value: Option<&str>) -> PropertyPresence {
            match value {
                None => PropertyPresence::Absent,
                Some("") => PropertyPresence::Empty,
                Some(v) => PropertyPresence::Present(v.to_string()),
            }
        }

        match source {
            // Same three-way answer `RoomResponse` gives, deliberately: source
            // not joined and field not in the row are both `Absent`, an empty
            // cell is `Empty`.
            Some(name) => match self.reference.get(name) {
                None => PropertyPresence::Absent,
                Some(record) => match record.fields.get(property) {
                    None => PropertyPresence::Absent,
                    Some(v) if v.is_empty() => PropertyPresence::Empty,
                    Some(v) => PropertyPresence::Present(v.clone()),
                },
            },
            None => match property {
                "$id" => intrinsic(Some(&self.door.id)),
                "$type_name" => intrinsic(Some(&self.door.type_name)),
                "$type_id" => intrinsic(Some(&self.door.type_id)),
                "$level_id" => intrinsic(Some(&self.door.level_id)),
                // The room references, as filterable fields. `$from_room=` with
                // no match is how a caller asks "which doors point at this
                // room"; an external door is `Absent` on its missing side, so it
                // fails every operator rather than matching a negative one.
                "$from_room" => intrinsic(self.door.from_room.as_deref()),
                "$to_room" => intrinsic(self.door.to_room.as_deref()),
                // Anything else walks the two property tiers, instance first —
                // the R2 rule, unchanged and not reimplemented here.
                canonical => crate::contract::property_presence(&self.door, canonical, &self.source, builtin_defs),
            },
        }
    }
}

/// Resolve one comparable/filterable field name against an assembled door, in
/// the same `source.property` vocabulary `/doors`' filter parses.
///
/// The door counterpart of `rooms::resolve_presence`, and it exists for the
/// identical reason: "what can I write before the dot" must have one answer
/// across filtering and comparison, or a name that filters correctly would
/// silently diff as nothing. It returns only the presence, not the resolved
/// namespace — a door has no joined sources to react to per source, which is
/// the one thing that genuinely differs from the rooms version.
pub fn resolve_presence(
    door: &OpeningResponse,
    field: &str,
    known: &std::collections::BTreeSet<String>,
    builtin: &[BuiltinPropertyDef],
) -> PropertyPresence {
    match super::rooms::split_namespace(field, known) {
        super::rooms::NamespaceSplit::Joined { source, property } => door.presence(Some(&source), property, builtin),
        super::rooms::NamespaceSplit::Unqualified(name) => door.presence(None, name, builtin),
        // Rejected at settings load and filter parse, so unreachable through
        // either configured path — degrades to "nothing to compare" rather than
        // panicking, the same discipline `rooms::resolve_presence` follows.
        super::rooms::NamespaceSplit::UnknownSource(_) => PropertyPresence::Absent,
    }
}

/// Everything that narrows a doors read.
#[derive(Default)]
pub struct OpeningScope<'a> {
    pub project: Option<&'a str>,
    /// Opaque building key, as for rooms — **a door's building is its owning
    /// room's building.** This became answerable only once
    /// `[doors] room_attribution` decided which room owns a door; before that,
    /// any answer would have settled the ownership question by accident.
    ///
    /// A **homeless door matches no building**, which is the honest reading and
    /// worth stating: it is not evidence the door belongs elsewhere, only that
    /// nothing attributes it, so it drops out of a building-scoped view exactly
    /// as a room with no classification does.
    pub building: Option<&'a str>,
    pub milestone: Option<&'a str>,
    pub filter: Option<&'a RoomFilter>,
}

/// One assembled read, before it is named for an entity.
///
/// **Deliberately not the wire type.** `/doors` must answer
/// `{"doors": [...], "schema_version": 2}` and `/windows`
/// `{"windows": [...], "schema_version": 1}`, so the outermost key and the
/// version are per entity even though everything inside them is not. The
/// assembly produces this; each endpoint wraps it in its own result. Same line
/// that decided the envelopes: share everything except what would change a
/// serde key.
pub struct Assembled {
    pub revision: String,
    pub openings: Vec<OpeningResponse>,
    pub phase_by_model: BTreeMap<String, BTreeMap<String, Option<String>>>,
}

/// Which opening entity a read addresses, and the **only** place the per-entity
/// answers live.
///
/// Four things vary between a doors read and a windows read: the storage kind,
/// which reference sources join, which settings section supplies the policy, and
/// which milestone pin map applies. Every one of them is a lookup, and gathering
/// them here is what keeps `assemble_openings` free of `if doors` — the same
/// role `ENTITY_EXPORTERS` plays on the producer side.
///
/// It also fixes the one thing the generic signature cannot check. The assembly
/// is generic over the envelope `P` and takes this kind separately, so in
/// principle someone could pass `Windows` with `DoorPayload`. The pairing is
/// made once, in `doors()` and `windows()` below, and every caller goes through
/// those — so the mismatch is not merely unlikely, there is nowhere to write it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpeningKind {
    Doors,
    Windows,
}

impl OpeningKind {
    pub fn snapshot_kind(self) -> SnapshotKind {
        match self {
            OpeningKind::Doors => SnapshotKind::Doors,
            OpeningKind::Windows => SnapshotKind::Windows,
        }
    }

    pub fn reference_entity(self) -> ReferenceEntity {
        match self {
            OpeningKind::Doors => ReferenceEntity::Doors,
            OpeningKind::Windows => ReferenceEntity::Windows,
        }
    }

    /// This entity's policy section on one project's resolved settings.
    pub fn policy(self, bundle: &crate::state::ProjectSettings) -> &crate::settings::OpeningPolicy {
        match self {
            OpeningKind::Doors => &bundle.doors,
            OpeningKind::Windows => &bundle.windows,
        }
    }

    /// This entity's snapshot pins on one milestone. Three separate maps,
    /// because the entities are pushed independently and their snapshot ids do
    /// not correspond.
    pub fn pins(self, milestone: &crate::settings::Milestone) -> &BTreeMap<String, String> {
        match self {
            OpeningKind::Doors => &milestone.door_attachments,
            OpeningKind::Windows => &milestone.window_attachments,
        }
    }
}

/// The merged doors payload, exactly as `/doors` has always answered it.
#[derive(Serialize)]
pub struct DoorsResult {
    pub schema_version: u32,
    /// Stable content revision over the contributing `(model, snapshot)` pairs,
    /// same role and same construction as `RoomsResult::revision`: a consumer
    /// compares this one field instead of re-hashing the payload.
    pub revision: String,
    pub doors: Vec<OpeningResponse>,
    /// The Revit phase each contributing model's doors were filtered to, keyed
    /// by project id then model id. Read off each snapshot, never off the
    /// lineage's current phase.
    pub phase_by_model: BTreeMap<String, BTreeMap<String, Option<String>>>,
}

impl DoorsResult {
    pub fn from_assembled(assembled: Assembled) -> Self {
        Self {
            schema_version: crate::contract::SUPPORTED_DOOR_SCHEMA,
            revision: assembled.revision,
            doors: assembled.openings,
            phase_by_model: assembled.phase_by_model,
        }
    }
}

/// The merged windows payload. A separate type from `DoorsResult` for its two
/// differing keys and nothing else — see `Assembled`.
#[derive(Serialize)]
pub struct WindowsResult {
    pub schema_version: u32,
    pub revision: String,
    pub windows: Vec<OpeningResponse>,
    pub phase_by_model: BTreeMap<String, BTreeMap<String, Option<String>>>,
}

impl WindowsResult {
    pub fn from_assembled(assembled: Assembled) -> Self {
        Self {
            schema_version: crate::contract::SUPPORTED_WINDOW_SCHEMA,
            revision: assembled.revision,
            windows: assembled.openings,
            phase_by_model: assembled.phase_by_model,
        }
    }
}

/// A stable content revision for a `DoorsResult`. Duplicated from
/// `rooms::scoped_revision` rather than shared: that one takes room-scoped
/// tuples, and the shared part is three lines of hashing whose meaning ("which
/// snapshot did each model contribute") is per entity.
fn openings_revision<P: OpeningEnvelope>(scoped: &[(ModelKey, P)]) -> String {
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
    fn room(&self) -> Option<&RoomRef> {
        match self {
            SideOrigin::Authored(r) | SideOrigin::Derived(r) => Some(r),
            SideOrigin::Unresolved(_) => None,
        }
    }
}

/// Both of a door's sides, and where each came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoomOrigin {
    pub from_room: SideOrigin,
    pub to_room: SideOrigin,
}

/// The smallest step that still leaves the wall, in feet (~15 mm).
///
/// A **centreline** model has a wall gap of zero — neighbouring rooms already
/// tile, so their boundaries are one shared line. Probing by zero would test
/// that line itself, where containment is undefined and a room may or may not
/// claim its own edge. Any positive step lands cleanly in one room or the other,
/// so this is a floor rather than a tolerance to tune.
const MIN_PROBE_FT: f64 = 0.05;

/// Every room a door could be resolved against, prepared once per read.
///
/// Built once rather than per door: the rooms come from storage, and reading
/// them inside the door loop would be one storage read per door.
struct Candidates {
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
fn build_candidates<P: OpeningEnvelope>(
    state: &AppState,
    scope: &OpeningScope<'_>,
    mode: RoomResolution,
    opening_payloads: &[(ModelKey, P)],
) -> Result<Candidates, ServiceError> {
    let registry = state.settings();
    let stored = state.all_snapshots(scope.project).map_err(ServiceError::Internal)?;
    let (scoped, _) = super::rooms::scope_payloads(state, &registry, stored, scope.project, scope.milestone)?;

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
    for (key, payload) in opening_payloads {
        if scope.project.is_some_and(|p| payload.project().id != p) {
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
    /// Resolve one door's two sides.
    fn locate(&self, door: &Opening, model_id: &str) -> room_locator::Sides {
        let unresolved = |why| room_locator::Sides {
            from: room_locator::Located::Unresolved(why),
            to: room_locator::Located::Unresolved(why),
        };
        let Some(mut point) = room_locator::position_of(door.insertion_point, &door.loops) else {
            return unresolved(Unresolved::NoPosition);
        };
        let Some(&elevation) = self.elevation.get(&(model_id.to_string(), door.level_id.clone())) else {
            // The door names a level its model's rooms snapshot does not carry,
            // so there is no axis to compare on. Nothing to probe.
            return unresolved(Unresolved::NoCandidate);
        };
        let mut normal = door.through_wall_normal;

        let candidates: &[room_locator::Candidate] = if self.shared_frame {
            // The candidates have been placed, so the door has to be too. A
            // model with no transform cannot be compared against ones that were
            // placed — it would be probed in the wrong frame.
            let Some(transform) = self.transform_by_model.get(model_id) else {
                return unresolved(Unresolved::NoPosition);
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

        let placement = room_locator::Placement { point, normal, elevation };
        room_locator::locate(&placement, candidates, gap.max(MIN_PROBE_FT))
    }
}

/// Resolve every door in one project against its rooms, for the QA report.
///
/// **Latest-based, with no milestone scope**, matching `/validation` as a whole:
/// the report is about the data being served now, and the drift it exists to
/// catch is precisely a rooms snapshot moving on without its doors.
///
/// Returns an empty map when resolution is off, so the caller needs no branch
/// beyond the one that decides whether to ask.
pub fn locate_project_openings<P: OpeningEnvelope + serde::de::DeserializeOwned>(
    state: &AppState,
    project_id: &str,
    mode: RoomResolution,
    stored: &[(ModelKey, P)],
) -> Result<BTreeMap<(String, String), room_locator::Sides>, ServiceError> {
    let mut out = BTreeMap::new();
    if mode == RoomResolution::Off {
        return Ok(out);
    }
    let scope = OpeningScope { project: Some(project_id), ..OpeningScope::default() };
    let candidates = build_candidates(state, &scope, mode, stored)?;
    for (key, payload) in stored.iter().filter(|(_, p)| p.project().id == project_id) {
        for door in payload.openings() {
            out.insert((key.model_id.clone(), door.id.clone()), candidates.locate(door, &payload.model().id));
        }
    }
    Ok(out)
}

/// Fold one side's authored reference and one side's derived answer into the
/// single origin the response carries.
///
/// **Authored always wins.** A door's `to_room` is the modeller's assignment —
/// what the door *serves*, which is not always what it opens into — so geometry
/// replacing it would be the reconciliation `CLAUDE.md` forbids. Geometry fills
/// what the model left absent, and disagrees audibly with what it did not
/// (`DoorReport::room_geometry_mismatches`).
fn side_origin(authored: Option<&str>, model_id: &str, derived: &room_locator::Located) -> SideOrigin {
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
fn building_by_room(
    state: &AppState,
    scope: &OpeningScope<'_>,
) -> Result<BTreeMap<(String, String), String>, ServiceError> {
    let rooms = super::rooms::assemble_rooms(
        state,
        &super::rooms::RoomScope { project: scope.project, milestone: scope.milestone, ..Default::default() },
    )?;
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

/// Merge every stored model's doors into one payload, scoped by `OpeningScope`.
///
/// Returns `Ok(None)` when nothing has ever been pushed — the adapter's "204 No
/// Content" case, same contract as `assemble_rooms`. A filter or scope that
/// merely matches nothing is `Ok(Some)` with an empty list: the store has data,
/// the question just has an empty answer.
///
/// A project with no registered settings contributes nothing, matching
/// `assemble_rooms`' skip-on-read policy — a model with nothing to resolve
/// canonical property names against has no home in the response.
///
/// Over the 100-line discovery threshold and kept whole: it is three sequential
/// phases over one scoped set — scope, prepare (buildings, candidates), derive —
/// and each reads the ones before it. Splitting them into helpers would move the
/// order into call sites without removing it, and the order is the part a reader
/// has to see.
#[allow(clippy::too_many_lines)]
pub fn assemble_openings<P: OpeningEnvelope + serde::de::DeserializeOwned>(
    state: &AppState,
    kind: OpeningKind,
    scope: &OpeningScope<'_>,
) -> Result<Option<Assembled>, ServiceError> {
    // "Nothing pushed at all" is asked of the index, not of this scoped read:
    // an unknown project reads empty and still deserves a 200 with an empty
    // list, per the contract above.
    if !state.has_any_snapshot(kind.snapshot_kind()).map_err(ServiceError::Internal)? {
        return Ok(None);
    }
    let stored: Vec<(ModelKey, P)> = state
        .all_opening_snapshots(kind.snapshot_kind(), scope.project)
        .map_err(ServiceError::Internal)?;
    let registry = state.settings();

    // Phase 1 — scope to the request, substituting a milestone's pinned doors
    // snapshot for the model's latest where one is pinned. Same discipline as
    // `rooms::scope_payloads`: a project without the named milestone, or a model
    // that milestone does not pin, contributes nothing, and a pin whose snapshot
    // no longer exists is skipped with a warning rather than failing the read
    // ("signal, not error").
    let mut scoped: Vec<(ModelKey, P)> = Vec::new();
    for (key, payload) in stored {
        if scope.project.is_some_and(|p| payload.project().id != p) {
            continue;
        }
        let Some(bundle) = registry.settings_for(&payload.project().id) else {
            continue;
        };
        match scope.milestone {
            None => scoped.push((key, payload)),
            Some(wanted) => {
                let Some(ms) = bundle.milestones.iter().find(|m| m.name == wanted) else {
                    continue;
                };
                let Some(pinned_id) = kind.pins(ms).get(&key.model_id) else {
                    continue;
                };
                match state.get_opening_snapshot::<P>(kind.snapshot_kind(), &key, pinned_id).map_err(ServiceError::Internal)? {
                    Some(pinned) => scoped.push((key, pinned)),
                    None => tracing::warn!(
                        "milestone '{}' pins {} snapshot {:?} for {}/{}, but no such snapshot exists — skipping the model",
                        wanted,
                        kind.snapshot_kind().label(),
                        pinned_id,
                        key.project_id,
                        key.model_id
                    ),
                }
            }
        }
    }

    let revision = openings_revision(&scoped);
    let mut phase_by_model: BTreeMap<String, BTreeMap<String, Option<String>>> = BTreeMap::new();
    let mut openings: Vec<OpeningResponse> = Vec::new();

    // A door's building is its owning room's building, so a building scope needs
    // the rooms classified. Resolved **only when a building filter is actually
    // given** — it is a second storage read plus a classification pass, and the
    // overwhelmingly common doors read does not need it.
    let building_of_room = match scope.building {
        Some(_) => building_by_room(state, scope)?,
        None => BTreeMap::new(),
    };

    // Geometric resolution, when a project asks for it. Off is the default and
    // costs nothing: no storage read, no candidates, and every side reports
    // whatever the model stated.
    //
    // Resolved per project rather than per model, because `Project` mode probes
    // across models by design. A read spanning projects with different settings
    // therefore builds one candidate set per project that wants one.
    let mut candidates_by_project: BTreeMap<String, Candidates> = BTreeMap::new();
    for (_, payload) in &scoped {
        let mode = registry
            .settings_for(&payload.project().id)
            .map(|b| kind.policy(b).room_resolution)
            .unwrap_or_default();
        if mode == RoomResolution::Off || candidates_by_project.contains_key(&payload.project().id) {
            continue;
        }
        let project_scope = OpeningScope { project: Some(&payload.project().id), ..OpeningScope::default() };
        let project_scope = OpeningScope { milestone: scope.milestone, ..project_scope };
        candidates_by_project
            .insert(payload.project().id.clone(), build_candidates(state, &project_scope, mode, &scoped)?);
    }

    // Phase 2 — derive the response doors, applying the property filter *after*
    // assembly so a predicate sees the same resolved vocabulary a consumer does.
    for (key, payload) in &scoped {
        phase_by_model
            .entry(key.project_id.clone())
            .or_default()
            .insert(key.model_id.clone(), payload.phase().map(str::to_string));

        let bundle = registry.settings_for(&payload.project().id);
        let builtin: &[BuiltinPropertyDef] = bundle.map(|b| b.builtin_properties.as_slice()).unwrap_or_default();
        let attribution = bundle.map(|b| kind.policy(b).room_attribution).unwrap_or_default();

        // The doors half of R4: this project's sources declaring
        // `entity = "doors"`, resolved once per model rather than per door.
        // Rooms scope theirs the same way in `assemble_scoped_rooms`.
        let sources: BTreeMap<&str, &ReferenceData> = bundle
            .map(|b| {
                b.reference
                    .iter()
                    .filter(|(_, cfg)| cfg.entity == kind.reference_entity())
                    .filter_map(|(name, cfg)| Some((name.as_str(), cfg.data.as_ref()?)))
                    .collect()
            })
            .unwrap_or_default();

        for door in payload.openings() {
            // One join per configured source: read its link property off the
            // DOOR -- instance tier then type tier, the R2 rule -- and look up
            // the record. `lookup_property` is the same function rooms use; a
            // door is simply another `PropertyTiers`.
            let reference: BTreeMap<String, ReferenceRecord> = sources
                .iter()
                .filter_map(|(name, data)| {
                    let record =
                        crate::contract::lookup_property(door, &data.link_property, &payload.model().source, builtin)
                            .and_then(|key| data.by_id.get(&key).cloned())?;
                    Some(((*name).to_string(), record))
                })
                .collect();

            // Authored first, geometry only where the model said nothing —
            // and the probe is skipped entirely for a door that stated both,
            // since nothing it could find would be used. The QA report runs the
            // same probe on those doors deliberately, because there the
            // disagreement is the finding.
            let derived = match candidates_by_project.get(&payload.project().id) {
                Some(candidates) if door.from_room.is_none() || door.to_room.is_none() => {
                    candidates.locate(door, &payload.model().id)
                }
                _ => room_locator::Sides {
                    from: room_locator::Located::Unresolved(Unresolved::NoCandidate),
                    to: room_locator::Located::Unresolved(Unresolved::NoCandidate),
                },
            };
            let room_origin = RoomOrigin {
                from_room: side_origin(door.from_room.as_deref(), &payload.model().id, &derived.from),
                to_room: side_origin(door.to_room.as_deref(), &payload.model().id, &derived.to),
            };
            let owner_rooms_qualified: Vec<RoomRef> = attribution
                .owners(room_origin.from_room.room(), room_origin.to_room.room())
                .into_iter()
                .cloned()
                .collect();

            let response = OpeningResponse {
                // Same-model owners only, so the field keeps meaning exactly
                // what it always did. A cross-model owner appears in
                // `owner_rooms_qualified` and nowhere else.
                owner_rooms: owner_rooms_qualified
                    .iter()
                    .filter(|r| r.model_id == payload.model().id)
                    .map(|r| r.room_id.clone())
                    .collect(),
                owner_rooms_qualified,
                room_origin,
                door: door.clone(),
                project_id: payload.project().id.clone(),
                model_id: payload.model().id.clone(),
                reference,
                source: payload.model().source.clone(),
            };
            // A homeless door matches no building — see `OpeningScope::building`.
            // Room ids are unique only within a model, so the lookup is keyed on
            // the pair, never the bare room id — and it reads the *qualified*
            // owners, because a derived owner may live in a linked model and the
            // door's own model id would be the wrong half of that key.
            if let Some(wanted) = scope.building
                && !response.owner_rooms_qualified.iter().any(|room| {
                    building_of_room
                        .get(&(room.model_id.clone(), room.room_id.clone()))
                        .is_some_and(|b| b == wanted)
                })
            {
                continue;
            }
            if scope.filter.is_none_or(|f| f.matches(&response, builtin)) {
                openings.push(response);
            }
        }
    }

    Ok(Some(Assembled { revision, openings, phase_by_model }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        CustomValue, DoorPayload, Model, Project, RoomPayload, Snapshot, SUPPORTED_DOOR_SCHEMA, SUPPORTED_SCHEMA,
    };
    use crate::state::ProjectSettings;
    use crate::storage::MemStore;
    use std::collections::{BTreeSet, HashMap};

    /// The kind this module's tests exercise. Doors, because doors are what
    /// has real stored data and a pinned wire shape to regress against --
    /// the windows path is the same code with two lookups changed, and it is
    /// covered where those lookups live (`OpeningKind`) rather than by a
    /// second copy of every scenario below.
    const OPENING_KIND: OpeningKind = OpeningKind::Doors;

    fn make_door(id: &str, from_room: Option<&str>, to_room: Option<&str>, props: &[(&str, &str)]) -> Opening {
        let mut properties = BTreeMap::new();
        for (k, v) in props {
            properties.insert(k.to_string(), CustomValue { value: v.to_string(), storage_type: None });
        }
        Opening {
            id: id.to_string(),
            level_id: "1".to_string(),
            loops: vec![],
            from_room: from_room.map(str::to_string),
            to_room: to_room.map(str::to_string),
            // These helpers exercise references and properties, never placement,
            // so `None` here is the honest input rather than a stub: a door with
            // no position and no direction is a state the contract carries.
            insertion_point: None,
            through_wall_normal: None,
            type_id: "t1".to_string(),
            type_name: "Single".to_string(),
            properties,
            type_properties: BTreeMap::new(),
        }
    }

    fn bundle() -> ProjectSettings {
        ProjectSettings {
            reference: BTreeMap::new(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec![],
            milestones: vec![],
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            windows: Default::default(),
            hierarchy_exclusions: vec![],
        }
    }

    // ---------- the wire shape, pinned ----------

    /// The `/doors` payload exactly as it serialises today, with `revision`
    /// normalised out. Lifted out of the test so the function stays under the
    /// `too_many_lines` trigger and so this reads as what it is: recorded
    /// output, not code. Regenerate it only when a wire change is INTENDED.
    const EXPECTED_DOORS_WIRE: &str = r#"{
  "doors": [
    {
      "from_room": "r1",
      "id": "d1",
      "insertion_point": {
        "x": 1.5,
        "y": 0.125
      },
      "level_id": "lvl1",
      "loops": [
        {
          "points": [
            {
              "x": 0.0,
              "y": 0.0
            },
            {
              "x": 3.0,
              "y": 0.0
            },
            {
              "x": 3.0,
              "y": 0.25
            }
          ]
        }
      ],
      "model_id": "m1",
      "owner_rooms": [
        "r2"
      ],
      "owner_rooms_qualified": [
        {
          "model_id": "m1",
          "room_id": "r2"
        }
      ],
      "project_id": "p1",
      "properties": {
        "Mark": {
          "storage_type": "String",
          "value": "D-101"
        }
      },
      "room_origin": {
        "from_room": {
          "origin": "authored",
          "value": {
            "model_id": "m1",
            "room_id": "r1"
          }
        },
        "to_room": {
          "origin": "authored",
          "value": {
            "model_id": "m1",
            "room_id": "r2"
          }
        }
      },
      "through_wall_normal": {
        "x": 0.0,
        "y": 1.0
      },
      "to_room": "r2",
      "type_id": "t1",
      "type_name": "Single",
      "type_properties": {
        "Door Leaf Thickness": {
          "storage_type": null,
          "value": "40.0"
        }
      }
    }
  ],
  "phase_by_model": {
    "p1": {
      "m1": "New Construction"
    }
  },
  "revision": "<pinned-out>",
  "schema_version": 2
}"#;

    /// **The `/doors` response, pinned byte for byte.**
    ///
    /// This test exists for one job that no other test here does: to make the
    /// *generalisation* of the door stack into a shared `Opening` provably
    /// inert. Every other test asserts a field it cares about, so a rename that
    /// quietly dropped `#[serde(flatten)]`, reordered a struct, changed a
    /// `skip_serializing_if`, or renamed a wire key could pass all of them and
    /// still change what a consumer receives. The viewer and the MCP tools read
    /// these exact names.
    ///
    /// It is deliberately a whole-payload string compare rather than a set of
    /// field assertions, because the failure it guards against is *unknown*: if
    /// the shape changes at all, the diff should say so, and the reviewer should
    /// have to look at it. A test that only checked the fields somebody thought
    /// of would not have caught the thing nobody thought of.
    ///
    /// **`revision` is normalised out, and that is not laziness.** It is a
    /// `DefaultHasher` digest, and std does not promise that hasher's output is
    /// stable across Rust versions — pinning it would turn a toolchain upgrade
    /// into a mysterious failure in a test about serde. It cannot vary under a
    /// rename anyway, since it hashes ids and timestamps rather than type names.
    /// Its stability *within* a run is asserted separately below, which is the
    /// property consumers actually rely on.
    #[test]
    fn test_the_doors_wire_shape_is_pinned() {
        let door = Opening {
            id: "d1".to_string(),
            level_id: "lvl1".to_string(),
            loops: vec![crate::contract::Loop {
                points: vec![
                    crate::contract::Point2D { x: 0.0, y: 0.0 },
                    crate::contract::Point2D { x: 3.0, y: 0.0 },
                    crate::contract::Point2D { x: 3.0, y: 0.25 },
                ],
            }],
            from_room: Some("r1".to_string()),
            to_room: Some("r2".to_string()),
            insertion_point: Some(crate::contract::Point2D { x: 1.5, y: 0.125 }),
            through_wall_normal: Some(crate::contract::Point2D { x: 0.0, y: 1.0 }),
            type_id: "t1".to_string(),
            type_name: "Single".to_string(),
            properties: BTreeMap::from([(
                "Mark".to_string(),
                CustomValue { value: "D-101".to_string(), storage_type: Some("String".to_string()) },
            )]),
            type_properties: BTreeMap::from([(
                "Door Leaf Thickness".to_string(),
                CustomValue { value: "40.0".to_string(), storage_type: None },
            )]),
        };

        let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), bundle())]), None);
        state
            .set_snapshot(RoomPayload {
                schema_version: SUPPORTED_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
                phase: Some("New Construction".to_string()),
                model_to_shared: None,
                room_boundary: None,
                levels: vec![],
                rooms: vec![room_rect("r1", 0.0, 10.0), room_rect("r2", 10.5, 20.0)],
            })
            .unwrap();
        state
            .set_door_snapshot(DoorPayload {
                schema_version: SUPPORTED_DOOR_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
                phase: Some("New Construction".to_string()),
                model_to_shared: None,
                levels: vec![],
                doors: vec![door],
            })
            .unwrap();

        let result = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .expect("data");
        let again = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .expect("data");
        assert_eq!(result.revision, again.revision, "revision is stable for unchanged data");
        assert!(!result.revision.is_empty(), "revision is always emitted");

        let wire = DoorsResult::from_assembled(result);
        let mut json = serde_json::to_value(&wire).expect("serialises");
        json["revision"] = serde_json::Value::String("<pinned-out>".to_string());
        let actual = serde_json::to_string_pretty(&json).expect("re-serialises");

        assert_eq!(actual, EXPECTED_DOORS_WIRE, "the /doors wire shape changed");
    }

    // ---------- geometric resolution ----------

    /// A door in the wall between two rooms, with no `from_room`/`to_room` — the
    /// split-model case, where Revit populates neither.
    fn wall_door(id: &str) -> Opening {
        Opening {
            insertion_point: Some(crate::contract::Point2D { x: 10.25, y: 5.0 }),
            through_wall_normal: Some(crate::contract::Point2D { x: 1.0, y: 0.0 }),
            level_id: "lvl1".to_string(),
            ..make_door(id, None, None, &[])
        }
    }

    fn room_rect(id: &str, x0: f64, x1: f64) -> crate::contract::Room {
        crate::contract::Room {
            id: id.to_string(),
            name: id.to_string(),
            level_id: "lvl1".to_string(),
            loops: vec![crate::contract::Loop {
                points: vec![
                    crate::contract::Point2D { x: x0, y: 0.0 },
                    crate::contract::Point2D { x: x1, y: 0.0 },
                    crate::contract::Point2D { x: x1, y: 10.0 },
                    crate::contract::Point2D { x: x0, y: 10.0 },
                ],
            }],
            properties: BTreeMap::new(),
        }
    }

    /// A state holding one model's rooms (left | wall | right) and one door in
    /// the wall, with `room_resolution` set.
    fn state_with_wall(mode: crate::settings::RoomResolution, doors: Vec<Opening>) -> AppState {
        let mut bundle = bundle();
        bundle.doors.room_resolution = mode;
        let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), bundle)]), None);
        state
            .set_snapshot(RoomPayload {
                schema_version: SUPPORTED_SCHEMA,
                project: Project { id: "p1".into(), name: "P".into() },
                model: Model { id: "m1".into(), name: "M".into(), source: "revit".into() },
                snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".into() },
                phase: Some("New Construction".into()),
                model_to_shared: None,
                room_boundary: Some(crate::contract::RoomBoundary::FinishFace),
                levels: vec![crate::contract::Level { id: "lvl1".into(), name: "Level 1".into(), elevation: 0.0 }],
                rooms: vec![room_rect("left", 0.0, 10.0), room_rect("right", 10.5, 20.0)],
            })
            .unwrap();
        state
            .set_door_snapshot(DoorPayload {
                schema_version: SUPPORTED_DOOR_SCHEMA,
                project: Project { id: "p1".into(), name: "P".into() },
                model: Model { id: "m1".into(), name: "M".into(), source: "revit".into() },
                snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".into() },
                phase: Some("New Construction".into()),
                model_to_shared: None,
                levels: vec![],
                doors,
            })
            .unwrap();
        state
    }

    /// **The split-model fix.** Revit left both references empty; the geometry
    /// fills them, and the door is attributed instead of homeless.
    #[test]
    fn test_geometry_fills_a_door_that_states_no_rooms() {
        let state = state_with_wall(crate::settings::RoomResolution::SameModel, vec![wall_door("d1")]);
        let result = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .unwrap();
        let door = &result.openings[0];

        assert_eq!(
            door.room_origin.to_room,
            SideOrigin::Derived(RoomRef { model_id: "m1".into(), room_id: "right".into() }),
            "+normal is the to-side"
        );
        assert_eq!(
            door.room_origin.from_room,
            SideOrigin::Derived(RoomRef { model_id: "m1".into(), room_id: "left".into() })
        );
        assert_eq!(door.owner_rooms, vec!["right".to_string()], "attributed, not homeless");
        assert_eq!(door.door.to_room, None, "the stored reference is untouched");
    }

    /// A state with the rooms in ONE model and the doors in ANOTHER that has no
    /// rooms at all — the facade/envelope split. `door_levels` is what the doors
    /// model declares on its own envelope; passing `&[]` reproduces the state
    /// before `DoorPayload::levels` existed.
    fn state_split_models(
        mode: crate::settings::RoomResolution,
        door_levels: &[(&str, f64)],
        doors: Vec<Opening>,
    ) -> AppState {
        let mut bundle = bundle();
        bundle.doors.room_resolution = mode;
        let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), bundle)]), None);
        // The rooms model. Identity placement on both sides, so the shared frame
        // is a no-op and the test is about the elevation lookup, not the affine.
        state
            .set_snapshot(RoomPayload {
                schema_version: SUPPORTED_SCHEMA,
                project: Project { id: "p1".into(), name: "P".into() },
                model: Model { id: "interior".into(), name: "M".into(), source: "revit".into() },
                snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".into() },
                phase: Some("New Construction".into()),
                model_to_shared: Some(ModelToShared { matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] }),
                room_boundary: Some(crate::contract::RoomBoundary::FinishFace),
                levels: vec![crate::contract::Level { id: "lvl1".into(), name: "Level 1".into(), elevation: 0.0 }],
                rooms: vec![room_rect("left", 0.0, 10.0), room_rect("right", 10.5, 20.0)],
            })
            .unwrap();
        // The facade model: doors, no rooms, and its OWN level ids — which never
        // match the interior model's, exactly as in Revit.
        state
            .set_door_snapshot(DoorPayload {
                schema_version: SUPPORTED_DOOR_SCHEMA,
                project: Project { id: "p1".into(), name: "P".into() },
                model: Model { id: "facade".into(), name: "F".into(), source: "revit".into() },
                snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".into() },
                phase: Some("New Construction".into()),
                model_to_shared: Some(ModelToShared { matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] }),
                levels: door_levels
                    .iter()
                    .map(|(id, elevation)| crate::contract::Level {
                        id: (*id).into(),
                        name: "Level 1".into(),
                        elevation: *elevation,
                    })
                    .collect(),
                doors,
            })
            .unwrap();
        state
    }

    fn facade_door(id: &str, level_id: &str) -> Opening {
        Opening { level_id: level_id.to_string(), ..wall_door(id) }
    }

    /// **The doors-only model, which is what `DoorPayload::levels` exists for.**
    /// Its levels let `locate` put the door on an elevation axis; the rooms it
    /// then finds are in a *different* model, so the answer is model-qualified.
    #[test]
    fn test_a_doors_only_model_resolves_against_another_models_rooms() {
        let state = state_split_models(
            crate::settings::RoomResolution::Project,
            &[("facade-lvl", 0.0)],
            vec![facade_door("d1", "facade-lvl")],
        );
        let result = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .unwrap();
        let door = &result.openings[0];

        assert_eq!(
            door.room_origin.to_room,
            SideOrigin::Derived(RoomRef { model_id: "interior".into(), room_id: "right".into() }),
            "the room is in the interior model, not the door's own"
        );
        assert_eq!(
            door.owner_rooms_qualified,
            vec![RoomRef { model_id: "interior".into(), room_id: "right".into() }],
            "a cross-model owner is only expressible on the qualified list"
        );
        assert!(
            door.owner_rooms.is_empty(),
            "the bare list stays same-model: a foreign room id there would resolve against the wrong model"
        );
    }

    /// **Without the levels it is unreachable, not merely unresolved** — and the
    /// distinction is the whole reason the field was added. The geometry is
    /// identical to the test above; only the doors envelope's level set is gone,
    /// and `locate` gives up before it probes anything.
    #[test]
    fn test_a_doors_only_model_with_no_levels_cannot_be_probed() {
        let state =
            state_split_models(crate::settings::RoomResolution::Project, &[], vec![facade_door("d1", "facade-lvl")]);
        let result = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .unwrap();
        let door = &result.openings[0];

        assert_eq!(door.room_origin.to_room, SideOrigin::Unresolved(Unresolved::NoCandidate));
        assert!(door.owner_rooms_qualified.is_empty(), "homeless, with the geometry never consulted");
    }

    /// **A rooms snapshot wins over the doors envelope's copy.** The doors model
    /// declares its level at a nonsense elevation; the rooms snapshot declares
    /// the same id at the real one, and the door still resolves — so a model
    /// that pushes both cannot be broken by a stale duplicate.
    #[test]
    fn test_the_rooms_snapshot_wins_over_the_doors_envelope_levels() {
        let mut bundle = bundle();
        bundle.doors.room_resolution = crate::settings::RoomResolution::SameModel;
        let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), bundle)]), None);
        state
            .set_snapshot(RoomPayload {
                schema_version: SUPPORTED_SCHEMA,
                project: Project { id: "p1".into(), name: "P".into() },
                model: Model { id: "m1".into(), name: "M".into(), source: "revit".into() },
                snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".into() },
                phase: Some("New Construction".into()),
                model_to_shared: None,
                room_boundary: Some(crate::contract::RoomBoundary::FinishFace),
                levels: vec![crate::contract::Level { id: "lvl1".into(), name: "Level 1".into(), elevation: 0.0 }],
                rooms: vec![room_rect("left", 0.0, 10.0), room_rect("right", 10.5, 20.0)],
            })
            .unwrap();
        state
            .set_door_snapshot(DoorPayload {
                schema_version: SUPPORTED_DOOR_SCHEMA,
                project: Project { id: "p1".into(), name: "P".into() },
                model: Model { id: "m1".into(), name: "M".into(), source: "revit".into() },
                snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".into() },
                phase: Some("New Construction".into()),
                model_to_shared: None,
                levels: vec![crate::contract::Level { id: "lvl1".into(), name: "Level 1".into(), elevation: 9999.0 }],
                doors: vec![wall_door("d1")],
            })
            .unwrap();

        let result = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .unwrap();
        assert_eq!(
            result.openings[0].owner_rooms,
            vec!["right".to_string()],
            "resolved at the rooms snapshot's elevation, not the doors envelope's 9999"
        );
    }

    /// **Off is the default and it derives nothing.** Turning resolution on
    /// changes `owner_rooms` for exactly these doors, which is why it is opt-in.
    #[test]
    fn test_resolution_off_leaves_the_door_homeless() {
        let state = state_with_wall(crate::settings::RoomResolution::Off, vec![wall_door("d1")]);
        let result = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .unwrap();
        let door = &result.openings[0];

        assert!(door.owner_rooms.is_empty(), "homeless, as before");
        assert!(matches!(door.room_origin.to_room, SideOrigin::Unresolved(_)));
    }

    /// **Authored always wins.** The model says this door opens into `left`,
    /// which is behind it geometrically — a cupboard-off-a-corridor shape. The
    /// stated answer is served unchanged and the geometry does not override it.
    #[test]
    fn test_an_authored_reference_is_never_overridden_by_geometry() {
        let door = Opening { to_room: Some("left".into()), ..wall_door("d1") };
        let state = state_with_wall(crate::settings::RoomResolution::SameModel, vec![door]);
        let result = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .unwrap();
        let door = &result.openings[0];

        assert_eq!(
            door.room_origin.to_room,
            SideOrigin::Authored(RoomRef { model_id: "m1".into(), room_id: "left".into() }),
            "the modeller's assignment stands"
        );
        assert_eq!(door.owner_rooms, vec!["left".to_string()]);
        // The other side was absent, so geometry filled that one.
        assert_eq!(
            door.room_origin.from_room,
            SideOrigin::Derived(RoomRef { model_id: "m1".into(), room_id: "left".into() })
        );
    }

    /// A door with no plan direction resolves neither side, and says why. A
    /// guessed direction would be worse than none.
    #[test]
    fn test_a_door_with_no_direction_resolves_nothing() {
        let door = Opening { through_wall_normal: None, ..wall_door("d1") };
        let state = state_with_wall(crate::settings::RoomResolution::SameModel, vec![door]);
        let result = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .unwrap();

        assert_eq!(result.openings[0].room_origin.to_room, SideOrigin::Unresolved(Unresolved::NoDirection));
    }

    /// `owner_rooms` keeps meaning "this model's rooms"; the qualified list
    /// carries the model id so a cross-model owner has somewhere to be said.
    #[test]
    fn test_qualified_owners_carry_the_model_id() {
        let state = state_with_wall(crate::settings::RoomResolution::SameModel, vec![wall_door("d1")]);
        let result = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .unwrap();
        let door = &result.openings[0];

        assert_eq!(
            door.owner_rooms_qualified,
            vec![RoomRef { model_id: "m1".into(), room_id: "right".into() }]
        );
        assert_eq!(
            door.owner_rooms,
            vec!["right".to_string()],
            "the bare list is the same rooms, unqualified"
        );
    }

    /// **R4 end to end: a source declaring `entity = "doors"` joins onto doors.**
    ///
    /// Before this, `[sources.reference.<name>]` meant "for rooms" with nothing
    /// saying so, and a source configured for anything else parsed, loaded and
    /// joined nowhere. The filter grammar was already written for this day --
    /// `OpeningResponse::presence` answered `Absent` for every source-qualified
    /// field specifically so the same predicate would start *matching* rather
    /// than change status once the join existed.

    #[tokio::test]
    async fn test_a_doors_scoped_reference_source_joins_onto_doors() {
        // Row 1 is labels, row 2 col 0 names the door property holding the key.
        let csv = b"DoorRef,FireRating
Door Mark,FireRating
D-101,60
";
        let data = crate::reference::load_reference_from_bytes(csv).expect("csv loads");

        let mut settings = bundle();
        settings.reference.insert(
            "schedule".to_string(),
            crate::state::ProjectReferenceSource { entity: ReferenceEntity::Doors, data: Some(data), fields: vec![] },
        );

        let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), settings)]), None);
        state
            .set_door_snapshot(DoorPayload {
                schema_version: SUPPORTED_DOOR_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
                phase: Some("New Construction".to_string()),
                model_to_shared: None,
                levels: vec![],
                doors: vec![
                    make_door("d1", None, Some("r1"), &[("Door Mark", "D-101")]),
                    // No matching key: an unmatched row is a signal, not an
                    // error, so this door simply joins nothing.
                    make_door("d2", None, Some("r1"), &[("Door Mark", "D-999")]),
                ],
            })
            .unwrap();

        let result = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .expect("data");
        let joined = result.openings.iter().find(|d| d.door.id == "d1").expect("d1");
        assert_eq!(
            joined.reference.get("schedule").and_then(|r| r.fields.get("FireRating")),
            Some(&"60".to_string()),
            "the door schedule joined onto the door"
        );

        let unmatched = result.openings.iter().find(|d| d.door.id == "d2").expect("d2");
        assert!(unmatched.reference.is_empty(), "an unmatched key joins nothing, and is not an error");

        // And the filter grammar reaches it -- the same `source.property` shape
        // a room filter writes, which is the whole reason the namespace stayed
        // flat rather than nesting the entity.
        let builtin: &[BuiltinPropertyDef] = &[];
        assert_eq!(
            joined.presence(Some("schedule"), "FireRating", builtin),
            PropertyPresence::Present("60".to_string())
        );
        assert_eq!(
            unmatched.presence(Some("schedule"), "FireRating", builtin),
            PropertyPresence::Absent,
            "a door that joined nothing answers Absent, exactly as a room does"
        );
    }

    /// The other half of the scoping: a **rooms** source must not join onto
    /// doors. Without the entity check a door whose link property happened to
    /// collide would pick up a room's columns.
    #[tokio::test]
    async fn test_a_rooms_scoped_reference_source_does_not_join_onto_doors() {
        let csv = b"DoorRef,FireRating
Door Mark,FireRating
D-101,60
";
        let data = crate::reference::load_reference_from_bytes(csv).expect("csv loads");

        let mut settings = bundle();
        settings.reference.insert(
            "schedule".to_string(),
            crate::state::ProjectReferenceSource { entity: ReferenceEntity::Rooms, data: Some(data), fields: vec![] },
        );

        let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), settings)]), None);
        state
            .set_door_snapshot(DoorPayload {
                schema_version: SUPPORTED_DOOR_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
                phase: Some("New Construction".to_string()),
                model_to_shared: None,
                levels: vec![],
                doors: vec![make_door("d1", None, Some("r1"), &[("Door Mark", "D-101")])],
            })
            .unwrap();

        let result = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .expect("data");
        assert!(
            result.openings[0].reference.is_empty(),
            "a rooms-scoped source is not this entity's source, even when the key matches"
        );
    }

    fn state_with(doors: Vec<(&str, &str, Vec<Opening>)>) -> AppState {
        let mut projects = HashMap::new();
        for (project, _, _) in &doors {
            projects.insert(project.to_string(), bundle());
        }
        let state = AppState::new(Box::new(MemStore::new()), projects, None);
        for (project, model, list) in doors {
            state
                .set_door_snapshot(DoorPayload {
                    schema_version: SUPPORTED_DOOR_SCHEMA,
                    project: Project { id: project.to_string(), name: "P".to_string() },
                    model: Model { id: model.to_string(), name: "M".to_string(), source: "revit".to_string() },
                    snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
                    phase: Some("New Construction".to_string()),
                    model_to_shared: None,
                    levels: vec![],
                    doors: list,
                })
                .unwrap();
        }
        state
    }

    fn filter(expr: &str) -> RoomFilter {
        RoomFilter::parse_query(expr, &BTreeSet::new()).expect("parses")
    }

    /// Nothing pushed at all is `None` — the adapter's 204, distinct from a
    /// scope that matched nothing.
    #[test]
    fn test_no_doors_pushed_is_none() {
        let state = AppState::new(Box::new(MemStore::new()), HashMap::new(), None);
        assert!(assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .is_none());
    }

    /// A scope that matches nothing is an empty list, not `None`: the store has
    /// data, the question just has an empty answer.
    #[test]
    fn test_a_scope_matching_nothing_is_an_empty_list() {
        let state = state_with(vec![("p1", "m1", vec![make_door("d1", Some("r1"), None, &[])])]);
        let result = assemble_openings::<DoorPayload>(
            &state,
            OPENING_KIND,
            &OpeningScope { project: Some("nope"), ..Default::default() },
        )
        .unwrap()
        .expect("the store has data");
        assert!(result.openings.is_empty());
    }

    /// Doors from every model merge, each carrying the model it came from —
    /// which is what makes its room references resolvable.
    #[test]
    fn test_doors_merge_carrying_their_model_identity() {
        let state = state_with(vec![
            ("p1", "m1", vec![make_door("d1", Some("r1"), None, &[])]),
            ("p1", "m2", vec![make_door("d1", Some("r1"), None, &[])]),
        ]);
        let result = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .expect("data");
        assert_eq!(result.openings.len(), 2);
        let models: BTreeSet<&str> = result.openings.iter().map(|d| d.model_id.as_str()).collect();
        assert_eq!(
            models,
            BTreeSet::from(["m1", "m2"]),
            "the same door id in two models stays distinguishable"
        );
        assert_eq!(result.phase_by_model["p1"]["m1"].as_deref(), Some("New Construction"));
    }

    /// The property filter reaches a door's own properties, using the same
    /// grammar `/rooms` uses.
    #[test]
    fn test_filter_matches_a_door_property() {
        let state = state_with(vec![(
            "p1",
            "m1",
            vec![
                make_door("d1", Some("r1"), None, &[("Mark", "29")]),
                make_door("d2", Some("r2"), None, &[("Mark", "33")]),
            ],
        )]);
        let f = filter("Mark=29");
        let result = assemble_openings::<DoorPayload>(
            &state,
            OPENING_KIND,
            &OpeningScope { filter: Some(&f), ..Default::default() },
        )
        .unwrap()
        .expect("data");
        assert_eq!(result.openings.len(), 1);
        assert_eq!(result.openings[0].door.id, "d1");
    }

    /// The room references are filterable intrinsics — `$to_room=` is how a
    /// caller asks "which doors open into this room", which is the read the
    /// whole door→room link exists for.
    #[test]
    fn test_filter_matches_a_room_reference_intrinsic() {
        let state = state_with(vec![(
            "p1",
            "m1",
            vec![
                make_door("d1", Some("r1"), Some("r2"), &[]),
                make_door("d2", Some("r3"), None, &[]),
            ],
        )]);
        let f = filter("$to_room=r2");
        let result = assemble_openings::<DoorPayload>(
            &state,
            OPENING_KIND,
            &OpeningScope { filter: Some(&f), ..Default::default() },
        )
        .unwrap()
        .expect("data");
        assert_eq!(result.openings.len(), 1);
        assert_eq!(result.openings[0].door.id, "d1");

        // An external door is `Absent` on its missing side, so it fails every
        // operator — including the negative one. "This door has no to_room" is
        // not evidence that its to_room differs from r2.
        let f = filter("$to_room!=r2");
        let result = assemble_openings::<DoorPayload>(
            &state,
            OPENING_KIND,
            &OpeningScope { filter: Some(&f), ..Default::default() },
        )
        .unwrap()
        .expect("data");
        assert!(
            result.openings.is_empty(),
            "d2 has no to_room, so it does not match a negative operator either"
        );
    }

    /// The two property tiers are both reachable through the filter, instance
    /// first — the R2 rule applied through the read path rather than restated.
    #[test]
    fn test_filter_reaches_the_type_tier() {
        let mut door = make_door("d1", Some("r1"), None, &[("Door Leaf Thickness", "")]);
        door.type_properties.insert(
            "Door Leaf Thickness".to_string(),
            CustomValue { value: "40.0".to_string(), storage_type: None },
        );
        let state = state_with(vec![("p1", "m1", vec![door])]);

        let f = filter("Door Leaf Thickness=40.0");
        let result = assemble_openings::<DoorPayload>(
            &state,
            OPENING_KIND,
            &OpeningScope { filter: Some(&f), ..Default::default() },
        )
        .unwrap()
        .expect("data");
        assert_eq!(result.openings.len(), 1, "a blank instance value does not shadow the type's");
    }

    /// A source-qualified predicate resolves `Absent` rather than erroring:
    /// doors carry no reference joins until R4, and `Absent` is the same answer
    /// a room gets for a source it did not join. The day R4 lands, this
    /// predicate starts matching instead of changing status.
    #[test]
    fn test_a_source_qualified_filter_matches_no_door() {
        let state = state_with(vec![("p1", "m1", vec![make_door("d1", Some("r1"), None, &[])])]);
        let known = BTreeSet::from(["drofus".to_string()]);
        let f = RoomFilter::parse_query("drofus.NetArea=25.5", &known).expect("parses against a known source");

        let result = assemble_openings::<DoorPayload>(
            &state,
            OPENING_KIND,
            &OpeningScope { filter: Some(&f), ..Default::default() },
        )
        .unwrap()
        .expect("data");
        assert!(result.openings.is_empty());
    }

    /// An unregistered project's doors are skipped on read, matching
    /// `assemble_rooms` — there is nothing to resolve canonical names against.
    #[test]
    fn test_an_unregistered_projects_doors_are_skipped() {
        let state = AppState::new(Box::new(MemStore::new()), HashMap::new(), None);
        state
            .set_door_snapshot(DoorPayload {
                schema_version: SUPPORTED_DOOR_SCHEMA,
                project: Project { id: "ghost".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
                phase: Some("New Construction".to_string()),
                model_to_shared: None,
                levels: vec![],
                doors: vec![make_door("d1", Some("r1"), None, &[])],
            })
            .unwrap();

        let result = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .expect("the store has data");
        assert!(result.openings.is_empty());
    }

    /// The revision is stable between two idle reads and moves when the
    /// contributing snapshot changes — the "has anything changed?" signal, same
    /// contract as `/rooms`.
    #[test]
    fn test_revision_is_stable_and_moves_on_a_push() {
        let state = state_with(vec![("p1", "m1", vec![make_door("d1", Some("r1"), None, &[])])]);
        let first = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .unwrap()
            .revision;
        let again = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .unwrap()
            .revision;
        assert_eq!(first, again, "two idle reads agree");

        state
            .set_door_snapshot(DoorPayload {
                schema_version: SUPPORTED_DOOR_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-03-01T00:00:00Z".to_string() },
                phase: Some("New Construction".to_string()),
                model_to_shared: None,
                levels: vec![],
                doors: vec![make_door("d1", Some("r1"), None, &[])],
            })
            .unwrap();
        let after = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .unwrap()
            .revision;
        assert_ne!(first, after, "a new snapshot moves it");
    }

    /// `owner_rooms` on the read follows the project's policy, and an empty
    /// list is the homeless signal rather than a missing field.
    #[test]
    fn test_owner_rooms_follows_the_configured_policy() {
        let doors = vec![
            make_door("both", Some("r1"), Some("r2"), &[]),
            make_door("from-only", Some("r3"), None, &[]),
            make_door("homeless", None, None, &[]),
        ];

        let owners_for = |policy: crate::settings::RoomAttribution| {
            let mut settings = bundle();
            settings.doors.room_attribution = policy;
            let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), settings)]), None);
            state
                .set_door_snapshot(DoorPayload {
                    schema_version: SUPPORTED_DOOR_SCHEMA,
                    project: Project { id: "p1".to_string(), name: "P".to_string() },
                    model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                    snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
                    phase: Some("New Construction".to_string()),
                    model_to_shared: None,
                    levels: vec![],
                    doors: doors.clone(),
                })
                .unwrap();
            let r = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
                .unwrap()
                .unwrap();
            r.openings.into_iter().map(|d| (d.door.id, d.owner_rooms)).collect::<BTreeMap<_, _>>()
        };

        // The decided default: opens-into, else opens-from, else homeless.
        let chain = owners_for(crate::settings::RoomAttribution::ToRoomThenFromRoom);
        assert_eq!(chain["both"], vec!["r2".to_string()]);
        assert_eq!(chain["from-only"], vec!["r3".to_string()]);
        assert!(chain["homeless"].is_empty());

        // A narrower policy leaves the from-only door homeless.
        let strict = owners_for(crate::settings::RoomAttribution::ToRoom);
        assert_eq!(strict["both"], vec!["r2".to_string()]);
        assert!(strict["from-only"].is_empty());

        // `both` is why this is a list: one door, two owners, to_room first.
        let both = owners_for(crate::settings::RoomAttribution::Both);
        assert_eq!(both["both"], vec!["r2".to_string(), "r1".to_string()]);
        assert_eq!(both["from-only"], vec!["r3".to_string()], "one-sided doors still yield one owner");
    }

    /// **`?building=` scopes doors by their OWNING room's building** — the
    /// capability the attribution decision unlocked.
    ///
    /// Also pins the two rules that make it correct: a homeless door matches no
    /// building (it is not evidence of belonging elsewhere), and the lookup is
    /// keyed on `(model, room)` so a same-numbered room in another model cannot
    /// resolve it — the collision this codebase guards everywhere doors touch
    /// room ids.
    #[test]
    fn test_building_scope_follows_the_owning_room() {
        let mut settings = bundle();
        settings.hierarchy = vec![crate::settings::HierarchyTier {
            name: "Building".to_string(),
            code_property: None,
            name_property: Some("Building".to_string()),
        }];
        let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), settings)]), None);

        // Two models, each with a room id "r1" — in DIFFERENT buildings.
        let room = |id: &str, building: &str| crate::contract::Room {
            id: id.to_string(),
            name: id.to_string(),
            level_id: "1".to_string(),
            loops: vec![],
            properties: BTreeMap::from([(
                "Building".to_string(),
                CustomValue { value: building.to_string(), storage_type: None },
            )]),
        };
        for (model, building) in [("m1", "North"), ("m2", "South")] {
            state
                .set_snapshot(crate::contract::RoomPayload {
                    schema_version: crate::contract::SUPPORTED_SCHEMA,
                    project: Project { id: "p1".to_string(), name: "P".to_string() },
                    model: Model { id: model.to_string(), name: "M".to_string(), source: "revit".to_string() },
                    snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
                    phase: Some("New Construction".to_string()),
                    model_to_shared: None,
                    room_boundary: None,
                    levels: vec![],
                    rooms: vec![room("r1", building)],
                })
                .unwrap();
        }
        // m1's door owns m1's r1 (North); m2's door owns m2's r1 (South).
        for model in ["m1", "m2"] {
            state
                .set_door_snapshot(DoorPayload {
                    schema_version: SUPPORTED_DOOR_SCHEMA,
                    project: Project { id: "p1".to_string(), name: "P".to_string() },
                    model: Model { id: model.to_string(), name: "M".to_string(), source: "revit".to_string() },
                    snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
                    phase: Some("New Construction".to_string()),
                    model_to_shared: None,
                    levels: vec![],
                    doors: vec![
                        make_door(&format!("{model}-owned"), None, Some("r1"), &[]),
                        make_door(&format!("{model}-homeless"), None, None, &[]),
                    ],
                })
                .unwrap();
        }

        let in_building = |key: &str| {
            let r = assemble_openings::<DoorPayload>(
                &state,
                OPENING_KIND,
                &OpeningScope { building: Some(key), ..Default::default() },
            )
            .unwrap()
            .unwrap();
            r.openings.into_iter().map(|d| d.door.id).collect::<BTreeSet<_>>()
        };

        assert_eq!(
            in_building("|North"),
            BTreeSet::from(["m1-owned".to_string()]),
            "m2's door owns m2's r1, which is South — a same-numbered room in another model must not resolve it"
        );
        assert_eq!(in_building("|South"), BTreeSet::from(["m2-owned".to_string()]));
        assert!(in_building("|Nowhere").is_empty());

        // Both homeless doors are absent from every building, and present when
        // no building is asked for — the filter excludes them, nothing else does.
        let all = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .unwrap();
        assert_eq!(all.openings.len(), 4, "unscoped, homeless doors are served normally");
    }

    /// A milestone serves the doors snapshot it pins, not the model's latest.
    #[test]
    fn test_milestone_serves_the_pinned_doors_snapshot() {
        let mut settings = bundle();
        settings.milestones = vec![crate::settings::Milestone {
            name: "Stage 2".to_string(),
            date: "2026-02-01".to_string(),
            reference_snapshots: BTreeMap::new(),
            attachments: BTreeMap::new(),
            door_attachments: BTreeMap::from([("m1".to_string(), "2026-02-01T00:00:00Z".to_string())]),
            window_attachments: Default::default(),
        }];
        let state = AppState::new(
            Box::new(
                crate::storage::FsStore::new(
                    std::env::temp_dir().join(format!("roommate-doors-milestone-{}", std::process::id())),
                )
                .unwrap(),
            ),
            HashMap::from([("p1".to_string(), settings)]),
            None,
        );
        let push = |ts: &str, id: &str| {
            state
                .set_door_snapshot(DoorPayload {
                    schema_version: SUPPORTED_DOOR_SCHEMA,
                    project: Project { id: "p1".to_string(), name: "P".to_string() },
                    model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                    snapshot: Snapshot { taken_at: ts.to_string() },
                    phase: Some("New Construction".to_string()),
                    model_to_shared: None,
                    levels: vec![],
                    doors: vec![make_door(id, Some("r1"), None, &[])],
                })
                .unwrap()
        };
        push("2026-02-01T00:00:00Z", "pinned");
        push("2026-06-01T00:00:00Z", "latest");

        let latest = assemble_openings::<DoorPayload>(&state, OPENING_KIND, &OpeningScope::default())
            .unwrap()
            .unwrap();
        assert_eq!(latest.openings[0].door.id, "latest");

        let pinned = assemble_openings::<DoorPayload>(
            &state,
            OPENING_KIND,
            &OpeningScope { milestone: Some("Stage 2"), ..Default::default() },
        )
        .unwrap()
        .unwrap();
        assert_eq!(pinned.openings[0].door.id, "pinned");

        // A model the milestone does not pin contributes nothing, matching the
        // rooms discipline rather than silently falling back to latest.
        let unknown = assemble_openings::<DoorPayload>(
            &state,
            OPENING_KIND,
            &OpeningScope { milestone: Some("nope"), ..Default::default() },
        )
        .unwrap()
        .unwrap();
        assert!(unknown.openings.is_empty());
    }
}
