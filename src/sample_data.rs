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
