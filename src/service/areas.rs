//! Hierarchy area footprints — the first geometry-processing service.
//!
//! **Read `docs/STRATEGY-AREA-CALCULATION.md` before changing anything here.**
//! This module is the one place in the pipeline where the *definition* of the
//! output is contested rather than read off the model — what a wall belongs to
//! is a convention, and reasonable standards disagree. Two prior designs were
//! reversed over exactly that. What follows is how the current one works; that
//! document is why it is the one chosen, and how it relates to IPMS 3 and
//! DIN 277.
//!
//! Per hierarchy group, per level: the group's rooms, plus the wall bands the
//! group encloses on its own. Two exact pieces, unioned — never one arbitrated
//! blob.
//!
//! ## The wall zone, built once per level
//!
//! Everything rests on one object, computed once for the whole level:
//!
//! ```text
//! wall_zone = (close(all rooms, gap/2) ∪ all rooms) − all rooms
//! ```
//!
//!
//! It is the set of gaps that are narrow enough to be walls, and **nothing
//! else**: it contains no room (they are subtracted), and no void wider than
//! `gap`, because a morphological close at radius `gap/2` provably cannot fill
//! one. That single sentence replaces two mechanisms the earlier version
//! needed — a room-clip pass to stop a group swallowing a neighbour's room, and
//! a width classifier to tell a wall from a courtyard. Both are now true by
//! construction.
//!
//! A group's own share is then whatever the group's rooms close over,
//! **intersected with that ceiling**:
//!
//! ```text
//! fill(P)      = close(rooms under P, gap/2) ∩ wall_zone
//! footprint(P) = rooms under P ∪ fill(P)
//! ```
//!
//! ## Why this gets the ownership rule right for free
//!
//! The design's rule is *a wall between two groups belongs to neither; it fills
//! at their common ancestor*. That falls out of the formula rather than being
//! enforced on top of it. Take the band between room `a` of group A and room `b`
//! of group B. Dilating A's rooms by `gap/2` reaches into the band, but the
//! erode pulls straight back out again — nothing merged across it, because A is
//! only on one side. So `fill(A)` does not contain it, and neither does
//! `fill(B)`. Their common ancestor holds both `a` and `b`, its close merges
//! across the band, and it claims the band exactly once.
//!
//! Both halves of that are standard morphology, and it is worth naming them,
//! because the operator's textbook properties are doing the work here rather
//! than anything invented. Writing `φ` for the close:
//!
//! - **`φ` is increasing** (`X ⊆ Y ⟹ φ(X) ⊆ φ(Y)`). Applied to
//!   `rooms under P ⊆ rooms under parent`, that gives `fill(P) ⊆ fill(parent)` —
//!   the nesting that makes areas additive,
//!   `parent = Σ children + the bands the parent is the first to enclose`.
//! - **`φ` does not distribute over union**: `φ(X ∪ Y) ⊇ φ(X) ∪ φ(Y)`, and the
//!   containment is strict exactly where a gap between `X` and `Y` closes. That
//!   strict part **is** the wall between two groups. The ownership rule is not a
//!   policy layered on the geometry; it is where those two sets differ.
//!
//! (Serra 1982; Soille. Note that closing *is* idempotent, `φ(φ(X)) = φ(X)` —
//! an earlier version of this module and its handover both claimed otherwise.
//! Non-distribution over union is the property that was actually meant, and it
//! is the stronger statement. See `wall_zone` for where the *approximation*
//! departs from the operator.)
//!
//! ## The one place it needs help: wall junctions
//!
//! Where four rooms meet at a `+` junction, the little square of wall at the
//! centre is bounded by all four. If two of them belong to A and two to B, and
//! the wall is thin enough that each pair's diagonal closes, **both** A and B
//! fill that square. That is a genuine double count, not float noise — and the
//! design rule already answers it: bounded by two groups, so it belongs to
//! neither. [`resolve_sibling_overlaps`] withdraws it from both, symmetrically,
//! and the common ancestor picks it up, which keeps additivity exact. It is a
//! junction-sized correction inside the wall band, not the whole-footprint
//! arbitration the earlier version needed.
//!
//! ## Centreline models do none of this
//!
//! When the level's declared regime is centreline (see `contract::RoomBoundary`
//! and `settings::AreaPolicy`), `gap` is **zero**: rooms already tile
//! edge-to-edge with the walls inside them, so there is nothing to bridge and
//! nothing to fill. The wall zone is empty, no close runs, and a footprint is
//! the plain union of its rooms. Bevels, chamfers, spikes and sibling overlaps
//! cannot arise, because the operation that produces them never executes. That
//! is the payoff of declaring the regime rather than guessing it — see
//! Superseded/HANDOVER-areas-boundary-location.md.
//!
//! **History / reversed decisions.** An earlier version stripped *every*
//! interior ring, so enclosed open space always counted as area, and it only
//! dissolved rooms that shared an edge (centreline); real models turned out to
//! be face-of-wall, where it silently produced disjoint islands. The close-based
//! rule replaced it. That rule in turn ran the close **per group per tier**,
//! independently over overlapping neighbourhoods, then unioned the results —
//! and since `φ(X) ∪ φ(Y) ⊊ φ(X ∪ Y)`, that is simply a different (smaller)
//! set than closing the union, with the difference landing in the shared walls.
//! Every artifact (bevelled corners, 45° chamfers, a million-foot spike,
//! sibling overlaps) then had to be arbitrated after the fact. The wall zone
//! replaces the arbitration with a partition: we were using an area operator to
//! answer a topology question. See Superseded/handover-hierarchical-void-closure.md and its
//! reconciliation note.
//!
//! ## Cost, measured rather than assumed
//!
//! The reformulation trades **one large boolean pass per level** (the wall zone:
//! a close, a union and a difference over every room on the floor) for the
//! per-group clip and full-footprint overlap resolution it deletes. Measured on
//! `/areas`, warm, before → after:
//!
//! | project | groups × levels | before | after |
//! |---|---|---|---|
//! | `big-plate` | 132 groups, 2 levels | 6.0 s | **1.12 s** |
//! | `sample-project` | 10 groups, 7 levels | 0.43 s | 0.71 s |
//! | `House A` | 12 groups, 3 levels | 0.35 s | 0.35 s |
//!
//! So it is dramatically cheaper where there is grouping to amortise it against,
//! and ~0.3 s dearer on a project with barely one group per level, where the
//! level-wide pass is a fixed cost with almost nothing to spread it over. That is
//! the honest shape of the trade, and it is the right way round: the expensive
//! case is the one that was unusable. Do not "fix" the small regression by
//! special-casing a single-group level to reuse the level close — that was tried
//! and measured, and it changed neither number, because the cost is the boolean
//! difference over the room set, not the close.
//!
//! Transport-agnostic like every `service` module: it imports `geo` and
//! `crate::contract`, never `axum`/`rmcp`.
//!
//! The number this produces is an **aggregated room footprint** — room area plus
//! enclosed wall bands, minus genuine voids — NOT net room area and NOT a
//! standards-based gross. Which standard the project claims is declared
//! separately (`[areas] measurement_standard`) and echoed on the response, so
//! the figure never travels without its definition.

use std::collections::BTreeMap;

use geo::algorithm::buffer::{BufferStyle, LineJoin};
use geo::{
    Area, BooleanOps, BoundingRect, Buffer, Coord, Distance, Euclidean, LineString, MultiPolygon, Polygon,
};
use serde::Serialize;

use crate::classify::TierValue;
use crate::contract::{Level, Loop, Room, RoomBoundary, SUPPORTED_SCHEMA};
use crate::settings::{AreaPolicy, HierarchyExclusion, MeasurementStandard};
use crate::state::AppState;

use super::rooms::{assemble_rooms, RoomScope};
use super::ServiceError;

/// Perpendicular distance (in model units — feet) below which a vertex is
/// treated as lying *on* the line through its neighbours and dropped. Only
/// true-collinear points and float noise fall under this; a genuine corner sits
/// orders of magnitude above it. Tight on purpose: it removes redundant
/// vertices, never real geometry.
const COLLINEAR_EPS_FT: f64 = 1e-6;

/// The wall gap in force on each level of one request, in model units — feet.
///
/// Per level, not per request, because the boundary regime is a *model* fact and
/// level dedup can put two models on one level (see
/// `rooms::RoomsResult::boundary_by_level`). A centreline level resolves to
/// **zero** — not a small number — and that zero is what switches the whole
/// close pipeline off for it.
///
/// The number itself used to be `MAX_WALL_FT`, a constant here, alongside an
/// identical `WALL_MAX_FT` in `adjacency` — the same physical quantity declared
/// twice, free to drift. It is now declared once per project as `[areas]
/// max_wall_thickness` and resolved through the regime.
pub struct LevelGaps {
    by_level: BTreeMap<String, f64>,
    /// Used for a level the map does not name — a level that contributed no
    /// rooms, or a caller (the unit tests) that only cares about one regime.
    default_ft: f64,
}

impl LevelGaps {
    /// One gap for every level — the form the geometry tests use, where the
    /// regime under test is the point and the level ids are incidental.
    pub fn uniform(gap_ft: f64) -> Self {
        Self { by_level: BTreeMap::new(), default_ft: gap_ft }
    }

    /// Resolve each level's gap from the project's policy and the regime that
    /// level's contributing models declared.
    pub fn from_policy(policy: &AreaPolicy, boundary_by_level: &BTreeMap<String, RoomBoundary>) -> Self {
        Self {
            by_level: boundary_by_level.iter().map(|(id, b)| (id.clone(), policy.wall_gap_ft(*b))).collect(),
            // An unnamed level has no declared regime to read, so it falls to
            // the same conservative reading an undeclared model gets.
            default_ft: policy.wall_gap_ft(RoomBoundary::FinishFace),
        }
    }

    fn for_level(&self, level_id: &str) -> f64 {
        self.by_level.get(level_id).copied().unwrap_or(self.default_ft)
    }
}

/// Build a `geo` polygon from a room's **outer** loop only. Interior loops
/// (the room's own holes — a column or shaft) are dropped here by construction:
/// the footprint is the outline you'd trace around the room, so a room's own
/// void never subtracts from a group footprint. `None` for a loop that can't
/// form a polygon (fewer than three points — e.g. an unplaced room).
fn room_outer_polygon(room: &Room) -> Option<Polygon<f64>> {
    let outer = room.loops.first()?;
    if outer.points.len() < 3 {
        return None;
    }
    Some(Polygon::new(loop_to_linestring(outer), vec![]))
}

fn loop_to_linestring(l: &Loop) -> LineString<f64> {
    LineString::from(l.points.iter().map(|p| Coord { x: p.x, y: p.y }).collect::<Vec<_>>())
}

/// The footprint of one isolated set of rooms at one gap — the single-group
/// case, where the group *is* the level so its own rooms are the only thing
/// bounding the wall zone. Used by the geometry tests and by any caller with no
/// hierarchy context; [`level_groups`] does not go through it, because a real
/// level's wall zone is bounded by every group's rooms at once.
///
/// The result is a `MultiPolygon`: disconnected islands (two wings of one
/// department, too far apart to bridge) are all kept, and genuine courtyards
/// survive as interior rings whose area is excluded.
pub fn group_footprint(rooms: &[Room], gap_ft: f64) -> MultiPolygon<f64> {
    let base = union_of_rooms(rooms.iter());
    let dirs = edge_dirs(&base);
    let zone = wall_zone(&base, gap_ft);
    clean_rings(&base.union(&zone), &dirs, gap_ft)
}

