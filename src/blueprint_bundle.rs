//! The sheets and tags that go with a room blueprint, and what accepting them does (plan decision 27).
//!
//! A blueprint holds only its room; its sheets live in the sheet library bound to it and its tags in
//! the tag rules bound to it. They arrive together with a push of the blueprint (`Bundle`), wait with
//! the revision while it is pending, and are applied when it is accepted (or at once for a new
//! blueprint, which has nothing to wait for). **Applying reconciles**:
//!
//! - the sheets the push carries are bound to the blueprint; a sheet bound to it that the push no
//!   longer carries is **unbound** and stays in the library as a generic sheet;
//! - the tags (observations from Revit, or the rules of an exported bundle) are bound to it; a rule
//!   bound to it that the push no longer writes is **unlinked**: its samples are merged into the
//!   generic rule with the same key and it is removed (`tag_rule_binding::unlink_blueprint`).
//!
//! Tags are only reconciled when the bundle says it carries them (`observations` or `rules` present),
//! so a push that did not read tags never turns a blueprint's tags generic.
//!
//! The bundle is also what Export and Import move between projects: the bound sheets and rules, read
//! by `GET .../blueprints/{id}/bundle`, ride the file beside the room and go through the same apply.

use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http_error::{projects_dir, to_http};
use crate::service::blueprint_model::BlueprintSheet;
use crate::sheets::{self, PushRequest as SheetsPush, PushedSheet, SheetRoom};
use crate::state::Shared;
use crate::tag_rule_binding;
use crate::tag_rules::{Observation, ObservationOutcome, ObservationsRequest, TagRule};

/// What travels with a blueprint push. `observations` and `rules` are kept as JSON until they are applied.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Bundle {
    #[serde(default)]
    pub sheets: Vec<BlueprintSheet>,
    /// The type parameter the tags' codes were read from.
    #[serde(default)]
    pub type_code_parameter: String,
    /// Tags as read from Revit (`tag_rules::Observation`). Present, even when empty, means "the push read tags".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observations: Option<Value>,
    /// Tag rules of an exported bundle (`tag_rules::TagRule`, samples included).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Value>,
}

/// What applying a bundle did.
#[derive(Debug, Default, Serialize)]
pub struct Applied {
    /// Sheets the library took, bound to the blueprint.
    pub sheets: usize,
    /// Sheets that were bound and are no longer carried: generic now.
    pub unlinked_sheets: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<ObservationOutcome>,
    /// Rules that were bound and are no longer written: merged into generic.
    pub unlinked_tag_rules: usize,
}

fn invalid(what: &str, e: serde_json::Error) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, format!("the bundle's {what} cannot be read: {e}"))
}

/// The sheets go to the sheet library, **bound to the blueprint** and made for its room. The library
/// matches by sheet identity, so a re-push updates a sheet and a changed number waits for a person.
/// They carry no tags (the extractor takes those out before a sheet leaves Revit).
pub fn bind_to_library(
    state: &Shared,
    project: &str,
    blueprint: &str,
    room: Option<&SheetRoom>,
    sheets: &[BlueprintSheet],
) -> Result<usize, (StatusCode, String)> {
    let projects = projects_dir(state)?;
    let summary = room.and_then(|r| crate::sheets_api::summary_of(state, project, r));
    let pushed: Vec<PushedSheet> = sheets
        .iter()
        .map(|s| PushedSheet {
            revit_sheet_id: s.revit_sheet_id.clone(),
            document_title: s.document_title.clone(),
            sheet_number: s.sheet_number.clone(),
            sheet_name: s.sheet_name.clone(),
            room: room.cloned(),
            blueprint: Some(blueprint.to_string()),
            phase: None,
            room_summary: summary,
            size_mm: s.size_mm,
            rooms: Vec::new(),
            sheet: s.sheet.clone(),
        })
        .collect();
    sheets::push(&projects, project, SheetsPush { sheets: pushed })
        .map(|outcomes| outcomes.len())
        .map_err(to_http)
}

