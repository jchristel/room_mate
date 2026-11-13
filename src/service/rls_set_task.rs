//! Making a `create_rls_set` task: a set of adjacent rooms, the views the set preview gives it, and where a person put
//! each one on which sheet in the composer (the "Road map plan" tab of the blueprints plan).
//!
//! **The views are the server's, the layout is the person's.** The request names the rooms, the library sheet and,
//! per sheet, the composer's view ids at a point on the paper. The preview is worked out again here from the same
//! rooms and sheet, so the crop, the elevation lines and every size in the task are the server's numbers and a page
//! cannot send geometry of its own; the page only says where. A view id the preview does not have, one placed twice,
//! one neither placed nor left out, and an empty sheet are refusals: the layout IS the task (Revit cannot change it
//! underneath), so an ambiguous one is not queued. A view that runs past the paper is a finding, as in the composer:
//! a person may mean it, and the title block's drawable area is not known here either.
//!
//! **Paper positions turn into Revit's sheet frame here, once.** The composer keeps a view's top left corner with y
//! down from the top of the paper; the task carries its CENTRE in mm from the paper's bottom left corner, y up, because
//! a viewport is placed by its box centre (`SetBoxCenter`, see CLAUDE.md) and a sheet's y runs up. Paper is landscape.
//!
//! Geometry is in the rooms' MODEL frame, mm, as the preview's is (`model_room_ring_in`), so nothing is undone here.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::rls_plan::PlanCrop;
use super::rls_set::{preview, SetPreview, SetRequest, SetViews};
use super::sheet_suggest::Context;
use super::task_build::{finding, room_facts_in};
use super::ServiceError;
use crate::service::blueprint_model::BlueprintSheet;
use crate::sheets::SheetRecord;
use crate::state::AppState;
use crate::tasks::{NewTask, TaskKind, TaskRoom, ROOM_OWN_PHASE};

/// More sheets than this in one set is a mistake, not a layout.
const MAX_SHEETS: usize = 50;

