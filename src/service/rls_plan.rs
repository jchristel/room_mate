//! What a create-RLS task tells Revit beyond "copy this sheet": the sheet size, and where each
//! elevation marker and its views go in THIS room (`docs`: the RLS methodology tab).
//!
//! **The template sheet is the source and the room is the target.** A library sheet was made for
//! one room and carries, per elevation viewport, the marker, the view's index on it, the
//! elevation line, the direction and the depth (duHast `DataViewElevation`). Making the same
//! sheet for another room needs those put into that room, which is what this does.
//!
//! **Transfer is by the walls, never by coordinates.** Both rooms are read at the coarse outline
//! (the jogs under 350 mm taken out, as blueprints are), the template's wall that an elevation
//! faces (`elevation_facing`) is mapped to the target's by the cyclic shift that fits the shapes
//! (`blueprint_resolve::best_shift`, no reflection), and then:
//! - the marker keeps its **fraction along** the wall and its **distance from** the wall,
//! - each view's line ends and direction keep their offsets from the marker, measured in the
//!   frame of the wall that view faces (so a view looking at another wall turns with that wall),
//! - the **depth** keeps its length, because a view is a stretch of wall to show and not a share
//!   of the room. `depth_to_wall_mm` says how far the faced wall is from the line, for a person
//!   who wants the view to stop there ("a simple command: to room boundary").
//!
//! **Rooms with a different number of coarse edges transfer nothing** (the blueprint rule, for
//! the same reason): the result says so and the sheet is still made, without elevation
//! placement. A request may override a view's depth, or ask for it to the wall; the draft is
//! built by the same read the task is made from (`POST .../tasks/rls-preview`), so a preview is
//! the task, not a second opinion.
//!
//! Positions are in the TARGET model's own frame, mm, like every placement in a task. The
//! plug-in does not read them yet.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::blueprint_geometry::{coarse_outline, dot, edges, sub, Edge, Pt};
use super::blueprint_resolve::best_shift;
use super::elevation_facing::{read_sheet, Elevation};
use super::rls_tags::{TagPlan, TagRequest};

/// The sizes a sheet can be asked for; the plug-in maps each to a title block family.
pub const SIZES: [&str; 4] = ["a0", "a1", "a2", "a3"];

/// A person's changes to what was worked out, per view.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct DepthOverride {
    pub view_id: i64,
    /// A depth in mm. Wins over `to_wall`.
    #[serde(default)]
    pub depth_mm: Option<f64>,
    /// Look as far as the wall the view faces.
    #[serde(default)]
    pub to_wall: bool,
}

/// What a request adds to making an RLS from a library sheet.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct RlsRequest {
    /// `a0` to `a3`. Absent: the template's own size.
    #[serde(default)]
    pub sheet_size: Option<String>,
    #[serde(default)]
    pub depths: Vec<DepthOverride>,
    /// Replace room layout sheets the room already has.
    #[serde(default)]
    pub overwrite_rls: bool,
    /// Work out which items are tagged where (`rls_tags`) and with what a person decided. Absent:
    /// no tag plan, which also skips reading the room's FF&E.
    #[serde(default)]
    pub tags: Option<TagRequest>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewPlan {
    pub view_id: i64,
    /// Index of the view on its marker.
    pub marker_index: i64,
    pub line_start: [f64; 2],
    pub line_end: [f64; 2],
    /// Unit vector in plan the view looks along.
    pub looks: [f64; 2],
    pub depth_mm: f64,
    /// How far the faced wall is from the line along `looks`; absent when the view meets it too
    /// obliquely to say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_to_wall_mm: Option<f64>,
    /// The target room's coarse edge this view faces.
    pub faces_edge: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarkerPlan {
    pub marker_id: i64,
    pub location: [f64; 2],
    pub views: Vec<ViewPlan>,
}

/// Where the room tag goes in this room: the model frame, mm. Present when the room tag rule was
/// written against a room boundary (plan decision 27); otherwise the plug-in applies the rule's
/// offset in the plan's axes, as before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomTagPlan {
    pub head: [f64; 2],
    pub rule_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RlsPlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet_size: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<MarkerPlan>,
    /// Which items are tagged in which view, and which are left for a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<TagPlan>,
    /// The room tag's place from its rule's boundary recipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_tag: Option<RoomTagPlan>,
}

/// `a1` for ` A1 `, or why it is not a size.
pub fn check_size(size: &str) -> Result<String, String> {
    let s = size.trim().to_ascii_lowercase();
    if SIZES.contains(&s.as_str()) {
        Ok(s)
    } else {
        Err(format!("'{size}' is not a sheet size (a0, a1, a2 or a3)"))
    }
}

