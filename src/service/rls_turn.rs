//! How far a create-RLS task's new plan view is turned, so a room at an angle sits on its sheet the way the
//! template's room sits on the template sheet.
//!
//! **The walls decide, not the room's angle** (2026-11-12). A room at an angle is squared up by any of four
//! turns a quarter apart, and the room's angle (`dominant_angle_deg`) names only the one within 45 degrees, which
//! at 45 is decided by rounding: measured in Revit on a room at 45, the angle rule turned it -45 and a
//! longest-wall rule +45, a quarter turn apart, one putting its long side up the sheet and the other across it.
//! The template sheet already says which: its plan shows its room a certain way round. So each template wall is
//! paired with the target's by the cyclic shift that fits the shapes ([`wall_pairing`], over
//! `blueprint_resolve::shift_costs`, the pairing the elevations and a bound room tag are placed by too, so the plan,
//! the elevations and the tag agree on which wall is which), and the
//! turn is the length-weighted mean of the pairs' differences in direction, over the whole circle. An L built the
//! other way round gets a half turn, so its notch is where the template's is on the sheet.
//!
//! **A tie of shape goes to the smaller turn** ([`wall_pairing`]). A rectangle fits its template two ways a half
//! turn apart (a square four ways), and shape cannot choose: best_shift alone turned a rectangle at 30 degrees by
//! -150, upside down on the sheet. So of the pairings within `SHAPE_TIE` of the best, the one needing the least
//! turn is taken, and the elevations are placed by the SAME pairing (`rls_plan::transfer`), so a view that looked
//! at the template's bottom wall looks at the wall at the bottom of this room's plan.
//!
//! **Rooms with a different number of coarse edges** cannot be paired; they are squared up by the smallest turn,
//! the difference of the two rooms' angles folded into (-45, 45], and say so (`TurnBy::RoomAngle`).
//!
//! **The template's plan view is taken to be unturned** in its own model: duHast exports its crop's Min and Max,
//! not its transform, so a template made from a turned view would be read as if it were not.
//!
//! **How the plug-in turns it** was measured in Revit 2026 (the rotated room probe): a plan ignores a turned
//! `CropBox.Transform`, so the element that draws the crop (the view's id + 2, with no category) is rotated
//! about the crop's centre (duHast `views_crop.rotate_view_crop_without_transaction_wrapper`). A turn about the
//! crop's own centre keeps its size, so the crop can be set first and turned after.

use serde::{Deserialize, Serialize};

use super::blueprint_geometry::{coarse_outline, dominant_angle_deg, edges, Pt};
use super::blueprint_resolve::{shift_costs, SHAPE_TIE};

/// A turn smaller than this is no turn: a square room's float noise, or a room a hair off its template, keeps the
/// plan as it was.
pub const MIN_TURN_DEG: f64 = 0.05;

/// How the turn was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnBy {
    /// Each template wall paired with this room's (the elevations' pairing).
    Walls,
    /// The rooms have a different number of coarse edges: the smallest turn that squares this room up the way
    /// the template's is.
    RoomAngle,
}

/// The new plan view's turn: counter-clockwise, in degrees, of the view's right direction from the model's x axis,
/// about the centre of its crop.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ViewTurn {
    pub angle_deg: f64,
    pub by: TurnBy,
}

/// `deg` brought into (-180, 180].
fn wrap(deg: f64) -> f64 {
    let d = deg.rem_euclid(360.0);
    if d > 180.0 {
        d - 360.0
    } else {
        d
    }
}

/// `deg` brought into (-45, 45].
fn fold_quarter(deg: f64) -> f64 {
    let d = deg.rem_euclid(90.0);
    if d > 45.0 {
        d - 90.0
    } else {
        d
    }
}

