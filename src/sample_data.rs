//! The sample project in `samples/house-a/` still works with this code.
//!
//! It is stored data: snapshots Revit pushed, and a blueprint and an assignment that
//! `scripts/make_sample.py` made by calling the server. Stored data goes stale the day a format changes and
//! nobody opens the sample to find out, so this test boots the server on a COPY of it (a read must not
//! write into the repo) and reads every entity, the milestones that compare two pushes, the blueprint, the
//! assignment, and the task the assignment makes. When one of those fails, a format changed under the
//! sample: re-run `python scripts/make_sample.py` and commit what it writes.

use std::path::{Path, PathBuf};

use crate::assignments;
use crate::blueprints;
use crate::service::comparison::compare_milestones;
use crate::service::items::{assemble_items, ItemScope};
use crate::service::milestones::list_milestones;
use crate::service::rooms::{assemble_rooms, RoomScope};
use crate::service::spaces::{assemble_spaces, SpaceScope};
use crate::service::task_build::{populate_from_assignment, PopulateParams};
use crate::sheets;
use crate::state::Shared;
use crate::tag_rules;

const PROJECT: &str = "House A";

fn sample_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join("house-a")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The server on a copy of the sample, and where the copy is.
fn booted(tag: &str) -> (Shared, PathBuf) {
    let copy = std::env::temp_dir().join(format!("roommate-sample-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&copy).ok();
    copy_tree(&sample_dir(), &copy);
    let state = crate::bootstrap::build_state(&copy.join("server.toml"), &copy.join("projects"))
        .expect("the sample's settings and store load");
    (state, copy.join("projects"))
}

#[test]
fn test_every_entity_in_the_sample_reads() {
    let (state, copy) = booted("entities");
    let rooms = assemble_rooms(&state, &RoomScope { project: Some(PROJECT), ..Default::default() })
        .unwrap()
        .expect("rooms");
    assert!(rooms.rooms.len() > 10, "{} rooms", rooms.rooms.len());
    let ffe = assemble_items(&state, &ItemScope { project: Some(PROJECT), ..Default::default() })
        .unwrap()
        .expect("ffe");
    assert!(!ffe.ffe.is_empty());
    assert!(assemble_spaces(
        &state,
        &SpaceScope {
            project: Some(PROJECT),
            model: None,
            milestone: None,
            filter: None,
            storeys: None
        }
    )
    .unwrap()
    .is_some());
    std::fs::remove_dir_all(copy.parent().unwrap()).ok();
}

#[test]
fn test_the_two_milestones_compare_the_earlier_push_with_the_current_one() {
    let (state, copy) = booted("milestones");
    let listed = list_milestones(&state, PROJECT).unwrap();
    let names: Vec<&str> = listed.milestones.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names.len(), 2, "{names:?}");
    let (baseline, other) = ("Earlier push".to_string(), "Current push".to_string());
    assert!(names.contains(&baseline.as_str()) && names.contains(&other.as_str()), "{names:?}");
    let compared = compare_milestones(&state, PROJECT, &baseline, std::slice::from_ref(&other)).unwrap();
    assert!(compared.comparison_key_configured);
    std::fs::remove_dir_all(copy.parent().unwrap()).ok();
}

#[test]
fn test_the_authored_data_loads_and_the_assignment_makes_a_task() {
    let (state, projects) = booted("authored");
    let list = blueprints::list(&projects, PROJECT).unwrap();
    assert_eq!(list.len(), 1, "one sample blueprint");
    let blueprint = blueprints::load(&projects, PROJECT, &list[0].id).unwrap();
    assert!(blueprint.in_use && !blueprint.revisions.is_empty());

    let rows = assignments::list(&projects, PROJECT).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].blueprint_missing, "the assignment's blueprint is there");
    let assignment = assignments::list_raw(&projects, PROJECT).unwrap().remove(0);
    let params = PopulateParams {
        phase: "New Construction".into(),
        overwrite_layout: false,
        create_rls: false,
        overwrite_rls: false,
    };
    let task = populate_from_assignment(&state, &projects, PROJECT, &assignment, &params)
        .expect("the sample assignment makes a task");
    assert!(!task.items.is_empty(), "the blueprint's items land in the other room");

    // Whatever sheets and rules the sample carries must belong to something that exists: a sheet bound to a
    // blueprint that is not there, or a rule bound to one, would be a sample that contradicts itself.
    let ids: Vec<String> = list.iter().map(|b| b.id.clone()).collect();
    for sheet in sheets::load_all(&projects, PROJECT).unwrap() {
        assert!(
            sheet.blueprint.as_ref().is_none_or(|b| ids.contains(b)),
            "sheet {} names a missing blueprint",
            sheet.sheet_number
        );
    }
    for rule in tag_rules::load_all(&projects, PROJECT).unwrap() {
        assert!(
            rule.key.blueprint.is_empty() || ids.contains(&rule.key.blueprint),
            "rule {} names a missing blueprint",
            rule.id
        );
    }
    std::fs::remove_dir_all(projects.parent().unwrap()).ok();
}

