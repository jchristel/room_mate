//! A room layout sheet SET for adjacent rooms (multi-room RLS), the server's first half: which rooms, which library
//! sheet, and the views the set gets with their size on paper. A preview: nothing is stored (2026-11-13; the plan is
//! the "Road map plan" tab of the blueprints plan, "Review 2026-11-13: multi-room RLS").
//!
//! **The rooms must be one adjacent group** by the server's own adjacency (`service::adjacency`: a shared wall, same
//! level, the project's default wall tolerance), in one model, so one plan covers them. A set that is not is a
//! finding naming its groups, with no views: advice for the picker, never an error.
//!
//! **A set starts from a library sheet, matched as if the set were one room**: the smallest rectangle round the
//! convex hull of all the rooms' points, their summed area and summed item count. That is the figure the extractor
//! stores for a library sheet that covers several rooms (`room_m/room_figures.py`), so a set and such a sheet are
//! measured alike and `sheet_suggest` ranks them with no new rule. The sheet supplies the scales and the plan's
//! framing; the layout of views on sheets is a person's, later (the composer).
//!
//! **The views**, all in the rooms' model's own frame, mm:
//! - ONE plan round every room: the template crop's offsets from its room's box, kept round the set's box
//!   (`rls_plan::plan_crop_turned`), turned by the set's walls together (`rls_turn::set_turn`).
//! - FOUR elevations per room, one per side of the room's own squared box (its `dominant_angle_deg`, so a room at
//!   another angle than the set is still elevated square): the marker at the box's centre, each view across the
//!   side's full length, looking at it from the marker, as deep as the wall plus [`DEPTH_PAST_WALL_MM`], as tall as
//!   the room's clear height (`connectivity::clear_heights`, else [`DEFAULT_HEIGHT_MM`] and a finding). The four use
//!   marker slots 0, 2, 4, 6 looking along the box's -x, +y, +x, -y, as every House A template marker does.
//! - One schedule per room and one 3D view, at the template's size on paper: a schedule's height is its rows and a
//!   3D view's its projection, which only Revit knows, so both are marked `estimate`.
//!
//! The lead room, whose parameters fill the set's own names on the plug-in side, is the largest by area (a tie: the
//! first asked for).

use std::collections::BTreeMap;

use geo::{ConvexHull, Coord, MultiPoint, Point};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::adjacency::{compute_adjacency_indexed, default_wall_max};
use super::blueprint_capture::qualify_room_in;
use super::blueprint_geometry::{coarse_outline, dominant_angle_deg, rotate, signed_area, Pt};
use super::connectivity::{clear_heights, HeightFrom};
use super::elevation_facing::model_room_ring_in;
use super::rls_plan::{plan_crop_turned, PlanCrop};
use super::rls_turn::set_turn;
use super::room_locator::RoomRef;
use super::sheet_suggest::{enclosing_rectangle, fit_percent, Context, Signature};
use super::task_build::finding;
use super::ServiceError;
use crate::sheets::{SheetRecord, SheetRoom};
use crate::state::AppState;

/// How far past the wall an elevation looks: enough to show what stands against it, not the room behind.
pub const DEPTH_PAST_WALL_MM: f64 = 300.0;

/// A room's clear height when no ceiling covers it and it states none.
pub const DEFAULT_HEIGHT_MM: f64 = 2700.0;

/// The plan's margin round the set when the template's crop or room cannot be read (the plug-in's `CROP_MARGIN_MM`).
pub const FALLBACK_CROP_MARGIN_MM: f64 = 600.0;

/// How many library sheets the answer offers.
const SUGGESTIONS: usize = 3;

const M2_PER_MM2: f64 = 1.0e-6;

/// The marker slots of the four elevations, with the side of the room's squared box each looks at, as the unit
/// direction it looks along in the box's own frame.
const SIDES: [(i64, Pt); 4] = [(0, (-1.0, 0.0)), (2, (0.0, 1.0)), (4, (1.0, 0.0)), (6, (0.0, -1.0))];

