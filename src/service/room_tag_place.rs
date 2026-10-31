//! Where a room tag goes, written against the room's BOUNDARY rather than its middle (plan
//! decision 27).
//!
//! A room tag is learned from a tag a person placed in a plan and applied to another room, so it
//! is stored as a *recipe* against the boundary, the way an elevation marker is (`rls_plan`):
//!
//! - **A tag that belongs to a room blueprint is written against that room's COARSE outline**
//!   (`Boundary::Wall`): the edge nearest the tag head, how far along it as a fraction of its
//!   length, and how far into the room as paper millimetres. It is applied to another room by
//!   the cyclic shift that fits the shapes (`best_shift`), and a room with another number of
//!   coarse edges places nothing and says so, exactly as the elevations do.
//! - **A generic tag has no outline of its own to attach to**, so it is written against the
//!   smallest rectangle round the room (`Boundary::Rectangle`): its four sides in a fixed order
//!   (below, right, above, left of the rectangle's long axis, which points towards +x) work for a
//!   room of any shape and any edge count. The long axis of a square is whichever edge comes
//!   first, which is arbitrary but the same for the same room.
//!
//! Distances off the boundary are paper millimetres, so a rule holds at any view scale, and the
//! fraction along is unitless, so it holds for a room of any size. Pure: no state, no Revit.

use super::blueprint_geometry::{coarse_outline, dist, dot, edges, sub, Edge, Pt};
use super::blueprint_resolve::best_shift;
use crate::tag_rules::{Boundary, BoundaryOf};

/// The smallest rectangle (over the outline's own edge directions) round `ring`, as a counter-
/// clockwise ring of four corners whose first edge runs along the LONG axis towards +x.
pub fn oriented_rectangle(ring: &[Pt]) -> Option<Vec<Pt>> {
    let ring = coarse_outline(ring);
    if ring.len() < 3 {
        return None;
    }
    // (area, origin, u, w, h): the smallest by area, the first winning a tie.
    let mut best: Option<(f64, Pt, Pt, f64, f64)> = None;
    for e in edges(&ring) {
        let (u, v) = (e.t, (-e.t.1, e.t.0));
        let (mut lo_u, mut hi_u, mut lo_v, mut hi_v) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for &p in &ring {
            let (a, b) = (dot(p, u), dot(p, v));
            lo_u = lo_u.min(a);
            hi_u = hi_u.max(a);
            lo_v = lo_v.min(b);
            hi_v = hi_v.max(b);
        }
        let (w, h) = (hi_u - lo_u, hi_v - lo_v);
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        if best.is_none_or(|(a, ..)| w * h < a - 1e-9) {
            let origin = (u.0 * lo_u + v.0 * lo_v, u.1 * lo_u + v.1 * lo_v);
            best = Some((w * h, origin, u, w, h));
        }
    }
    let (_, origin, mut u, mut w, mut h) = best?;
    let v = (-u.1, u.0);
    let corner = |o: Pt, u: Pt, v: Pt, w: f64, h: f64| -> Vec<Pt> {
        vec![
            o,
            (o.0 + u.0 * w, o.1 + u.1 * w),
            (o.0 + u.0 * w + v.0 * h, o.1 + u.1 * w + v.1 * h),
            (o.0 + v.0 * h, o.1 + v.1 * h),
        ]
    };
    let mut ring = corner(origin, u, v, w, h);
    // The long axis first: turn a quarter so that edge 0 is the long one.
    if h > w {
        ring.rotate_left(1);
        u = v;
        std::mem::swap(&mut w, &mut h);
    }
    // And towards +x (or +y when it is vertical): turn half a turn, which keeps it counter-clockwise.
    if u.0 < -1e-9 || (u.0.abs() <= 1e-9 && u.1 < 0.0) {
        ring.rotate_left(2);
    }
    Some(ring)
}

/// The nearest edge of `edges` to `p`, by the distance to the segment.
fn nearest_edge(es: &[Edge], p: Pt) -> usize {
    let mut best = (0usize, f64::MAX);
    for (i, e) in es.iter().enumerate() {
        let along = dot(sub(p, e.a), e.t).clamp(0.0, e.len);
        let d = dist(p, e.point(along, 0.0));
        if d < best.1 {
            best = (i, d);
        }
    }
    best.0
}

fn recipe(es: &[Edge], p: Pt, of: BoundaryOf, scale: f64) -> Option<Boundary> {
    if es.is_empty() || scale <= 0.0 {
        return None;
    }
    let i = nearest_edge(es, p);
    let e = &es[i];
    if e.len <= 0.0 {
        return None;
    }
    let d = sub(p, e.a);
    Some(Boundary { of, edge: i, along: dot(d, e.t) / e.len, off: dot(d, e.n) / scale })
}

/// A tag head written against the room it was placed in. `bound` picks the wall recipe (against
/// the room's coarse outline) over the rectangle one. `ring` is the room's fine outline in the
/// same frame as `head`, and `scale` the view's scale.
pub fn boundary_of(ring: &[Pt], head: Pt, bound: bool, scale: f64) -> Option<Boundary> {
    if bound {
        recipe(&edges(&coarse_outline(ring)), head, BoundaryOf::Wall, scale)
    } else {
        recipe(&edges(&oriented_rectangle(ring)?), head, BoundaryOf::Rectangle, scale)
    }
}

