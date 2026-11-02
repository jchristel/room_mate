//! Which wall of a room each elevation on a sheet looks at.
//!
//! A library or blueprint sheet carries, per elevation viewport, where its marker sits, which
//! index of the marker the view is, the elevation line in the model, the direction it looks and
//! how far (`DataViewElevation`). The room is a ring. Nothing stored says which wall a view
//! shows, so it is **worked out here, at read time, and stored nowhere**: a room that moves is
//! followed.
//!
//! **The rule.** A room edge is *faced* by an elevation when the room side of the wall is the
//! side the viewer sees (the edge's inward normal points back at the viewer) and some part of
//! the edge lies inside the view's footprint: between the two ends of the elevation line and no
//! further from it than the view looks. `square_on` is the cosine of the angle between the wall
//! and the view (1 = looked at straight on, near 0 = grazing); edges below
//! [`MIN_SQUARE_ON`] are not faced, and a faced edge shorter than [`MIN_VISIBLE_MM`] inside the
//! footprint is not reported (a corner clipped by the view's edge is not a wall it shows).
//!
//! **Occlusion is not judged.** A nib or a wall further along the same line of sight can hide a
//! wall behind it; the answer is what the view *covers*, not what it shows unobstructed.
//!
//! **The frame is the model's own.** The sheet was exported from one document, in that
//! document's coordinates, so the room (read in the project's frame) is put back through the
//! inverse of that model's placement, as a task is.
//!
//! **Signal, not error.** An elevation with no geometry (a sheet exported before the markers
//! were read), a view looking straight down, or a room that is not on the server is a finding
//! beside the rest of the answer, never a refusal.

use serde::Serialize;
use serde_json::Value;

use crate::sheets::{SheetRecord, SheetRoom};
use crate::state::AppState;

use super::blueprint_capture::FT_TO_MM;
use super::blueprint_geometry::{dist, dot, edges, signed_area, simplify, sub, unit, Pt, FINE_TOLERANCE_MM};
use super::room_locator::RoomRef;
use super::rooms::{assemble_rooms, RoomScope};
use super::task_build::project_to_model;
use super::ServiceError;

/// Edges whose normal is within about 84 degrees of the viewer count as faced.
pub const MIN_SQUARE_ON: f64 = 0.1;
/// A faced edge must have at least this much of its length inside the view's footprint.
pub const MIN_VISIBLE_MM: f64 = 100.0;

/// One wall an elevation looks at.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Face {
    /// Index of the edge in the room's outline (the 30 mm outline, counter-clockwise), which
    /// is the order `edges` and the blueprint viewer use.
    pub edge: usize,
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub edge_mm: f64,
    /// How much of the edge lies inside the view's footprint.
    pub visible_mm: f64,
    /// Cosine of the angle between the wall and the view: 1 straight on, near 0 grazing.
    pub square_on: f64,
    /// The closest the covered part of the wall comes to the elevation line.
    pub nearest_mm: f64,
}

/// One elevation viewport and the walls it covers, largest first.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Elevation {
    pub view_id: i64,
    /// The viewport's centre on the sheet, in the sheet's units, as exported.
    pub sheet_centre: Option<[f64; 2]>,
    pub marker_id: Option<i64>,
    pub marker_index: Option<i64>,
    pub line_start: Option<[f64; 3]>,
    pub line_end: Option<[f64; 3]>,
    /// Unit vector in plan the view looks along (away from the viewer).
    pub looks: Option<[f64; 2]>,
    /// How far the view looks, mm. Absent when the view is not depth limited or it is unknown.
    pub depth_mm: Option<f64>,
    pub faces: Vec<Face>,
    pub findings: Vec<String>,
}

