//! Where a tag head sits relative to its element, in paper millimetres: along the element's facing (plan) or along a view's right and up
//! (elevation), and which side of an item an elevation sees (`view_side`).

use super::*;

/// The element's facing in plan: its local X, projected. `None` when it has no plan component.
pub(super) fn facing_of(loc: &Location) -> Option<(f64, f64)> {
    let row = loc.rotation_coord.as_ref()?.data.first()?;
    let (x, y) = (*row.first()?, *row.get(1)?);
    let len = x.hypot(y);
    (len > 1e-6).then(|| (x / len, y / len))
}

/// A model-space point as an offset from the element, in its frame, in paper mm.
pub(super) fn to_offset(p: P3, from: P3, facing: (f64, f64), scale: f64) -> Offset {
    let (dx, dy) = (p.x - from.x, p.y - from.y);
    Offset {
        x: (dx * facing.0 + dy * facing.1) / scale,
        y: (-dx * facing.1 + dy * facing.0) / scale,
    }
}

/// How square-on an item's facing must be to the direction a view looks for the item to count as seen front on:
/// the cosine of about 25 degrees. Anything less is `side`, so an item at 45 degrees is read as seen from the side.
pub const FRONT_ON: f64 = 0.9;

/// Whether a viewport type is an elevation (an elevation's offsets are along the view, not the plan).
pub fn is_elevation(viewport_type: &str) -> bool {
    viewport_type.trim().eq_ignore_ascii_case("elevation")
}

/// A unit horizontal direction, or `None` for a vector with no plan part (a view looking straight down).
pub fn plan_direction(x: f64, y: f64) -> Option<(f64, f64)> {
    let len = x.hypot(y);
    (len > 1e-6).then(|| (x / len, y / len))
}

/// `front` when the item's plan facing is parallel (or opposite) to the direction the view looks, else `side`.
pub fn view_side(facing: (f64, f64), looks: (f64, f64)) -> &'static str {
    if (facing.0 * looks.0 + facing.1 * looks.1).abs() >= FRONT_ON {
        "front"
    } else {
        "side"
    }
}

/// The viewer's right in plan for a view looking along `looks` (an elevation is vertical, so up is +z).
pub fn view_right(looks: (f64, f64)) -> (f64, f64) {
    (looks.1, -looks.0)
}

/// A model-space point as an offset from the element in an ELEVATION: x along the viewer's right, y up, paper mm.
pub(super) fn to_view_offset(p: P3, from: P3, looks: (f64, f64), scale: f64) -> Offset {
    let right = view_right(looks);
    Offset {
        x: ((p.x - from.x) * right.0 + (p.y - from.y) * right.1) / scale,
        y: (p.z - from.z) / scale,
    }
}

pub(super) fn wrap(angle: f64) -> f64 {
    let two_pi = std::f64::consts::TAU;
    let mut a = angle % two_pi;
    if a > std::f64::consts::PI {
        a -= two_pi;
    } else if a <= -std::f64::consts::PI {
        a += two_pi;
    }
    a
}