/// The level's wall zone: every gap narrow enough to be a wall, and nothing
/// else. See the module header — this is the object the whole pipeline rests on.
///
/// Three things are load-bearing about the expression:
///
/// - **`closed ∪ all` before the subtraction.** Bevel joins only ever *cut*
///   corners, so `close` is not reliably extensive here and a convex corner can
///   come back chipped. Unioning the original rooms in first means the outer
///   boundary is the rooms' own exact boundary wherever it faces open space —
///   which is why the finished footprints have no chipped corners to repair.
/// - **`− all`.** What remains contains no room. A group's fill is a subset of
///   this, so a group's footprint can never reach inside a neighbour's room —
///   the failing case (a riser narrower than the close radius, swallowed whole)
///   that previously needed a dedicated clip pass against other groups' rooms.
/// - **The gap ceiling is the close's own reach.** A close at radius `gap/2`
///   cannot bridge anything wider than `gap`, so a courtyard, atrium or
///   lightwell is simply never in this set. No width test, no erode-to-empty
///   classifier: "wider than a wall stays open" is arithmetic, not a rule.
///
/// `gap_ft <= 0` (a centreline level) short-circuits to empty: rooms already
/// tile, so there is no gap to classify and no close to run.
///
/// **Deliberately not de-belled here**, though an earlier draft was, and the
/// reason is worth keeping: a wall band is a long thin sliver, so its two long
/// sides are *parallel*, and [`corner_of_chamfer`]'s parallel-flank guard —
/// which exists because near-parallel flanks intersect at near-infinity and
/// produced a million-foot spike — correctly refuses to sharpen a chamfer at the
/// end of one. Measured on House A, sharpening the zone left five invented edge
/// directions where sharpening only the finished footprints leaves two. The
/// chords are cut by this close, but they are only *sharpenable* once the band
/// has been unioned onto its group's rooms and the flanks are room edges meeting
/// at a real corner. So the de-bevel runs on the emitted geometry, once per
/// group per tier — which costs what the old pipeline cost, and is the one part
/// of it that was never the problem.
fn wall_zone(all_rooms: &MultiPolygon<f64>, gap_ft: f64) -> MultiPolygon<f64> {
    if gap_ft <= 0.0 || all_rooms.0.is_empty() {
        return MultiPolygon::new(vec![]);
    }
    let closed = morphological_close(all_rooms, gap_ft / 2.0);
    closed.union(all_rooms).difference(all_rooms)
}

/// Drop the close's invented corner chords, then drop redundant collinear
/// vertices, on every ring of every polygon (exterior and interiors alike).
///
/// **Sharpen first, dedup second — do not reverse this**, and the reason is a
/// measured trap rather than a preference. Sharpening needs each flank of a
/// chamfer to be one whole edge, and the boolean ops that build a footprint
/// plant T-vertices that split flanks in two, so running `dedup_collinear`
/// *first* looks like an obvious improvement: on the synthetic `showcase`
/// fixture it removes all fourteen surviving 1.06 ft (= `gap/2 · √2`) chamfers.
/// On real House A data the same change is destructive — it lets
/// [`sharpen_bevels`] reconstruct a "corner" across genuine geometry, producing
/// a **20.8 ft invented diagonal**, dropping one department to 0.72× its rooms'
/// area, and reintroducing a 2.5 ft² sibling overlap. Cosmetic chamfers on a
/// generated fixture are not worth real area loss on a real model, so the
/// residual stays. If this is revisited, the fix is to make the sharpener robust
/// to split flanks (walk past collinear neighbours to find the true flank), not
/// to pre-simplify the ring underneath it.
fn clean_rings(mp: &MultiPolygon<f64>, dirs: &[f64], scale_ft: f64) -> MultiPolygon<f64> {
    let clean = |ring: &LineString<f64>| dedup_collinear(&sharpen_bevels(ring, dirs, scale_ft));
    MultiPolygon::new(
        mp.iter()
            .map(|p| Polygon::new(clean(p.exterior()), p.interiors().iter().map(clean).collect()))
            .collect(),
    )
}

/// Union a set of rooms' outer polygons. Pairwise into an accumulator — fine for
/// the room counts per group; if profiling later shows it matters, geo's
/// `unary_union` over the whole set is the drop-in replacement (measure first —
/// STRATEGY.md).
fn union_of_rooms<'a>(rooms: impl Iterator<Item = &'a Room>) -> MultiPolygon<f64> {
    let mut acc: MultiPolygon<f64> = MultiPolygon::new(vec![]);
    for room in rooms {
        if let Some(poly) = room_outer_polygon(room) {
            acc = acc.union(&MultiPolygon::new(vec![poly]));
        }
    }
    acc
}

/// Morphological close: dilate by `r`, then erode by `r`.
///
/// This is the single operation that makes the aggregation modelling-agnostic.
/// Dilating grows every polygon outward by `r`; two rooms separated by a gap ≤
/// `2r` (a finish-face wall) merge, and an interior band ≤ `2r` (a wall sliver)
/// closes. Eroding by the same `r` pulls the outer boundary back to where it
/// was, but the merged/closed regions stay filled — the definition of a close.
/// A void wider than `2r` shrinks under the dilate but never closes, so it
/// survives the erode as an interior ring: real courtyards stay open, wall bands
/// do not. Callers pass `r = gap/2`, so the level's wall gap is exactly the
/// widest wall that gets filled.
///
/// **Bevel joins, not miter.** A miter join extends a convex corner to a sharp
/// point that the erode does not always clean up, leaving a spike — visible as a
/// triangular flag where two department footprints meet. Bevel never spikes: it
/// cuts each corner with a short chord. That cut is then repaired by unioning the
/// original geometry back (see [`close_and_clean`]), so the net result is sharp
/// corners with no spikes — which miter could not give.
fn morphological_close(mp: &MultiPolygon<f64>, r: f64) -> MultiPolygon<f64> {
    if r <= 0.0 || mp.0.is_empty() {
        return mp.clone();
    }
    // LineJoin isn't Clone in i_overlay, so build the style fresh for each pass.
    let dilated = mp.buffer_with_style(BufferStyle::new(r).line_join(LineJoin::Bevel));
    dilated.buffer_with_style(BufferStyle::new(-r).line_join(LineJoin::Bevel))
}

/// Angular tolerance (radians, ~1.1°) for calling two edge directions "the same".
/// Absorbs float noise and a real model's slight non-orthogonality, while a 45°
/// chamfer (0.785 rad off any axis) is nowhere near a real direction.
const DIR_TOL_RAD: f64 = 0.02;

/// The set of edge *directions* (line orientations, mod π) present in a geometry.
/// This is the reference for what counts as a real edge: an edge direction the
/// input never had is one the close invented. Deduplicated within `DIR_TOL_RAD`
/// so the list stays tiny (two entries for an axis-aligned building, a few more
/// for one with angled wings).
fn edge_dirs(mp: &MultiPolygon<f64>) -> Vec<f64> {
    let mut dirs: Vec<f64> = Vec::new();
    let mut add = |a: f64| {
        if !dirs.iter().any(|&d| angular_diff(a, d) <= DIR_TOL_RAD) {
            dirs.push(a);
        }
    };
    for poly in mp.iter() {
        for ring in std::iter::once(poly.exterior()).chain(poly.interiors()) {
            let pts = &ring.0;
            for w in pts.windows(2) {
                if let Some(a) = dir_angle(w[0], w[1]) {
                    add(a);
                }
            }
        }
    }
    dirs
}

/// Line orientation of an edge as an angle in `[0, π)` (direction is unsigned —
/// `p→q` and `q→p` are the same line). `None` for a zero-length edge.
fn dir_angle(p: Coord<f64>, q: Coord<f64>) -> Option<f64> {
    let (dx, dy) = (q.x - p.x, q.y - p.y);
    if dx.hypot(dy) < 1e-6 {
        return None;
    }
    Some(dy.atan2(dx).rem_euclid(std::f64::consts::PI))
}

/// Smallest angle between two orientations in `[0, π)`.
fn angular_diff(a: f64, b: f64) -> f64 {
    let d = (a - b).abs() % std::f64::consts::PI;
    d.min(std::f64::consts::PI - d)
}

/// Remove the diagonal chords the close cuts across corners — the "mitre"
/// artifacts — restoring the sharp corner in their place.
///
/// The rule is the user's own, made exact: **an output edge whose direction was
/// not in the input is an artifact.** The close cuts a corner with a chord at a
/// direction (usually 45°) that no wall had. So an edge whose orientation is not
/// in `dirs` (the directions present in the pre-close geometry) is replaced by
/// the intersection of its two neighbours — the corner they define — at any
/// chamfer size. A genuinely angled wall (a rotated pool, a splayed wing) keeps
/// its direction *because that direction is in `dirs`*, so it is never touched.
/// This is why the test references the starting polygon and not a fixed idea of
/// "axis-aligned": a building drawn on the diagonal is handled the same way.
///
/// Iterated to a fixpoint so chained chamfers at a complex junction resolve one
/// corner per pass. Operates on one closed ring.
fn sharpen_bevels(ring: &LineString<f64>, dirs: &[f64], scale_ft: f64) -> LineString<f64> {
    let raw = &ring.0;
    let closed = raw.len() >= 2 && raw.first() == raw.last();
    let mut pts: Vec<Coord<f64>> = if closed { raw[..raw.len() - 1].to_vec() } else { raw.to_vec() };

    let same = |p: &Coord<f64>, q: &Coord<f64>| (p.x - q.x).abs() <= COLLINEAR_EPS_FT && (p.y - q.y).abs() <= COLLINEAR_EPS_FT;
    let is_real_dir = |p: Coord<f64>, q: Coord<f64>| match dir_angle(p, q) {
        Some(a) => dirs.iter().any(|&d| angular_diff(a, d) <= DIR_TOL_RAD),
        None => true, // degenerate edge — nothing to sharpen
    };

    for _ in 0..8 {
        let n = pts.len();
        if n < 4 {
            break;
        }
        let mut out = pts.clone();
        let mut changed = false;
        for i in 0..n {
            let (a, b, c, d) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n], pts[(i + 2) % n]);
            if is_real_dir(b, c) {
                continue; // a direction the input had — keep it
            }
            if let Some(x) = corner_of_chamfer(a, b, c, d, scale_ft) {
                out[i] = x;
                out[(i + 1) % n] = x; // collapse the chord onto the corner it cut
                changed = true;
            }
        }
        // Drop consecutive duplicates (the collapsed pairs and any exact repeats).
        let mut deduped: Vec<Coord<f64>> = Vec::with_capacity(out.len());
        for p in out {
            if deduped.last().is_none_or(|q| !same(q, &p)) {
                deduped.push(p);
            }
        }
        while deduped.len() >= 2 && same(&deduped[0], &deduped[deduped.len() - 1]) {
            deduped.pop();
        }
        pts = deduped;
        if !changed {
            break;
        }
    }

    if pts.len() < 3 {
        return ring.clone();
    }
    pts.push(pts[0]); // re-close
    LineString::from(pts)
}

/// The corner that a chamfer chord `b→c` cut off, given its flanking edges
/// `a→b` and `c→d` — or `None` when this is demonstrably *not* a cut corner.
///
/// The guards are the important part, and their absence caused a real bug: two
/// near-parallel flanking edges intersect at near-infinity, which turned a
/// chamfer into a **million-foot spike** that inflated one department's area
/// 200-fold and overlapped every other footprint. A chamfer we cannot sharpen
/// safely must be left as-is; a runaway vertex is far worse than a visible bevel.
///
/// Three conditions, all necessary:
/// - the flanking edges are not parallel (`denom` non-degenerate);
/// - the intersection lies **forward of `b`** along `a→b` (`t > 1`) and **before
///   `d`** along `c→d` (`u < 1`) — i.e. both flanks genuinely reach it without
///   reversing, which is what "this chord cut a corner" means geometrically. `u`
///   is bounded at `d`, not at `c`: a chamfer's corner often falls *along* the
///   following edge rather than behind its start, and rejecting those left visible
///   chamfers at diagonal-to-orthogonal junctions;
/// - it is **near the chord**: within `max(2·chord, scale_ft)` of both ends, where
///   `scale_ft` is the level's wall gap — the largest distance the close could
///   have moved anything.
///   A real 90° chamfer puts its corner ~0.71·chord away, so this is generous,
///   while a near-parallel pair lands orders of magnitude outside it.
fn corner_of_chamfer(a: Coord<f64>, b: Coord<f64>, c: Coord<f64>, d: Coord<f64>, scale_ft: f64) -> Option<Coord<f64>> {
    let (rx, ry) = (b.x - a.x, b.y - a.y);
    let (sx, sy) = (d.x - c.x, d.y - c.y);
    let denom = rx * sy - ry * sx;
    if denom.abs() < 1e-9 {
        return None; // parallel flanking edges — no corner to restore
    }
    let (qx, qy) = (c.x - a.x, c.y - a.y);
    let t = (qx * sy - qy * sx) / denom;
    let u = (qx * ry - qy * rx) / denom;
    // Must extend a->b past b, and land before d along c->d (so the surviving
    // X->d edge keeps that flank's direction). Small tolerances so a corner
    // sitting essentially on b or d still qualifies.
    if t < 0.999 || u > 0.999 {
        return None;
    }
    let x = Coord { x: a.x + t * rx, y: a.y + t * ry };
    let dist = |p: Coord<f64>, q: Coord<f64>| (p.x - q.x).hypot(p.y - q.y);
    let limit = (2.0 * dist(b, c)).max(scale_ft);
    if dist(x, b) > limit || dist(x, c) > limit {
        return None; // corner is implausibly far — refuse rather than spike
    }
    Some(x)
}