/// The ISO size a measured sheet is, as `a0`..`a3`: the title block's bounding box is a few mm off
/// its nominal paper (A3 measured 425.8 x 297), so each side may be this share away.
const SIZE_TOLERANCE: f64 = 0.03;

pub fn size_token(width_mm: f64, height_mm: f64) -> Option<&'static str> {
    let (long, short) = if width_mm >= height_mm {
        (width_mm, height_mm)
    } else {
        (height_mm, width_mm)
    };
    [
        ("a0", 1189.0, 841.0),
        ("a1", 841.0, 594.0),
        ("a2", 594.0, 420.0),
        ("a3", 420.0, 297.0),
    ]
    .into_iter()
    .find(|(_, l, sh)| (long - l).abs() <= l * SIZE_TOLERANCE && (short - sh).abs() <= sh * SIZE_TOLERANCE)
    .map(|(token, _, _)| token)
}

/// Whether a sheet shows any elevation viewport.
pub fn has_elevations(sheet: &Value) -> bool {
    sheet
        .get("view_ports")
        .and_then(Value::as_array)
        .is_some_and(|ports| ports.iter().any(|p| p.get("vp_type").and_then(Value::as_str) == Some("elevation")))
}

/// A point as `(along, off)` in an edge's frame: along it from `a`, and into the room.
fn in_frame(p: Pt, e: &Edge) -> Pt {
    let d = sub(p, e.a);
    (dot(d, e.t), dot(d, e.n))
}

/// Components `(t, n)` of a vector in an edge's frame.
fn comps(v: Pt, e: &Edge) -> Pt {
    (dot(v, e.t), dot(v, e.n))
}

fn from_comps(c: Pt, e: &Edge) -> Pt {
    (e.t.0 * c.0 + e.n.0 * c.1, e.t.1 * c.0 + e.n.1 * c.1)
}

/// Put every elevation of `sheet` (made for `template`) into `target`. Both rings are the FINE
/// outline, counter-clockwise, in their own model's frame; the coarse outline is taken here.
/// Returns what could be placed and a message for what could not.
pub fn transfer(sheet: &Value, template: &[Pt], target: &[Pt], request: &RlsRequest) -> (Vec<MarkerPlan>, Vec<String>) {
    let (tc, gc) = (coarse_outline(template), coarse_outline(target));
    let mut notes = Vec::new();
    let (sheet_markers, elevations) = read_sheet(sheet, &tc);
    if elevations.is_empty() {
        return (Vec::new(), notes);
    }
    if tc.len() != gc.len() || tc.len() < 3 {
        notes.push(format!(
            "the elevations were not placed: the template room has {} edges and this room {} (the coarse outlines must have the same number)",
            tc.len(),
            gc.len()
        ));
        return (Vec::new(), notes);
    }
    let (shift, _) = best_shift(&tc, &gc);
    let (te, ge) = (edges(&tc), edges(&gc));

    // The views that can be placed, with the wall each faces.
    let mut placeable: Vec<(&Elevation, usize, i64, i64)> = Vec::new();
    for ev in &elevations {
        let (Some(marker_id), Some(index)) = (ev.marker_id, ev.marker_index) else {
            notes.push(format!("view {}: it has no marker in the sheet, so it was not placed", ev.view_id));
            continue;
        };
        match ev.faces.first() {
            Some(face) if ev.line_start.is_some() && ev.line_end.is_some() && ev.looks.is_some() => {
                placeable.push((ev, face.edge, marker_id, index));
            }
            _ => notes.push(format!(
                "view {}: it faces no wall of the template room, so it was not placed",
                ev.view_id
            )),
        }
    }

    let mut markers: Vec<MarkerPlan> = Vec::new();
    // Each marker is placed once, from its lowest-numbered view, so its views share one point.
    let mut ids: Vec<i64> = placeable.iter().map(|p| p.2).collect();
    ids.sort_unstable();
    ids.dedup();
    for marker_id in ids {
        let mut views: Vec<&(&Elevation, usize, i64, i64)> = placeable.iter().filter(|p| p.2 == marker_id).collect();
        views.sort_by_key(|p| p.3);
        let (first, first_edge, ..) = *views[0];
        let m = sheet_markers
            .iter()
            .find(|sm| sm.marker_id == marker_id)
            .and_then(|sm| sm.location)
            .map_or_else(|| first.line_midpoint(), |l| (l[0], l[1]));
        let t_edge = &te[first_edge];
        let g_edge = &ge[(first_edge + shift) % ge.len()];
        let (along, off) = in_frame(m, t_edge);
        let along_target = if t_edge.len > 0.0 { along / t_edge.len * g_edge.len } else { along };
        let location = g_edge.point(along_target, off);

        let mut plans = Vec::new();
        for &&(ev, edge, _, index) in &views {
            let (t_e, g_i) = (&te[edge], (edge + shift) % ge.len());
            let g_e = &ge[g_i];
            let (Some(s), Some(e), Some(l)) = (ev.line_start, ev.line_end, ev.looks) else {
                continue;
            };
            let from_m = |p: Pt| from_comps(comps(sub(p, m), t_e), g_e);
            let (ds, de) = (from_m((s[0], s[1])), from_m((e[0], e[1])));
            let looks = from_comps(comps((l[0], l[1]), t_e), g_e);
            let line_start = (location.0 + ds.0, location.1 + ds.1);
            let line_end = (location.0 + de.0, location.1 + de.1);
            let mid = ((line_start.0 + line_end.0) / 2.0, (line_start.1 + line_end.1) / 2.0);
            // Distance from the line to the faced wall, along the way the view looks.
            let toward = -dot(looks, g_e.n);
            let to_wall = (toward > 0.1).then(|| dot(sub(mid, g_e.a), g_e.n) / toward).filter(|d| *d >= 0.0);
            let mut depth = ev.depth_mm.unwrap_or_else(|| to_wall.unwrap_or(0.0));
            if let Some(o) = request.depths.iter().find(|o| o.view_id == ev.view_id) {
                match (o.depth_mm, o.to_wall, to_wall) {
                    (Some(d), _, _) if d > 0.0 => depth = d,
                    (_, true, Some(w)) => depth = w,
                    (_, true, None) => {
                        notes.push(format!("view {}: it meets its wall too obliquely to look as far as it", ev.view_id))
                    }
                    _ => {}
                }
            }
            plans.push(ViewPlan {
                view_id: ev.view_id,
                marker_index: index,
                line_start: [line_start.0, line_start.1],
                line_end: [line_end.0, line_end.1],
                looks: [looks.0, looks.1],
                depth_mm: depth,
                depth_to_wall_mm: to_wall,
                faces_edge: g_i,
            });
        }
        markers.push(MarkerPlan { marker_id, location: [location.0, location.1], views: plans });
    }
    (markers, notes)
}