/// What the composer sends.
#[derive(Debug, Clone, Deserialize)]
pub struct SetTaskRequest {
    pub room_ids: Vec<String>,
    /// The library sheet the set starts from (the preview's `sheet_id`): its scales and the plan's framing.
    pub sheet_id: String,
    /// `a0` to `a3`, every sheet of the set.
    pub sheet_size: String,
    #[serde(default)]
    pub overwrite_rls: bool,
    pub sheets: Vec<SheetLayout>,
    /// Views a person chose not to make.
    #[serde(default)]
    pub left_out: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SheetLayout {
    pub views: Vec<PlacedView>,
}

/// A view on a sheet as the composer holds it: its id and its top left corner, paper mm, y down.
#[derive(Debug, Clone, Deserialize)]
pub struct PlacedView {
    pub view: String,
    pub x_mm: f64,
    pub y_mm: f64,
}

/// What the task holds for the plug-in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RlsSetPlan {
    pub sheet_size: String,
    /// Width then height, mm, landscape.
    pub paper_mm: [f64; 2],
    /// The room whose parameters fill the set's own names (sheet number and name, the plan, the 3D view).
    pub lead_room: String,
    /// Every room of the set, the lead first.
    pub rooms: Vec<TaskRoom>,
    /// The plan of the whole set; absent when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<SetPlanTask>,
    /// Each room's marker and the elevations of it that are made; a room whose four are all left out has none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<SetMarker>,
    /// The rooms that get a schedule.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedules: Vec<String>,
    /// Whether the set's one 3D view is made (the template's direction, the plan crop's footprint).
    #[serde(default)]
    pub three_d: bool,
    pub sheets: Vec<SetSheet>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetPlanTask {
    pub crop: PlanCrop,
    pub scale: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetMarker {
    pub room_id: String,
    pub location: [f64; 2],
    pub views: Vec<SetElevationView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetElevationView {
    pub marker_index: i64,
    pub line_start: [f64; 2],
    pub line_end: [f64; 2],
    pub looks: [f64; 2],
    pub depth_mm: f64,
    /// The room's clear height: the crop runs this far up from the room's floor.
    pub height_mm: f64,
    pub scale: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetViewKind {
    Plan,
    Elevation,
    Schedule,
    ThreeD,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetSheet {
    pub views: Vec<SetSheetView>,
}

/// A viewport of a set sheet: which view, its box centre in mm from the paper's bottom left corner (y up), and its
/// size on paper (an estimate for a schedule or the 3D view).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetSheetView {
    pub kind: SetViewKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker_index: Option<i64>,
    pub centre_mm: [f64; 2],
    pub size_mm: [f64; 2],
}

/// The paper of a size token, landscape, mm (the composer's `PAPER_MM`).
fn paper_of(size: &str) -> Option<[f64; 2]> {
    match size.trim().to_ascii_lowercase().as_str() {
        "a0" => Some([1189.0, 841.0]),
        "a1" => Some([841.0, 594.0]),
        "a2" => Some([594.0, 420.0]),
        "a3" => Some([420.0, 297.0]),
        _ => None,
    }
}

/// A view the preview offers: what it is and its size on paper.
#[derive(Debug, Clone, PartialEq)]
struct Offered {
    kind: SetViewKind,
    room_id: Option<String>,
    marker_index: Option<i64>,
    size_mm: [f64; 2],
}

/// The composer's view ids (`rlsSet.ts` `portsOf`) for what the preview offers. Keep the two spellings in step.
fn offered(views: &SetViews) -> BTreeMap<String, Offered> {
    let mut out = BTreeMap::new();
    let plain = |kind, size_mm| Offered { kind, room_id: None, marker_index: None, size_mm };
    out.insert("plan".to_string(), plain(SetViewKind::Plan, views.plan.paper_mm));
    if let Some(t) = &views.three_d {
        out.insert("three_d".to_string(), plain(SetViewKind::ThreeD, t.paper_mm));
    }
    for e in &views.elevations {
        out.insert(
            format!("elevation:{}:{}", e.room_id, e.marker_index),
            Offered {
                kind: SetViewKind::Elevation,
                room_id: Some(e.room_id.clone()),
                marker_index: Some(e.marker_index),
                size_mm: e.paper_mm,
            },
        );
    }
    for s in &views.schedules {
        out.insert(
            format!("schedule:{}", s.room_id),
            Offered {
                kind: SetViewKind::Schedule,
                room_id: Some(s.room_id.clone()),
                marker_index: None,
                size_mm: s.paper_mm,
            },
        );
    }
    out
}

fn refuse<T>(reason: impl Into<String>) -> Result<T, ServiceError> {
    Err(ServiceError::Invalid(reason.into()))
}

/// The preview's own findings that refused the set, as one reason.
fn refusal_of(p: &SetPreview) -> String {
    let said: Vec<&str> = p.findings.iter().filter_map(|f| f.get("message").and_then(Value::as_str)).collect();
    if said.is_empty() {
        "the rooms do not make a set".to_string()
    } else {
        said.join("; ")
    }
}

/// The task for `request`, or the reason it cannot be made. `sheets` and `ctx` are what the preview reads (the
/// library without the sheets of blueprints not in use, the project's rooms and item counts).
pub fn set_task(
    state: &AppState,
    project: &str,
    ctx: &Context,
    sheets: &[SheetRecord],
    request: &SetTaskRequest,
) -> Result<NewTask, ServiceError> {
    let Some(paper_mm) = paper_of(&request.sheet_size) else {
        return refuse(format!("sheet size {:?} is not one of a0, a1, a2, a3", request.sheet_size));
    };
    let Some(template) = sheets.iter().find(|s| s.id == request.sheet_id) else {
        return refuse(format!("no library sheet {:?}", request.sheet_id));
    };
    let p = preview(
        state,
        project,
        ctx,
        sheets,
        &SetRequest { room_ids: request.room_ids.clone(), sheet_id: Some(request.sheet_id.clone()) },
    )?;
    if !p.valid {
        return refuse(refusal_of(&p));
    }
    let Some(views) = &p.views else {
        return refuse(format!("the set has no views on that sheet: {}", refusal_of(&p)));
    };
    // what the preview said about the LIBRARY (sheets it could not rank) is not about this task
    let mut findings: Vec<Value> = p
        .findings
        .iter()
        .filter(|f| f.get("kind").and_then(Value::as_str) != Some("sheets_not_ranked"))
        .cloned()
        .collect();
    let layout = check_layout(&offered(views), request, paper_mm, &mut findings)?;

    let made: BTreeSet<&str> = layout.placed.iter().map(String::as_str).collect();
    let rooms_read = ctx.rooms();
    let lead_id = p.lead_room.clone().unwrap_or_default();
    let ordered: Vec<_> = p
        .rooms
        .iter()
        .filter(|r| r.room_id == lead_id)
        .chain(p.rooms.iter().filter(|r| r.room_id != lead_id))
        .collect();
    let rooms: Vec<TaskRoom> = ordered
        .iter()
        .map(|r| {
            let at = super::room_locator::RoomRef { model_id: r.model_id.clone(), room_id: r.room_id.clone() };
            room_facts_in(rooms_read, &at).unwrap_or(TaskRoom {
                room_id: r.room_id.clone(),
                number: r.number.clone(),
                name: r.name.clone(),
            })
        })
        .collect();
    let Some(lead) = rooms.first().cloned() else {
        return refuse("the set has no rooms");
    };
    let model_id = ordered[0].model_id.clone();

    let room_ids: Vec<&str> = ordered.iter().map(|r| r.room_id.as_str()).collect();
    let markers = set_markers(&room_ids, views, &made);

    let plan = RlsSetPlan {
        sheet_size: request.sheet_size.trim().to_ascii_lowercase(),
        paper_mm,
        lead_room: lead.room_id.clone(),
        plan: made
            .contains("plan")
            .then(|| SetPlanTask { crop: views.plan.crop.clone(), scale: views.plan.scale }),
        markers,
        schedules: ordered
            .iter()
            .filter(|r| made.contains(format!("schedule:{}", r.room_id).as_str()))
            .map(|r| r.room_id.clone())
            .collect(),
        three_d: made.contains("three_d"),
        rooms,
        sheets: layout.sheets,
    };
    Ok(NewTask {
        kind: TaskKind::CreateRlsSet,
        model_id,
        room: lead,
        phase: ROOM_OWN_PHASE.to_string(),
        overwrite_layout: false,
        create_rls: false,
        overwrite_rls: request.overwrite_rls,
        update_mode: None,
        dry_run: false,
        blueprint: None,
        library_sheet_id: Some(template.id.clone()),
        items: Vec::new(),
        // the template, for what only it knows: view types, label places, the 3D direction
        sheets: vec![BlueprintSheet {
            revit_sheet_id: template.revit_sheet_id.clone(),
            document_title: template.document_title.clone(),
            sheet_number: template.sheet_number.clone(),
            sheet_name: template.sheet_name.clone(),
            size_mm: template.size_mm,
            sheet: template.sheet.clone(),
        }],
        rls: None,
        rls_set: Some(plan),
        tag_rules: Vec::new(),
        findings,
    })
}

/// Each room's marker with the elevations of it that are made, in `rooms`' order; a room with none made has no marker.
fn set_markers(rooms: &[&str], views: &SetViews, made: &BTreeSet<&str>) -> Vec<SetMarker> {
    rooms
        .iter()
        .filter_map(|&room| {
            let of_room: Vec<_> = views.elevations.iter().filter(|e| e.room_id == room).collect();
            let kept: Vec<SetElevationView> = of_room
                .iter()
                .filter(|e| made.contains(format!("elevation:{}:{}", e.room_id, e.marker_index).as_str()))
                .map(|e| SetElevationView {
                    marker_index: e.marker_index,
                    line_start: e.line_start,
                    line_end: e.line_end,
                    looks: e.looks,
                    depth_mm: e.depth_mm,
                    height_mm: e.height_mm,
                    scale: e.scale,
                })
                .collect();
            let location = of_room.first()?.marker;
            (!kept.is_empty()).then(|| SetMarker { room_id: room.to_string(), location, views: kept })
        })
        .collect()
}

/// A layout that passed: the sheets as the task carries them and the ids of the views made.
#[derive(Debug)]
struct Checked {
    sheets: Vec<SetSheet>,
    placed: Vec<String>,
}

/// Check the composer's layout against the views offered and turn it into sheet coordinates. Refusals for a layout
/// that does not say exactly what to make; a finding for a view past the paper's edge.
fn check_layout(
    offered: &BTreeMap<String, Offered>,
    request: &SetTaskRequest,
    paper_mm: [f64; 2],
    findings: &mut Vec<Value>,
) -> Result<Checked, ServiceError> {
    if request.sheets.is_empty() {
        return refuse("a set needs at least one sheet");
    }
    if request.sheets.len() > MAX_SHEETS {
        return refuse(format!("a set has at most {MAX_SHEETS} sheets"));
    }
    for id in &request.left_out {
        if !offered.contains_key(id) {
            return refuse(format!("left out {id:?} is not a view of this set"));
        }
    }
    let [w, h] = paper_mm;
    let mut placed: Vec<String> = Vec::new();
    let mut sheets = Vec::new();
    for (i, sheet) in request.sheets.iter().enumerate() {
        if sheet.views.is_empty() {
            return refuse(format!("sheet {} is empty: remove it or put a view on it", i + 1));
        }
        let mut out = Vec::new();
        for v in &sheet.views {
            let Some(o) = offered.get(&v.view) else {
                return refuse(format!("{:?} is not a view of this set", v.view));
            };
            if placed.contains(&v.view) || request.left_out.contains(&v.view) {
                return refuse(format!("{:?} is on more than one sheet, or placed and left out", v.view));
            }
            if !(v.x_mm.is_finite() && v.y_mm.is_finite()) {
                return refuse(format!("{:?} has no position", v.view));
            }
            let [vw, vh] = o.size_mm;
            if v.x_mm < 0.0 || v.y_mm < 0.0 || v.x_mm + vw > w || v.y_mm + vh > h {
                findings.push(finding(
                    "view_off_paper",
                    format!("{} runs past the edge of the paper on sheet {}", v.view, i + 1),
                ));
            }
            placed.push(v.view.clone());
            out.push(SetSheetView {
                kind: o.kind,
                room_id: o.room_id.clone(),
                marker_index: o.marker_index,
                centre_mm: [v.x_mm + vw / 2.0, h - (v.y_mm + vh / 2.0)],
                size_mm: o.size_mm,
            });
        }
        sheets.push(SetSheet { views: out });
    }
    let waiting: Vec<&str> = offered
        .keys()
        .filter(|id| !placed.contains(id) && !request.left_out.contains(id))
        .map(String::as_str)
        .collect();
    if !waiting.is_empty() {
        return refuse(format!(
            "{} view(s) are on no sheet and not left out: {}",
            waiting.len(),
            waiting.join(", ")
        ));
    }
    Ok(Checked { sheets, placed })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer() -> BTreeMap<String, Offered> {
        let mut m = BTreeMap::new();
        m.insert(
            "plan".to_string(),
            Offered {
                kind: SetViewKind::Plan,
                room_id: None,
                marker_index: None,
                size_mm: [100.0, 50.0],
            },
        );
        m.insert(
            "elevation:r:0".to_string(),
            Offered {
                kind: SetViewKind::Elevation,
                room_id: Some("r".to_string()),
                marker_index: Some(0),
                size_mm: [40.0, 20.0],
            },
        );
        m
    }

    fn request(sheets: Vec<Vec<(&str, f64, f64)>>, left_out: &[&str]) -> SetTaskRequest {
        SetTaskRequest {
            room_ids: vec!["r".to_string()],
            sheet_id: "s".to_string(),
            sheet_size: "a3".to_string(),
            overwrite_rls: false,
            sheets: sheets
                .into_iter()
                .map(|views| SheetLayout {
                    views: views
                        .into_iter()
                        .map(|(view, x_mm, y_mm)| PlacedView { view: view.to_string(), x_mm, y_mm })
                        .collect(),
                })
                .collect(),
            left_out: left_out.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn a_layout_turns_top_left_corners_into_centres_from_the_bottom_left() {
        let mut findings = Vec::new();
        let r = request(vec![vec![("plan", 15.0, 15.0)], vec![("elevation:r:0", 100.0, 200.0)]], &[]);
        let c = check_layout(&offer(), &r, [420.0, 297.0], &mut findings).unwrap();
        assert_eq!(c.sheets[0].views[0].centre_mm, [65.0, 297.0 - 40.0]);
        assert_eq!(c.sheets[1].views[0].centre_mm, [120.0, 297.0 - 210.0]);
        assert_eq!(c.sheets[1].views[0].kind, SetViewKind::Elevation);
        assert_eq!(c.sheets[1].views[0].marker_index, Some(0));
        assert!(findings.is_empty());
    }

    #[test]
    fn a_layout_that_does_not_say_exactly_what_to_make_is_refused() {
        let mut f = Vec::new();
        let o = offer();
        let paper = [420.0, 297.0];
        let refused = |r: SetTaskRequest, f: &mut Vec<Value>| match check_layout(&o, &r, paper, f) {
            Err(ServiceError::Invalid(m)) => m,
            other => panic!("expected a refusal, got {other:?}"),
        };
        assert!(refused(request(vec![vec![("plan", 15.0, 15.0)]], &[]), &mut f).contains("elevation:r:0"));
        assert!(
            refused(request(vec![vec![("plan", 15.0, 15.0), ("nope", 0.0, 0.0)]], &[]), &mut f).contains("not a view")
        );
        assert!(
            refused(request(vec![vec![("plan", 15.0, 15.0)], vec![("plan", 15.0, 15.0)]], &[]), &mut f)
                .contains("more than one sheet")
        );
        assert!(refused(request(vec![vec![("plan", 15.0, 15.0)], vec![]], &["elevation:r:0"]), &mut f)
            .contains("sheet 2 is empty"));
        assert!(refused(request(vec![], &[]), &mut f).contains("at least one sheet"));
        // left out is enough: the elevation is not made
        let ok = check_layout(&o, &request(vec![vec![("plan", 15.0, 15.0)]], &["elevation:r:0"]), paper, &mut f);
        assert_eq!(ok.unwrap().placed, vec!["plan".to_string()]);
    }

    #[test]
    fn a_view_past_the_paper_is_a_finding_and_still_made() {
        let mut findings = Vec::new();
        let r = request(vec![vec![("plan", 400.0, 15.0), ("elevation:r:0", 15.0, 100.0)]], &[]);
        let c = check_layout(&offer(), &r, [420.0, 297.0], &mut findings).unwrap();
        assert_eq!(c.placed.len(), 2);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["kind"], "view_off_paper");
    }

    #[test]
    fn the_paper_sizes_are_the_composers() {
        assert_eq!(paper_of(" A1 "), Some([841.0, 594.0]));
        assert_eq!(paper_of("a4"), None);
    }
}