/// An elevation marker and the views of it on the sheet, by index.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Marker {
    pub marker_id: i64,
    pub location: Option<[f64; 3]>,
    /// `(index on the marker, view id)`, in index order.
    pub views: Vec<(i64, i64)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct Elevations {
    pub room: Option<SheetRoom>,
    pub markers: Vec<Marker>,
    pub elevations: Vec<Elevation>,
    pub findings: Vec<String>,
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

fn point3(v: Option<&Value>) -> Option<[f64; 3]> {
    let v = v?;
    Some([num(v, "x")?, num(v, "y")?, num(v, "z")?])
}

/// Walk the edges of `ring` (counter-clockwise, mm) for those an elevation faces.
pub fn faces(ring: &[Pt], start: Pt, end: Pt, looks: Pt, depth: Option<f64>) -> Vec<Face> {
    let width = dist(start, end);
    if width <= 0.0 {
        return Vec::new();
    }
    let r = unit(sub(end, start));
    let mut out = Vec::new();
    for (i, e) in edges(ring).into_iter().enumerate() {
        let square_on = -dot(e.n, looks);
        if square_on < MIN_SQUARE_ON {
            continue;
        }
        // Along the edge by u in 0..=1: how far across the line, and how far in front of it.
        let d = sub(e.b, e.a);
        let p = sub(e.a, start);
        let (mut u0, mut u1) = (0.0, 1.0);
        let inside = clip(dot(p, r), dot(d, r), 0.0, width, &mut u0, &mut u1)
            && clip(dot(p, looks), dot(d, looks), 0.0, depth.unwrap_or(f64::INFINITY), &mut u0, &mut u1);
        if !inside {
            continue;
        }
        let visible = (u1 - u0) * e.len;
        if visible < MIN_VISIBLE_MM {
            continue;
        }
        let t = |u: f64| dot(p, looks) + u * dot(d, looks);
        out.push(Face {
            edge: i,
            a: [e.a.0, e.a.1],
            b: [e.b.0, e.b.1],
            edge_mm: e.len,
            visible_mm: visible,
            square_on,
            nearest_mm: t(u0).min(t(u1)).max(0.0),
        });
    }
    out.sort_by(|x, y| y.visible_mm.total_cmp(&x.visible_mm));
    out
}

/// Narrow `[u0, u1]` to where `p0 + u * dp` stays within `lo..=hi`; false when nothing is left.
fn clip(p0: f64, dp: f64, lo: f64, hi: f64, u0: &mut f64, u1: &mut f64) -> bool {
    if dp.abs() < 1e-9 {
        return p0 >= lo && p0 <= hi;
    }
    let (mut a, mut b) = ((lo - p0) / dp, (hi - p0) / dp);
    if a > b {
        std::mem::swap(&mut a, &mut b);
    }
    *u0 = u0.max(a);
    *u1 = u1.min(b);
    *u0 < *u1
}

/// Read every elevation viewport of a sheet against a room outline (counter-clockwise, mm,
/// in the sheet's model frame).
pub fn read_sheet(sheet: &Value, ring: &[Pt]) -> (Vec<Marker>, Vec<Elevation>) {
    let mut elevations = Vec::new();
    for port in sheet.get("view_ports").and_then(Value::as_array).into_iter().flatten() {
        if port.get("vp_type").and_then(Value::as_str) != Some("elevation") {
            continue;
        }
        let Some(view) = port.get("view") else { continue };
        let mut findings = Vec::new();
        let line_start = point3(view.get("line_start"));
        let line_end = point3(view.get("line_end"));
        let direction = point3(view.get("view_direction"));
        let marker_id = view.get("marker_id").and_then(Value::as_i64);
        let marker_index = view.get("marker_index").and_then(Value::as_i64);
        if marker_id.is_none() {
            findings.push("the sheet has no marker for this view (exported before markers were read)".into());
        }
        // A far clip of "none" means the view is not limited by its crop box depth.
        let limited = view.get("far_clip_mode").and_then(Value::as_i64) != Some(0);
        let depth_mm = if limited { num(view, "view_depth") } else { None };
        let mut looks = None;
        if let Some(d) = direction {
            let h = d[0].hypot(d[1]);
            if h < 1e-6 {
                findings.push("the view looks straight up or down, so it faces no wall".into());
            } else {
                // The view direction points at the viewer; the view looks the other way.
                looks = Some([-d[0] / h, -d[1] / h]);
            }
        }
        let mut found = Vec::new();
        match (line_start, line_end, looks) {
            (Some(s), Some(e), Some(l)) => {
                found = faces(ring, (s[0], s[1]), (e[0], e[1]), (l[0], l[1]), depth_mm);
                if found.is_empty() {
                    findings.push("no wall of the room lies in this view".into());
                }
            }
            _ => findings.push("the elevation line or direction was not exported".into()),
        }
        elevations.push(Elevation {
            view_id: port.get("view_id").and_then(Value::as_i64).unwrap_or(-1),
            sheet_centre: port.get("centre_point").and_then(|c| Some([num(c, "x")?, num(c, "y")?])),
            marker_id,
            marker_index,
            line_start,
            line_end,
            looks,
            depth_mm,
            faces: found,
            findings,
        });
    }

    let mut markers: Vec<Marker> = Vec::new();
    for ev in &elevations {
        let (Some(id), Some(index)) = (ev.marker_id, ev.marker_index) else {
            continue;
        };
        let location = sheet_marker_location(sheet, ev.view_id);
        match markers.iter_mut().find(|m| m.marker_id == id) {
            Some(m) => m.views.push((index, ev.view_id)),
            None => markers.push(Marker { marker_id: id, location, views: vec![(index, ev.view_id)] }),
        }
    }
    for m in &mut markers {
        m.views.sort();
    }
    (markers, elevations)
}

fn sheet_marker_location(sheet: &Value, view_id: i64) -> Option<[f64; 3]> {
    sheet
        .get("view_ports")?
        .as_array()?
        .iter()
        .find(|p| p.get("view_id").and_then(Value::as_i64) == Some(view_id))
        .and_then(|p| point3(p.get("view")?.get("marker_location")))
}

/// The answer for one library sheet: its room's outline in the sheet's model frame, then
/// [`read_sheet`]. A sheet with no room, or whose room is not on the server, still lists its
/// elevations (and their markers) with a finding in place of the walls.
pub fn for_sheet(state: &AppState, project: &str, record: &SheetRecord) -> Result<Elevations, ServiceError> {
    let mut out = Elevations { room: record.room.clone(), ..Default::default() };
    let ring = match &record.room {
        None => {
            out.findings.push("the sheet has no room, so there are no walls to face".into());
            Vec::new()
        }
        Some(room) => match model_room_ring(state, project, room)? {
            Some(r) => r,
            None => {
                out.findings.push("the sheet's room is not on the server".into());
                Vec::new()
            }
        },
    };
    let (markers, elevations) = read_sheet(&record.sheet, &ring);
    out.markers = markers;
    out.elevations = elevations;
    Ok(out)
}

/// A sheet's room as a ring in the TARGET MODEL's frame (the sheet is read in model coordinates),
/// counter-clockwise, at the fine tolerance. Not `blueprint_capture::room_ring`, which answers in the
/// project's plan frame.
pub fn model_room_ring(state: &AppState, project: &str, room: &SheetRoom) -> Result<Option<Vec<Pt>>, ServiceError> {
    let target = RoomRef { model_id: room.model_id.clone(), room_id: room.room_id.clone() };
    let Some(rooms) = assemble_rooms(state, &RoomScope { project: Some(project), ..Default::default() })? else {
        return Ok(None);
    };
    let Some(found) = rooms.rooms.iter().find(|r| r.model_id == target.model_id && r.room.id == target.room_id) else {
        return Ok(None);
    };
    let Some(outer) = found.room.loops.first() else {
        return Ok(None);
    };
    let to_model = project_to_model(state, project, &room.model_id)?;
    let mut ring: Vec<Pt> = outer.points.iter().map(|p| to_model.point((p.x * FT_TO_MM, p.y * FT_TO_MM))).collect();
    if signed_area(&ring) < 0.0 {
        ring.reverse();
    }
    Ok(Some(simplify(&ring, FINE_TOLERANCE_MM)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 4 m by 3 m, counter-clockwise from the origin: edges 0 bottom, 1 right, 2 top, 3 left.
    fn room() -> Vec<Pt> {
        vec![(0.0, 0.0), (4000.0, 0.0), (4000.0, 3000.0), (0.0, 3000.0)]
    }

    #[test]
    fn a_view_from_inside_faces_the_wall_it_looks_at() {
        // Standing at y = 1500 looking at the bottom wall (towards -y), 3 m wide, 2 m deep.
        let f = faces(&room(), (500.0, 1500.0), (3500.0, 1500.0), (0.0, -1.0), Some(2000.0));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].edge, 0);
        assert!((f[0].visible_mm - 3000.0).abs() < 1e-6);
        assert!((f[0].square_on - 1.0).abs() < 1e-9);
        assert!((f[0].nearest_mm - 1500.0).abs() < 1e-6);
    }

    #[test]
    fn the_view_is_clipped_by_its_width_and_its_depth() {
        let narrow = faces(&room(), (1000.0, 1500.0), (2000.0, 1500.0), (0.0, -1.0), Some(2000.0));
        assert!((narrow[0].visible_mm - 1000.0).abs() < 1e-6);
        // The wall is 1500 mm from the line: a view that looks 1000 mm does not reach it.
        assert!(faces(&room(), (500.0, 1500.0), (3500.0, 1500.0), (0.0, -1.0), Some(1000.0)).is_empty());
        // Not depth limited, it does.
        assert_eq!(faces(&room(), (500.0, 1500.0), (3500.0, 1500.0), (0.0, -1.0), None).len(), 1);
    }

    #[test]
    fn the_wall_behind_the_viewer_and_the_grazing_walls_are_not_faced() {
        let f = faces(&room(), (500.0, 1500.0), (3500.0, 1500.0), (0.0, -1.0), None);
        assert!(f.iter().all(|x| x.edge != 2), "the top wall is behind the viewer");
        assert!(f.iter().all(|x| x.edge != 1 && x.edge != 3), "the side walls are edge on");
    }

    #[test]
    fn a_turned_view_faces_the_wall_at_an_angle() {
        // 45 degrees to the bottom wall: still faced, square_on is cos 45.
        let looks = (-(0.5f64).sqrt(), -(0.5f64).sqrt());
        let r = (looks.1, -looks.0); // to the viewer's right
        let c = (2000.0, 1500.0);
        let f = faces(
            &room(),
            (c.0 - r.0 * 3000.0, c.1 - r.1 * 3000.0),
            (c.0 + r.0 * 3000.0, c.1 + r.1 * 3000.0),
            looks,
            None,
        );
        assert!(f.iter().any(|x| x.edge == 0 && (x.square_on - 0.5f64.sqrt()).abs() < 1e-9));
        assert!(f.iter().any(|x| x.edge == 3), "and the left wall, which is also at 45 degrees");
    }

    #[test]
    fn the_sheet_is_read_into_markers_by_index() {
        let view = |id: i64, index: i64, d: (f64, f64)| {
            json!({
                "vp_type": "elevation", "view_id": id, "centre_point": {"x": 1.0, "y": 2.0},
                "view": {
                    "marker_id": 77, "marker_index": index,
                    "marker_location": {"x": 2000.0, "y": 1500.0, "z": 0.0},
                    "view_direction": {"x": d.0, "y": d.1, "z": 0.0},
                    "line_start": {"x": 500.0, "y": 1500.0, "z": 0.0},
                    "line_end": {"x": 3500.0, "y": 1500.0, "z": 0.0},
                    "view_depth": 2500.0, "far_clip_mode": 1
                }
            })
        };
        // The bottom wall is looked at by a view whose direction points at the viewer (+y).
        let sheet = json!({ "view_ports": [view(11, 1, (0.0, 0.0)), view(10, 0, (0.0, 1.0))] });
        let (markers, elevations) = read_sheet(&sheet, &room());
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].views, vec![(0, 10), (1, 11)]);
        assert_eq!(markers[0].location, Some([2000.0, 1500.0, 0.0]));
        let looking = elevations.iter().find(|e| e.view_id == 10).unwrap();
        assert_eq!(looking.faces.len(), 1);
        assert_eq!(looking.faces[0].edge, 0);
        let down = elevations.iter().find(|e| e.view_id == 11).unwrap();
        assert!(down.faces.is_empty() && !down.findings.is_empty());
    }

    #[test]
    fn a_sheet_exported_before_markers_were_read_says_so() {
        let sheet = json!({ "view_ports": [{ "vp_type": "elevation", "view_id": 5, "view": {} }] });
        let (markers, elevations) = read_sheet(&sheet, &room());
        assert!(markers.is_empty());
        assert_eq!(elevations.len(), 1);
        assert!(elevations[0].findings.len() >= 2);
    }
}