impl Elevation {
    /// The middle of the elevation line, for a marker whose own point was not exported. A view
    /// with no line was not placeable before this is asked.
    fn line_midpoint(&self) -> Pt {
        match (self.line_start, self.line_end) {
            (Some(s), Some(e)) => ((s[0] + e[0]) / 2.0, (s[1] + e[1]) / 2.0),
            _ => (0.0, 0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The measured A3 of the Rumpus sheet (425.8 x 297) is A3, either way round; a size between
    /// two papers is none.
    #[test]
    fn test_a_measured_sheet_is_the_nearest_iso_size_or_none() {
        assert_eq!(size_token(425.8, 297.0), Some("a3"));
        assert_eq!(size_token(297.0, 420.0), Some("a3"));
        assert_eq!(size_token(841.0, 1189.0), Some("a0"));
        assert_eq!(size_token(500.0, 350.0), None);
    }

    fn rect(w: f64, h: f64) -> Vec<Pt> {
        vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
    }

    /// A sheet whose one marker sits at the middle of a 4 m x 3 m room with its four views
    /// looking at the four walls, each 2.5 m deep and 2 m wide.
    fn sheet() -> Value {
        let view = |id: i64, index: i64, look: (f64, f64), line: ([f64; 2], [f64; 2])| {
            json!({ "vp_type": "elevation", "view_id": id, "view": {
                "marker_id": 7, "marker_index": index,
                "marker_location": {"x": 2000.0, "y": 1500.0, "z": 0.0},
                "view_direction": {"x": -look.0, "y": -look.1, "z": 0.0},
                "line_start": {"x": line.0[0], "y": line.0[1], "z": 0.0},
                "line_end": {"x": line.1[0], "y": line.1[1], "z": 0.0},
                "view_depth": 2500.0, "far_clip_mode": 1 } })
        };
        // Each line is 0.5 m in front of the marker, towards its wall, and sees 2 m of it.
        json!({ "view_ports": [
            view(1, 0, (0.0, -1.0), ([1000.0, 1000.0], [3000.0, 1000.0])),
            view(2, 1, (1.0, 0.0), ([2500.0, 500.0], [2500.0, 2500.0])),
            view(3, 2, (0.0, 1.0), ([3000.0, 2000.0], [1000.0, 2000.0])),
            view(4, 3, (-1.0, 0.0), ([1500.0, 2500.0], [1500.0, 500.0])),
        ]})
    }

    #[test]
    fn a_bigger_room_keeps_the_marker_in_the_middle_and_the_depth_as_it_was() {
        let (markers, notes) = transfer(&sheet(), &rect(4000.0, 3000.0), &rect(6000.0, 4000.0), &RlsRequest::default());
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(markers.len(), 1);
        let m = &markers[0];
        // Marker 2 m along the 4 m wall and 1.5 m off it: the middle along, the same distance off.
        // Placed from the lowest view (index 0, which faces the bottom wall), so x is half of 6 m.
        assert!((m.location[0] - 3000.0).abs() < 1e-6, "{:?}", m.location);
        assert!((m.location[1] - 1500.0).abs() < 1e-6);
        assert_eq!(m.views.len(), 4);
        let bottom = m.views.iter().find(|v| v.view_id == 1).unwrap();
        assert!((bottom.depth_mm - 2500.0).abs() < 1e-9);
        assert_eq!(bottom.faces_edge, 0);
        assert!((bottom.looks[1] + 1.0).abs() < 1e-9);
        // The wall is 1 m from the line (the marker is 1.5 m off it, the line 0.5 m nearer), and
        // that is what "to the wall" gives.
        assert!((bottom.depth_to_wall_mm.unwrap() - 1000.0).abs() < 1e-6);
        assert!(
            (bottom.line_start[1] - 1000.0).abs() < 1e-6,
            "the line keeps its place relative to the marker"
        );
        // The line keeps its 2 m width, centred on the marker.
        assert!((bottom.line_end[0] - bottom.line_start[0] - 2000.0).abs() < 1e-6);
    }

    #[test]
    fn a_room_turned_a_quarter_turn_turns_the_views_with_it() {
        // The same 4 x 3 room, wound from another corner and turned 90 degrees: the views still
        // face the walls they faced, whatever their coordinates.
        let turned: Vec<Pt> = rect(4000.0, 3000.0).iter().map(|&(x, y)| (-y + 10_000.0, x + 5_000.0)).collect();
        let (markers, notes) = transfer(&sheet(), &rect(4000.0, 3000.0), &turned, &RlsRequest::default());
        assert!(notes.is_empty(), "{notes:?}");
        let m = &markers[0];
        let bottom = m.views.iter().find(|v| v.view_id == 1).unwrap();
        // The bottom wall became the one at x = 10000 - 0... look straight at it, whatever that is.
        let g = edges(&coarse_outline(&turned));
        let wall = &g[bottom.faces_edge];
        assert!(
            (dot(wall.n, (-bottom.looks[0], -bottom.looks[1])) - 1.0).abs() < 1e-9,
            "the view looks straight at its wall"
        );
        // And the marker is inside the room, 1.5 m off that wall.
        assert!((dot(sub((m.location[0], m.location[1]), wall.a), wall.n) - 1500.0).abs() < 1e-6);
    }

    #[test]
    fn depth_can_be_overridden_to_a_length_or_to_the_wall() {
        let request = RlsRequest {
            sheet_size: None,
            overwrite_rls: false,
            tags: None,
            depths: vec![
                DepthOverride { view_id: 1, depth_mm: Some(900.0), to_wall: false },
                DepthOverride { view_id: 2, depth_mm: None, to_wall: true },
            ],
        };
        let (markers, _) = transfer(&sheet(), &rect(4000.0, 3000.0), &rect(4000.0, 3000.0), &request);
        let v = |id| markers[0].views.iter().find(|x| x.view_id == id).unwrap().clone();
        assert_eq!(v(1).depth_mm, 900.0);
        assert!((v(2).depth_mm - v(2).depth_to_wall_mm.unwrap()).abs() < 1e-9);
        assert_eq!(v(3).depth_mm, 2500.0, "an untouched view keeps the template's depth");
    }

    #[test]
    fn rooms_with_different_edge_counts_place_nothing_and_say_so() {
        let l_room = vec![
            (0.0, 0.0),
            (4000.0, 0.0),
            (4000.0, 1500.0),
            (2000.0, 1500.0),
            (2000.0, 3000.0),
            (0.0, 3000.0),
        ];
        let (markers, notes) = transfer(&sheet(), &rect(4000.0, 3000.0), &l_room, &RlsRequest::default());
        assert!(markers.is_empty());
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("4 edges") && notes[0].contains("6"));
    }

    #[test]
    fn a_sheet_with_no_elevations_has_nothing_to_say() {
        let (markers, notes) = transfer(
            &json!({ "view_ports": [] }),
            &rect(4000.0, 3000.0),
            &rect(4000.0, 3000.0),
            &RlsRequest::default(),
        );
        assert!(markers.is_empty() && notes.is_empty());
    }

    #[test]
    fn sizes_are_checked_and_lowercased() {
        assert_eq!(check_size(" A1 ").unwrap(), "a1");
        assert!(check_size("a4").is_err());
    }
}