/// Apply a bundle to a blueprint that exists and whose revision is current: bind its sheets and tags
/// and unlink what it no longer carries. `room` is the blueprint's source room, when it has one.
pub fn apply(
    state: &Shared,
    project: &str,
    blueprint: &str,
    room: Option<&SheetRoom>,
    bundle: &Bundle,
) -> Result<Applied, (StatusCode, String)> {
    let projects = projects_dir(state)?;
    let mut applied = Applied::default();
    if !bundle.sheets.is_empty() {
        applied.sheets = bind_to_library(state, project, blueprint, room, &bundle.sheets)?;
        let keep: Vec<(String, String)> = bundle
            .sheets
            .iter()
            .map(|s| (s.revit_sheet_id.clone(), s.document_title.clone()))
            .collect();
        applied.unlinked_sheets = sheets::unbind_missing(&projects, project, blueprint, &keep).map_err(to_http)?;
    }
    let mut written: Option<std::collections::HashSet<String>> = None;
    if let Some(observations) = &bundle.observations {
        let observations: Vec<Observation> =
            serde_json::from_value(observations.clone()).map_err(|e| invalid("tag observations", e))?;
        let request = ObservationsRequest {
            blueprint: Some(blueprint.to_string()),
            type_code_parameter: bundle.type_code_parameter.clone(),
            observations,
        };
        let outcome = crate::tag_rules_api::observe_for(state, project, request)?;
        written.get_or_insert_with(Default::default).extend(outcome.rule_ids.iter().cloned());
        applied.tags = Some(outcome);
    }
    if let Some(rules) = &bundle.rules {
        let rules: Vec<TagRule> = serde_json::from_value(rules.clone()).map_err(|e| invalid("tag rules", e))?;
        let ids = tag_rule_binding::import_bound(&projects, project, blueprint, rules).map_err(to_http)?;
        written.get_or_insert_with(Default::default).extend(ids);
    }
    if let Some(keep) = written {
        applied.unlinked_tag_rules =
            tag_rule_binding::unlink_blueprint(&projects, project, blueprint, &keep).map_err(to_http)?;
    }
    Ok(applied)
}

/// A blueprint's bound sheets and rules, as a bundle to export.
pub fn export(state: &Shared, project: &str, blueprint: &str) -> Result<Bundle, (StatusCode, String)> {
    let projects = projects_dir(state)?;
    let records = sheets::bound_to(&projects, project, blueprint).map_err(to_http)?;
    let rules = tag_rule_binding::bound_to(&projects, project, blueprint).map_err(to_http)?;
    let type_code_parameter = rules
        .iter()
        .map(|r| r.key.code_parameter.clone())
        .find(|p| !p.is_empty())
        .unwrap_or_default();
    Ok(Bundle {
        sheets: records
            .iter()
            .map(|r| BlueprintSheet {
                revit_sheet_id: r.revit_sheet_id.clone(),
                document_title: r.document_title.clone(),
                sheet_number: r.sheet_number.clone(),
                sheet_name: r.sheet_name.clone(),
                size_mm: r.size_mm,
                sheet: r.sheet.clone(),
            })
            .collect(),
        type_code_parameter,
        observations: None,
        rules: Some(serde_json::to_value(&rules).map_err(|e| invalid("tag rules", e))?),
    })
}

/// `GET /projects/{id}/blueprints/{blueprint}/bundle`: what Export puts in the file beside the room.
pub async fn http_get_bundle(
    State(state): State<Shared>,
    UrlPath((project_id, blueprint)): UrlPath<(String, String)>,
) -> Result<Json<Bundle>, (StatusCode, String)> {
    export(&state, &project_id, &blueprint).map(Json)
}