/// The sample was pushed before items carried a placement type, so the audit of its assignment is the
/// old-data case: every element of the room reads "cannot tell" and is flagged, never taken for level-based,
/// and a blueprint captured before hosting was recorded is judged by size with the assumption counted.
#[test]
fn test_the_audit_of_the_sample_assignment_says_what_it_cannot_tell() {
    use crate::service::blueprint_audit::{audit_assignment, Unsupported, VerdictKind};
    let (state, projects) = booted("audit");
    let assignment = assignments::list_raw(&projects, PROJECT).unwrap().remove(0);
    let got = audit_assignment(&state, &projects, PROJECT, &assignment.id)
        .expect("the sample assignment audits")
        .expect("its room is in the sample");
    assert_eq!(got.assignment_id, assignment.id);
    assert_eq!(got.type_code_parameter, "Assembly Code");
    assert!(got.audit.layout_items > 0, "the blueprint places items in the room");
    assert!(
        got.audit.placement_assumed > 0,
        "a blueprint captured before hosting was recorded rests on an assumption"
    );
    let in_room: Vec<_> = got.audit.verdicts.iter().filter(|v| v.element_id.is_some()).collect();
    assert_eq!(got.audit.elements, in_room.len(), "every element of the room has exactly one verdict");
    assert!(
        in_room
            .iter()
            .all(|v| v.kind == VerdictKind::UnsupportedHost && v.unsupported == Some(Unsupported::PlacementUnknown)),
        "no element of the old snapshot states how it is placed, so none is judged"
    );
    assert!(
        audit_assignment(&state, &projects, PROJECT, "no-such-assignment").unwrap().is_none(),
        "an unknown assignment is not found, not an error"
    );
    std::fs::remove_dir_all(projects.parent().unwrap()).ok();
}

/// A room's addition is a room-local copy of a blueprint item, and it must reach everything that reads an assignment:
/// the task a Revit session is handed (which is what places it) and the audit (which says it is missing until it is).
#[test]
fn test_a_rooms_addition_reaches_the_task_and_the_audit_as_a_copy_of_its_item() {
    use crate::assignments::{AssignRequest, ItemAddition};
    use crate::service::blueprint_audit::{audit_assignment, VerdictKind};
    let (state, projects) = booted("additions");
    let before = assignments::list_raw(&projects, PROJECT).unwrap().remove(0);
    let blueprint = blueprints::load(&projects, PROJECT, &before.blueprint_id).unwrap();
    let original = blueprint
        .revision(before.revision)
        .unwrap()
        .content
        .items
        .iter()
        .find(|i| !i.excluded)
        .unwrap()
        .clone();
    let params = PopulateParams {
        phase: "New Construction".into(),
        overwrite_layout: false,
        create_rls: false,
        overwrite_rls: false,
    };
    let task_before = populate_from_assignment(&state, &projects, PROJECT, &before, &params).unwrap();

    let after = assignments::assign(
        &projects,
        PROJECT,
        AssignRequest {
            blueprint_id: before.blueprint_id.clone(),
            revision: Some(before.revision),
            room: before.room.clone(),
            room_name: before.room_name.clone(),
            mapping: before.mapping.clone(),
            overrides: None,
            additions: Some(vec![ItemAddition {
                id: "add-1".into(),
                clone_of: original.id.clone(),
                edge: 0,
                along_mm: 600,
                off_mm: 400,
                facing_deg: 0,
            }]),
            note: None,
        },
    )
    .unwrap();
    let task = populate_from_assignment(&state, &projects, PROJECT, &after, &params).unwrap();
    assert_eq!(task.items.len(), task_before.items.len() + 1, "one more item for the session to place");
    let added = task.items.iter().find(|i| i.item_id == "add-1").expect("the addition is in the task");
    assert_eq!(added.type_name, original.type_name, "a copy of the item it clones");
    assert_eq!(added.family_name, original.family_name);

    let audit = audit_assignment(&state, &projects, PROJECT, &after.id).unwrap().unwrap();
    assert_eq!(
        audit.audit.layout_items,
        task.items.len(),
        "the audit judges the same layout the task places"
    );
    assert!(
        audit.audit.verdicts.iter().any(|v| v.item_id.as_deref() == Some("add-1")
            && matches!(v.kind, VerdictKind::Missing | VerdictKind::NoCode | VerdictKind::UnsupportedHost)),
        "the addition is a layout item like any other: not in the room until a session places it"
    );
    std::fs::remove_dir_all(projects.parent().unwrap()).ok();
}