/// Measured area of a footprint — the area of the actual dissolved polygon, holes
/// subtracted (`unsigned_area` on a `MultiPolygon` already nets interior rings
/// out). Never a sum of child areas, which mishandles the enclosed wall bands.
pub fn footprint_area(footprint: &MultiPolygon<f64>) -> f64 {
    footprint.unsigned_area()
}

/// A room paired with its resolved classification path — the input to the tier
/// dissolve. In production this comes straight off a `RoomResponse` (which
/// already carries both `room` and `classification`); kept as a borrowed pair so
/// `areas` doesn't depend on the `rooms` service.
pub struct ClassifiedRoom<'a> {
    pub room: &'a Room,
    pub path: &'a [TierValue],
}

/// One hierarchy group's dissolved footprint at one tier, on one level.
#[derive(Debug)]
pub struct AreaGroup {
    pub level_id: String,
    /// The resolved path prefix identifying this group, outermost first. Its
    /// length is the tier depth: a top-tier group has one element, a bottom-tier
    /// group the full path. The last element is this group's own tier value.
    pub path: Vec<TierValue>,
    pub footprint: MultiPolygon<f64>,
    pub area: f64,
    /// `false` when a Case-A (`group`) exclusion withholds this group from its
    /// parent's dissolve — the group is still reported (its own area is real),
    /// but it does not contribute to any tier above it. Always `true` for a group
    /// no exclusion targets.
    pub counted_upward: bool,
}

/// The Phase-2 pipeline: per-level, per-tier footprints for a set of classified
/// rooms. Rooms are partitioned by level first (footprints never union across
/// floors — a per-level decision), then each level builds its wall zone once and
/// hands every tier its share of it. Every tier's area is measured from that
/// tier's own polygon, never summed from children. Returns bottom-tier groups
/// first, then each tier above.
///
/// `gaps` supplies the wall gap per level, which is where the declared boundary
/// regime enters the geometry — see [`LevelGaps`].
pub fn hierarchy_area_groups(
    rooms: &[ClassifiedRoom],
    exclusions: &[HierarchyExclusion],
    gaps: &LevelGaps,
) -> Vec<AreaGroup> {
    // Case-B exclusion: drop excluded rooms before they become geometry, so they
    // vanish from every tier including their own bottom group — and, since the
    // wall zone is built from the survivors, so do the walls only they bounded.
    // Partition the survivors by level, first-seen order for determinism.
    let mut by_level: Vec<(String, Vec<&ClassifiedRoom>)> = Vec::new();
    for cr in rooms {
        if is_room_excluded(&cr.room.id, exclusions) {
            continue;
        }
        match by_level.iter_mut().find(|(lid, _)| lid == &cr.room.level_id) {
            Some((_, v)) => v.push(cr),
            None => by_level.push((cr.room.level_id.clone(), vec![cr])),
        }
    }

    let mut out = Vec::new();
    for (level_id, level_rooms) in &by_level {
        out.extend(level_groups(level_id, level_rooms, exclusions, gaps.for_level(level_id)));
    }
    out
}

/// Withdraw from **both** claimants any area they each claim, at one tier.
///
/// This is not arbitration papering over an overlap the design did not expect —
/// it is the design's own rule applied to the one shape that still needs it. At a
/// wall junction where four rooms meet, the small square at the centre is bounded
/// by all four; if two belong to A and two to B, each pair's diagonal is short
/// enough to close and both groups fill the square. Bounded by two groups means
/// it belongs to neither, so it comes out of both, and their common ancestor —
/// whose close merges all four rooms — claims it exactly once. That is what keeps
/// `parent = Σ children + newly-enclosed bands` exact rather than approximate.
///
/// Everything else that used to land here is gone: a group's fill is a subset of
/// the wall zone, which contains no rooms, so a footprint can no longer reach
/// into a neighbour's room, and the close no longer bulges past the level's own
/// outer boundary. What remains is junction-sized.
///
/// Intersections are all computed *before* any subtraction, so the outcome does
/// not depend on group order.
fn resolve_sibling_overlaps(groups: &mut [(Vec<TierValue>, MultiPolygon<f64>)]) {
    let n = groups.len();
    if n < 2 {
        return;
    }
    let boxes: Vec<_> = groups.iter().map(|(_, fp)| fp.bounding_rect()).collect();
    let mut contested: Vec<MultiPolygon<f64>> = vec![MultiPolygon::new(vec![]); n];
    for i in 0..n {
        for j in (i + 1)..n {
            // Bounding-box reject first: most sibling pairs are nowhere near each
            // other, and this keeps the pass off the expensive boolean path.
            match (boxes[i], boxes[j]) {
                (Some(a), Some(b)) => {
                    if a.max().x < b.min().x || b.max().x < a.min().x || a.max().y < b.min().y || b.max().y < a.min().y {
                        continue;
                    }
                }
                _ => continue,
            }
            let inter = groups[i].1.intersection(&groups[j].1);
            if inter.0.is_empty() {
                continue;
            }
            contested[i] = contested[i].union(&inter);
            contested[j] = contested[j].union(&inter);
        }
    }
    for (g, c) in groups.iter_mut().zip(contested) {
        if !c.0.is_empty() {
            g.1 = g.1.difference(&c);
        }
    }
}

/// One bottom-tier group on one level: its full classification path, the union
/// of its rooms, and the depth (if any) at which a Case-A exclusion withholds it.
struct BottomGroup {
    path: Vec<TierValue>,
    rooms: MultiPolygon<f64>,
    /// `Some(k)` when an exclusion names the tier at index `k` of this path. The
    /// group at `path[..=k]` is still reported, but contributes to no tier above
    /// it — it participates at depth `d` only for `d >= k`.
    excluded_at: Option<usize>,
}

/// The depth at which a Case-A (`group`) exclusion withholds this path, if any —
/// the shallowest tier whose resolved value an exclusion names. Shallowest
/// because withholding at tier `k` already removes the group from every tier
/// above `k`, so a second, deeper match would change nothing.
fn excluded_depth(path: &[TierValue], exclusions: &[HierarchyExclusion]) -> Option<usize> {
    (0..path.len()).find(|&k| is_group_excluded(&path[..=k], exclusions))
}

/// One level's whole pipeline. Kept in one function on purpose: the wall zone,
/// the per-depth fills and the roll-up are three views of one construction, and
/// the invariant that makes it correct — each depth's footprints are disjoint and
/// nested inside their parent's — is only checkable by reading them together.
fn level_groups(
    level_id: &str,
    rooms: &[&ClassifiedRoom],
    exclusions: &[HierarchyExclusion],
    gap_ft: f64,
) -> Vec<AreaGroup> {
    let num_tiers = rooms.iter().map(|r| r.path.len()).max().unwrap_or(0);
    if num_tiers == 0 {
        return Vec::new(); // no hierarchy configured -> no groups
    }

    // Gather rooms into bottom-tier groups by full-path key (first-seen order).
    // `classify_room` guarantees a uniform-depth path, so `path.len() ==
    // num_tiers` for every room; anything shorter is not a group we can place.
    let mut grouped: Vec<(Vec<TierValue>, Vec<&Room>)> = Vec::new();
    for cr in rooms {
        if cr.path.len() != num_tiers {
            continue;
        }
        match grouped.iter_mut().find(|(p, _)| path_eq(p, cr.path)) {
            Some((_, v)) => v.push(cr.room),
            None => grouped.push((cr.path.to_vec(), vec![cr.room])),
        }
    }

    let bottom: Vec<BottomGroup> = grouped
        .into_iter()
        .map(|(path, rs)| BottomGroup {
            rooms: union_of_rooms(rs.into_iter()),
            excluded_at: excluded_depth(&path, exclusions),
            path,
        })
        .collect();

    // The level's own geometry: every surviving room, and the wall zone those
    // rooms enclose. Both computed exactly once — that is the whole point of the
    // reformulation. `dirs` comes from the rooms, never from the closed result,
    // because a direction the *rooms* never had is by definition one the close
    // invented.
    let mut all_rooms: MultiPolygon<f64> = MultiPolygon::new(vec![]);
    for g in &bottom {
        all_rooms = all_rooms.union(&g.rooms);
    }
    let dirs = edge_dirs(&all_rooms);
    let zone = wall_zone(&all_rooms, gap_ft);

    // Walk the tiers deepest-first so the output keeps its established order
    // (bottom-tier groups first, then each tier above). Each depth is computed
    // from the **bottom groups directly**, never from the tier below it.
    //
    // Not because closing is non-idempotent — it is idempotent, `φ(φ(X)) = φ(X)`
    // (see the module header). The reason is that `geo`'s bevel-join offset is
    // not a true closing: it is not even extensive, since a bevel only ever cuts
    // a corner. So `approx(approx(X))` really does drift where `φ(φ(X))` would
    // not, and re-closing an already-closed footprint compounds the corner error
    // once per tier. Closing the raw rooms once per prefix keeps every tier at
    // exactly one approximation deep.
    let mut results: Vec<AreaGroup> = Vec::new();
    for depth in (0..num_tiers).rev() {
        // Group the participating bottom groups by their prefix at this depth.
        let mut prefixes: Vec<(Vec<TierValue>, MultiPolygon<f64>)> = Vec::new();
        for g in &bottom {
            // A Case-A exclusion at tier k withholds this group from every tier
            // above k. It still appears at k itself — its own area is real.
            if g.excluded_at.is_some_and(|k| depth < k) {
                continue;
            }
            let prefix = &g.path[..=depth];
            match prefixes.iter_mut().find(|(p, _)| path_eq(p, prefix)) {
                Some((_, acc)) => *acc = acc.union(&g.rooms),
                None => prefixes.push((prefix.to_vec(), g.rooms.clone())),
            }
        }

        // Each prefix's share of the wall zone: the bands its own rooms close
        // over. A band with this prefix's rooms on one side only is not among
        // them — that band belongs to an ancestor, which is the ownership rule
        // falling out of the arithmetic rather than being imposed on it.
        let mut footprints: Vec<(Vec<TierValue>, MultiPolygon<f64>)> = prefixes
            .into_iter()
            .map(|(path, own_rooms)| {
                let fill = if gap_ft > 0.0 {
                    morphological_close(&own_rooms, gap_ft / 2.0).intersection(&zone)
                } else {
                    MultiPolygon::new(vec![])
                };
                (path, own_rooms.union(&fill))
            })
            .collect();
        resolve_sibling_overlaps(&mut footprints);

        results.extend(footprints.into_iter().map(|(path, fp)| {
            let cleaned = clean_rings(&fp, &dirs, gap_ft);
            emit(level_id, path, &cleaned, exclusions)
        }));
    }

    results
}

fn emit(level_id: &str, path: Vec<TierValue>, footprint: &MultiPolygon<f64>, exclusions: &[HierarchyExclusion]) -> AreaGroup {
    let counted_upward = !is_group_excluded(&path, exclusions);
    AreaGroup {
        level_id: level_id.to_string(),
        area: footprint_area(footprint),
        footprint: footprint.clone(),
        path,
        counted_upward,
    }
}

/// Case B — is this room id withheld before geometry (stage 1)?
fn is_room_excluded(id: &str, exclusions: &[HierarchyExclusion]) -> bool {
    exclusions.iter().any(|e| match e {
        HierarchyExclusion::Rooms { ids } => ids.iter().any(|x| x == id),
        HierarchyExclusion::Group { .. } => false,
    })
}

