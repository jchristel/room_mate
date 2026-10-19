//! Plan geometry for blueprints: the room outline reduced to the edges a recipe is
//! written against, and the arithmetic that turns an edge, a distance along it and
//! a distance into the room back into a point.
//!
//! **Everything here is millimetres.** The wire is decimal feet; the one place that
//! converts is `blueprint_capture`, so a figure in this module is never ambiguous.
//!
//! **An outline is held at two tolerances, and which one a caller wants is the
//! question** (`docs/PLAN-blueprints-stage0.md`). The *coarse* outline drops jogs
//! under [`COARSE_TOLERANCE_MM`] -- the 0.3 to 0.5 m nibs at columns and risers --
//! because two rooms of one type differ in exactly those, and an edge count that
//! includes them matched only half of RHH's same-named patient rooms. The *fine*
//! outline keeps them ([`FINE_TOLERANCE_MM`]) because the items that sit in those
//! nibs are outside the coarse outline and would be reported as findings that are
//! not. Edges, mapping and recipes use the coarse one; "is it inside" uses the fine.
//!
//! **No reflection anywhere.** Every outline is wound counter-clockwise, so an edge's
//! left-hand normal points into the room, and a blueprint fits a room by a cyclic
//! shift of edges only. A mirror image is a different blueprint (decision 7).

/// A plan point, millimetres.
pub type Pt = (f64, f64);

/// Jogs shorter than this are dropped from the coarse outline. Chosen on RHH, where it
/// took `1 BED B`'s edge-count mismatch between siblings from 54% to 16%.
pub const COARSE_TOLERANCE_MM: f64 = 350.0;

/// The fine outline's tolerance: a vertex that is not a corner (a collinear point, a
/// few millimetres of survey noise), and nothing more.
pub const FINE_TOLERANCE_MM: f64 = 30.0;

pub fn dist(a: Pt, b: Pt) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

pub fn sub(a: Pt, b: Pt) -> Pt {
    (a.0 - b.0, a.1 - b.1)
}

pub fn dot(a: Pt, b: Pt) -> f64 {
    a.0 * b.0 + a.1 * b.1
}

pub fn cross(a: Pt, b: Pt) -> f64 {
    a.0 * b.1 - a.1 * b.0
}

pub fn unit(a: Pt) -> Pt {
    let l = a.0.hypot(a.1);
    if l == 0.0 {
        (1.0, 0.0)
    } else {
        (a.0 / l, a.1 / l)
    }
}

/// Positive for a counter-clockwise ring.
pub fn signed_area(p: &[Pt]) -> f64 {
    (0..p.len()).map(|i| cross(p[i], p[(i + 1) % p.len()])).sum::<f64>() / 2.0
}

/// The mean of the vertices. Only a reference origin: nothing is defined against it.
pub fn centroid(p: &[Pt]) -> Pt {
    let n = p.len().max(1) as f64;
    (p.iter().map(|q| q.0).sum::<f64>() / n, p.iter().map(|q| q.1).sum::<f64>() / n)
}

fn seg_dist(pt: Pt, a: Pt, b: Pt) -> f64 {
    let ab = sub(b, a);
    let l2 = dot(ab, ab);
    let t = if l2 == 0.0 { 0.0 } else { (dot(sub(pt, a), ab) / l2).clamp(0.0, 1.0) };
    dist(pt, (a.0 + ab.0 * t, a.1 + ab.1 * t))
}

fn line_dist(pt: Pt, a: Pt, b: Pt) -> f64 {
    let ab = sub(b, a);
    let l = ab.0.hypot(ab.1);
    if l == 0.0 {
        dist(pt, a)
    } else {
        cross(ab, sub(pt, a)).abs() / l
    }
}