/// The turn, in degrees in (-180, 180], that carries `tc`'s edges onto `gc`'s paired by `shift`: the
/// length-weighted mean of each pair's difference in direction. None when the edges have no length.
fn turn_of(tc: &[Pt], gc: &[Pt], shift: usize) -> Option<f64> {
    let (te, ge) = (edges(tc), edges(gc));
    let (mut s, mut c) = (0.0, 0.0);
    for (i, t) in te.iter().enumerate() {
        let g = &ge[(i + shift) % ge.len()];
        let d = g.t.1.atan2(g.t.0) - t.t.1.atan2(t.t.0);
        let w = t.len.min(g.len);
        s += w * d.sin();
        c += w * d.cos();
    }
    (s.hypot(c) >= 1e-9).then(|| wrap(s.atan2(c).to_degrees()))
}

/// Which of `gc`'s edges each of `tc`'s is (the cyclic shift) and the turn between them, for two COARSE outlines
/// with the same number of edges: of the shifts within `SHAPE_TIE` of the best fit, the one needing the least turn.
/// None when the counts differ or the outlines have no length.
pub fn wall_pairing(tc: &[Pt], gc: &[Pt]) -> Option<(usize, f64)> {
    if tc.len() != gc.len() || tc.len() < 3 {
        return None;
    }
    let costs = shift_costs(tc, gc);
    let best = costs.iter().map(|c| c.1).fold(f64::INFINITY, f64::min);
    costs
        .into_iter()
        .filter(|c| c.1 <= best + SHAPE_TIE)
        .filter_map(|(shift, _)| Some((shift, turn_of(tc, gc, shift)?)))
        .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
}

/// How far to turn the plan view of `target` so it shows the room as the template's plan shows `template`. Both
/// are the FINE outline, counter-clockwise, each in its own model's frame; the coarse outline is taken here. None
/// when no turn is needed ([`MIN_TURN_DEG`]) or the outlines have no length.
pub fn view_turn(template: &[Pt], target: &[Pt]) -> Option<ViewTurn> {
    let (tc, gc) = (coarse_outline(template), coarse_outline(target));
    let turn = if tc.len() == gc.len() && tc.len() >= 3 {
        ViewTurn { angle_deg: wall_pairing(&tc, &gc)?.1, by: TurnBy::Walls }
    } else {
        ViewTurn {
            angle_deg: fold_quarter(dominant_angle_deg(target) - dominant_angle_deg(template)),
            by: TurnBy::RoomAngle,
        }
    };
    (turn.angle_deg.abs() >= MIN_TURN_DEG).then_some(turn)
}