/// Case A — is this group (identified by its own, last tier value) withheld from
/// its parent's dissolve (stage 2)? Matches an exclusion whose `tier` names this
/// group's tier and whose `value` equals the resolved code or name.
fn is_group_excluded(path: &[TierValue], exclusions: &[HierarchyExclusion]) -> bool {
    let Some(last) = path.last() else { return false };
    exclusions.iter().any(|e| match e {
        HierarchyExclusion::Group { tier, value } => {
            tier == &last.tier
                && (last.code.as_deref() == Some(value.as_str())
                    || last.name.as_deref() == Some(value.as_str()))
        }
        HierarchyExclusion::Rooms { .. } => false,
    })
}

// ============================ endpoint / wire shape ============================

/// Wire result of `GET /projects/{id}/areas`: per-level, per-tier dissolved
/// footprints for one project's rooms, scoped like `/rooms`. One computation
/// feeds both asks — the plan-view overlay (uses `polygons`) and the summary
/// table (uses `area`/`counted_upward`, ignores `polygons`).
#[derive(Serialize)]
pub struct AreasResult {
    pub schema_version: u32,

    /// The measurement standard this project declares (`[areas]
    /// measurement_standard`), or `null` when it declares none.
    ///
    /// Echoed rather than merely stored because **an area figure without its
    /// definition is precisely what measurement standards exist to prevent**. A
    /// consumer showing these numbers can now say what they mean, and `null` is
    /// an honest "undeclared" rather than a silent implication that some
    /// particular standard applies.
    pub measurement_standard: Option<MeasurementStandard>,

    /// The wall gap actually applied to each level, in feet, keyed by level id —
    /// the resolved product of the project's `max_wall_thickness` and each
    /// level's declared boundary regime. `0` means the level was treated as
    /// centreline and no close ran at all.
    ///
    /// This is the number that decides how much wall each footprint contains, so
    /// it belongs beside the areas rather than being reconstructible only by
    /// cross-referencing settings against `/rooms`.
    pub wall_gap_by_level: BTreeMap<String, f64>,

    /// The scoped level set (same shape `/rooms` returns) so the viewer can draw
    /// each level's footprints on that level's plan.
    pub levels: Vec<Level>,
    pub groups: Vec<AreaGroupResponse>,
}

/// One footprint polygon in wire shape: an exterior ring plus any interior rings
/// (genuine voids the close left open). Each ring is a list of `[x, y]` points
/// with the closing point dropped; the viewer re-closes and renders exterior +
/// holes as one even-odd path so a void reads as open, matching the area.
#[derive(Serialize)]
pub struct FootprintPolygon {
    pub exterior: Vec<[f64; 2]>,
    /// Interior rings (real voids). Omitted when the footprint has none, so the
    /// common wall-band-only case stays a bare exterior on the wire.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Vec<[f64; 2]>>,
}

/// One group's dissolved footprint at one tier on one level, in wire shape.
#[derive(Serialize)]
pub struct AreaGroupResponse {
    pub level_id: String,
    /// Resolved classification prefix, outermost first (the group's identity).
    pub path: Vec<TierValue>,
    /// Measured **aggregated room footprint** area — room area plus enclosed wall
    /// bands, minus genuine voids; NOT net room area (see the module's naming
    /// note). Matches `polygons` (holes are netted out of both).
    pub area: f64,
    /// `false` when a Case-A exclusion withholds this group from tiers above it.
    pub counted_upward: bool,
    /// One entry per island; each carries its exterior ring and any voids.
    pub polygons: Vec<FootprintPolygon>,
}

impl From<AreaGroup> for AreaGroupResponse {
    fn from(g: AreaGroup) -> Self {
        AreaGroupResponse {
            level_id: g.level_id,
            path: g.path,
            area: g.area,
            counted_upward: g.counted_upward,
            polygons: polygons_of(&g.footprint),
        }
    }
}

/// Drop a ring's closing point (the viewer re-closes) and project to `[x, y]`.
fn ring_coords(ring: &LineString<f64>) -> Vec<[f64; 2]> {
    let pts = &ring.0;
    let n = if pts.len() >= 2 && pts.first() == pts.last() { pts.len() - 1 } else { pts.len() };
    pts[..n].iter().map(|c| [c.x, c.y]).collect()
}

fn polygons_of(mp: &MultiPolygon<f64>) -> Vec<FootprintPolygon> {
    mp.iter()
        .map(|poly| FootprintPolygon {
            exterior: ring_coords(poly.exterior()),
            holes: poly.interiors().iter().map(ring_coords).collect(),
        })
        .collect()
}

/// Fraction of a level's room pairs that must tile edge-to-edge before the level
/// reads as centreline. Well below half on purpose: this is a *diagnostic*
/// threshold, and the two regimes are far apart in practice (a centreline model
/// is ~100% coincident, a finish-face one ~0%), so anything near the middle is
/// itself worth reporting rather than a boundary case to tune.
const COINCIDENT_SHARE_FOR_CENTRELINE: f64 = 0.25;

/// Sampling ceiling for the contradiction check. It runs on every `/areas`
/// request, so it must stay O(1) in room count — a few dozen nearest-neighbour
/// probes is plenty to tell "rooms touch" from "rooms float inside their walls",
/// and a wrong regime is a whole-model property, never a per-room one.
const REGIME_SAMPLE_ROOMS: usize = 40;

/// Warn when a level's **declared** regime disagrees with the gaps its rooms
/// actually have.
///
/// "Signal, not error", deliberately. A model declaring finish face whose rooms
/// tile edge-to-edge — or the reverse — is almost certainly a mis-set document
/// option or a stale project fallback, and it will quietly produce footprints
/// that are wrong by roughly the wall area of the whole floor. But the server
/// cannot know which of the two is the mistake, and refusing to answer would
/// take away the very view that makes the problem visible. So: compute, log, and
/// let the number be inspected.
///
/// The measurement is the same one that found House A's 0.317 ft gaps: for a
/// bounded sample of rooms, the distance to the nearest other room on the level.
/// A level reading as centreline is one where most of those distances are zero.
fn warn_on_regime_contradiction(
    project: &str,
    rooms: &[ClassifiedRoom],
    boundary_by_level: &BTreeMap<String, RoomBoundary>,
) {
    let mut by_level: BTreeMap<&str, Vec<Polygon<f64>>> = BTreeMap::new();
    for cr in rooms {
        if let Some(poly) = room_outer_polygon(cr.room) {
            by_level.entry(cr.room.level_id.as_str()).or_default().push(poly);
        }
    }

    for (level_id, polys) in by_level {
        let Some(declared) = boundary_by_level.get(level_id) else { continue };
        let Some(share) = coincident_share(&polys) else { continue };
        let measured = if share >= COINCIDENT_SHARE_FOR_CENTRELINE {
            RoomBoundary::Centreline
        } else {
            RoomBoundary::FinishFace
        };
        if measured != *declared {
            tracing::warn!(
                "project '{project}', level '{level_id}': rooms are declared {declared:?} but measure as \
                 {measured:?} ({:.0}% of sampled neighbouring rooms touch exactly). Footprints will be off by \
                 roughly the floor's wall area — check the model's Area & Volume Computations setting, or the \
                 project's [areas] boundary_location fallback.",
                share * 100.0
            );
        }
    }
}

/// Share of sampled rooms whose nearest neighbour on the level is *touching*
/// (gap within float noise). `None` when there are too few rooms to say
/// anything — a level with one room has no gaps to measure, and guessing from
/// nothing is worse than staying quiet.
fn coincident_share(polys: &[Polygon<f64>]) -> Option<f64> {
    if polys.len() < 2 {
        return None;
    }
    // Stride rather than take the first N: rooms arrive in model order, which on
    // a real export is spatially clustered, so the head of the list can be one
    // wing of one floor.
    let stride = polys.len().div_ceil(REGIME_SAMPLE_ROOMS).max(1);
    let mut sampled = 0usize;
    let mut touching = 0usize;
    for (i, poly) in polys.iter().enumerate().step_by(stride) {
        let nearest = polys
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, other)| Euclidean.distance(poly, other))
            .fold(f64::INFINITY, f64::min);
        if !nearest.is_finite() {
            continue;
        }
        sampled += 1;
        if nearest <= COLLINEAR_EPS_FT {
            touching += 1;
        }
    }
    (sampled > 0).then(|| touching as f64 / sampled as f64)
}

/// Assemble the areas result for one project, scoped like `/rooms`. Reuses
/// `assemble_rooms` for the scoped, classified room set (respecting
/// project/building/milestone exactly as `/rooms` does — a milestone view reuses
/// its pinned snapshots), so grouping runs off the same classification the room
/// render already resolved. Exclusions come from the project's resolved bundle
/// (server-used config, unlike client-only colour plans), as does the `[areas]`
/// policy that sizes the wall zone. `Ok(None)` when nothing has ever been pushed
/// (the handler's 204), mirroring `assemble_rooms`.
pub fn assemble_areas(
    state: &AppState,
    project: &str,
    building: Option<&str>,
    milestone: Option<&str>,
) -> Result<Option<AreasResult>, ServiceError> {
    let scope = RoomScope { project: Some(project), building, milestone, ..Default::default() };
    let Some(rooms) = assemble_rooms(state, &scope)? else {
        return Ok(None);
    };

    let registry = state.settings();
    let bundle = registry.settings_for(project);
    let exclusions = bundle.map(|b| b.hierarchy_exclusions.clone()).unwrap_or_default();
    let policy = bundle.map(|b| b.areas.clone()).unwrap_or_default();

    let classified: Vec<ClassifiedRoom> = rooms
        .rooms
        .iter()
        .map(|r| ClassifiedRoom { room: &r.room, path: &r.classification })
        .collect();

    // Diagnostic only, and deliberately not a rejection: a declared regime that
    // the geometry contradicts is a signal worth surfacing, not grounds for
    // refusing to answer. See `warn_on_regime_contradiction`.
    warn_on_regime_contradiction(project, &classified, &rooms.boundary_by_level);

    let gaps = LevelGaps::from_policy(&policy, &rooms.boundary_by_level);
    let groups = hierarchy_area_groups(&classified, &exclusions, &gaps)
        .into_iter()
        .map(AreaGroupResponse::from)
        .collect();

    Ok(Some(AreasResult {
        schema_version: SUPPORTED_SCHEMA,
        measurement_standard: policy.measurement_standard,
        wall_gap_by_level: rooms
            .boundary_by_level
            .iter()
            .map(|(id, b)| (id.clone(), policy.wall_gap_ft(*b)))
            .collect(),
        levels: rooms.levels,
        groups,
    }))
}

/// Two classification prefixes name the same group when their resolved values
/// match tier-for-tier (`tier` label is positional and always agrees, so only
/// code/name/undefined need comparing).
fn path_eq(a: &[TierValue], b: &[TierValue]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.code == y.code && x.name == y.name && x.undefined == y.undefined)
}