#[test]
fn test_solve_picks_the_sheet_whose_room_is_the_same_size() {
    use crate::service::sheet_suggest;
    use crate::sheets::SheetRecord;
    let (state, projects) = booted("solve");
    let ctx = sheet_suggest::context(&state, PROJECT).unwrap().expect("rooms");
    let rooms = assemble_rooms(&state, &RoomScope { project: Some(PROJECT), ..Default::default() })
        .unwrap()
        .unwrap();
    let first = &rooms.rooms[0];
    let me = crate::service::room_locator::RoomRef { model_id: first.model_id.clone(), room_id: first.room.id.clone() };
    let own = sheet_suggest::suggest_in(&ctx, &me, &[])
        .expect("the first room has an outline")
        .target
        .signature;
    let sheet = |id: &str, scale: f64| -> SheetRecord {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1, "id": id, "taken_at": "", "revit_sheet_id": id, "document_title": "D",
            "sheet_number": id, "sheet_name": "", "received_at": "", "sheet": {},
            "room_summary": {"x_mm": 4000.0, "y_mm": 2000.0 * own.proportion.recip() * 2.0,
                             "area_m2": own.area_m2 * scale, "items": own.items}
        }))
        .expect("a sheet record")
    };
    let sheets = vec![sheet("far", 4.0), sheet("twin", 1.0)];
    let solved = sheet_suggest::solve(&ctx, &[me.room_id.clone(), "no-such-room".into()], &sheets);
    assert_eq!(solved.len(), 2);
    assert_eq!(solved[0].choices[0].sheet_id, "twin");
    assert!(solved[0].choices[0].fit > solved[0].choices[1].fit);
    assert!(
        solved[1].reason.is_some() && solved[1].choices.is_empty(),
        "an unknown room is a verdict, not an error"
    );
    std::fs::remove_dir_all(projects.parent().unwrap()).ok();
}

