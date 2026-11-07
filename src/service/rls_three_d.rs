//! What a new 3D view of a create-RLS task hides and sees through: the room's walls on the camera's side, as EDGES
//! of the room's outline, and the items standing along them.
//!
//! **The server decides, the plug-in applies** (2026-11-08), as for tags and elevations. The server knows the
//! camera: the plug-in makes each 3D view looking along the template view's `forward_direction` at a section box
//! cut to the plan crop (`rls_plan::plan_crop`, else the room's box plus the margin), and duHast's
//! `view_3d_create` puts the eye back along that direction from the box's centre. So the eye in plan is computed
//! here the same way, and the old plug-in rule (`room_m.view_walls.walls_to_hide`) runs on the room's outline:
//! an edge the line from the eye to the room's middle crosses, and every edge facing the camera (the camera on its
//! outside by [`FACING`] of the distance, about 3 degrees), so from a corner both walls of the corner go.
//!
//! **An edge, never a wall id.** The server's outline names no wall, so the task says WHERE to hide and the
//! plug-in hides every wall with a face along an edge, within [`HIDE_BAND_MM`] (`view_walls.walls_along`). That
//! also catches a lining wall in front of the bounding one. A face, not the location line: the outline runs along a
//! wall's face and a thick wall's centre line is further off than the band.
//!
//! **What stands along a hidden edge is seen through**: the room's FF&E with a footprint corner within
//! [`SEE_THROUGH_BAND_MM`] of one (a wardrobe, a basin; not the bed in the middle).
//!
//! Only the outer loop is judged (`model_room_ring` reads no holes), so a column's walls are never hidden from
//! here. Positions are in the TARGET model's own frame, mm.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::blueprint_geometry::{dot, sub, Pt};
use super::rls_plan::PlanCrop;
use super::rls_tags::RoomItem;

/// How far a wall's face may be from a hidden edge and still be the wall of that edge: the outline is the face
/// itself, so this is for a finish, a lining or a boundary that Revit drew a little off.
pub const HIDE_BAND_MM: f64 = 200.0;

/// How far from a hidden edge an item counts as standing along that wall.
pub const SEE_THROUGH_BAND_MM: f64 = 600.0;

/// The section box margin round the room when there is no plan crop: the plug-in's `SECTION_MARGIN_MM`.
pub const SECTION_MARGIN_MM: f64 = 600.0;

/// A wall faces the camera when the camera is outside its line by at least this share of the distance between
/// them: a wall the camera looks along stays (`room_m.view_walls.FACING`).
pub const FACING: f64 = 0.05;

/// One new 3D view, by its place among the template's 3D viewports (from 1): the sheet's view ids are not usable
/// (a 3D view exports `-1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreeDPlan {
    pub index: usize,
    /// The camera in plan, and the point it looks at (the middle of the room's box).
    pub eye: [f64; 2],
    pub looks_at: [f64; 2],
    /// The edges of the room's outline whose walls are hidden, each `[start, end]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<[[f64; 2]; 2]>,
    /// How far a wall's face may be from a hidden edge ([`HIDE_BAND_MM`]), carried so the plug-in reads one number.
    pub band_mm: f64,
    /// The items to make transparent in this view.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub see_through: Vec<String>,
}

fn is_three_d(port: &Value) -> bool {
    port.get("vp_type")
        .and_then(Value::as_str)
        .is_some_and(|t| t.trim().eq_ignore_ascii_case("three d"))
}

/// Whether a sheet shows any 3D viewport (duHast names the type "three d").
pub fn has_three_d(sheet: &Value) -> bool {
    sheet
        .get("view_ports")
        .and_then(Value::as_array)
        .is_some_and(|ports| ports.iter().any(is_three_d))
}

fn xyz(v: Option<&Value>) -> Option<[f64; 3]> {
    let v = v?;
    let n = |k: &str| v.get(k).and_then(Value::as_f64);
    Some([n("x")?, n("y")?, n("z")?])
}