/// The ring with every vertex that lies within `tol` of the line through its
/// neighbours removed, counter-clockwise.
///
/// Removes one vertex at a time and starts again, because deleting a vertex changes
/// its neighbours' distances. A ring that would fall below three vertices is left as
/// it stands rather than collapsed.
pub fn simplify(ring: &[Pt], tol: f64) -> Vec<Pt> {
    let mut p: Vec<Pt> = ring.to_vec();
    loop {
        let mut deduped: Vec<Pt> = Vec::with_capacity(p.len());
        for &a in &p {
            if deduped.last().is_none_or(|&last| dist(a, last) > 1.0) {
                deduped.push(a);
            }
        }
        if deduped.len() > 1 && dist(deduped[0], deduped[deduped.len() - 1]) <= 1.0 {
            deduped.pop();
        }
        p = deduped;
        if p.len() <= 3 {
            break;
        }
        let n = p.len();
        match (0..n).find(|&i| line_dist(p[i], p[(i + n - 1) % n], p[(i + 1) % n]) < tol) {
            Some(i) => {
                p.remove(i);
            }
            None => break,
        }
    }
    if signed_area(&p) < 0.0 {
        p.reverse();
    }
    p
}

/// One side of an outline, with the frame a recipe is written in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edge {
    pub a: Pt,
    pub b: Pt,
    /// Unit direction from `a` to `b`.
    pub t: Pt,
    /// Unit normal pointing **into** the room (the left of `t`, for a counter-clockwise ring).
    pub n: Pt,
    pub len: f64,
}

pub fn edges(ring: &[Pt]) -> Vec<Edge> {
    (0..ring.len())
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            let t = unit(sub(b, a));
            Edge { a, b, t, n: (-t.1, t.0), len: dist(a, b) }
        })
        .collect()
}

impl Edge {
    /// The point `along` mm from `a` and `off` mm into the room. `along` may lie outside
    /// `0..=len`: nothing here clamps, so a placement is never moved by being asked for.
    pub fn point(&self, along: f64, off: f64) -> Pt {
        (self.a.0 + self.t.0 * along + self.n.0 * off, self.a.1 + self.t.1 * along + self.n.1 * off)
    }
}

/// How far the room extends from edge `i`, at `foot` on it, along its inward normal: the
/// distance to the next wall in front. The edge's own length when nothing is in front,
/// which is what a percentage of depth then means.
pub fn depth_at(ring: &[Pt], i: usize, foot: Pt) -> f64 {
    let es = edges(ring);
    let e = es[i];
    let origin = (foot.0 + e.n.0 * 0.5, foot.1 + e.n.1 * 0.5);
    let mut best = f64::INFINITY;
    for (j, f) in es.iter().enumerate() {
        if j == i {
            continue;
        }
        let d = sub(f.b, f.a);
        let den = cross(e.n, d);
        if den.abs() < 1e-9 {
            continue;
        }
        let w = sub(f.a, origin);
        let s = cross(w, d) / den;
        let u = cross(w, e.n) / den;
        if s > 0.0 && (-1e-6..=1.0 + 1e-6).contains(&u) {
            best = best.min(s);
        }
    }
    if best.is_finite() {
        best
    } else {
        e.len
    }
}

/// Whether `pt` is in the ring, or within `tol` of its boundary. A point on a wall is
/// inside: wall devices are placed on the wall line.
pub fn inside(pt: Pt, ring: &[Pt], tol: f64) -> bool {
    let n = ring.len();
    if (0..n).any(|i| seg_dist(pt, ring[i], ring[(i + 1) % n]) < tol) {
        return true;
    }
    let mut c = false;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if (a.1 > pt.1) != (b.1 > pt.1) && pt.0 < (b.0 - a.0) * (pt.1 - a.1) / (b.1 - a.1) + a.0 {
            c = !c;
        }
    }
    c
}

/// The four corners of a box centred at `c`, `hu` mm either side along unit `f` and
/// `hv` mm either side across it.
pub fn box_corners(c: Pt, f: Pt, hu: f64, hv: f64) -> [Pt; 4] {
    let v = (-f.1, f.0);
    [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)]
        .map(|(sx, sy)| (c.0 + sx * hu * f.0 + sy * hv * v.0, c.1 + sx * hu * f.1 + sy * hv * v.1))
}