#[derive(Debug, Clone, Deserialize)]
pub struct SetRequest {
    pub room_ids: Vec<String>,
    /// The library sheet to start from; absent: the best match.
    #[serde(default)]
    pub sheet_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetRoom {
    pub room_id: String,
    pub model_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    pub area_m2: f64,
    pub height_mm: f64,
    /// `ceiling`, `room_property` or `default`.
    pub height_from: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetSuggestion {
    pub sheet_id: String,
    pub sheet_number: String,
    pub sheet_name: String,
    /// `sheet_suggest::fit_percent`: 100 is a twin.
    pub fit: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetPlanView {
    pub crop: PlanCrop,
    pub scale: f64,
    /// The crop at the scale, width then height, mm on paper (the view's own frame: turned with the crop).
    pub paper_mm: [f64; 2],
}

#[derive(Debug, Clone, Serialize)]
pub struct SetElevation {
    pub room_id: String,
    pub marker_index: i64,
    pub marker: [f64; 2],
    pub line_start: [f64; 2],
    pub line_end: [f64; 2],
    /// Unit vector in plan the view looks along.
    pub looks: [f64; 2],
    pub depth_mm: f64,
    pub height_mm: f64,
    pub scale: f64,
    pub paper_mm: [f64; 2],
}

#[derive(Debug, Clone, Serialize)]
pub struct SetSchedule {
    pub room_id: String,
    pub paper_mm: [f64; 2],
    pub estimate: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetThreeD {
    pub paper_mm: [f64; 2],
    pub estimate: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetViews {
    pub plan: SetPlanView,
    pub elevations: Vec<SetElevation>,
    pub schedules: Vec<SetSchedule>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub three_d: Option<SetThreeD>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetPreview {
    /// False when the rooms cannot make a set (unknown, in several models, not one adjacent group): `findings` say
    /// why and there are no views.
    pub valid: bool,
    pub rooms: Vec<SetRoom>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead_room: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
    pub suggestions: Vec<SetSuggestion>,
    /// The library sheet the views are worked out from: the one asked for, else the best match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sheet_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub views: Option<SetViews>,
    pub findings: Vec<Value>,
}

impl SetPreview {
    fn refused(rooms: Vec<SetRoom>, findings: Vec<Value>) -> Self {
        SetPreview {
            valid: false,
            rooms,
            lead_room: None,
            signature: None,
            suggestions: Vec::new(),
            sheet_id: None,
            views: None,
            findings,
        }
    }
}

/// The groups `n` items fall into, joined by `edges` (index pairs): each group's indices, in order.
fn groups(n: usize, edges: &[(usize, usize)]) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for &(a, b) in edges {
        let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
        if ra != rb {
            parent[ra.max(rb)] = ra.min(rb);
        }
    }
    let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        let r = root(&mut parent, i);
        by_root.entry(r).or_default().push(i);
    }
    by_root.into_values().collect()
}

/// The long and short sides of the smallest rectangle round every point of `rings`: the convex hull's, as the
/// extractor measures a sheet that covers several rooms.
pub fn set_rectangle(rings: &[Vec<Pt>]) -> Option<(f64, f64)> {
    let points: MultiPoint<f64> = rings.iter().flatten().map(|&(x, y)| Point::from(Coord { x, y })).collect();
    let hull: Vec<Pt> = points.convex_hull().exterior().coords().map(|c| (c.x, c.y)).collect();
    let hull = match hull.split_last() {
        Some((last, rest)) if rest.first() == Some(last) => rest.to_vec(),
        _ => hull,
    };
    enclosing_rectangle(&hull)
}

/// The paper size of a viewport of the template, width then height, mm (its box on the sheet, title left out).
fn paper_of(port: &Value) -> Option<[f64; 2]> {
    let b = port.get("bounding_box")?;
    let n = |k: &str| b.get(k).and_then(Value::as_f64);
    let (w, h) = (n("max_x")? - n("min_x")?, n("max_y")? - n("min_y")?);
    (w > 0.0 && h > 0.0).then_some([w, h])
}

fn ports_of<'a>(sheet: &'a Value, kind: &str) -> Vec<&'a Value> {
    sheet
        .get("view_ports")
        .and_then(Value::as_array)
        .map(|ports| {
            ports
                .iter()
                .filter(|p| {
                    p.get("vp_type")
                        .and_then(Value::as_str)
                        .is_some_and(|t| t.trim().eq_ignore_ascii_case(kind))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn scale_of(port: &Value) -> Option<f64> {
    port.get("view")?.get("view_scale")?.as_f64().filter(|s| *s > 0.0)
}

/// The four elevations of one room (`ring`: its fine outline, model frame, mm), each at `scale`, `height_mm` tall.
pub fn room_elevations(room_id: &str, ring: &[Pt], height_mm: f64, scale: f64) -> Vec<SetElevation> {
    let angle = dominant_angle_deg(&coarse_outline(ring));
    let local: Vec<Pt> = ring.iter().map(|&p| rotate(p, -angle)).collect();
    let (x0, x1) = local
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.0), b.max(p.0)));
    let (y0, y1) = local
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.1), b.max(p.1)));
    if !(x1 > x0 && y1 > y0) {
        return Vec::new();
    }
    let centre = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (half_w, half_h) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    let marker = rotate(centre, angle);
    SIDES
        .iter()
        .map(|&(index, looks)| {
            // across the side it looks at: its length is the other axis of the box, its distance this one's half
            let (width, to_wall) = if looks.0 != 0.0 { (2.0 * half_h, half_w) } else { (2.0 * half_w, half_h) };
            // the view's right: the direction it looks along turned a quarter clockwise
            let right = (looks.1, -looks.0);
            let at = |k: f64| rotate((centre.0 + right.0 * k, centre.1 + right.1 * k), angle);
            let (start, end) = (at(-width / 2.0), at(width / 2.0));
            let looks = rotate(looks, angle);
            SetElevation {
                room_id: room_id.to_string(),
                marker_index: index,
                marker: [marker.0, marker.1],
                line_start: [start.0, start.1],
                line_end: [end.0, end.1],
                looks: [looks.0, looks.1],
                depth_mm: to_wall + DEPTH_PAST_WALL_MM,
                height_mm,
                scale,
                paper_mm: [width / scale, height_mm / scale],
            }
        })
        .collect()
}

/// What a set of `request.room_ids` would be, against the project's library `sheets` (the caller leaves out those of
/// blueprints not in use, as `/sheets/solve` does). `ctx` holds the project's rooms and item counts.
pub fn preview(
    state: &AppState,
    project: &str,
    ctx: &Context,
    sheets: &[SheetRecord],
    request: &SetRequest,
) -> Result<SetPreview, ServiceError> {
    let rooms_read = ctx.rooms();
    let mut findings = Vec::new();

    let Some(refs) = qualify_all(rooms_read, &request.room_ids, &mut findings)? else {
        return Ok(SetPreview::refused(Vec::new(), findings));
    };
    let (rings, set_rooms) = describe(state, project, rooms_read, &refs, &mut findings)?;
    adjacency_finding(state, project, rooms_read, &refs, &mut findings);
    let refused = [
        "room_unknown",
        "room_ambiguous",
        "several_models",
        "room_has_no_outline",
        "not_adjacent",
    ];
    if findings
        .iter()
        .any(|f| f.get("kind").and_then(Value::as_str).is_some_and(|k| refused.contains(&k)))
    {
        return Ok(SetPreview::refused(set_rooms, findings));
    }

    // the lead room: the largest, the first asked for on a tie
    let lead = set_rooms
        .iter()
        .enumerate()
        .max_by(|(i, a), (j, b)| a.area_m2.total_cmp(&b.area_m2).then(j.cmp(i)))
        .map(|(_, r)| r.room_id.clone());

    let signature = set_signature(ctx, &refs, &rings, &set_rooms);
    let ranked = rank_set(ctx, sheets, signature.as_ref(), &mut findings);
    let suggestions: Vec<SetSuggestion> = ranked
        .iter()
        .take(SUGGESTIONS)
        .map(|s| SetSuggestion {
            sheet_id: s.sheet_id.clone(),
            sheet_number: s.sheet_number.clone(),
            sheet_name: s.sheet_name.clone(),
            fit: fit_percent(s.distance),
        })
        .collect();
    let chosen = match &request.sheet_id {
        Some(id) => match sheets.iter().find(|s| &s.id == id) {
            Some(s) => Some(s),
            None => {
                findings.push(finding("sheet_unknown", format!("no library sheet {id:?}")));
                None
            }
        },
        None => ranked.first().and_then(|s| sheets.iter().find(|x| x.id == s.sheet_id)),
    };
    let Some(sheet) = chosen else {
        if request.sheet_id.is_none() {
            findings.push(finding("no_sheet", "no library sheet could be ranked against the set".to_string()));
        }
        return Ok(SetPreview {
            valid: true,
            rooms: set_rooms,
            lead_room: lead,
            signature,
            suggestions,
            sheet_id: None,
            views: None,
            findings,
        });
    };

    let views = set_views(state, project, rooms_read, sheet, &refs, &rings, &set_rooms, &mut findings)?;
    Ok(SetPreview {
        valid: true,
        rooms: set_rooms,
        lead_room: lead,
        signature,
        suggestions,
        sheet_id: Some(sheet.id.clone()),
        views,
        findings,
    })
}

/// Each asked-for room qualified to its one model, once each; an unknown or ambiguous id, and rooms in several models,
/// are findings. `None` when no room is left.
fn qualify_all(
    rooms_read: &super::rooms::RoomsResult,
    ids: &[String],
    findings: &mut Vec<Value>,
) -> Result<Option<Vec<RoomRef>>, ServiceError> {
    let mut refs: Vec<RoomRef> = Vec::new();
    for id in ids {
        match qualify_room_in(rooms_read, id) {
            Ok(Some(r)) if !refs.contains(&r) => refs.push(r),
            Ok(Some(_)) => {}
            Ok(None) => findings.push(finding("room_unknown", format!("room {id:?} is not in this project"))),
            Err(ServiceError::Invalid(reason)) => findings.push(finding("room_ambiguous", reason)),
            Err(e) => return Err(e),
        }
    }
    if refs.is_empty() {
        findings.push(finding("no_rooms", "a set needs at least one room".to_string()));
        return Ok(None);
    }
    let mut models: Vec<&str> = refs.iter().map(|r| r.model_id.as_str()).collect();
    models.sort_unstable();
    models.dedup();
    if models.len() > 1 {
        findings.push(finding(
            "several_models",
            format!(
                "the rooms are in {} models ({}); a set is one model's rooms",
                models.len(),
                models.join(", ")
            ),
        ));
    }
    Ok(Some(refs))
}

/// Each room's outline in its model's frame and what the answer says of it (name, number, area, clear height).
fn describe(
    state: &AppState,
    project: &str,
    rooms_read: &super::rooms::RoomsResult,
    refs: &[RoomRef],
    findings: &mut Vec<Value>,
) -> Result<(Vec<Vec<Pt>>, Vec<SetRoom>), ServiceError> {
    let found: Vec<_> = refs
        .iter()
        .map(|r| rooms_read.rooms.iter().find(|x| x.model_id == r.model_id && x.room.id == r.room_id))
        .collect();
    let mut rings: Vec<Vec<Pt>> = Vec::new();
    for r in refs {
        match model_room_ring_in(state, project, rooms_read, &SheetRoom::from(r))? {
            Some(ring) => rings.push(ring),
            None => {
                findings.push(finding("room_has_no_outline", format!("room {:?} has no outline", r.room_id)));
                rings.push(Vec::new());
            }
        }
    }
    let heights = clear_heights(state, project, rooms_read, refs)?;
    let set_rooms: Vec<SetRoom> = refs
        .iter()
        .zip(&found)
        .zip(&rings)
        .map(|((r, f), ring)| {
            let (height_mm, height_from) = match heights.get(r) {
                Some(&(h, HeightFrom::Ceiling)) => (h, "ceiling"),
                Some(&(h, HeightFrom::RoomProperty)) => (h, "room_property"),
                None => (DEFAULT_HEIGHT_MM, "default"),
            };
            SetRoom {
                room_id: r.room_id.clone(),
                model_id: r.model_id.clone(),
                name: f.map(|f| f.room.name.clone()).unwrap_or_default(),
                number: f
                    .and_then(|f| f.room.properties.get("Number"))
                    .map(|v| v.value.trim().to_string())
                    .filter(|n| !n.is_empty()),
                area_m2: signed_area(ring).abs() * M2_PER_MM2,
                height_mm,
                height_from,
            }
        })
        .collect();
    let defaulted: Vec<&str> = set_rooms
        .iter()
        .filter(|r| r.height_from == "default")
        .map(|r| r.room_id.as_str())
        .collect();
    if !defaulted.is_empty() {
        findings.push(finding(
            "height_defaulted",
            format!(
                "no ceiling covers and no height is stated for room(s) {}: their elevations are {DEFAULT_HEIGHT_MM} mm tall",
                defaulted.join(", ")
            ),
        ));
    }
    Ok((rings, set_rooms))
}

/// A finding when the rooms are not one group of adjacent rooms by the server's adjacency (`service::adjacency`, the
/// project's default wall tolerance, as the viewer's adjacency band reads it).
fn adjacency_finding(
    state: &AppState,
    project: &str,
    rooms_read: &super::rooms::RoomsResult,
    refs: &[RoomRef],
    findings: &mut Vec<Value>,
) {
    let found: Vec<_> = refs
        .iter()
        .filter_map(|r| rooms_read.rooms.iter().find(|x| x.model_id == r.model_id && x.room.id == r.room_id))
        .collect();
    let contract_rooms: Vec<_> = found.iter().map(|f| &f.room).collect();
    if contract_rooms.len() == refs.len() && refs.len() > 1 {
        let policy = state.settings().settings_for(project).map(|b| b.areas.clone()).unwrap_or_default();
        let wall_max = default_wall_max(&policy, rooms_read);
        let edges: Vec<(usize, usize)> = compute_adjacency_indexed(&contract_rooms, wall_max)
            .into_iter()
            .map(|e| (e.ia, e.ib))
            .collect();
        let parts = groups(refs.len(), &edges);
        if parts.len() > 1 {
            let named: Vec<String> = parts
                .iter()
                .map(|g| g.iter().map(|&i| refs[i].room_id.as_str()).collect::<Vec<_>>().join(" + "))
                .collect();
            findings.push(finding(
                "not_adjacent",
                format!(
                    "the rooms are {} groups that share no wall ({}); a set is one group of adjacent rooms",
                    parts.len(),
                    named.join("; ")
                ),
            ));
        }
    }
}

/// The set as one room: the rectangle round all its rooms, their summed area and item count.
fn set_signature(ctx: &Context, refs: &[RoomRef], rings: &[Vec<Pt>], set_rooms: &[SetRoom]) -> Option<Signature> {
    set_rectangle(rings).map(|(long, short)| Signature {
        area_m2: set_rooms.iter().map(|r| r.area_m2).sum(),
        proportion: if short > 0.0 { long / short } else { 1.0 },
        items: refs.iter().map(|r| ctx.item_count(r)).sum(),
    })
}

/// The library sheets ranked against the set, best first, and a finding for those that cannot be ranked.
fn rank_set(
    ctx: &Context,
    sheets: &[SheetRecord],
    signature: Option<&Signature>,
    findings: &mut Vec<Value>,
) -> Vec<super::sheet_suggest::Suggestion> {
    let (ranked, unresolved, unassigned) = match signature {
        Some(sig) => ctx.rank_sheets(sig, sheets),
        None => (Vec::new(), Vec::new(), 0),
    };
    if unassigned + unresolved.len() > 0 {
        findings.push(finding(
            "sheets_not_ranked",
            format!(
                "{} of {} library sheets cannot be ranked: they carry no room figures and name no room on the server \
                 (push them again from the Sheets button)",
                unassigned + unresolved.len(),
                sheets.len()
            ),
        ));
    }
    ranked
}

/// The views of the set on `sheet`'s scales. `None` (and a finding) when the sheet has no plan viewport to take a
/// scale from.
#[allow(clippy::too_many_arguments)]
fn set_views(
    state: &AppState,
    project: &str,
    rooms_read: &super::rooms::RoomsResult,
    sheet: &SheetRecord,
    refs: &[RoomRef],
    rings: &[Vec<Pt>],
    set_rooms: &[SetRoom],
    findings: &mut Vec<Value>,
) -> Result<Option<SetViews>, ServiceError> {
    let body = &sheet.sheet;
    let Some(plan_scale) = ports_of(body, "floor plan").first().and_then(|p| scale_of(p)) else {
        findings.push(finding(
            "sheet_has_no_plan",
            format!("library sheet {} has no plan viewport", sheet.sheet_number),
        ));
        return Ok(None);
    };
    let template = match &sheet.room {
        Some(r) => model_room_ring_in(state, project, rooms_read, r)?,
        None => None,
    };
    let all: Vec<Pt> = rings.iter().flatten().copied().collect();
    let turn = set_turn(template.as_deref().unwrap_or(&[]), rings);
    let crop = template.as_deref().and_then(|t| plan_crop_turned(body, t, &all, turn)).or_else(|| {
        findings.push(finding(
            "plan_framing_unknown",
            format!(
                "the library sheet's room or plan crop cannot be read: the plan is the set plus {FALLBACK_CROP_MARGIN_MM} mm"
            ),
        ));
        fallback_crop(&all, turn)
    });
    let Some(crop) = crop else {
        return Ok(None);
    };
    let plan = SetPlanView {
        paper_mm: [
            (crop.max[0] - crop.min[0]) / plan_scale,
            (crop.max[1] - crop.min[1]) / plan_scale,
        ],
        crop,
        scale: plan_scale,
    };
    let elevation_scale = ports_of(body, "elevation").first().and_then(|p| scale_of(p)).unwrap_or(plan_scale);
    let elevations = refs
        .iter()
        .zip(rings)
        .zip(set_rooms)
        .flat_map(|((r, ring), room)| room_elevations(&r.room_id, ring, room.height_mm, elevation_scale))
        .collect();
    let schedule_paper = ports_of(body, "schedule").first().and_then(|p| paper_of(p));
    let schedules = match schedule_paper {
        Some(paper_mm) => refs
            .iter()
            .map(|r| SetSchedule { room_id: r.room_id.clone(), paper_mm, estimate: true })
            .collect(),
        None => Vec::new(),
    };
    let three_d = ports_of(body, "three d")
        .first()
        .and_then(|p| paper_of(p))
        .map(|paper_mm| SetThreeD { paper_mm, estimate: true });
    Ok(Some(SetViews { plan, elevations, schedules, three_d }))
}

/// The set's box in the turned frame plus [`FALLBACK_CROP_MARGIN_MM`], when the template's framing is unknown.
fn fallback_crop(all: &[Pt], turn: Option<super::rls_turn::ViewTurn>) -> Option<PlanCrop> {
    let deg = turn.map_or(0.0, |t| t.angle_deg);
    let seen: Vec<Pt> = all.iter().map(|&p| rotate(p, -deg)).collect();
    let (x0, x1) = seen.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.0), b.max(p.0)));
    let (y0, y1) = seen.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.1), b.max(p.1)));
    if !(x1 > x0 && y1 > y0) {
        return None;
    }
    let m = FALLBACK_CROP_MARGIN_MM;
    let half = ((x1 - x0) / 2.0 + m, (y1 - y0) / 2.0 + m);
    let centre = rotate(((x0 + x1) / 2.0, (y0 + y1) / 2.0), deg);
    Some(PlanCrop {
        min: [centre.0 - half.0, centre.1 - half.1],
        max: [centre.0 + half.0, centre.1 + half.1],
        offsets_mm: super::rls_plan::CropOffsets { left: m, right: m, bottom: m, top: m },
        turn,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Vec<Pt> {
        vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
    }

    #[test]
    fn rooms_joined_by_walls_are_one_group_and_a_room_apart_is_another() {
        assert_eq!(vec![vec![0, 1, 2]], groups(3, &[(0, 1), (2, 1)]));
        assert_eq!(vec![vec![0, 1], vec![2]], groups(3, &[(1, 0)]));
    }

    #[test]
    fn a_set_is_measured_round_all_its_rooms_as_the_extractor_measures_one_sheet() {
        // a 4 x 3 room and a 2 x 3 one beside it: one 6 x 3 rectangle, whatever the walls between
        let (long, short) =
            set_rectangle(&[rect(0.0, 0.0, 4000.0, 3000.0), rect(4000.0, 0.0, 2000.0, 3000.0)]).unwrap();
        assert!((long - 6000.0).abs() < 1e-6 && (short - 3000.0).abs() < 1e-6, "{long} x {short}");
        // an L of two rooms: the hull's rectangle, not either room's
        let (long, short) =
            set_rectangle(&[rect(0.0, 0.0, 4000.0, 2000.0), rect(0.0, 2000.0, 2000.0, 2000.0)]).unwrap();
        assert!((long - 4000.0).abs() < 1e-6 && (short - 4000.0).abs() < 1e-6, "{long} x {short}");
    }

    /// A 4 x 3 room: the marker at its middle, the west view across the 3 m wall 2 m away, the north one across
    /// the 4 m wall 1.5 m away; slots 0, 2, 4, 6 look west, north, east, south.
    #[test]
    fn each_room_gets_four_elevations_square_to_its_own_walls() {
        let els = room_elevations("r", &rect(0.0, 0.0, 4000.0, 3000.0), 2400.0, 50.0);
        let looks: Vec<(i64, [f64; 2])> = els.iter().map(|e| (e.marker_index, e.looks)).collect();
        assert_eq!(vec![(0, [-1.0, 0.0]), (2, [0.0, 1.0]), (4, [1.0, 0.0]), (6, [0.0, -1.0])], looks);
        assert!(els.iter().all(|e| e.marker == [2000.0, 1500.0]));
        let west = &els[0];
        assert!((west.depth_mm - (2000.0 + DEPTH_PAST_WALL_MM)).abs() < 1e-9);
        assert!(
            (west.paper_mm[0] - 60.0).abs() < 1e-9 && (west.paper_mm[1] - 48.0).abs() < 1e-9,
            "{:?}",
            west.paper_mm
        );
        let north = &els[1];
        assert!((north.paper_mm[0] - 80.0).abs() < 1e-9 && (north.depth_mm - 1800.0).abs() < 1e-9);
        // the line runs across the view, along its right
        let along = (north.line_end[0] - north.line_start[0], north.line_end[1] - north.line_start[1]);
        assert!((along.0 - 4000.0).abs() < 1e-6 && along.1.abs() < 1e-6, "{along:?}");
    }

    #[test]
    fn a_room_at_an_angle_is_elevated_square_to_its_own_walls() {
        let ring: Vec<Pt> = rect(0.0, 0.0, 4000.0, 3000.0).into_iter().map(|p| rotate(p, 30.0)).collect();
        let els = room_elevations("r", &ring, 2400.0, 50.0);
        let west = rotate((-1.0, 0.0), 30.0);
        assert!((els[0].looks[0] - west.0).abs() < 1e-9 && (els[0].looks[1] - west.1).abs() < 1e-9);
        assert!((els[0].paper_mm[0] - 60.0).abs() < 1e-6, "{:?}", els[0].paper_mm);
    }

    #[test]
    fn a_viewport_s_paper_size_is_its_box_on_the_sheet() {
        let port = json!({"vp_type": "schedule", "bounding_box": {"min_x": 10.0, "min_y": 20.0, "max_x": 143.0, "max_y": 85.0}});
        assert_eq!(Some([133.0, 65.0]), paper_of(&port));
        assert_eq!(None, paper_of(&json!({"vp_type": "schedule"})));
    }

    #[test]
    fn without_the_template_s_framing_the_plan_is_the_set_plus_a_margin() {
        let crop = fallback_crop(&rect(0.0, 0.0, 4000.0, 3000.0), None).unwrap();
        assert_eq!([-600.0, -600.0], crop.min);
        assert_eq!([4600.0, 3600.0], crop.max);
    }
}