/// The distance from `p` to the segment `a`-`b`.
fn to_segment(p: Pt, a: Pt, b: Pt) -> f64 {
    let ab = sub(b, a);
    let len2 = dot(ab, ab);
    let t = if len2 > 0.0 { (dot(sub(p, a), ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let foot = (a.0 + ab.0 * t, a.1 + ab.1 * t);
    ((p.0 - foot.0).powi(2) + (p.1 - foot.1).powi(2)).sqrt()
}

fn orient(o: Pt, a: Pt, b: Pt) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

/// Whether segment `p1`-`p2` meets `q1`-`q2`; touching counts, so a line through a corner meets both walls.
fn segments_cross(p1: Pt, p2: Pt, q1: Pt, q2: Pt) -> bool {
    const EPS: f64 = 1e-9;
    let within = |a: Pt, b: Pt, p: Pt| {
        a.0.min(b.0) - EPS <= p.0 && p.0 <= a.0.max(b.0) + EPS && a.1.min(b.1) - EPS <= p.1 && p.1 <= a.1.max(b.1) + EPS
    };
    let (d1, d2) = (orient(q1, q2, p1), orient(q1, q2, p2));
    let (d3, d4) = (orient(p1, p2, q1), orient(p1, p2, q2));
    if ((d1 > EPS && d2 < -EPS) || (d1 < -EPS && d2 > EPS)) && ((d3 > EPS && d4 < -EPS) || (d3 < -EPS && d4 > EPS)) {
        return true;
    }
    (d1.abs() <= EPS && within(q1, q2, p1))
        || (d2.abs() <= EPS && within(q1, q2, p2))
        || (d3.abs() <= EPS && within(p1, p2, q1))
        || (d4.abs() <= EPS && within(p1, p2, q2))
}

/// Whether the camera at `eye` is on the outside of edge `a`-`b` of a counter-clockwise ring, by [`FACING`].
fn faces(eye: Pt, a: Pt, b: Pt) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-9 {
        return false;
    }
    let outward = (dy / len, -dx / len);
    let m = ((a.0 + b.0) / 2.0 - eye.0, (a.1 + b.1) / 2.0 - eye.1);
    let distance = (m.0 * m.0 + m.1 * m.1).sqrt();
    -dot(outward, m) > FACING * distance
}

/// The edges of `ring` (counter-clockwise) to hide from `eye` looking at `looks_at`: crossed by the line between,
/// or facing the camera.
pub fn edges_to_hide(eye: Pt, looks_at: Pt, ring: &[Pt]) -> Vec<(Pt, Pt)> {
    (0..ring.len())
        .map(|i| (ring[i], ring[(i + 1) % ring.len()]))
        .filter(|&(a, b)| segments_cross(eye, looks_at, a, b) || faces(eye, a, b))
        .collect()
}

fn bounds(ring: &[Pt]) -> Option<(Pt, Pt)> {
    let first = *ring.first()?;
    Some(ring.iter().fold((first, first), |(lo, hi), p| {
        ((lo.0.min(p.0), lo.1.min(p.1)), (hi.0.max(p.0), hi.1.max(p.1)))
    }))
}

/// Every 3D viewport of `sheet` made for the room `ring` (the fine outline, counter-clockwise, in the target
/// model's frame, mm): where its camera is, the edges whose walls go and the `items` seen through. A viewport with
/// no plan direction (looking straight down) hides nothing and still has its place in the list.
pub fn three_ds(sheet: &Value, ring: &[Pt], crop: Option<&PlanCrop>, items: &[RoomItem]) -> Vec<ThreeDPlan> {
    let Some((lo, hi)) = bounds(ring) else {
        return Vec::new();
    };
    let looks_at = ((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0);
    // the section box in plan, as the plug-in cuts it
    let (box_lo, box_hi) = match crop {
        Some(c) => ((c.min[0], c.min[1]), (c.max[0], c.max[1])),
        None => (
            (lo.0 - SECTION_MARGIN_MM, lo.1 - SECTION_MARGIN_MM),
            (hi.0 + SECTION_MARGIN_MM, hi.1 + SECTION_MARGIN_MM),
        ),
    };
    let centre = ((box_lo.0 + box_hi.0) / 2.0, (box_lo.1 + box_hi.1) / 2.0);
    let ports = sheet.get("view_ports").and_then(Value::as_array).cloned().unwrap_or_default();
    ports
        .iter()
        .filter(|p| is_three_d(p))
        .enumerate()
        .map(|(n, port)| {
            let view = port.get("view");
            let forward = xyz(view.and_then(|v| v.get("forward_direction")));
            // the plug-in stands the camera back twice the box's diagonal; its height is the template's
            let heights = view.and_then(|v| v.get("section_z_above_floor_mm"));
            let dz = heights
                .and_then(|h| Some(h.get("max")?.as_f64()? - h.get("min")?.as_f64()?))
                .unwrap_or(0.0);
            let (dx, dy) = (box_hi.0 - box_lo.0, box_hi.1 - box_lo.1);
            let distance = 2.0 * (dx * dx + dy * dy + dz * dz).sqrt();
            let eye = forward.and_then(|f| {
                let len = (f[0] * f[0] + f[1] * f[1]).sqrt();
                (len > 1e-6).then(|| (centre.0 - f[0] / len * distance, centre.1 - f[1] / len * distance))
            });
            let hide = eye.map(|e| edges_to_hide(e, looks_at, ring)).unwrap_or_default();
            let see_through = items
                .iter()
                .filter(|it| {
                    it.footprint
                        .iter()
                        .any(|&p| hide.iter().any(|&(a, b)| to_segment(p, a, b) <= SEE_THROUGH_BAND_MM))
                })
                .map(|it| it.id.clone())
                .collect();
            let eye = eye.unwrap_or(centre);
            ThreeDPlan {
                index: n + 1,
                eye: [eye.0, eye.1],
                looks_at: [looks_at.0, looks_at.1],
                hide: hide.iter().map(|&(a, b)| [[a.0, a.1], [b.0, b.1]]).collect(),
                band_mm: HIDE_BAND_MM,
                see_through,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 4 x 3 m, counter-clockwise: south, east, north, west.
    fn room() -> Vec<Pt> {
        vec![(0.0, 0.0), (4000.0, 0.0), (4000.0, 3000.0), (0.0, 3000.0)]
    }

    fn item(id: &str, footprint: Vec<Pt>) -> RoomItem {
        RoomItem {
            id: id.into(),
            family: "F".into(),
            type_name: "T".into(),
            category: "Furniture".into(),
            type_code: None,
            footprint,
            facing: None,
            nested: false,
            properties: vec![],
            type_properties: vec![],
        }
    }

    fn sheet(forward: [f64; 3]) -> Value {
        json!({"view_ports": [
            {"vp_type": "floor plan", "view": {}},
            {"vp_type": "three d", "view": {"forward_direction": {"x": forward[0], "y": forward[1], "z": forward[2]},
                                            "section_z_above_floor_mm": {"min": -600.0, "max": 2700.0}}},
        ]})
    }

    /// `docs/3D wall hiding logic.pdf`: from off the south west corner the line to the middle crosses one wall,
    /// and the other still stands across the near edge of the view.
    #[test]
    fn from_a_corner_both_walls_of_the_corner_are_hidden() {
        let hidden = edges_to_hide((-6000.0, -2000.0), (2000.0, 1500.0), &room());
        assert_eq!(vec![((0.0, 0.0), (4000.0, 0.0)), ((0.0, 3000.0), (0.0, 0.0))], hidden);
    }

    #[test]
    fn square_on_only_the_near_wall_is_hidden_and_one_looked_along_stays() {
        assert_eq!(
            vec![((0.0, 0.0), (4000.0, 0.0))],
            edges_to_hide((2000.0, -20000.0), (2000.0, 1500.0), &room())
        );
        assert!(!faces((-10000.0, 10.0), (0.0, 0.0), (4000.0, 0.0)));
    }

    /// The template looks north east and down; the camera stands back to the south west, so the south and west
    /// walls go and the wardrobe against the south wall is seen through, the bed in the middle is not.
    #[test]
    fn a_template_3d_view_gives_its_camera_its_edges_and_the_items_along_them() {
        let items = vec![
            item("wardrobe", vec![(500.0, 50.0), (1700.0, 50.0), (1700.0, 650.0), (500.0, 650.0)]),
            item("bed", vec![(1200.0, 900.0), (2800.0, 900.0), (2800.0, 2100.0), (1200.0, 2100.0)]),
            item("desk", vec![(3500.0, 2900.0), (3950.0, 2900.0), (3950.0, 2990.0), (3500.0, 2990.0)]),
        ];
        let plans = three_ds(&sheet([203.2, 203.2, -101.6]), &room(), None, &items);
        assert_eq!(1, plans.len());
        let p = &plans[0];
        assert_eq!(1, p.index);
        assert!(p.eye[0] < 0.0 && p.eye[1] < 0.0, "{:?}", p.eye);
        assert_eq!([2000.0, 1500.0], p.looks_at);
        assert_eq!(vec![[[0.0, 0.0], [4000.0, 0.0]], [[0.0, 3000.0], [0.0, 0.0]]], p.hide);
        assert_eq!(HIDE_BAND_MM, p.band_mm);
        assert_eq!(vec!["wardrobe".to_string()], p.see_through);
    }

    #[test]
    fn looking_straight_down_hides_nothing_and_a_sheet_without_3d_has_no_plans() {
        let plans = three_ds(&sheet([0.0, 0.0, -1.0]), &room(), None, &[]);
        assert!(plans[0].hide.is_empty());
        assert!(three_ds(&json!({"view_ports": [{"vp_type": "floor plan"}]}), &room(), None, &[]).is_empty());
        assert!(three_ds(&sheet([1.0, 1.0, -1.0]), &[], None, &[]).is_empty());
    }

    #[test]
    fn the_camera_stands_back_from_the_plan_crop_centre_when_there_is_one() {
        let crop = PlanCrop {
            min: [-1000.0, -1000.0],
            max: [3000.0, 3000.0],
            offsets_mm: super::super::rls_plan::CropOffsets { left: 0.0, right: 0.0, bottom: 0.0, top: 0.0 },
        };
        let p = &three_ds(&sheet([0.0, 1.0, -1.0]), &room(), Some(&crop), &[])[0];
        assert!((p.eye[0] - 1000.0).abs() < 1e-6, "{:?}", p.eye);
    }
}
