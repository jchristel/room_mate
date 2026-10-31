//! HTTP adapters for tag rules -- the thin half of `crate::tag_rules`.
//!
//! The extractor `POST`s observations (the tags it read from a sheet's viewports); everyone else
//! reads. Reads are also MCP tools; writes are HTTP-only, as for blueprints and sheets.
//!
//! `observe_for` is the one way tags are taken in: the Sheets button's route calls it directly, and a
//! blueprint's bundle calls it when its revision is accepted (`blueprint_bundle::apply`), so both
//! see the same blueprint items and the same room boundaries.

use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    Json,
};

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::blueprints;
use crate::http_error::{projects_dir, to_http};
use crate::service::blueprint_geometry::Pt;
use crate::service::elevation_facing::model_room_ring;
use crate::service::room_tag_place::boundary_of;
use crate::sheets::SheetRoom;
use crate::state::Shared;
use crate::tag_rules::{
    self, Boundary, ObservationOutcome, ObservationsRequest, ObserveContext, RuleSummary, TagRule, P3,
};

/// Which blueprint each sheet of a push is bound to (by document title and Revit sheet id), and each such
/// blueprint's item ids.
type BoundSheets = (HashMap<(String, String), String>, HashMap<String, HashSet<String>>);

/// Rooms read for one push, by `(model id, room id)`; `None` is a room the server does not hold.
type RoomRings = RefCell<HashMap<(String, i64), Option<Vec<Pt>>>>;

pub async fn http_list_tag_rules(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
) -> Result<Json<Vec<RuleSummary>>, (StatusCode, String)> {
    tag_rules::list(&projects_dir(&state)?, &project_id).map(Json).map_err(to_http)
}

pub async fn http_get_tag_rule(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
) -> Result<Json<TagRule>, (StatusCode, String)> {
    tag_rules::load(&projects_dir(&state)?, &project_id, &id).map(Json).map_err(to_http)
}

/// The extractor's push: observations in, rules out. 200 with how many samples were taken in and
/// which tags were skipped and why.
pub async fn http_observe_tags(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(request): Json<ObservationsRequest>,
) -> Result<Json<ObservationOutcome>, (StatusCode, String)> {
    observe_for(&state, &project_id, request).map(Json)
}

/// Take observations in with what the server knows around them (plan decision 27): the blueprint the
/// push is for and its items, and the rooms the room tags were placed in. Also the way a blueprint's
/// bundle is applied (`blueprint_bundle::apply`).
pub fn observe_for(
    state: &Shared,
    project_id: &str,
    request: ObservationsRequest,
) -> Result<ObservationOutcome, (StatusCode, String)> {
    let projects = projects_dir(state)?;
    let blueprint = match request.blueprint.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        Some(id) => Some((id.to_string(), blueprint_item_ids(&projects, project_id, id)?)),
        None => None,
    };
    // A push that names no blueprint still keeps the tags of a sheet that is bound to one bound.
    let (sheet_blueprints, blueprint_items) = if blueprint.is_some() {
        Default::default()
    } else {
        bound_sheets_of(&projects, project_id, &request)
    };
    // A room is read from the server once per push, however many tags are on it.
    let rooms: RoomRings = RefCell::default();
    let boundary = |model: &str, room: i64, head: P3, bound: bool, scale: f64| -> Option<Boundary> {
        let mut cache = rooms.borrow_mut();
        let ring = cache
            .entry((model.to_string(), room))
            .or_insert_with(|| {
                let at = SheetRoom { model_id: model.to_string(), room_id: room.to_string() };
                model_room_ring(state, project_id, &at).ok().flatten()
            })
            .as_deref()?;
        boundary_of(ring, (head.x, head.y), bound, scale)
    };
    let ctx = ObserveContext { blueprint, sheet_blueprints, blueprint_items, boundary: Some(&boundary) };
    tag_rules::observe(&projects, project_id, request, &ctx).map_err(to_http)
}

/// The blueprint each sheet of a push is bound to in the library, and those blueprints' items. A blueprint
/// that cannot be read is skipped: its sheets' tags are then taken in as generic, as they were before.
fn bound_sheets_of(projects: &std::path::Path, project: &str, request: &ObservationsRequest) -> BoundSheets {
    let wanted: HashSet<(&str, &str)> = request
        .observations
        .iter()
        .map(|o| (o.sheet.document_title.as_str(), o.sheet.revit_sheet_id.as_str()))
        .collect();
    let mut by_sheet = HashMap::new();
    let mut items: HashMap<String, HashSet<String>> = HashMap::new();
    for record in crate::sheets::load_all(projects, project).unwrap_or_default() {
        let Some(blueprint) = record.blueprint.clone() else {
            continue;
        };
        if !wanted.contains(&(record.document_title.as_str(), record.revit_sheet_id.as_str())) {
            continue;
        }
        if !items.contains_key(&blueprint) {
            match blueprint_item_ids(projects, project, &blueprint) {
                Ok(ids) => {
                    items.insert(blueprint.clone(), ids);
                }
                Err(_) => continue,
            }
        }
        by_sheet.insert((record.document_title, record.revit_sheet_id), blueprint);
    }
    (by_sheet, items)
}

