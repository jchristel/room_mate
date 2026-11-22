//! How well a blueprint's outline fits a room's: the cost of each cyclic shift of edges by shape (`shape_cost`, `shift_costs`), the best
//! shift, and the placement of the whole outline (`fit_outlines`). No reflection: a mirror is a different blueprint.

use super::*;

pub(super) fn turns(ring: &[Pt]) -> Vec<f64> {
    let es = edges(ring);
    (0..es.len())
        .map(|i| {
            let (a, b) = (es[i].t, es[(i + 1) % es.len()].t);
            cross(a, b).atan2(dot(a, b)).to_degrees()
        })
        .collect()
}

pub(super) fn shape_cost(a: &[Edge], ta: &[f64], b: &[Edge], tb: &[f64], shift: usize) -> f64 {
    let k = a.len();
    (0..k)
        .map(|j| {
            let m = (j + shift) % k;
            let len = (b[m].len / a[j].len).ln().abs();
            let turn = if (ta[j] - tb[m]).abs() > TURN_TOLERANCE_DEG { TURN_PENALTY } else { 0.0 };
            len + turn
        })
        .sum()
}

/// The shape cost of every cyclic shift of `b`'s edges against `a`'s, in shift order.
pub(in crate::service) fn shift_costs(a: &[Pt], b: &[Pt]) -> Vec<(usize, f64)> {
    let (ea, eb) = (edges(a), edges(b));
    let (ta, tb) = (turns(a), turns(b));
    (0..ea.len()).map(|s| (s, shape_cost(&ea, &ta, &eb, &tb, s))).collect()
}

/// The shift with the lowest shape cost, and that cost.
pub(in crate::service) fn best_shift(a: &[Pt], b: &[Pt]) -> (usize, f64) {
    shift_costs(a, b).into_iter().min_by(|x, y| x.1.total_cmp(&y.1)).unwrap_or((0, f64::MAX))
}

/// The ring reflected across the x axis, still counter-clockwise.
pub(super) fn reflected(ring: &[Pt]) -> Vec<Pt> {
    ring.iter().rev().map(|&(x, y)| (x, -y)).collect()
}

/// How the blueprint's coarse outline sits on the target's, and what to say about it.
pub(super) fn fit_outlines(
    bp_ring: &[Pt],
    t_coarse: &[Pt],
    shift: Option<usize>,
    doors: (&[BlueprintDoor], &[BlueprintDoor]),
    findings: &mut Vec<Finding>,
) -> (Fit, Vec<Candidate>) {
    let (es_bp, es_t) = (edges(bp_ring), edges(t_coarse));
    let (ta, tb) = (turns(bp_ring), turns(t_coarse));
    let k = t_coarse.len();
    let mut candidates: Vec<Candidate> = (0..k)
        .map(|s| Candidate {
            shift: s,
            cost: shape_cost(&es_bp, &ta, &es_t, &tb, s),
            door_gap_mm: door_gap(doors.0, doors.1, &(0..k).map(|i| (i + s) % k).collect::<Vec<_>>(), &es_t),
        })
        .filter(|c| c.cost <= FIT_OK)
        .collect();
    // Doors choose between rotations the shape cannot tell apart: those within `SHAPE_TIE` of the best
    // cost. A rotation that fits clearly worse is not promoted because a door happens to line up.
    let best = candidates.iter().map(|c| c.cost).fold(f64::INFINITY, f64::min);
    candidates.sort_by(|a, b| {
        let (ta, tb) = (a.cost > best + SHAPE_TIE, b.cost > best + SHAPE_TIE);
        ta.cmp(&tb)
            .then(a.door_gap_mm.unwrap_or(0.0).total_cmp(&b.door_gap_mm.unwrap_or(0.0)))
            .then(a.cost.total_cmp(&b.cost))
            .then(a.shift.cmp(&b.shift))
    });
    let (auto_shift, auto_cost) = match candidates.first() {
        Some(c) => (c.shift, c.cost),
        None => best_shift(bp_ring, t_coarse),
    };
    let s = shift.unwrap_or(auto_shift);
    let cost = if shift.is_some() {
        shape_cost(&es_bp, &turns(bp_ring), &es_t, &turns(t_coarse), s)
    } else {
        auto_cost
    };
    let mirror_cost = best_shift(&reflected(bp_ring), t_coarse).1;
    // A mirror image only counts when NO rotation fits: a rectangle's reflection is itself a rotation,
    // so a badly pinned shift on a symmetric room is a wrong mapping, not a mirrored room.
    let kind = if cost <= FIT_OK {
        FitKind::Plain
    } else if auto_cost <= FIT_OK {
        FitKind::NoFit
    } else if mirror_cost <= FIT_OK {
        FitKind::MirrorImage
    } else {
        FitKind::NoFit
    };
    match kind {
        FitKind::MirrorImage => findings.push(Finding::MirrorImage {
            message: "this room is the blueprint's shape reflected; rotation cannot reach it, so it needs a blueprint authored for the other hand".to_string(),
        }),
        FitKind::NoFit if auto_cost <= FIT_OK => findings.push(Finding::ShapeMismatch {
            message: format!("shift {s} does not fit the room's shape (cost {cost:.2}); shift {auto_shift} does"),
        }),
        FitKind::NoFit => findings.push(Finding::ShapeMismatch {
            message: format!("the outlines have the same number of edges but not the same shape (cost {cost:.2})"),
        }),
        _ => {}
    }
    (Fit { kind, shift: s, cost: Some(cost), mirror_cost: Some(mirror_cost) }, candidates)
}