/// A room layout sheet SET (`rls_set`): OFFICE 00.02 and WC 00.03 share a wall and make one; OFFICE and GARAGE 00.04
/// do not. The library sheet is made here (the sample has none) with the OFFICE as its room, so the plan keeps that
/// sheet's framing round both rooms.
#[test]
fn test_two_adjacent_rooms_make_a_set_and_two_apart_do_not() {
    use crate::service::rls_set::{preview, SetRequest};
    use crate::service::sheet_suggest;
    use crate::sheets::SheetRecord;
    let (state, projects) = booted("rls-set");
    let ctx = sheet_suggest::context(&state, PROJECT).unwrap().expect("rooms");
    let office = ctx.rooms().rooms.iter().find(|r| r.room.id == "2626834").expect("OFFICE 00.02");
    let sheet: SheetRecord = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "id": "office", "taken_at": "", "revit_sheet_id": "1", "document_title": "D",
        "sheet_number": "50H-00.02", "sheet_name": "OFFICE", "received_at": "",
        "room": {"model_id": office.model_id, "room_id": "2626834"},
        "room_summary": {"x_mm": 4500.0, "y_mm": 3700.0, "area_m2": 16.7, "items": 20},
        "sheet": {"view_ports": [
            {"vp_type": "floor plan", "view": {"view_scale": 50.0,
                "bounding_box": {"min_x": -100000.0, "min_y": -100000.0, "max_x": 100000.0, "max_y": 100000.0}}},
            {"vp_type": "elevation", "view": {"view_scale": 25.0}},
            {"vp_type": "schedule", "bounding_box": {"min_x": 0.0, "min_y": 0.0, "max_x": 133.0, "max_y": 65.0}}
        ]}
    }))
    .expect("a sheet record");
    let ask = |ids: &[&str]| {
        let request = SetRequest { room_ids: ids.iter().map(|s| s.to_string()).collect(), sheet_id: None };
        preview(&state, PROJECT, &ctx, std::slice::from_ref(&sheet), &request).unwrap()
    };

    let set = ask(&["2626836", "2626834"]);
    assert!(set.valid, "{:?}", set.findings);
    assert_eq!(Some("2626834".to_string()), set.lead_room, "the larger room leads");
    assert_eq!(Some("office".to_string()), set.sheet_id);
    let views = set.views.expect("views");
    assert_eq!(8, views.elevations.len(), "four per room");
    assert!(
        views.elevations.iter().all(|e| e.scale == 25.0),
        "the elevations take the sheet's elevation scale"
    );
    assert_eq!(2, views.schedules.len());
    assert!(set.rooms.iter().all(|r| r.height_mm > 2000.0), "{:?}", set.rooms);
    assert!(views.plan.scale == 50.0 && views.plan.paper_mm.iter().all(|&mm| mm > 0.0));

    let apart = ask(&["2626834", "2626844"]);
    assert!(!apart.valid && apart.views.is_none());
    assert!(apart.findings.iter().any(|f| f["kind"] == "not_adjacent"), "{:?}", apart.findings);

    // The composer's layout becomes ONE task: every view but the OFFICE's west elevation, on two A1 sheets.
    use crate::service::rls_set_task::{set_task, PlacedView, SetTaskRequest, SetViewKind, SheetLayout};
    let at = |view: &str, x_mm: f64| PlacedView { view: view.to_string(), x_mm, y_mm: 20.0 };
    let mut second: Vec<PlacedView> = Vec::new();
    for (i, e) in views.elevations.iter().enumerate() {
        let id = format!("elevation:{}:{}", e.room_id, e.marker_index);
        if id != "elevation:2626834:0" {
            second.push(at(&id, 20.0 + 90.0 * i as f64));
        }
    }
    let request = SetTaskRequest {
        room_ids: vec!["2626836".to_string(), "2626834".to_string()],
        sheet_id: "office".to_string(),
        sheet_size: "a1".to_string(),
        overwrite_rls: false,
        sheets: vec![
            SheetLayout {
                views: vec![
                    at("plan", 20.0),
                    at("schedule:2626834", 300.0),
                    at("schedule:2626836", 500.0),
                ],
            },
            SheetLayout { views: second },
        ],
        left_out: vec!["elevation:2626834:0".to_string()],
    };
    let new = set_task(&state, PROJECT, &ctx, std::slice::from_ref(&sheet), &request).expect("a set task");
    assert_eq!("2626834", new.room.room_id, "the task's room is the lead");
    let plan = new.rls_set.as_ref().expect("the set's plan");
    assert_eq!(
        vec!["2626834", "2626836"],
        plan.rooms.iter().map(|r| r.room_id.as_str()).collect::<Vec<_>>()
    );
    assert_eq!(
        vec![3, 4],
        plan.markers.iter().map(|m| m.views.len()).collect::<Vec<_>>(),
        "the left-out elevation is not made"
    );
    assert_eq!(2, plan.sheets.len());
    let first = &plan.sheets[0].views[0];
    assert_eq!(SetViewKind::Plan, first.kind);
    assert_eq!(
        [
            20.0 + views.plan.paper_mm[0] / 2.0,
            594.0 - (20.0 + views.plan.paper_mm[1] / 2.0)
        ],
        first.centre_mm,
        "a centre from the bottom left"
    );
    assert!(plan.plan.is_some() && !plan.three_d, "the sheet has no 3D viewport to give");

    let mut missing = request.clone();
    missing.left_out.clear();
    assert!(
        matches!(set_task(&state, PROJECT, &ctx, std::slice::from_ref(&sheet), &missing), Err(crate::service::ServiceError::Invalid(m)) if m.contains("elevation:2626834:0")),
        "a view neither placed nor left out refuses the layout"
    );
    std::fs::remove_dir_all(projects.parent().unwrap()).ok();
}