/// The ids of the items a room blueprint holds in its current revision: a tag on one of them is bound
/// to the blueprint. (A revision waiting to be accepted does not count: its tags wait with it.)
fn blueprint_item_ids(
    projects: &std::path::Path,
    project: &str,
    blueprint: &str,
) -> Result<HashSet<String>, (StatusCode, String)> {
    let b = blueprints::load(projects, project, blueprint).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("the tags name blueprint {blueprint:?}, which cannot be read: {e:?}"),
        )
    })?;
    Ok(b.current()
        .into_iter()
        .flat_map(|r| r.content.items.iter())
        .map(|i| i.id.clone())
        .collect())
}

pub async fn http_delete_tag_rule(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    tag_rules::delete(&projects_dir(&state)?, &project_id, &id).map_err(to_http)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    //! `observe_for` through a real `AppState`: the blueprint's items and the room a room tag sat in are
    //! read from the server, which is what the store alone cannot do. Fixtures are this module's own.
    use super::*;
    use crate::blueprints::{CreateRequest, PushRequest, SourceRoom};
    use crate::contract::{Level, Loop, Model, Point2D, Project, Room, RoomPayload, Snapshot, SUPPORTED_SCHEMA};
    use crate::service::blueprint_capture::{capture, SourceItem};
    use crate::state::{AppState, ProjectSettings};
    use crate::storage::MemStore;
    use crate::tag_rules::{BoundaryOf, Observation};
    use serde_json::{json, Value};
    use std::path::{Path, PathBuf};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roommate-observe-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn state(dir: Option<&Path>) -> Shared {
        let settings = ProjectSettings {
            spaces: Default::default(),
            reference: std::collections::BTreeMap::new(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec!["$name".to_string()],
            milestones: vec![],
            anchor_model: None,
            tag_type_code: None,
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            windows: Default::default(),
            ffe: Default::default(),
            hierarchy_exclusions: vec![],
            routing: Default::default(),
        };
        let state = AppState::new(Box::new(MemStore::new()), [("p1".to_string(), settings)].into(), None);
        std::sync::Arc::new(match dir {
            Some(d) => state.with_projects_dir(d.to_path_buf()),
            None => state,
        })
    }

    /// Room 5 of model `m1`, 10 ft by 8 ft with its corner at the origin.
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

    fn content(ids: &[&str]) -> crate::service::blueprint_model::RevisionContent {
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
        capture(&ring, sources).unwrap().content
    }

    fn make_blueprint(dir: &Path, ids: &[&str]) -> String {
        let source = SourceRoom { model_id: "m1".into(), room_id: "5".into(), room_name: "Bed".into() };
        blueprints::create(
            dir,
            "p1",
            CreateRequest {
                name: "1 BED".into(),
                source: Some(source),
                note: None,
                bundle: None,
                content: content(ids),
            },
        )
        .unwrap()
        .id
    }

    fn observation(v: Value) -> Observation {
        serde_json::from_value(v).unwrap()
    }

    fn item_tag(tag_id: i64, element: i64) -> Observation {
        observation(json!({
            "viewport_type": "floor plan", "view_id": 7, "view_scale": 50.0,
            "sheet": { "document_title": "m1", "revit_sheet_id": "100", "sheet_number": "RLS-1" },
            "tag": {
                "id": tag_id, "tag_kind": "independent",
                "tag_type": { "category": "Furniture Tags", "family_name": "FTag", "type_name": "Std" },
                "point": { "x": 800.0, "y": 300.0, "z": 0.0 },
                "tagged_elements": [{
                    "host_element_id": element, "category": "Furniture", "family_name": "Bed", "type_name": "Queen",
                    "type_code": "BED-01",
                    "location_point": { "translation_coord": { "x": 500.0, "y": 300.0, "z": 0.0 },
                        "rotation_coord": { "data": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] } }
                }]
            }
        }))
    }

    fn room_tag(tag_id: i64, room: i64) -> Observation {
        observation(json!({
            "viewport_type": "floor plan", "view_id": 7, "view_scale": 50.0,
            "sheet": { "document_title": "m1", "revit_sheet_id": "100", "sheet_number": "RLS-1" },
            "tag": {
                "id": tag_id, "tag_kind": "room",
                "tag_type": { "category": "Room Tags", "family_name": "Room Tag", "type_name": "Std" },
                "point": { "x": 1016.0, "y": 400.0, "z": 0.0 },
                "tagged_elements": [{ "host_element_id": room, "category": "Rooms",
                    "location_point": { "translation_coord": { "x": 1500.0, "y": 1200.0, "z": 0.0 } } }]
            }
        }))
    }

    fn request(blueprint: Option<&str>, parameter: &str, observations: Vec<Observation>) -> ObservationsRequest {
        ObservationsRequest {
            blueprint: blueprint.map(str::to_string),
            type_code_parameter: parameter.to_string(),
            observations,
        }
    }

    fn rules(dir: &Path) -> Vec<TagRule> {
        tag_rules::load_all(dir, "p1").unwrap()
    }

    #[test]
    fn test_a_tag_on_one_of_the_blueprints_items_is_bound_and_a_revision_still_pending_does_not_count() {
        let dir = temp_dir("items");
        let state = state(Some(&dir));
        let id = make_blueprint(&dir, &["101"]);
        // A re-push waiting to be accepted brings item 150; its tags wait with it, so 150 is not yet the blueprint's.
        blueprints::push(
            &dir,
            "p1",
            &id,
            PushRequest { origin: None, note: None, bundle: None, content: content(&["101", "150"]) },
        )
        .unwrap();

        let out = observe_for(
            &state,
            "p1",
            request(Some(&id), "Assembly Code", vec![item_tag(1, 101), item_tag(2, 150)]),
        )
        .unwrap();

        assert_eq!((out.samples, out.skipped.len(), out.rule_ids.len()), (2, 0, 2));
        let mut scopes: Vec<(String, String)> =
            rules(&dir).into_iter().map(|r| (r.key.blueprint, r.key.instance)).collect();
        scopes.sort();
        assert_eq!(scopes, [("".to_string(), "".to_string()), (id.clone(), "101".to_string())]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_blueprint_that_cannot_be_read_is_a_bad_request_that_names_it() {
        let dir = temp_dir("missing");
        let state = state(Some(&dir));
        let (status, message) =
            observe_for(&state, "p1", request(Some("nope"), "Assembly Code", vec![item_tag(1, 101)])).unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(message.contains("nope"), "{message}");
        assert!(rules(&dir).is_empty(), "nothing was taken in");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_room_tag_is_written_against_a_wall_for_a_blueprint_a_rectangle_side_without_one_and_the_axes_when_the_room_is_missing(
    ) {
        let dir = temp_dir("rooms");
        let state = state(Some(&dir));
        push_room(&state);
        let id = make_blueprint(&dir, &["101"]);

        observe_for(&state, "p1", request(Some(&id), "", vec![room_tag(1, 5)])).unwrap();
        let bound = rules(&dir).remove(0);
        let b = bound.placement.boundary.expect("room 5 is on the server");
        assert_eq!((bound.key.blueprint.as_str(), b.of), (id.as_str(), BoundaryOf::Wall));

        // The same tag pushed as a generic one moves its sample: one rule, now against the rectangle.
        observe_for(&state, "p1", request(None, "", vec![room_tag(1, 5)])).unwrap();
        let all = rules(&dir);
        assert_eq!(all.len(), 1);
        assert_eq!(
            (all[0].key.blueprint.as_str(), all[0].placement.boundary.map(|b| b.of)),
            ("", Some(BoundaryOf::Rectangle))
        );

        // A tag on a room the server does not hold keeps only the offset in the plan's axes.
        observe_for(&state, "p1", request(None, "", vec![room_tag(2, 9)])).unwrap();
        // (It joins the generic room rule, whose answer keeps the boundary the other sample gives it.)
        let sample = rules(&dir).into_iter().flat_map(|r| r.samples).find(|s| s.source.tag_id == 2).unwrap();
        assert_eq!(sample.boundary, None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_without_a_type_code_parameter_item_tags_are_skipped_with_a_reason_and_room_tags_are_kept() {
        let dir = temp_dir("no-parameter");
        let state = state(Some(&dir));
        push_room(&state);
        let out = observe_for(&state, "p1", request(None, "", vec![item_tag(1, 101), room_tag(2, 5)])).unwrap();
        assert_eq!((out.samples, out.skipped.len()), (1, 1));
        assert!(out.skipped[0].reason.contains("tag_type_code"), "{}", out.skipped[0].reason);
        assert_eq!(rules(&dir).len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_server_with_no_settings_directory_says_so() {
        let state = state(None);
        let (status, _) = observe_for(&state, "p1", request(None, "Assembly Code", vec![])).unwrap_err();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    fn bind_sheet(dir: &Path, sheet_id: &str, blueprint: Option<&str>) {
        let pushed = crate::sheets::PushedSheet {
            revit_sheet_id: sheet_id.into(),
            document_title: "m1".into(),
            sheet_number: format!("RLS-{sheet_id}"),
            sheet_name: String::new(),
            room: None,
            blueprint: blueprint.map(str::to_string),
            phase: None,
            room_summary: None,
            size_mm: None,
            rooms: vec![],
            sheet: json!({ "view_ports": [] }),
        };
        crate::sheets::push(dir, "p1", crate::sheets::PushRequest { sheets: vec![pushed] }).unwrap();
    }

    fn on_sheet(mut o: Observation, sheet_id: &str) -> Observation {
        o.sheet.revit_sheet_id = sheet_id.into();
        o
    }

    #[test]
    fn test_the_sheets_button_after_a_blueprint_push_keeps_a_bound_sheets_tags_bound() {
        let dir = temp_dir("sheets-button");
        let state = state(Some(&dir));
        push_room(&state);
        let id = make_blueprint(&dir, &["101"]);
        bind_sheet(&dir, "100", Some(&id));
        bind_sheet(&dir, "200", None);

        // One push, naming no blueprint (the Sheets button): sheet 100 is bound, sheet 200 is generic.
        let tags = vec![item_tag(1, 101), on_sheet(item_tag(2, 101), "200"), room_tag(3, 5)];
        let out = observe_for(&state, "p1", request(None, "Assembly Code", tags)).unwrap();
        assert_eq!((out.samples, out.skipped.len()), (3, 0));

        let all = rules(&dir);
        let bound_item = all
            .iter()
            .find(|r| r.key.tag_category == "Furniture Tags" && !r.key.blueprint.is_empty())
            .expect("sheet 100's tag stayed bound");
        assert_eq!(
            (bound_item.key.blueprint.as_str(), bound_item.key.instance.as_str()),
            (id.as_str(), "101")
        );
        assert_eq!(
            bound_item
                .samples
                .iter()
                .map(|s| s.source.sheet.revit_sheet_id.as_str())
                .collect::<Vec<_>>(),
            ["100"]
        );
        let generic_item = all
            .iter()
            .find(|r| r.key.tag_category == "Furniture Tags" && r.key.blueprint.is_empty())
            .expect("sheet 200's tag is generic");
        assert_eq!(
            generic_item
                .samples
                .iter()
                .map(|s| s.source.sheet.revit_sheet_id.as_str())
                .collect::<Vec<_>>(),
            ["200"]
        );
        // The room tag came from sheet 100 too, so it is bound and written against the wall.
        let room = all.iter().find(|r| r.key.tag_category == "Room Tags").unwrap();
        assert_eq!(
            (room.key.blueprint.as_str(), room.placement.boundary.map(|b| b.of)),
            (id.as_str(), Some(BoundaryOf::Wall))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_pushing_a_bound_sheets_tags_again_does_not_move_them_and_a_named_blueprint_wins() {
        let dir = temp_dir("again");
        let state = state(Some(&dir));
        let id = make_blueprint(&dir, &["101"]);
        bind_sheet(&dir, "100", Some(&id));
        observe_for(&state, "p1", request(Some(&id), "Assembly Code", vec![item_tag(1, 101)])).unwrap();
        let before: Vec<String> = rules(&dir).into_iter().map(|r| r.id).collect();

        observe_for(&state, "p1", request(None, "Assembly Code", vec![item_tag(1, 101)])).unwrap();
        let after = rules(&dir);
        assert_eq!(
            after.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
            before,
            "the same rule, not a generic copy"
        );
        assert_eq!(after[0].sample_count, 1, "replaced, not doubled");

        // A sheet bound to a blueprint that is named explicitly in the push is that blueprint's, whatever the library says.
        let other = {
            let source = SourceRoom { model_id: "m1".into(), room_id: "5".into(), room_name: "Bed".into() };
            blueprints::create(
                &dir,
                "p1",
                CreateRequest {
                    name: "2 BED".into(),
                    source: Some(source),
                    note: None,
                    bundle: None,
                    content: content(&["101"]),
                },
            )
            .unwrap()
            .id
        };
        observe_for(&state, "p1", request(Some(&other), "Assembly Code", vec![item_tag(1, 101)])).unwrap();
        let moved = rules(&dir);
        assert_eq!((moved.len(), moved[0].key.blueprint.as_str()), (1, other.as_str()));
        std::fs::remove_dir_all(&dir).ok();
    }
}