#[cfg(test)]
mod tests {
    //! The apply step and the export, end to end through a real `AppState`: a room on the server, a
    //! blueprint on disk, sheets in the library and rules in the tag store. Fixtures are this module's
    //! own (the house rule is a helper per module, not a shared one).
    use super::*;
    use crate::blueprints::{self, CreateRequest};
    use crate::contract::{Level, Loop, Model, Point2D, Project, Room, RoomPayload, Snapshot, SUPPORTED_SCHEMA};
    use crate::service::blueprint_capture::{capture, SourceItem};
    use crate::service::blueprint_model::RevisionContent;
    use crate::state::{AppState, ProjectSettings};
    use crate::storage::MemStore;
    use crate::tag_rules::BoundaryOf;
    use serde_json::json;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roommate-bundle-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn settings() -> ProjectSettings {
        ProjectSettings {
            spaces: Default::default(),
            reference: std::collections::BTreeMap::new(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec!["$name".to_string()],
            milestones: vec![],
            anchor_model: None,
            tag_type_code: Some("Assembly Code".to_string()),
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            windows: Default::default(),
            ffe: Default::default(),
            hierarchy_exclusions: vec![],
            routing: Default::default(),
        }
    }

    fn state(dir: &std::path::Path, projects: &[&str]) -> Shared {
        let bundles = projects.iter().map(|p| (p.to_string(), settings())).collect();
        std::sync::Arc::new(
            AppState::new(Box::new(MemStore::new()), bundles, None).with_projects_dir(dir.to_path_buf()),
        )
    }

    /// Room 5 of model `m1`: 10 ft by 8 ft (3048 by 2438.4 mm), the corner at the origin.
    fn push_room(state: &Shared) {
        let p = |x: f64, y: f64| Point2D { x, y };
        state
            .set_snapshot(RoomPayload {
                schema_version: SUPPORTED_SCHEMA,
                project: Project { id: "p1".into(), name: "P".into() },
                model: Model { id: "m1".into(), name: "M".into(), source: "revit".into() },
                snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".into() },
                phase: None,
                model_to_shared: None,
                room_boundary: None,
                levels: vec![Level { id: "l1".into(), name: "Level 1".into(), elevation: 0.0 }],
                rooms: vec![Room {
                    enclosure: None,
                    id: "5".into(),
                    name: "Bed".into(),
                    level_id: "l1".into(),
                    loops: vec![Loop { points: vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 8.0), p(0.0, 8.0)] }],
                    properties: std::collections::BTreeMap::new(),
                }],
            })
            .unwrap();
    }

    /// A blueprint of one room holding items with these ids.
    fn make_blueprint(dir: &std::path::Path, project: &str, name: &str, ids: &[&str]) -> blueprints::Blueprint {
        let ring = vec![(0.0, 0.0), (3048.0, 0.0), (3048.0, 2438.4), (0.0, 2438.4)];
        let sources = ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let c = (500.0 + 700.0 * i as f64, 300.0);
                SourceItem {
                    id: id.to_string(),
                    family_name: Some("Bed".into()),
                    type_name: "Queen".into(),
                    category: "Furniture".into(),
                    footprint: vec![
                        (c.0 - 250.0, c.1 - 250.0),
                        (c.0 + 250.0, c.1 - 250.0),
                        (c.0 + 250.0, c.1 + 250.0),
                        (c.0 - 250.0, c.1 + 250.0),
                    ],
                    facing: Some((0.0, 1.0)),
                    nested: false,
                    properties: vec![],
                    type_properties: vec![],
                }
            })
            .collect();
        let content: RevisionContent = capture(&ring, sources).unwrap().content;
        let source = blueprints::SourceRoom { model_id: "m1".into(), room_id: "5".into(), room_name: "Bed".into() };
        blueprints::create(
            dir,
            project,
            CreateRequest { name: name.into(), source: Some(source), note: None, bundle: None, content },
        )
        .unwrap()
    }

    fn room_link() -> SheetRoom {
        SheetRoom { model_id: "m1".into(), room_id: "5".into() }
    }

    fn sheet(id: &str, number: &str) -> BlueprintSheet {
        BlueprintSheet {
            revit_sheet_id: id.into(),
            document_title: "m1".into(),
            sheet_number: number.into(),
            sheet_name: String::new(),
            size_mm: None,
            sheet: json!({ "view_ports": [] }),
        }
    }

    fn located(x: f64, y: f64) -> Value {
        json!({ "translation_coord": { "x": x, "y": y, "z": 0.0 }, "rotation_coord": { "data": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] } })
    }

    /// A tag on the bed with element id `element`, read in the plan of sheet "100" at 1:50.
    fn item_tag(tag_id: i64, element: i64, code: &str) -> Value {
        json!({
            "viewport_type": "floor plan", "view_id": 7, "view_scale": 50.0,
            "sheet": { "document_title": "m1", "revit_sheet_id": "100", "sheet_number": "RLS-1" },
            "tag": {
                "id": tag_id, "tag_kind": "independent",
                "tag_type": { "category": "Furniture Tags", "family_name": "FTag", "type_name": "Std" },
                "point": { "x": 800.0, "y": 300.0, "z": 0.0 },
                "tagged_elements": [{
                    "host_element_id": element, "category": "Furniture", "family_name": "Bed", "type_name": "Queen",
                    "type_code": code, "location_point": located(500.0, 300.0)
                }]
            }
        })
    }

    /// The room tag of room 5, its head 400 mm above the bottom wall, a third of the way along it.
    fn room_tag(tag_id: i64) -> Value {
        json!({
            "viewport_type": "floor plan", "view_id": 7, "view_scale": 50.0,
            "sheet": { "document_title": "m1", "revit_sheet_id": "100", "sheet_number": "RLS-1" },
            "tag": {
                "id": tag_id, "tag_kind": "room",
                "tag_type": { "category": "Room Tags", "family_name": "Room Tag", "type_name": "Std" },
                "point": { "x": 1016.0, "y": 400.0, "z": 0.0 },
                "tagged_elements": [{ "host_element_id": 5, "category": "Rooms", "location_point": located(1500.0, 1200.0) }]
            }
        })
    }

    fn bundle(sheets: Vec<BlueprintSheet>, tags: Vec<Value>) -> Bundle {
        Bundle {
            sheets,
            type_code_parameter: "Assembly Code".into(),
            observations: Some(Value::Array(tags)),
            rules: None,
        }
    }

    fn rules(dir: &std::path::Path, project: &str) -> Vec<TagRule> {
        crate::tag_rules::load_all(dir, project).unwrap()
    }

    #[test]
    fn test_a_new_blueprints_bundle_binds_its_sheets_and_tags_with_the_room_tag_against_its_wall() {
        let dir = temp_dir("new");
        let state = state(&dir, &["p1"]);
        push_room(&state);
        let bp = make_blueprint(&dir, "p1", "1 BED", &["101", "102"]);

        let applied = apply(
            &state,
            "p1",
            &bp.id,
            Some(&room_link()),
            &bundle(vec![sheet("100", "RLS-1")], vec![item_tag(1, 101, "BED-01"), room_tag(2)]),
        )
        .unwrap();

        assert_eq!((applied.sheets, applied.unlinked_sheets, applied.unlinked_tag_rules), (1, 0, 0));
        assert_eq!(applied.tags.as_ref().map(|t| (t.samples, t.skipped.len())), Some((2, 0)));
        let bound = sheets::bound_to(&dir, "p1", &bp.id).unwrap();
        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].room, Some(room_link()), "made for the blueprint's room");
        let all = rules(&dir, "p1");
        let item = all.iter().find(|r| r.key.tag_category == "Furniture Tags").unwrap();
        assert_eq!(
            (item.key.blueprint.as_str(), item.key.instance.as_str(), item.key.type_code.as_str()),
            (bp.id.as_str(), "101", "BED-01")
        );
        let room = all.iter().find(|r| r.key.tag_category == "Room Tags").unwrap();
        assert_eq!(room.key.blueprint, bp.id);
        let b = room
            .placement
            .boundary
            .expect("the room is on the server, so the tag is written against its wall");
        assert_eq!((b.of, b.edge), (BoundaryOf::Wall, 0));
        assert!((b.along - 1016.0 / 3048.0).abs() < 1e-6 && (b.off - 400.0 / 50.0).abs() < 1e-6, "{b:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_tag_on_an_item_the_blueprint_does_not_hold_stays_generic() {
        let dir = temp_dir("generic-item");
        let state = state(&dir, &["p1"]);
        push_room(&state);
        let bp = make_blueprint(&dir, "p1", "1 BED", &["101"]);
        apply(
            &state,
            "p1",
            &bp.id,
            Some(&room_link()),
            &bundle(vec![sheet("100", "RLS-1")], vec![item_tag(1, 101, "BED-01"), item_tag(2, 150, "BED-01")]),
        )
        .unwrap();
        let mut keys: Vec<(String, String)> =
            rules(&dir, "p1").into_iter().map(|r| (r.key.blueprint, r.key.instance)).collect();
        keys.sort();
        assert_eq!(keys, [("".to_string(), "".to_string()), (bp.id.clone(), "101".to_string())]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_applying_a_second_push_unbinds_the_sheet_it_dropped_and_unlinks_the_tag_it_no_longer_writes() {
        let dir = temp_dir("reconcile");
        let state = state(&dir, &["p1"]);
        push_room(&state);
        let bp = make_blueprint(&dir, "p1", "1 BED", &["101", "102"]);
        let first = bundle(
            vec![sheet("100", "RLS-1"), sheet("200", "RLS-2")],
            vec![item_tag(1, 101, "BED-01"), item_tag(2, 102, "BED-01")],
        );
        apply(&state, "p1", &bp.id, Some(&room_link()), &first).unwrap();
        assert_eq!((sheets::bound_to(&dir, "p1", &bp.id).unwrap().len(), rules(&dir, "p1").len()), (2, 2));

        // The re-push keeps sheet 100 and bed 101 and drops the other of each.
        let second = bundle(vec![sheet("100", "RLS-1")], vec![item_tag(1, 101, "BED-01")]);
        let applied = apply(&state, "p1", &bp.id, Some(&room_link()), &second).unwrap();

        assert_eq!((applied.unlinked_sheets, applied.unlinked_tag_rules), (1, 1));
        let still: Vec<String> = sheets::bound_to(&dir, "p1", &bp.id)
            .unwrap()
            .into_iter()
            .map(|s| s.revit_sheet_id)
            .collect();
        assert_eq!(still, ["100"]);
        let dropped = sheets::load_all(&dir, "p1")
            .unwrap()
            .into_iter()
            .find(|s| s.revit_sheet_id == "200")
            .unwrap();
        assert_eq!(dropped.blueprint, None, "kept in the library, generic again");
        let all = rules(&dir, "p1");
        assert_eq!(all.len(), 2);
        let generic = all
            .iter()
            .find(|r| r.key.blueprint.is_empty())
            .expect("bed 102's rule became the generic rule of its key");
        assert_eq!((generic.key.type_code.as_str(), generic.sample_count), ("BED-01", 1));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_bundle_that_read_no_tags_never_turns_the_blueprints_tags_generic() {
        let dir = temp_dir("no-tags");
        let state = state(&dir, &["p1"]);
        push_room(&state);
        let bp = make_blueprint(&dir, "p1", "1 BED", &["101"]);
        apply(
            &state,
            "p1",
            &bp.id,
            Some(&room_link()),
            &bundle(vec![sheet("100", "RLS-1")], vec![item_tag(1, 101, "BED-01")]),
        )
        .unwrap();
        let silent = Bundle {
            sheets: vec![sheet("100", "RLS-1")],
            type_code_parameter: String::new(),
            observations: None,
            rules: None,
        };
        let applied = apply(&state, "p1", &bp.id, Some(&room_link()), &silent).unwrap();
        assert_eq!((applied.unlinked_tag_rules, applied.tags.is_none()), (0, true));
        assert_eq!(rules(&dir, "p1")[0].key.blueprint, bp.id, "still bound");
        // Said to have read tags and found none: the rule is no longer written, so it goes generic.
        let none = bundle(vec![sheet("100", "RLS-1")], vec![]);
        assert_eq!(apply(&state, "p1", &bp.id, Some(&room_link()), &none).unwrap().unlinked_tag_rules, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_an_exported_bundle_imports_into_another_project_bound_to_the_new_blueprint() {
        let dir = temp_dir("export");
        let state = state(&dir, &["p1", "p2"]);
        push_room(&state);
        let bp = make_blueprint(&dir, "p1", "1 BED", &["101"]);
        apply(
            &state,
            "p1",
            &bp.id,
            Some(&room_link()),
            &bundle(vec![sheet("100", "RLS-1")], vec![item_tag(1, 101, "BED-01"), room_tag(2)]),
        )
        .unwrap();

        let exported = export(&state, "p1", &bp.id).unwrap();
        assert_eq!(exported.sheets.len(), 1);
        assert_eq!(exported.type_code_parameter, "Assembly Code");
        assert!(exported.observations.is_none(), "an export carries the rules, not the observations");

        let copy = make_blueprint(&dir, "p2", "1 BED", &["101"]);
        let applied = apply(&state, "p2", &copy.id, None, &exported).unwrap();
        assert_eq!(applied.sheets, 1);
        let imported = rules(&dir, "p2");
        assert_eq!(imported.len(), 2);
        assert!(imported.iter().all(|r| r.key.blueprint == copy.id), "re-keyed to the new blueprint");
        assert!(imported.iter().any(|r| r.key.instance == "101" && r.sample_count == 1));
        assert_eq!(sheets::bound_to(&dir, "p2", &copy.id).unwrap().len(), 1);
        assert_eq!(rules(&dir, "p1").len(), 2, "the source project is untouched");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_bundle_that_cannot_be_read_is_refused_and_a_server_with_no_directory_says_so() {
        let dir = temp_dir("refuse");
        let state = state(&dir, &["p1"]);
        push_room(&state);
        let bp = make_blueprint(&dir, "p1", "1 BED", &["101"]);
        let bad = Bundle { observations: Some(json!("not a list")), ..Default::default() };
        let (status, message) = apply(&state, "p1", &bp.id, None, &bad).unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(message.contains("tag observations"), "{message}");

        let bare: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), Default::default(), None));
        assert_eq!(
            apply(&bare, "p1", "x", None, &Bundle::default()).unwrap_err().0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