/// How far to turn the ONE plan of a set of adjacent rooms (`rls_set`): there is no single room to pair walls with,
/// so the set is squared up by the smallest turn, like rooms with differing edge counts (`TurnBy::RoomAngle`). Its
/// angle is the dominant angle of every room's coarse walls together, length-weighted, so a long corridor counts
/// for more than a cupboard off it. A room of the set at another angle than the rest sits skewed on the plan: the
/// elevations, which belong to each room, are still square to it.
pub fn set_turn(template: &[Pt], rooms: &[Vec<Pt>]) -> Option<ViewTurn> {
    let (mut s, mut c) = (0.0, 0.0);
    for ring in rooms {
        for e in edges(&coarse_outline(ring)) {
            let a = 4.0 * e.t.1.atan2(e.t.0);
            s += e.len * a.sin();
            c += e.len * a.cos();
        }
    }
    if s.hypot(c) < 1e-9 {
        return None;
    }
    let set_deg = s.atan2(c).to_degrees() / 4.0;
    let angle_deg = fold_quarter(set_deg - dominant_angle_deg(template));
    (angle_deg.abs() >= MIN_TURN_DEG).then_some(ViewTurn { angle_deg, by: TurnBy::RoomAngle })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::blueprint_geometry::rotate;

    #[test]
    fn a_set_turns_by_its_walls_together_and_the_longer_walls_win() {
        let corridor = turned(&rect(12_000.0, 1_800.0), 30.0, (0.0, 0.0));
        let cupboard = turned(&rect(900.0, 600.0), 10.0, (0.0, 5_000.0));
        let t = set_turn(&rect(4000.0, 3000.0), &[corridor.clone(), cupboard]).unwrap();
        assert!(t.angle_deg > 25.0 && t.angle_deg < 30.0, "{t:?}");
        assert_eq!(TurnBy::RoomAngle, t.by);
        assert_eq!(None, set_turn(&rect(4000.0, 3000.0), &[rect(5000.0, 2000.0), rect(1000.0, 1000.0)]));
    }

    fn rect(w: f64, h: f64) -> Vec<Pt> {
        vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
    }

    fn turned(ring: &[Pt], deg: f64, by: Pt) -> Vec<Pt> {
        ring.iter().map(|&p| rotate(p, deg)).map(|(x, y)| (x + by.0, y + by.1)).collect()
    }

    fn angle(template: &[Pt], target: &[Pt]) -> f64 {
        view_turn(template, target).map_or(0.0, |t| t.angle_deg)
    }

    #[test]
    fn a_room_at_30_degrees_turns_its_plan_30_degrees() {
        let t = view_turn(&rect(4000.0, 3000.0), &turned(&rect(5000.0, 3500.0), 30.0, (20_000.0, -8_000.0))).unwrap();
        assert!((t.angle_deg - 30.0).abs() < 1e-9, "{t:?}");
        assert_eq!(TurnBy::Walls, t.by);
    }

    /// The measured case: at 45 degrees the room's angle could go either way; the template's long side across the
    /// sheet decides, so the long side stays across it whichever way the room is drawn.
    #[test]
    fn at_45_degrees_the_template_s_walls_choose_the_quarter_turn() {
        let template = rect(4000.0, 3000.0); // long side along x
        let long_up_left = turned(&rect(4000.0, 3000.0), 45.0, (0.0, 0.0));
        let long_up_right = turned(&rect(4000.0, 3000.0), -45.0, (0.0, 0.0));
        assert!((angle(&template, &long_up_left) - 45.0).abs() < 1e-9);
        assert!((angle(&template, &long_up_right) + 45.0).abs() < 1e-9);
        // a portrait template turns the same landscape room the other quarter
        let portrait = rect(3000.0, 4000.0);
        let a = angle(&portrait, &long_up_left);
        assert!((a + 45.0).abs() < 1e-9 || (a - 135.0).abs() < 1e-9, "{a}");
    }

    /// An L whose arms are mirrored in no way: turned three quarters, it is turned back three quarters, not by the
    /// smallest turn that would square it with its notch in the wrong corner.
    #[test]
    fn an_l_turned_a_three_quarter_turn_is_turned_back_all_the_way() {
        let l = vec![
            (0.0, 0.0),
            (6000.0, 0.0),
            (6000.0, 2000.0),
            (2500.0, 2000.0),
            (2500.0, 4000.0),
            (0.0, 4000.0),
        ];
        let t = view_turn(&l, &turned(&l, 270.0, (1_000.0, 1_000.0))).unwrap();
        assert!((t.angle_deg + 90.0).abs() < 1e-6, "{t:?}");
    }

    #[test]
    fn a_square_room_or_one_a_hair_off_is_not_turned() {
        assert_eq!(None, view_turn(&rect(4000.0, 3000.0), &rect(4100.0, 2900.0)));
        assert_eq!(None, view_turn(&rect(4000.0, 3000.0), &turned(&rect(4000.0, 3000.0), 0.01, (0.0, 0.0))));
    }

    #[test]
    fn rooms_with_different_edge_counts_are_squared_up_by_the_smallest_turn() {
        let l = vec![
            (0.0, 0.0),
            (6000.0, 0.0),
            (6000.0, 2000.0),
            (2500.0, 2000.0),
            (2500.0, 4000.0),
            (0.0, 4000.0),
        ];
        let t = view_turn(&rect(4000.0, 3000.0), &turned(&l, 120.0, (0.0, 0.0))).unwrap();
        assert!((t.angle_deg - 30.0).abs() < 1e-6, "{t:?}");
        assert_eq!(TurnBy::RoomAngle, t.by);
    }
}