/// A library sheet carries its views to another room against the outline STORED with it, so a sheet with no room
/// link (the Sheets button) or whose room is not in this project (an import) still transfers. The stored outline
/// gives the same ring the link did; without either, the reason says which is missing.
#[test]
fn test_a_sheet_s_stored_outline_stands_in_for_its_room() {
    use crate::service::elevation_facing::{
        model_room_ring, template_ring, SHEET_NAMES_NO_ROOM, SHEET_ROOM_NOT_ON_SERVER,
    };
    use crate::sheets::{SheetRecord, SheetRoom};
    let (state, projects) = booted("outline");
    let rooms = assemble_rooms(&state, &RoomScope { project: Some(PROJECT), ..Default::default() })
        .unwrap()
        .unwrap();
    let office = rooms.rooms.iter().find(|r| r.room.id == "2626834").expect("OFFICE 00.02");
    let link = SheetRoom { model_id: office.model_id.clone(), room_id: "2626834".into() };
    let linked_ring = model_room_ring(&state, PROJECT, &link).unwrap().expect("the office's outline");
    let record = |room: Option<&SheetRoom>, outline: &[[f64; 2]]| -> SheetRecord {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1, "id": "s", "taken_at": "", "revit_sheet_id": "1", "document_title": "D",
            "sheet_number": "1", "sheet_name": "", "received_at": "", "sheet": {},
            "room": room, "room_outline": outline,
        }))
        .expect("a sheet record")
    };
    let stored: Vec<[f64; 2]> = linked_ring.iter().map(|&(x, y)| [x, y]).collect();
    let reversed: Vec<[f64; 2]> = stored.iter().rev().copied().collect();

    let from_link = template_ring(&state, PROJECT, Some(&rooms), &record(Some(&link), &[])).unwrap();
    assert_eq!(Ok(linked_ring.clone()), from_link);
    let from_outline = template_ring(&state, PROJECT, None, &record(None, &reversed)).unwrap();
    assert_eq!(
        Ok(linked_ring.clone()),
        from_outline.map(|r| rotate_to(&r, linked_ring[0])),
        "turned counter-clockwise"
    );

    assert_eq!(
        Err(SHEET_NAMES_NO_ROOM),
        template_ring(&state, PROJECT, Some(&rooms), &record(None, &[])).unwrap()
    );
    let gone = SheetRoom { model_id: office.model_id.clone(), room_id: "no-such-room".into() };
    assert_eq!(
        Err(SHEET_ROOM_NOT_ON_SERVER),
        template_ring(&state, PROJECT, Some(&rooms), &record(Some(&gone), &[])).unwrap()
    );
    assert_eq!(
        Ok(linked_ring),
        template_ring(&state, PROJECT, Some(&rooms), &record(Some(&gone), &stored)).unwrap(),
        "the stored outline wins over a link that no longer resolves"
    );
    std::fs::remove_dir_all(projects.parent().unwrap()).ok();
}

/// `ring` started at `first` (a reversed ring is the same ring from another vertex).
fn rotate_to(ring: &[(f64, f64)], first: (f64, f64)) -> Vec<(f64, f64)> {
    let at = ring.iter().position(|&p| p == first).unwrap_or(0);
    ring[at..].iter().chain(&ring[..at]).copied().collect()
}
