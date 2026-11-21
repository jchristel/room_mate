//! The geometry of an area: a group of rooms unioned into one footprint, closed across the wall gap and cleaned (collinear points, bevels
//! sharpened to the wall directions) so the outline is a floor plan and not a sliver. Pure geometry over `geo`; no policy.

use super::*;

/// Perpendicular distance (in model units — feet) below which a vertex is
/// treated as lying *on* the line through its neighbours and dropped. Only
/// true-collinear points and float noise fall under this; a genuine corner sits
/// orders of magnitude above it. Tight on purpose: it removes redundant
/// vertices, never real geometry.
pub(super) const COLLINEAR_EPS_FT: f64 = 1e-6;

/// Build a `geo` polygon from a room's **outer** loop only. Interior loops
/// (the room's own holes — a column or shaft) are dropped here by construction:
/// the footprint is the outline you'd trace around the room, so a room's own
/// void never subtracts from a group footprint. `None` for a loop that can't
/// form a polygon (fewer than three points — e.g. an unplaced room).
pub(super) fn room_outer_polygon(room: &Room) -> Option<Polygon<f64>> {
    let outer = room.loops.first()?;
    if outer.points.len() < 3 {
        return None;
    }
    Some(Polygon::new(loop_to_linestring(outer), vec![]))
}

pub(super) fn loop_to_linestring(l: &Loop) -> LineString<f64> {
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
pub(super) fn wall_zone(all_rooms: &MultiPolygon<f64>, gap_ft: f64) -> MultiPolygon<f64> {
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
pub(super) fn clean_rings(mp: &MultiPolygon<f64>, dirs: &[f64], scale_ft: f64) -> MultiPolygon<f64> {
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
pub(super) fn union_of_rooms<'a>(rooms: impl Iterator<Item = &'a Room>) -> MultiPolygon<f64> {
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
/// cuts each corner with a short chord. That cut is then repaired downstream —
/// [`wall_zone`] unions the original rooms back before subtracting them, and
/// [`clean_rings`] sharpens whatever chords survive onto the emitted footprints —
/// so the net result is sharp corners with no spikes, which miter could not give.
/// (This used to name a `close_and_clean` helper; the wall-zone reformulation
/// split that work across those two, and the repair is no longer one function.)
pub(super) fn morphological_close(mp: &MultiPolygon<f64>, r: f64) -> MultiPolygon<f64> {
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
pub(super) const DIR_TOL_RAD: f64 = 0.02;

/// The set of edge *directions* (line orientations, mod π) present in a geometry.
/// This is the reference for what counts as a real edge: an edge direction the
/// input never had is one the close invented. Deduplicated within `DIR_TOL_RAD`
/// so the list stays tiny (two entries for an axis-aligned building, a few more
/// for one with angled wings).
pub(super) fn edge_dirs(mp: &MultiPolygon<f64>) -> Vec<f64> {
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
pub(super) fn dir_angle(p: Coord<f64>, q: Coord<f64>) -> Option<f64> {
    let (dx, dy) = (q.x - p.x, q.y - p.y);
    if dx.hypot(dy) < 1e-6 {
        return None;
    }
    Some(dy.atan2(dx).rem_euclid(std::f64::consts::PI))
}

/// Smallest angle between two orientations in `[0, π)`.
pub(super) fn angular_diff(a: f64, b: f64) -> f64 {
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
pub(super) fn sharpen_bevels(ring: &LineString<f64>, dirs: &[f64], scale_ft: f64) -> LineString<f64> {
    let raw = &ring.0;
    let closed = raw.len() >= 2 && raw.first() == raw.last();
    let mut pts: Vec<Coord<f64>> = if closed { raw[..raw.len() - 1].to_vec() } else { raw.to_vec() };

    let same =
        |p: &Coord<f64>, q: &Coord<f64>| (p.x - q.x).abs() <= COLLINEAR_EPS_FT && (p.y - q.y).abs() <= COLLINEAR_EPS_FT;
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
pub(super) fn corner_of_chamfer(
    a: Coord<f64>,
    b: Coord<f64>,
    c: Coord<f64>,
    d: Coord<f64>,
    scale_ft: f64,
) -> Option<Coord<f64>> {
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

/// Drop vertices that lie on the straight line between their neighbours (and any
/// exact duplicates). Operates on a closed ring and returns a closed ring. Leaves
/// a triangle (three distinct vertices) untouched — nothing there is redundant.
pub(super) fn dedup_collinear(ring: &LineString<f64>) -> LineString<f64> {
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