/// Whether two convex polygons interpenetrate by more than `pen` mm (separating-axis test).
/// Touching, or sharing an edge, is not an overlap.
pub fn overlaps(p: &[Pt], q: &[Pt], pen: f64) -> bool {
    for poly in [p, q] {
        for i in 0..poly.len() {
            let e = sub(poly[(i + 1) % poly.len()], poly[i]);
            let axis = unit((-e.1, e.0));
            let (pa, qa): (Vec<f64>, Vec<f64>) =
                (p.iter().map(|&x| dot(x, axis)).collect(), q.iter().map(|&x| dot(x, axis)).collect());
            let (p_hi, q_hi) =
                (pa.iter().cloned().fold(f64::MIN, f64::max), qa.iter().cloned().fold(f64::MIN, f64::max));
            let (p_lo, q_lo) =
                (pa.iter().cloned().fold(f64::MAX, f64::min), qa.iter().cloned().fold(f64::MAX, f64::min));
            if p_hi.min(q_hi) - p_lo.max(q_lo) <= pen {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(w: f64, h: f64) -> Vec<Pt> {
        vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
    }

    #[test]
    fn test_a_clockwise_ring_comes_back_counter_clockwise() {
        let mut cw = rect(4000.0, 3000.0);
        cw.reverse();
        assert!(signed_area(&simplify(&cw, FINE_TOLERANCE_MM)) > 0.0);
    }

    /// House A's BED 01.06 arrives as five points: the fifth is a collinear point on the
    /// left wall. It must come out as the four-edge rectangle it is.
    #[test]
    fn test_a_collinear_vertex_is_dropped() {
        let mut ring = rect(3900.0, 4000.0);
        ring.push((0.0, 60.0));
        assert_eq!(simplify(&ring, FINE_TOLERANCE_MM).len(), 4);
    }

    /// The jog at a column: a 400 mm nib on the long wall. The fine outline keeps it, the
    /// coarse one does not, which is the whole reason there are two.
    #[test]
    fn test_a_jog_survives_the_fine_outline_and_not_the_coarse() {
        let ring = vec![
            (0.0, 0.0),
            (2000.0, 0.0),
            (2000.0, 300.0),
            (2400.0, 300.0),
            (2400.0, 0.0),
            (5000.0, 0.0),
            (5000.0, 4000.0),
            (0.0, 4000.0),
        ];
        assert_eq!(simplify(&ring, FINE_TOLERANCE_MM).len(), 8);
        assert_eq!(simplify(&ring, COARSE_TOLERANCE_MM).len(), 4);
    }

    #[test]
    fn test_the_normal_points_into_the_room() {
        let es = edges(&simplify(&rect(4000.0, 3000.0), FINE_TOLERANCE_MM));
        for e in es {
            let mid = e.point(e.len / 2.0, 100.0);
            assert!(inside(mid, &rect(4000.0, 3000.0), 0.0), "edge from {:?}", e.a);
        }
    }

    #[test]
    fn test_depth_is_the_distance_to_the_wall_in_front() {
        let ring = simplify(&rect(4000.0, 3000.0), FINE_TOLERANCE_MM);
        let es = edges(&ring);
        // Edge 0 runs along the bottom, so its depth is the room's height.
        assert!((depth_at(&ring, 0, es[0].point(1000.0, 0.0)) - 3000.0).abs() < 1.0);
        // Edge 1 runs up the right wall, so its depth is the room's width.
        assert!((depth_at(&ring, 1, es[1].point(1000.0, 0.0)) - 4000.0).abs() < 1.0);
    }

    #[test]
    fn test_boxes_overlap_only_when_they_interpenetrate() {
        let a = box_corners((0.0, 0.0), (1.0, 0.0), 300.0, 300.0);
        let touching = box_corners((600.0, 0.0), (1.0, 0.0), 300.0, 300.0);
        let over = box_corners((500.0, 0.0), (1.0, 0.0), 300.0, 300.0);
        let apart = box_corners((900.0, 0.0), (1.0, 0.0), 300.0, 300.0);
        assert!(!overlaps(&a, &touching, 5.0), "a shared edge is not an overlap");
        assert!(overlaps(&a, &over, 5.0));
        assert!(!overlaps(&a, &apart, 5.0));
    }

    #[test]
    fn test_a_point_on_the_wall_is_inside_and_one_beyond_it_is_not() {
        let ring = rect(4000.0, 3000.0);
        assert!(inside((0.0, 1000.0), &ring, 20.0));
        assert!(!inside((-200.0, 1000.0), &ring, 20.0));
        assert!(inside((2000.0, 1500.0), &ring, 20.0));
    }
}