/// Drop vertices that lie on the straight line between their neighbours (and any
/// exact duplicates). Operates on a closed ring and returns a closed ring. Leaves
/// a triangle (three distinct vertices) untouched — nothing there is redundant.
fn dedup_collinear(ring: &LineString<f64>) -> LineString<f64> {
    let pts = &ring.0;
    // A closed ring repeats its first point last; work over the distinct cycle.
    let distinct = if pts.len() >= 2 && pts.first() == pts.last() {
        &pts[..pts.len() - 1]
    } else {
        &pts[..]
    };
    let n = distinct.len();
    if n < 4 {
        return ring.clone();
    }

    let mut kept: Vec<Coord<f64>> = Vec::with_capacity(n);
    for i in 0..n {
        let prev = distinct[(i + n - 1) % n];
        let cur = distinct[i];
        let next = distinct[(i + 1) % n];

        // Perpendicular distance of `cur` from the line prev->next. Zero (within
        // eps) means collinear; a coincident prev/next with cur elsewhere is a
        // spike, also redundant for an area footprint.
        let cross = (cur.x - prev.x) * (next.y - prev.y) - (cur.y - prev.y) * (next.x - prev.x);
        let base = ((next.x - prev.x).powi(2) + (next.y - prev.y).powi(2)).sqrt();
        let dist = if base > 0.0 { cross.abs() / base } else { 0.0 };
        if dist > COLLINEAR_EPS_FT {
            kept.push(cur);
        }
    }

    // Guard: never simplify a ring out of existence.
    if kept.len() < 3 {
        return ring.clone();
    }
    kept.push(kept[0]); // re-close
    LineString::from(kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Point2D;

    /// The finish-face wall gap these geometry tests run at — the project
    /// default. Named locally so a test reads as "at the default gap" rather
    /// than repeating where the number is declared. The centreline regime is
    /// exercised by passing `0.0` explicitly, which is the point of it.
    const WALL_FT: f64 = AreaPolicy::DEFAULT_MAX_WALL_THICKNESS_FT;

    /// Build a room from one or more loops of `(x, y)` corners. First loop is the
    /// outer boundary; any further loops are the room's own holes.
    fn room(id: &str, loops: &[&[(f64, f64)]]) -> Room {
        Room {
            id: id.to_string(),
            name: id.to_string(),
            level_id: "L1".to_string(),
            loops: loops
                .iter()
                .map(|pts| Loop {
                    points: pts.iter().map(|&(x, y)| Point2D { x, y }).collect(),
                })
                .collect(),
            properties: Default::default(),
        }
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(f64, f64)> {
        vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    }

    /// Area comparison tolerant of the morphological close's corner artifact. The
    /// buffer bevels each convex corner by a sub-inch amount (see the module
    /// doc), so an N-corner footprint drifts from the exact figure by a fraction
    /// of a square foot — always far below the semantic differences these tests
    /// assert (a filled vs open courtyard is 100+ ft²). 1 ft² + 1% covers small
    /// rooms and large dissolves alike.
    fn approx_area(a: f64, b: f64) {
        let tol = 1.0 + 0.01 * b.abs();
        assert!((a - b).abs() < tol, "expected ~{b} (±{tol:.2}), got {a}");
    }

    // ---- Phase 2 helpers ----

    fn tv(tier: &str, name: &str) -> TierValue {
        TierValue { tier: tier.to_string(), code: None, name: Some(name.to_string()), undefined: false }
    }

    fn undef(tier: &str) -> TierValue {
        TierValue { tier: tier.to_string(), code: None, name: None, undefined: true }
    }

    /// A room on `level` with the given outer rectangle and classification path.
    fn croom(id: &str, level: &str, r: Vec<(f64, f64)>) -> Room {
        let mut room = room(id, &[&r]);
        room.level_id = level.to_string();
        room
    }

    /// Find the single group whose path matches `want` (by tier names / undefined).
    fn group<'a>(groups: &'a [AreaGroup], level: &str, want: &[TierValue]) -> &'a AreaGroup {
        groups
            .iter()
            .find(|g| g.level_id == level && path_eq(&g.path, want))
            .unwrap_or_else(|| panic!("no group {want:?} on {level}"))
    }

    /// Two departments dissolve into a building: each tier's area is measured
    /// from its own polygon, and the building equals the whole union.
    #[test]
    fn test_two_tier_dissolve_areas() {
        // DeptA = two adjacent rooms (area 200); DeptB = one room (area 100).
        let rooms = vec![
            (croom("a1", "L1", rect(0.0, 0.0, 10.0, 10.0)), vec![tv("Building", "B1"), tv("Dept", "A")]),
            (croom("a2", "L1", rect(10.0, 0.0, 20.0, 10.0)), vec![tv("Building", "B1"), tv("Dept", "A")]),
            (croom("b1", "L1", rect(20.0, 0.0, 30.0, 10.0)), vec![tv("Building", "B1"), tv("Dept", "B")]),
        ];
        let cls: Vec<ClassifiedRoom> = rooms.iter().map(|(r, p)| ClassifiedRoom { room: r, path: p }).collect();
        let g = hierarchy_area_groups(&cls, &[], &LevelGaps::uniform(WALL_FT));

        approx_area(group(&g, "L1", &[tv("Building", "B1"), tv("Dept", "A")]).area, 200.0);
        approx_area(group(&g, "L1", &[tv("Building", "B1"), tv("Dept", "B")]).area, 100.0);
        approx_area(group(&g, "L1", &[tv("Building", "B1")]).area, 300.0);
    }

    /// The correctness case for closing at every tier: two departments, each a
    /// solid shape on its own, together ring a courtyard that belongs to no room.
    /// Under the close rule the courtyard is WIDER than a wall, so it stays open —
    /// the building excludes it, and building area = Σ department areas (additive,
    /// the reverse of the old "parent fills the courtyard" behaviour).
    #[test]
    fn test_parent_keeps_courtyard_open() {
        // 30x30 frame. Dept A = bottom + left + top (a C, open on the right).
        // Dept B = the right bar. Together they ring a 10x10 courtyard.
        let path_a = vec![tv("Building", "B1"), tv("Dept", "A")];
        let path_b = vec![tv("Building", "B1"), tv("Dept", "B")];
        let rooms = vec![
            (croom("bottom", "L1", rect(0.0, 0.0, 30.0, 10.0)), path_a.clone()),
            (croom("left", "L1", rect(0.0, 10.0, 10.0, 20.0)), path_a.clone()),
            (croom("top", "L1", rect(0.0, 20.0, 30.0, 30.0)), path_a.clone()),
            (croom("right", "L1", rect(20.0, 10.0, 30.0, 20.0)), path_b.clone()),
        ];
        let cls: Vec<ClassifiedRoom> = rooms.iter().map(|(r, p)| ClassifiedRoom { room: r, path: p }).collect();
        let g = hierarchy_area_groups(&cls, &[], &LevelGaps::uniform(WALL_FT));

        let a = group(&g, "L1", &path_a).area; // C-shape, open on the right
        let b = group(&g, "L1", &path_b).area; // the bar
        let building_grp = group(&g, "L1", &[tv("Building", "B1")]);
        approx_area(a, 700.0);
        approx_area(b, 100.0);
        approx_area(building_grp.area, 800.0); // courtyard (100) EXCLUDED
        // Additive now: the building is exactly its two departments, no filled void.
        assert!((building_grp.area - (a + b)).abs() < 2.0, "building = Σ children (courtyard not claimed)");
        // The building footprint carries the courtyard as an open interior ring.
        assert_eq!(building_grp.footprint.0.len(), 1, "one island");
        assert_eq!(building_grp.footprint.0[0].interiors().len(), 1, "the courtyard survives as a hole");
    }

    /// **Regression: sibling departments must not overlap.** At a concave
    /// junction the close dilates wider than the partition, so the erode could not
    /// fully retract and each department kept a sliver *inside its neighbour's
    /// room* — measured at 1–5.5 ft² per pair on real data, with both groups
    /// counting the same area. The wall zone makes that unreachable: a fill is a
    /// subset of a set that contains no rooms at all. Asserted as a real polygon
    /// intersection, not a bbox test (concave footprints' bboxes overlap
    /// legitimately).
    #[test]
    fn test_sibling_footprints_do_not_overlap() {
        // Dept A is an L wrapping Dept B's room, all face-of-wall (0.4 ft walls),
        // which puts a concave junction right where the artifact appeared.
        let path_a = vec![tv("Building", "B1"), tv("Dept", "A")];
        let path_b = vec![tv("Building", "B1"), tv("Dept", "B")];
        let rooms = vec![
            (croom("a_bottom", "L1", rect(0.0, 0.0, 30.0, 10.0)), path_a.clone()),
            (croom("a_left", "L1", rect(0.0, 10.4, 10.0, 30.0)), path_a.clone()),
            (croom("b_notch", "L1", rect(10.4, 10.4, 30.0, 30.0)), path_b.clone()),
        ];
        let cls: Vec<ClassifiedRoom> = rooms.iter().map(|(r, p)| ClassifiedRoom { room: r, path: p }).collect();
        let g = hierarchy_area_groups(&cls, &[], &LevelGaps::uniform(WALL_FT));

        let fa = &group(&g, "L1", &path_a).footprint;
        let fb = &group(&g, "L1", &path_b).footprint;
        let overlap = fa.intersection(fb).unsigned_area();
        assert!(overlap < 0.01, "sibling footprints must be disjoint, overlapped {overlap} ft²");

        // And the parent still fills the wall between them: it exceeds the two
        // children (which no longer double-count it) rather than equalling them.
        let building = group(&g, "L1", &[tv("Building", "B1")]).area;
        let sum = group(&g, "L1", &path_a).area + group(&g, "L1", &path_b).area;
        assert!(building > sum + 1.0, "parent fills the inter-dept wall: {building} vs {sum}");
    }

    /// **Additivity is exact, by construction.** A parent equals the sum of its
    /// children plus exactly the bands it is the first tier to enclose — no
    /// double-counted wall, no area lost between tiers.
    ///
    /// This is the property the whole reformulation exists to deliver, and it is
    /// asserted to a hair (0.05 ft²) rather than the loose tolerance the other
    /// geometry tests use, because "roughly additive" is precisely what the
    /// per-group close gave and what was not good enough. The layout is chosen to
    /// make every ownership case appear at once: two departments each with two
    /// rooms, walls within a department (fill at the department), a wall between
    /// them (fill at the building), and a `+` junction in the middle (contested
    /// by both departments, so withdrawn from each and claimed by the building).
    #[test]
    fn test_tier_areas_are_exactly_additive() {
        let w = 0.4; // finish-face wall between every pair of rooms
        let path_a = vec![tv("Building", "B1"), tv("Dept", "A")];
        let path_b = vec![tv("Building", "B1"), tv("Dept", "B")];
        // A 2x2 of 10x10 rooms with a wall between each: A is the left column,
        // B the right, so the vertical wall is inter-department and the two
        // horizontal ones are intra-department. They cross in the middle.
        let x1 = 10.0 + w;
        let y1 = 10.0 + w;
        let rooms = [
            (croom("a_bl", "L1", rect(0.0, 0.0, 10.0, 10.0)), path_a.clone()),
            (croom("a_tl", "L1", rect(0.0, y1, 10.0, y1 + 10.0)), path_a.clone()),
            (croom("b_br", "L1", rect(x1, 0.0, x1 + 10.0, 10.0)), path_b.clone()),
            (croom("b_tr", "L1", rect(x1, y1, x1 + 10.0, y1 + 10.0)), path_b.clone()),
        ];
        let cls: Vec<ClassifiedRoom> = rooms.iter().map(|(r, p)| ClassifiedRoom { room: r, path: p }).collect();
        let g = hierarchy_area_groups(&cls, &[], &LevelGaps::uniform(WALL_FT));

        let a = group(&g, "L1", &path_a);
        let b = group(&g, "L1", &path_b);
        let building = group(&g, "L1", &[tv("Building", "B1")]);

        // Each department is its two rooms plus the horizontal wall between them.
        approx_area(a.area, 200.0 + 10.0 * w);
        approx_area(b.area, 200.0 + 10.0 * w);

        // The newly-enclosed area at the building tier is whatever the building's
        // polygon holds beyond its two children's — measured, not assumed.
        let children = a.footprint.union(&b.footprint);
        let newly_enclosed = building.footprint.difference(&children).unsigned_area();
        assert!(
            (building.area - (a.area + b.area + newly_enclosed)).abs() < 0.05,
            "parent {} != children {} + {} + newly enclosed {}",
            building.area,
            a.area,
            b.area,
            newly_enclosed
        );
        // And that newly-enclosed part is real: the inter-department wall plus
        // the junction square the two departments each had to give up.
        assert!(newly_enclosed > 10.0 * w, "the building claims the wall between the departments");
    }

    /// **Both regimes describe the same building, and they agree on it** — the
    /// same 30×10 plate drawn two ways measures 300 ft² either way. That is the
    /// claim of Decision 1, and this test is what would catch a regression in it.
    ///
    /// The departments *do not* agree, and that is correct rather than a
    /// shortfall worth tolerating: a centreline room's boundary sits in the
    /// middle of its walls, so the room polygon already contains half of every
    /// wall bounding it. A finish-face room contains none, and the wall is then
    /// attributed by the ownership rule — to the department when both sides are
    /// its own, to the building when they are not. So a department loses exactly
    /// half of each wall it shares with another department when the same building
    /// is redrawn to finish face. It reappears one tier up, which is why the
    /// totals still match. Anyone comparing two projects across regimes needs to
    /// know this, so it is asserted rather than left as an approximation.
    ///
    /// It also pins the payoff: at gap 0 no close runs, so the centreline answer
    /// is exact to the bit, not merely close.
    #[test]
    fn test_centreline_and_finish_face_agree_on_the_same_building() {
        let w = 0.4; // the real wall thickness
        let h = w / 2.0;
        let ceiling = 0.5; // the project's declared max_wall_thickness, > w
        let path_a = vec![tv("Building", "B1"), tv("Dept", "A")];
        let path_b = vec![tv("Building", "B1"), tv("Dept", "B")];

        // Centreline: three rooms tiling a 30x10 plate, boundaries on the wall
        // centrelines. Measured at gap 0 — the regime needs no close.
        let centreline = [
            (croom("r1", "L1", rect(0.0, 0.0, 10.0, 10.0)), path_a.clone()),
            (croom("r2", "L1", rect(10.0, 0.0, 20.0, 10.0)), path_a.clone()),
            (croom("r3", "L1", rect(20.0, 0.0, 30.0, 10.0)), path_b.clone()),
        ];
        // Finish face: the same plate, each internal boundary pulled back by half
        // a wall on each side, so a `w`-wide gap sits where the centreline was.
        let finish_face = [
            (croom("r1", "L1", rect(0.0, 0.0, 10.0 - h, 10.0)), path_a.clone()),
            (croom("r2", "L1", rect(10.0 + h, 0.0, 20.0 - h, 10.0)), path_a.clone()),
            (croom("r3", "L1", rect(20.0 + h, 0.0, 30.0, 10.0)), path_b.clone()),
        ];

        let measure = |rooms: &[(Room, Vec<TierValue>)], gap: f64| {
            let cls: Vec<ClassifiedRoom> =
                rooms.iter().map(|(r, p)| ClassifiedRoom { room: r, path: p }).collect();
            let g = hierarchy_area_groups(&cls, &[], &LevelGaps::uniform(gap));
            (
                group(&g, "L1", &path_a).area,
                group(&g, "L1", &path_b).area,
                group(&g, "L1", &[tv("Building", "B1")]).area,
            )
        };

        let (ca, cb, cbuild) = measure(&centreline, 0.0);
        let (fa, fb, fbuild) = measure(&finish_face, ceiling);

        // Centreline is exact: no close ran, so these are plain room unions.
        assert!((ca - 200.0).abs() < 1e-9, "centreline Dept A is exact, got {ca}");
        assert!((cb - 100.0).abs() < 1e-9, "centreline Dept B is exact, got {cb}");
        assert!((cbuild - 300.0).abs() < 1e-9, "centreline building is exact, got {cbuild}");

        // The building agrees between regimes — the whole point.
        approx_area(fbuild, cbuild);

        // Each department loses exactly the half of the A|B wall its centreline
        // room used to contain, and the building picks both halves up.
        let half_shared_wall = h * 10.0;
        approx_area(fa, ca - half_shared_wall);
        approx_area(fb, cb - half_shared_wall);
        // Tolerance 0.3 ft², not the 0.05 the additivity test uses, and the
        // difference is the honest one: that test compares *measured* polygons
        // against each other, where the construction is exact, while this
        // compares against the analytic ideal — which the close cannot quite
        // reach. Where the wall band meets the plate's open outer edge, the
        // bevelled erode cuts a ~r²/2 triangle off each corner of the fill. Four
        // such corners here, ~0.1 ft² total. That is the residual the handover
        // accepts by confining artifacts to the single outer boundary; it does
        // not grow with the number of groups or tiers, which is what mattered.
        assert!(
            (fbuild - (fa + fb + w * 10.0)).abs() < 0.3,
            "the building is its departments plus the whole wall between them: {fbuild} vs {fa} + {fb} + {}",
            w * 10.0
        );
    }

    /// A group's footprint must never swallow a neighbouring group's room, even
    /// when that room sits in a notch narrower than the close would bridge.
    #[test]
    fn test_footprint_does_not_claim_a_neighbours_room() {
        let path_a = vec![tv("Building", "B1"), tv("Dept", "A")];
        let path_b = vec![tv("Building", "B1"), tv("Dept", "B")];
        // B is a narrow 1 ft riser between two A rooms — narrower than WALL_FT,
        // exactly the case where A's close bridges straight across it.
        let rooms = vec![
            (croom("a1", "L1", rect(0.0, 0.0, 10.0, 10.0)), path_a.clone()),
            (croom("riser", "L1", rect(10.0, 0.0, 11.0, 10.0)), path_b.clone()),
            (croom("a2", "L1", rect(11.0, 0.0, 21.0, 10.0)), path_a.clone()),
        ];
        let cls: Vec<ClassifiedRoom> = rooms.iter().map(|(r, p)| ClassifiedRoom { room: r, path: p }).collect();
        let g = hierarchy_area_groups(&cls, &[], &LevelGaps::uniform(WALL_FT));

        let fa = &group(&g, "L1", &path_a).footprint;
        let riser = Polygon::new(
            LineString::from(rect(10.0, 0.0, 11.0, 10.0).iter().map(|&(x, y)| (x, y)).collect::<Vec<_>>()),
            vec![],
        );
        let stolen = fa.intersection(&MultiPolygon::new(vec![riser])).unsigned_area();
        assert!(stolen < 0.01, "A must not claim B's riser, took {stolen} ft²");
        approx_area(group(&g, "L1", &path_a).area, 200.0); // its own two rooms only
    }

    /// The same department on two levels forms two independent bottom groups and
    /// two independent building footprints — the pipeline never unions floors.
    #[test]
    fn test_per_level_groups_are_independent() {
        let path = vec![tv("Building", "B1"), tv("Dept", "A")];
        let rooms = vec![
            (croom("l1", "L1", rect(0.0, 0.0, 10.0, 10.0)), path.clone()),
            (croom("l2", "L2", rect(0.0, 0.0, 10.0, 20.0)), path.clone()),
        ];
        let cls: Vec<ClassifiedRoom> = rooms.iter().map(|(r, p)| ClassifiedRoom { room: r, path: p }).collect();
        let g = hierarchy_area_groups(&cls, &[], &LevelGaps::uniform(WALL_FT));

        approx_area(group(&g, "L1", &path).area, 100.0);
        approx_area(group(&g, "L2", &path).area, 200.0);
        // Building tier exists per level, each equal to its one department.
        approx_area(group(&g, "L1", &[tv("Building", "B1")]).area, 100.0);
        approx_area(group(&g, "L2", &[tv("Building", "B1")]).area, 200.0);
    }

    /// An `undefined` classification is a real group, not a dropped room — it
    /// dissolves and reports area like any other.
    #[test]
    fn test_undefined_bucket_is_a_real_group() {
        let rooms = vec![
            (croom("known", "L1", rect(0.0, 0.0, 10.0, 10.0)), vec![tv("Building", "B1"), tv("Dept", "A")]),
            (croom("unk", "L1", rect(10.0, 0.0, 20.0, 10.0)), vec![tv("Building", "B1"), undef("Dept")]),
        ];
        let cls: Vec<ClassifiedRoom> = rooms.iter().map(|(r, p)| ClassifiedRoom { room: r, path: p }).collect();
        let g = hierarchy_area_groups(&cls, &[], &LevelGaps::uniform(WALL_FT));

        approx_area(group(&g, "L1", &[tv("Building", "B1"), undef("Dept")]).area, 100.0);
        approx_area(group(&g, "L1", &[tv("Building", "B1")]).area, 200.0);
    }

    /// No hierarchy configured (empty paths) yields no groups, not a panic.
    #[test]
    fn test_no_hierarchy_yields_no_groups() {
        let r = croom("r", "L1", rect(0.0, 0.0, 10.0, 10.0));
        let cls = vec![ClassifiedRoom { room: &r, path: &[] }];
        assert!(hierarchy_area_groups(&cls, &[], &LevelGaps::uniform(WALL_FT)).is_empty());
    }

    // ---- Phase 3: exclusions ----

    /// Case A (`group`): the Outdoor department is withheld from the building
    /// dissolve — the building no longer includes it, but Outdoor is still
    /// reported (its own area intact) and flagged `counted_upward = false`.
    #[test]
    fn test_exclude_group_withholds_from_parent_but_keeps_group() {
        let path_in = vec![tv("Building", "B1"), tv("Dept", "Inside")];
        let path_out = vec![tv("Building", "B1"), tv("Dept", "Outdoor")];
        let rooms = vec![
            (croom("i1", "L1", rect(0.0, 0.0, 10.0, 10.0)), path_in.clone()),
            (croom("o1", "L1", rect(10.0, 0.0, 30.0, 10.0)), path_out.clone()),
        ];
        let cls: Vec<ClassifiedRoom> = rooms.iter().map(|(r, p)| ClassifiedRoom { room: r, path: p }).collect();
        let excl = vec![HierarchyExclusion::Group { tier: "Dept".to_string(), value: "Outdoor".to_string() }];
        let g = hierarchy_area_groups(&cls, &excl, &LevelGaps::uniform(WALL_FT));

        // Building excludes Outdoor: 100, not 100 + 200.
        approx_area(group(&g, "L1", &[tv("Building", "B1")]).area, 100.0);
        // Outdoor is still reported with its own real area, flagged not-counted.
        let outdoor = group(&g, "L1", &path_out);
        approx_area(outdoor.area, 200.0);
        assert!(!outdoor.counted_upward, "excluded group is marked not counted upward");
        // The included department is untouched and still counts upward.
        let inside = group(&g, "L1", &path_in);
        approx_area(inside.area, 100.0);
        assert!(inside.counted_upward);
    }

    /// Case B (`rooms`): an excluded room never becomes geometry, so it is gone
    /// from every tier — its own bottom group AND the building — the most
    /// destructive case (unlike Case A, it shrinks the group's own area too).
    #[test]
    fn test_exclude_rooms_drops_from_every_tier() {
        let path_a = vec![tv("Building", "B1"), tv("Dept", "A")];
        let rooms = vec![
            (croom("keep", "L1", rect(0.0, 0.0, 10.0, 10.0)), path_a.clone()),
            (croom("drop", "L1", rect(10.0, 0.0, 20.0, 10.0)), path_a.clone()),
        ];
        let cls: Vec<ClassifiedRoom> = rooms.iter().map(|(r, p)| ClassifiedRoom { room: r, path: p }).collect();
        let excl = vec![HierarchyExclusion::Rooms { ids: vec!["drop".to_string()] }];
        let g = hierarchy_area_groups(&cls, &excl, &LevelGaps::uniform(WALL_FT));

        // Bottom group and building both shrink to just the kept room.
        approx_area(group(&g, "L1", &path_a).area, 100.0);
        approx_area(group(&g, "L1", &[tv("Building", "B1")]).area, 100.0);
    }

    /// An exclusion at a middle tier (Department) withholds that whole subtree
    /// from the top tier while leaving the department and its sub-departments
    /// reported — the withhold propagates upward through the dissolve.
    #[test]
    fn test_exclude_group_at_middle_tier_propagates_up() {
        // 3 tiers: Building / Dept / Sub. Exclude Dept = "Outdoor".
        let inside = |sub: &str| vec![tv("Building", "B1"), tv("Dept", "Inside"), tv("Sub", sub)];
        let outdoor = |sub: &str| vec![tv("Building", "B1"), tv("Dept", "Outdoor"), tv("Sub", sub)];
        let rooms = vec![
            (croom("i", "L1", rect(0.0, 0.0, 10.0, 10.0)), inside("S1")),
            (croom("o", "L1", rect(10.0, 0.0, 30.0, 10.0)), outdoor("S2")),
        ];
        let cls: Vec<ClassifiedRoom> = rooms.iter().map(|(r, p)| ClassifiedRoom { room: r, path: p }).collect();
        let excl = vec![HierarchyExclusion::Group { tier: "Dept".to_string(), value: "Outdoor".to_string() }];
        let g = hierarchy_area_groups(&cls, &excl, &LevelGaps::uniform(WALL_FT));

        // Building = Inside subtree only.
        approx_area(group(&g, "L1", &[tv("Building", "B1")]).area, 100.0);
        // Outdoor department and its sub-department are still reported.
        approx_area(group(&g, "L1", &[tv("Building", "B1"), tv("Dept", "Outdoor")]).area, 200.0);
        approx_area(group(&g, "L1", &outdoor("S2")).area, 200.0);
        assert!(!group(&g, "L1", &[tv("Building", "B1"), tv("Dept", "Outdoor")]).counted_upward);
    }

    /// **Every emitted ring is simple** — no edge of a ring crosses another edge
    /// of the same ring.
    ///
    /// One of the two invariants the diagnostic harness checks against a live
    /// server (`scripts/check_areas.py`), kept here as well because it is cheap,
    /// and because a self-intersecting ring is the shape a spike makes on its way
    /// out: `unsigned_area` on a bow-tie quietly reports the difference of its two
    /// lobes, so a broken ring can look like a plausible number right up until
    /// someone renders it. Checked by brute-force segment crossing rather than
    /// with a validity predicate, so it tests the geometry and not `geo`'s
    /// opinion of it.
    #[test]
    fn test_emitted_rings_are_simple() {
        // A layout with every artifact-prone feature at once: finish-face walls,
        // a concave L, a narrow riser belonging to a third group, a diagonal
        // wing, and a courtyard.
        let w = 0.4;
        let b = |d: &str| vec![tv("Building", "B1"), tv("Dept", d)];
        let rooms = [
            (croom("a1", "L1", rect(0.0, 0.0, 20.0, 10.0)), b("A")),
            (croom("a2", "L1", rect(0.0, 10.0 + w, 10.0, 30.0)), b("A")),
            (croom("riser", "L1", rect(10.0 + w, 10.0 + w, 11.4 + w, 30.0)), b("C")),
            (croom("b1", "L1", rect(11.8 + w, 10.0 + w, 20.0, 30.0)), b("B")),
            // A diagonal wing: a real non-axis direction the de-bevel must keep.
            (
                croom("wing", "L1", vec![(20.0 + w, 0.0), (34.0 + w, 0.0), (28.0 + w, 12.0), (20.0 + w, 12.0)]),
                b("B"),
            ),
        ];
        let cls: Vec<ClassifiedRoom> = rooms.iter().map(|(r, p)| ClassifiedRoom { room: r, path: p }).collect();

        // Both regimes, because they exercise different code: one runs the close,
        // the other must not.
        for gap in [0.0, 0.5, WALL_FT] {
            let groups = hierarchy_area_groups(&cls, &[], &LevelGaps::uniform(gap));
            assert!(!groups.is_empty(), "gap {gap}: something must be emitted");
            for g in &groups {
                for poly in g.footprint.iter() {
                    for ring in std::iter::once(poly.exterior()).chain(poly.interiors()) {
                        assert!(
                            ring_is_simple(ring),
                            "gap {gap}, group {:?}: ring self-intersects",
                            g.path.last().and_then(|t| t.name.clone())
                        );
                    }
                    assert!(
                        poly.exterior().0.len() >= 4,
                        "gap {gap}: a ring collapsed to fewer than three distinct points"
                    );
                }
            }
        }
    }

    /// Brute-force "does any pair of non-adjacent edges of this closed ring
    /// cross". O(n²) and fine at test sizes.
    fn ring_is_simple(ring: &LineString<f64>) -> bool {
        let p = &ring.0;
        if p.len() < 4 {
            return true;
        }
        let n = p.len() - 1; // edge count on a closed ring
        let cross = |o: Coord<f64>, a: Coord<f64>, b: Coord<f64>| {
            (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
        };
        // Proper crossing only: rings legitimately touch at shared endpoints, and
        // a collinear overlap is what `dedup_collinear` is for, not a crossing.
        let crosses = |i: usize, j: usize| {
            let (a, b, c, d) = (p[i], p[i + 1], p[j], p[j + 1]);
            let (d1, d2) = (cross(a, b, c), cross(a, b, d));
            let (d3, d4) = (cross(c, d, a), cross(c, d, b));
            (d1 > 0.0) != (d2 > 0.0) && (d1 != 0.0 && d2 != 0.0)
                && (d3 > 0.0) != (d4 > 0.0) && (d3 != 0.0 && d4 != 0.0)
        };
        for i in 0..n {
            // Skip the previous, same and next edge — those share an endpoint.
            for j in (i + 2)..n {
                if i == 0 && j == n - 1 {
                    continue; // first and last edge share the closing vertex
                }
                if crosses(i, j) {
                    return false;
                }
            }
        }
        true
    }

    /// Distinct exterior vertices of a single-polygon footprint (closing point
    /// dropped) as rounded coords, for corner-count assertions.
    fn corner_count(poly: &Polygon<f64>) -> usize {
        let pts = &poly.exterior().0;
        pts[..pts.len().saturating_sub(1)].len()
    }

    /// A single plain room → one solid island, no holes, its own area, and —
    /// because the close's corner bevels are repaired by the union-back — **exactly
    /// four corners**, not the chipped octagon the bare close produces. This is
    /// the regression guard for the corner-artifact fix.
    #[test]
    fn test_single_room_outer_ring() {
        let fp = group_footprint(&[room("r", &[&rect(0.0, 0.0, 10.0, 8.0)])], WALL_FT);
        assert_eq!(fp.0.len(), 1, "one room -> one polygon");
        assert!(fp.0[0].interiors().is_empty(), "no holes");
        assert_eq!(corner_count(&fp.0[0]), 4, "corners restored sharp, no bevel chips");
        approx_area(footprint_area(&fp), 80.0);
    }

    /// Two edge-to-edge rooms dissolve to a clean rectangle — four corners, no
    /// bevel chips at the two outer corners of the merged block. The union-back
    /// repairs what the close would otherwise chip.
    #[test]
    fn test_dissolved_block_has_sharp_corners() {
        let fp = group_footprint(&[
            room("a", &[&rect(0.0, 0.0, 10.0, 10.0)]),
            room("b", &[&rect(10.0, 0.0, 20.0, 10.0)]),
        ], WALL_FT);
        assert_eq!(fp.0.len(), 1);
        assert_eq!(corner_count(&fp.0[0]), 4, "merged 20x10 block has exactly four corners");
        approx_area(footprint_area(&fp), 200.0);
    }

    /// A room with its own interior hole (a column): the hole is dropped at the
    /// source ([`room_outer_polygon`]), so the footprint is the full outer square.
    /// This is independent of the close's void handling — a room's OWN void is
    /// never a courtyard, it is filled by construction whatever its size.
    #[test]
    fn test_room_hole_is_ignored() {
        let outer = rect(0.0, 0.0, 10.0, 10.0);
        let hole = rect(3.0, 3.0, 7.0, 7.0);
        let fp = group_footprint(&[room("r", &[&outer, &hole])], WALL_FT);
        assert_eq!(fp.0.len(), 1);
        assert!(fp.0[0].interiors().is_empty());
        approx_area(footprint_area(&fp), 100.0); // hole filled, not 100 - 16
    }

    /// Two rooms far apart (gap ≫ `WALL_FT`) → two islands survive. The close
    /// bridges walls, never a corridor-width gap.
    #[test]
    fn test_two_disjoint_clusters_keep_two_islands() {
        let fp = group_footprint(&[
            room("a", &[&rect(0.0, 0.0, 10.0, 10.0)]),
            room("b", &[&rect(20.0, 0.0, 30.0, 10.0)]), // 10 ft gap
        ], WALL_FT);
        assert_eq!(fp.0.len(), 2, "a 10 ft gap is not a wall — islands stay separate");
        approx_area(footprint_area(&fp), 200.0);
    }

    /// Two rooms sharing an edge (centreline) dissolve to one solid island.
    #[test]
    fn test_adjacent_rooms_dissolve_no_sliver() {
        let fp = group_footprint(&[
            room("a", &[&rect(0.0, 0.0, 10.0, 10.0)]),
            room("b", &[&rect(10.0, 0.0, 20.0, 10.0)]),
        ], WALL_FT);
        assert_eq!(fp.0.len(), 1, "adjacent rooms merge into one polygon");
        assert!(fp.0[0].interiors().is_empty(), "no enclosed sliver");
        approx_area(footprint_area(&fp), 200.0);
    }

    /// **The face-of-wall case the old code could not do.** Two rooms separated by
    /// a 0.5 ft wall (drawn to finish face, so they do NOT share an edge) still
    /// dissolve into one island, and the wall band between them is filled — area
    /// is the two rooms PLUS the wall, not two disjoint islands.
    #[test]
    fn test_face_of_wall_rooms_dissolve_and_fill_wall() {
        let fp = group_footprint(&[
            room("a", &[&rect(0.0, 0.0, 10.0, 10.0)]),
            room("b", &[&rect(10.5, 0.0, 20.5, 10.0)]), // 0.5 ft wall gap
        ], WALL_FT);
        assert_eq!(fp.0.len(), 1, "a wall-width gap bridges into one footprint");
        assert!(fp.0[0].interiors().is_empty(), "the wall band is filled, not a hole");
        approx_area(footprint_area(&fp), 205.0); // 100 + 100 + 0.5*10 wall
    }

    /// Three rooms (STORAGE + HALL on top, STAIR full-width below) dissolve to one
    /// solid island of the right area.
    #[test]
    fn test_three_rooms_dissolve_to_one_island() {
        let storage = rect(0.0, 10.0, 15.0, 20.0); // top-left
        let hall = rect(15.0, 10.0, 24.0, 20.0); // top-right
        let stair = rect(0.0, 0.0, 24.0, 10.0); // full-width bottom
        let fp = group_footprint(&[room("storage", &[&storage]), room("hall", &[&hall]), room("stair", &[&stair])], WALL_FT);

        assert_eq!(fp.0.len(), 1, "three connected rooms -> one polygon");
        assert!(fp.0[0].interiors().is_empty());
        approx_area(footprint_area(&fp), 24.0 * 20.0);
    }

    /// `dedup_collinear` directly: a square with a redundant midpoint on one
    /// edge (and an exact duplicate corner) collapses back to four vertices,
    /// while a genuine corner is never dropped. Locked independently so the
    /// four-corner guarantee doesn't silently depend on the union backend.
    #[test]
    fn test_dedup_collinear_removes_only_redundant_points() {
        // Closed ring: corners of a 10x10 square + a collinear midpoint (10,5)
        // on the right edge + a duplicate of (0,0) at the end before closing.
        let ring = LineString::from(vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 5.0), // collinear on the right edge
            (10.0, 10.0),
            (0.0, 10.0),
            (0.0, 0.0), // closing point
        ]);
        let out = dedup_collinear(&ring);
        let distinct = &out.0[..out.0.len() - 1];
        assert_eq!(distinct.len(), 4, "the collinear midpoint is dropped, real corners kept");
        assert!(out.0.first() == out.0.last(), "ring stays closed");
    }

    /// `sharpen_bevels` directly, keyed to the input's edge directions.
    #[test]
    fn test_sharpen_bevels_removes_only_invented_directions() {
        use std::f64::consts::PI;
        let axis = [0.0, PI / 2.0]; // an axis-aligned input: only H and V directions

        // A LARGE 45° chamfer (4 ft chord) cut across the top-right 90° corner —
        // exactly the artifact, and far bigger than any length cap would catch.
        let beveled = LineString::from(vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 6.0), // up the right edge, stops 4 ft short
            (6.0, 10.0), // 45° chamfer across the corner
            (0.0, 10.0),
            (0.0, 0.0),
        ]);
        let out = sharpen_bevels(&beveled, &axis, WALL_FT);
        let d = &out.0[..out.0.len() - 1];
        assert_eq!(d.len(), 4, "the 45° chord collapses to one sharp corner, any size");
        assert!(d.iter().any(|c| (c.x - 10.0).abs() < 1e-6 && (c.y - 10.0).abs() < 1e-6), "corner restored at (10,10)");

        // The SAME diagonal chord, but now the input genuinely had that direction
        // (a splayed / rotated building): it must be kept, not sharpened.
        let diag_dir = dir_angle(Coord { x: 10.0, y: 6.0 }, Coord { x: 6.0, y: 10.0 }).unwrap();
        let dirs_with_diag = [0.0, PI / 2.0, diag_dir];
        let out = sharpen_bevels(&beveled, &dirs_with_diag, WALL_FT);
        assert_eq!(out.0[..out.0.len() - 1].len(), 5, "a real diagonal wall (direction in the input) is preserved");
    }

    /// **Regression: the million-foot spike.** A short odd-angled edge whose two
    /// flanking edges are near-PARALLEL has its "corner" at near-infinity. Without
    /// the guards in `corner_of_chamfer` this produced a vertex over 1,000,000 ft
    /// away on House A's Outdoor department, inflating its area 200-fold and
    /// overlapping every other footprint. The de-bevel must decline instead: a
    /// visible chamfer beats a runaway vertex.
    #[test]
    fn test_sharpen_bevels_never_spikes_on_near_parallel_flanks() {
        use std::f64::consts::PI;
        let axis = [0.0, PI / 2.0];
        // Flanks (0,0)->(20,0) and (20.05,10)->(0,10.02): near-parallel, so their
        // intersection is thousands of feet away. The joining edge is diagonal
        // (an "invented" direction), which is what invites the sharpen.
        let ring = LineString::from(vec![
            (0.0, 0.0),
            (20.0, 0.0),      // b : end of flank 1
            (20.05, 10.0),    // c : start of flank 2 (near-parallel to flank 1)
            (0.0, 10.02),
            (0.0, 0.0),
        ]);
        let out = sharpen_bevels(&ring, &axis, WALL_FT);
        let far = out.0.iter().any(|p| p.x.abs() > 1_000.0 || p.y.abs() > 1_000.0);
        assert!(!far, "must not invent a distant vertex: {:?}", out.0);
        // Area must stay in the same ballpark as the input (~200 ft²), not explode.
        let area = Polygon::new(out.clone(), vec![]).unsigned_area();
        assert!(area < 400.0, "area must not blow up, got {area}");
    }

    /// End-to-end guard on the same failure mode: every footprint vertex stays
    /// within a wall's reach of the rooms' own bounding box, at every tier. This
    /// is the invariant the spike violated by a factor of ~12,000.
    #[test]
    fn test_footprints_stay_within_room_bounds() {
        // A group mixing axis-aligned rooms with a genuinely angled one (the House
        // A shape that triggered the bug: an angled deck beside square rooms).
        let angled = vec![(30.0, 0.0), (52.0, 13.0), (48.0, 20.0), (26.0, 7.0)];
        let rooms = vec![
            room("sq1", &[&rect(0.0, 0.0, 10.0, 10.0)]),
            room("sq2", &[&rect(10.5, 0.0, 20.0, 10.0)]),
            room("deck", &[&angled]),
        ];
        let fp = group_footprint(&rooms, WALL_FT);

        // Input bounds.
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for r in &rooms {
            for p in &r.loops[0].points {
                x0 = x0.min(p.x); y0 = y0.min(p.y); x1 = x1.max(p.x); y1 = y1.max(p.y);
            }
        }
        let slack = WALL_FT;
        for poly in fp.iter() {
            for ring in std::iter::once(poly.exterior()).chain(poly.interiors()) {
                for c in &ring.0 {
                    assert!(
                        c.x >= x0 - slack && c.x <= x1 + slack && c.y >= y0 - slack && c.y <= y1 + slack,
                        "vertex {c:?} escaped the room bounds ({x0},{y0})..({x1},{y1})"
                    );
                }
            }
        }
    }

    /// Two rooms meant to abut, but whose shared edge coordinates disagree by
    /// float noise (Revit exports aren't bit-identical across rooms). They must
    /// still dissolve to one clean ring — no sliver polygon, no spurious second
    /// island. Confirms the union backend's precision covers noise-level
    /// mismatch, so no explicit vertex-snap pre-pass is needed at this scale.
    #[test]
    fn test_noise_level_gap_still_dissolves() {
        let a = rect(0.0, 0.0, 10.0, 10.0);
        // b's left edge is a hair off from a's right edge (x = 10 ± 1e-9).
        let b = vec![(10.0 + 1e-9, 0.0), (20.0, 0.0), (20.0, 10.0), (10.0 - 1e-9, 10.0)];
        let fp = group_footprint(&[room("a", &[&a]), room("b", &[&b])], WALL_FT);
        assert_eq!(fp.0.len(), 1, "noise-level mismatch must not split into two islands");
        approx_area(footprint_area(&fp), 200.0);
    }

    /// A ring of four rooms encloses a 10 ft courtyard. The courtyard is far wider
    /// than a wall, so the close leaves it OPEN as an interior ring and its area
    /// is excluded — the reverse of the old "fill the courtyard" behaviour.
    #[test]
    fn test_ring_of_rooms_keeps_courtyard_open() {
        // A 30x30 outer square as a 10-wide frame of four rooms around a 10x10 void.
        let fp = group_footprint(&[
            room("bottom", &[&rect(0.0, 0.0, 30.0, 10.0)]),
            room("top", &[&rect(0.0, 20.0, 30.0, 30.0)]),
            room("left", &[&rect(0.0, 10.0, 10.0, 20.0)]),
            room("right", &[&rect(20.0, 10.0, 30.0, 20.0)]),
        ], WALL_FT);
        assert_eq!(fp.0.len(), 1, "the frame dissolves to one outer ring");
        assert_eq!(fp.0[0].interiors().len(), 1, "the 10 ft courtyard survives as an open hole");
        approx_area(footprint_area(&fp), 800.0); // 30*30 minus the 10x10 courtyard
    }

    /// Narrow band fills, wide void stays open, in the SAME footprint. A face-of-
    /// wall frame (0.5 ft wall gaps at its corners) rings a 10 ft courtyard: the
    /// corner walls close (area exceeds the bare sum of the four bars) while the
    /// courtyard does not (area stays well under the filled-solid figure).
    #[test]
    fn test_narrow_wall_fills_wide_void_stays_open() {
        // Four bars around a 10x10 courtyard, each bar 0.5 ft short of its
        // neighbours so the four corners are wall gaps, not shared edges.
        let fp = group_footprint(&[
            room("bottom", &[&rect(0.0, 0.0, 30.0, 10.0)]),
            room("top", &[&rect(0.0, 20.0, 30.0, 30.0)]),
            room("left", &[&rect(0.0, 10.5, 10.0, 19.5)]), // 0.5 gap top & bottom
            room("right", &[&rect(20.0, 10.5, 30.0, 19.5)]),
        ], WALL_FT);
        assert_eq!(fp.0.len(), 1, "corner wall gaps bridge into one frame");
        assert_eq!(fp.0[0].interiors().len(), 1, "the courtyard stays open");
        let bare_bars = 300.0 + 300.0 + 90.0 + 90.0; // 780, no corner walls
        let filled_solid = 900.0; // if the courtyard were wrongly filled
        let area = footprint_area(&fp);
        assert!(area > bare_bars + 5.0, "corner walls are filled: {area} > {bare_bars}");
        assert!(area < filled_solid - 50.0, "courtyard is NOT filled: {area} < {filled_solid}");
    }

    // ---- Phase 4: the endpoint (assemble_areas end-to-end over AppState) ----

    mod endpoint {
        use super::*;
        use crate::contract::{CustomValue, Level, Model, Project, RoomPayload, Snapshot};
        use crate::settings::HierarchyTier;
        use crate::state::{AppState, ProjectSettings};
        use crate::storage::MemStore;
        use std::collections::{BTreeMap, HashMap};

        /// A 2-tier bundle (Building/Dept, each keyed on a name property), with
        /// the given footprint exclusions.
        fn bundle(exclusions: Vec<HierarchyExclusion>) -> ProjectSettings {
            ProjectSettings {
                drofus: None,
                hierarchy: vec![
                    HierarchyTier { name: "Building".to_string(), code_property: None, name_property: Some("bldg".to_string()) },
                    HierarchyTier { name: "Dept".to_string(), code_property: None, name_property: Some("dept".to_string()) },
                ],
                builtin_properties: vec![],
                room_label: vec!["$name".to_string()],
                drofus_fields: vec![],
                milestones: vec![],
                comparison_key: None,
                comparison_properties: vec![],
                areas: Default::default(),
                hierarchy_exclusions: exclusions,
            }
        }

        /// A room with an outer rectangle and `bldg`/`dept` classification props.
        fn geo_room(id: &str, bldg: &str, dept: &str, r: Vec<(f64, f64)>) -> Room {
            let mut properties = BTreeMap::new();
            properties.insert("bldg".to_string(), CustomValue { value: bldg.to_string(), storage_type: None });
            properties.insert("dept".to_string(), CustomValue { value: dept.to_string(), storage_type: None });
            Room {
                id: id.to_string(),
                name: id.to_string(),
                level_id: "L1".to_string(),
                loops: vec![Loop { points: r.iter().map(|&(x, y)| Point2D { x, y }).collect() }],
                properties,
            }
        }

        fn state_with(rooms: Vec<Room>, exclusions: Vec<HierarchyExclusion>) -> AppState {
            let registry = HashMap::from([("p1".to_string(), bundle(exclusions))]);
            let state = AppState::new(Box::new(MemStore::new()), registry, None);
            let payload = RoomPayload {
                schema_version: 5,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
                model_to_shared: None,
                room_boundary: None,
                levels: vec![Level { id: "L1".to_string(), name: "Level 1".to_string(), elevation: 0.0 }],
                rooms,
            };
            state.set_snapshot(payload).unwrap();
            state
        }

        fn find<'a>(r: &'a AreasResult, dept: Option<&str>) -> &'a AreaGroupResponse {
            r.groups
                .iter()
                .find(|g| match dept {
                    Some(d) => g.path.len() == 2 && g.path[1].name.as_deref() == Some(d),
                    None => g.path.len() == 1, // the Building group
                })
                .expect("group present")
        }

        /// End-to-end: the endpoint scopes + classifies via assemble_rooms, groups
        /// per tier, and returns wire shape with levels, areas, and footprint
        /// polygons.
        #[test]
        fn test_assemble_areas_happy_path() {
            let rooms = vec![
                geo_room("in", "B1", "Inside", vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]),
                geo_room("out", "B1", "Outdoor", vec![(10.0, 0.0), (30.0, 0.0), (30.0, 10.0), (10.0, 10.0)]),
            ];
            let state = state_with(rooms, vec![]);
            let r = assemble_areas(&state, "p1", None, None).unwrap().expect("store has data");

            assert_eq!(r.schema_version, SUPPORTED_SCHEMA);
            assert_eq!(r.levels.len(), 1);
            approx_area(find(&r, Some("Inside")).area, 100.0);
            approx_area(find(&r, Some("Outdoor")).area, 200.0);
            let building = find(&r, None);
            approx_area(building.area, 300.0);
            assert!(building.counted_upward);
            // One solid island, no voids (edge-to-edge rooms, no courtyard).
            assert_eq!(building.polygons.len(), 1);
            assert!(building.polygons[0].holes.is_empty(), "no voids in a solid block");
            assert!(building.polygons[0].exterior.len() >= 4, "at least the four outer corners");
        }

        /// A Case-A exclusion loaded from the project bundle takes effect through
        /// the endpoint: the building excludes Outdoor, which is still reported.
        #[test]
        fn test_assemble_areas_applies_bundle_exclusion() {
            let rooms = vec![
                geo_room("in", "B1", "Inside", vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]),
                geo_room("out", "B1", "Outdoor", vec![(10.0, 0.0), (30.0, 0.0), (30.0, 10.0), (10.0, 10.0)]),
            ];
            let excl = vec![HierarchyExclusion::Group { tier: "Dept".to_string(), value: "Outdoor".to_string() }];
            let state = state_with(rooms, excl);
            let r = assemble_areas(&state, "p1", None, None).unwrap().expect("store has data");

            approx_area(find(&r, None).area, 100.0); // building excludes Outdoor
            let outdoor = find(&r, Some("Outdoor"));
            approx_area(outdoor.area, 200.0); // still reported
            assert!(!outdoor.counted_upward);
        }

        /// Nothing pushed -> None (the handler's 204), mirroring assemble_rooms.
        #[test]
        fn test_assemble_areas_empty_store_is_none() {
            let registry = HashMap::from([("p1".to_string(), bundle(vec![]))]);
            let state = AppState::new(Box::new(MemStore::new()), registry, None);
            assert!(assemble_areas(&state, "p1", None, None).unwrap().is_none());
        }
    }
}