/// Where a recipe puts the tag in `target`. A wall recipe was written against `template` (the room
/// the sheet was made for) and is carried over by the shape fit; why it cannot be is the `Err`.
pub fn head_in(b: &Boundary, template: Option<&[Pt]>, target: &[Pt], scale: f64) -> Result<Pt, String> {
    let (es, index) = match b.of {
        BoundaryOf::Wall => {
            let Some(template) = template else {
                return Err("the room the sheet was made for is not on the server".to_string());
            };
            let (tc, gc) = (coarse_outline(template), coarse_outline(target));
            if tc.len() != gc.len() || tc.len() < 3 {
                return Err(format!(
                    "the room tag was not placed: the template room has {} edges and this room {} (the coarse outlines must have the same number)",
                    tc.len(),
                    gc.len()
                ));
            }
            if b.edge >= tc.len() {
                return Err("the room tag's edge is not one of the template room's".to_string());
            }
            let (shift, _) = best_shift(&tc, &gc);
            (edges(&gc), (b.edge + shift) % gc.len())
        }
        BoundaryOf::Rectangle => (edges(&oriented_rectangle(target).ok_or("this room has no area")?), b.edge),
    };
    let e = es.get(index).ok_or("the room tag's side is not one of the room's")?;
    Ok(e.point(b.along * e.len, b.off * scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, w: f64, h: f64) -> Vec<Pt> {
        vec![(x0, y0), (x0 + w, y0), (x0 + w, y0 + h), (x0, y0 + h)]
    }

    fn close(a: Pt, b: Pt) -> bool {
        dist(a, b) < 1e-6
    }

    #[test]
    fn test_the_rectangle_starts_on_the_long_axis_towards_plus_x_whatever_the_ring_started_with() {
        // 4 x 3 starting at the other corner, going the same way round, and the same room turned a quarter.
        let a = oriented_rectangle(&rect(0.0, 0.0, 4000.0, 3000.0)).unwrap();
        assert!(close(a[0], (0.0, 0.0)) && close(a[1], (4000.0, 0.0)), "{a:?}");
        let tall = oriented_rectangle(&rect(0.0, 0.0, 3000.0, 4000.0)).unwrap();
        assert!(
            dist(tall[0], tall[1]) > dist(tall[1], tall[2]),
            "the first edge is the long one: {tall:?}"
        );
        assert!(
            tall[1].0 > tall[0].0 || (tall[1].0 - tall[0].0).abs() < 1e-9,
            "it runs towards +x: {tall:?}"
        );
        let mut shifted = rect(0.0, 0.0, 4000.0, 3000.0);
        shifted.rotate_left(2);
        let b = oriented_rectangle(&shifted).unwrap();
        for (p, q) in a.iter().zip(&b) {
            assert!(close(*p, *q), "the starting corner of the ring does not matter: {a:?} {b:?}");
        }
    }

    #[test]
    fn test_a_generic_recipe_follows_the_room_to_one_of_another_size_and_position() {
        // 1:50. A tag 20 mm (paper) = 1000 mm into a 4 x 3 m room from its bottom side, 25% along.
        let from = rect(0.0, 0.0, 4000.0, 3000.0);
        let b = boundary_of(&from, (1000.0, 1000.0), false, 50.0).unwrap();
        assert_eq!((b.of, b.edge), (BoundaryOf::Rectangle, 0));
        assert!((b.along - 0.25).abs() < 1e-9 && (b.off - 20.0).abs() < 1e-9, "{b:?}");
        // A 8 x 5 m room elsewhere: 25% of 8 m along, 1 m up.
        let to = rect(10_000.0, 2_000.0, 8000.0, 5000.0);
        let at = head_in(&b, None, &to, 50.0).unwrap();
        assert!(close(at, (12_000.0, 3_000.0)), "{at:?}");
    }

    #[test]
    fn test_a_wall_recipe_goes_to_the_wall_the_shapes_match_and_needs_the_same_edge_count() {
        let template = rect(0.0, 0.0, 4000.0, 3000.0);
        // Near the top wall (edge 2), half way along, 500 mm in.
        let b = boundary_of(&template, (2000.0, 2500.0), true, 50.0).unwrap();
        assert_eq!((b.of, b.edge), (BoundaryOf::Wall, 2));
        assert!(
            (b.along - 0.5).abs() < 1e-9 && (b.off - 10.0).abs() < 1e-9,
            "a fraction along, paper mm in: {b:?}"
        );
        // The same room moved: the tag moves with it.
        let target = rect(5000.0, 1000.0, 4000.0, 3000.0);
        let at = head_in(&b, Some(&template), &target, 50.0).unwrap();
        assert!(close(at, (7000.0, 3500.0)), "{at:?}");
        // An L-shaped room has six edges: nothing is placed, and the reason says why.
        let l = vec![
            (0.0, 0.0),
            (4000.0, 0.0),
            (4000.0, 1500.0),
            (2000.0, 1500.0),
            (2000.0, 3000.0),
            (0.0, 3000.0),
        ];
        assert!(head_in(&b, Some(&template), &l, 50.0).unwrap_err().contains("edges"));
        assert!(head_in(&b, None, &target, 50.0).unwrap_err().contains("not on the server"));
    }

    #[test]
    fn test_nothing_is_written_for_a_ring_with_no_area_or_a_scale_that_is_not_one() {
        assert!(boundary_of(&[(0.0, 0.0), (1.0, 0.0)], (0.0, 0.0), false, 50.0).is_none());
        assert!(boundary_of(&rect(0.0, 0.0, 10.0, 10.0), (1.0, 1.0), true, 0.0).is_none());
    }
}
