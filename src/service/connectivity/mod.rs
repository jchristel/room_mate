//! Door connectivity: which rooms a person can walk between, and the shortest
//! route from one room to another.
//!
//! Transport-agnostic like every `service` module: the HTTP handler and the MCP
//! tool are both thin adapters over `assemble_connectivity`.
//!
//! **This is the doors-only graph, and that is the point of it, not a
//! shortcut.** Two rooms are connected here when a door joins them. A bay, an
//! open-plan area, an archway modelled as a wall opening or a shaft has no door,
//! so it is reported as **isolated** — a *finding about the method*, never about
//! the building. Isolated rooms are the worklist for authored connections, which
//! is why they are listed rather than hidden. See `docs/PLAN-connectivity.md`.
//!
//! **Not `/adjacency`, and it does not borrow its nodes.** Adjacency is shared
//! wall geometry on one level, and its nodes carry a bare room id. A room id is
//! unique only within a model, so every node here is a model-qualified
//! `RoomRef`. The door sides come from `OpeningResponse::room_origin`, which has
//! already applied the project's resolution policy (authored first, geometry
//! only to fill an absent side), so this module resolves nothing itself.
//!
//! **Edges are doors, open-zone walls, vertical links and passages.** A passage is
//! a straight line a person drew between two rooms on one storey, for a way through
//! that no door and no shared wall shows. Its edge carries TWO points, where the
//! line leaves the first room and where it enters the second, so the route is
//! centre, first crossing, second crossing, centre, and its width and height are
//! the ones the person stated.
//!
//! **Signal, not error, throughout.** A door naming one room is an *exit* on
//! that room, not an edge to an invented "outside". A door naming none is
//! counted. A door whose rooms fall outside the requested scope (another
//! building) is counted. No route between two rooms is a result with a reason
//! (`found: false`), never an HTTP error — only a room that does not exist, or
//! an ambiguous id, is the caller's fault.
//!
//! **The weight is an approximate walking distance**: centroid to the door's
//! insertion point to the next centroid. A centroid can lie outside an L-shaped
//! room and the straight line can cross a wall, so this ranks routes well and is
//! not a figure to quote as a distance. `Metric::Hops` is the alternative.
//!
//! **On length** (CODING-CONVENTIONS "Module structure & length"): past the
//! ~500-line trigger, but more than 300 of the lines are tests over one algorithm
//! (join, components, Dijkstra, resolve) that share a fixture. The seam if it
//! grows is `link`/`components`/`shortest`, which are already pure functions.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap};

use serde::Serialize;

use crate::connections;
use crate::contract::{DoorPayload, Level, Point2D, Room};
use crate::settings::RoutingPolicy;
use crate::state::AppState;

use super::adjacency::{centroid_of, compute_adjacency_indexed, default_wall_max};
use super::openings::{assemble_openings, OpeningKind, OpeningScope};
use super::room_locator::{outline_of, RoomRef};
use super::rooms::{assemble_rooms, RoomFilter, RoomScope};
use super::routing::{self, Method};
use super::ServiceError;

mod clearance;
mod edges;
mod heights;
mod paths;
use clearance::*;
use edges::*;
pub use heights::*;

mod graph;
mod types;
pub use graph::*;
pub(in crate::service) use paths::*;
pub use types::*;

pub const SCHEMA_VERSION: u32 = 1;

/// What narrows a connectivity read.
#[derive(Default)]
pub struct ConnectivityScope<'a> {
    pub building: Option<&'a str>,
    pub milestone: Option<&'a str>,
    /// Applies to the DOORS, so a predicate such as a fire rating or a width
    /// removes a door from the graph without a feature of its own. Rooms are
    /// deliberately not filtered: a graph missing the rooms a door leads to is
    /// worse than no graph (the rule `/adjacency` states for `?filter=`).
    pub door_filter: Option<&'a RoomFilter>,
    /// The width, in mm, of an object that has to make the trip. A door or an open wall
    /// narrower than this is not passable; absent or 0 checks nothing, as before. See
    /// [`Clearance`].
    pub clear_width_mm: Option<f64>,
    /// What the frame takes off a door's footprint width to give its clear opening.
    /// Absent means the project's `[routing] door_frame_allowance_mm`.
    pub frame_allowance_mm: Option<f64>,
    /// The height, in mm, of an object that has to make the trip. A door or a ROOM lower
    /// than this is not passable; absent or 0 checks nothing. See [`Clearance`].
    pub clear_height_mm: Option<f64>,
    /// The room property that holds a room's clear height, matched ignoring case and
    /// runs of spaces (one real project's is `Ceiling  Height`, with two). Absent means the project's
    /// `[routing] room_height_property`.
    pub height_property: Option<&'a str>,
    /// Treat every open zone's closed walls as open: ignore its `disconnects` and its
    /// `hub`. For a question about the building ("how far does a walk of L reach?") and not
    /// about how somebody drew their routes, where a cut is a statement about what a
    /// person walks through and a coverage question is not bound by it.
    pub ignore_cuts: bool,
}

/// One node per room, each carrying its clear height: from a ceiling when one covers it,
/// else from the room's height property.
fn room_nodes(
    rooms: &super::rooms::RoomsResult,
    from_ceilings: &BTreeMap<RoomRef, f64>,
    height_key: &str,
) -> Vec<Node> {
    rooms
        .rooms
        .iter()
        .map(|r| {
            let room = RoomRef { model_id: r.model_id.clone(), room_id: r.room.id.clone() };
            let ceiling = from_ceilings.get(&room).copied();
            Node {
                name: r.room.name.clone(),
                level_id: r.room.level_id.clone(),
                centroid: centroid_of(&r.room),
                degree: 0,
                exits: 0,
                component: 0,
                height_mm: ceiling.or_else(|| property_mm(&r.room.properties, &[height_key])),
                height_from_ceiling: ceiling.is_some(),
                room,
            }
        })
        .collect()
}

/// What a door's frame and stops take off the width its footprint measures, in mm.
///
/// The footprint is the door's overall width in plan, frame included: on a real project a
/// "970 x 2040" leaf has a footprint of about 1,055 mm, and its clear opening is nearer 900.
/// 150 is a rule of thumb for a standard single door, **an estimate and not a
/// measurement**, which is why a caller can state their own and why every width here
/// is reported as an estimate. No door in the real project measured carries a width property to use instead.
pub const DEFAULT_FRAME_ALLOWANCE_MM: f64 = RoutingPolicy::DEFAULT_DOOR_FRAME_ALLOWANCE_MM;

const MM_PER_FT: f64 = 304.8;

/// The asked width and frame allowance, from the two optional numbers a caller sends.
/// A width of 0 is "no check". Shared by the HTTP handler and the MCP tool so the two
/// cannot disagree on what is valid.
pub fn width_params(width_mm: Option<f64>, frame_mm: Option<f64>) -> Result<(Option<f64>, Option<f64>), ServiceError> {
    let ok = |v: f64, name: &str, max: f64| {
        if v.is_finite() && (0.0..=max).contains(&v) {
            Ok(v)
        } else {
            Err(ServiceError::Invalid(format!("{name} must be between 0 and {max} mm, not {v}")))
        }
    };
    let width = width_mm.map(|w| ok(w, "width_mm", 100_000.0)).transpose()?.filter(|w| *w > 0.0);
    let frame = frame_mm.map(|f| ok(f, "frame_mm", 2_000.0)).transpose()?;
    Ok((width, frame))
}

// ================================ wire shape ================================

/// Assemble the connectivity graph for one project, scoped like `/rooms`, and
/// the shortest route between two rooms when asked for one.
///
/// Reuses `assemble_rooms` and `assemble_openings` rather than reading storage,
/// so milestone pins, building scope, level dedup, the placement frame and the
/// door resolution policy all arrive as they do everywhere else. `building`
/// scopes the ROOMS only: a door is kept or dropped by whether its rooms are in
/// that set, which is what "a building's doors" means for a graph.
///
/// `Ok(None)` when no rooms have ever been pushed (the adapter's 204).
pub fn assemble_connectivity(
    state: &AppState,
    project: &str,
    scope: &ConnectivityScope<'_>,
    route_between: Option<(&Endpoint, &Endpoint)>,
    metric: Metric,
    method: Method,
    detail: Detail,
) -> Result<Option<ConnectivityResult>, ServiceError> {
    let Some(GraphParts {
        rooms,
        mut nodes,
        edges,
        counts,
        revision,
        doors_pushed,
        connections: connections_report,
        frame_allowance_mm,
    }) = build_graph(state, project, scope)?
    else {
        return Ok(None);
    };
    let components = components(&mut nodes, &edges);
    let isolated = isolated_rooms(&nodes, &components);

    let (path, clearance) = match route_between {
        None => (None, None),
        Some((from, to)) => {
            let (i, j) = (resolve(&nodes, from, "start")?, resolve(&nodes, to, "end")?);
            let ask = routing::Ask { from: i, to: j, from_at: from.at, to_at: to.at };
            let fit = Fit {
                width_mm: scope.clear_width_mm,
                height_mm: scope.clear_height_mm,
                frame_allowance_mm,
            };
            let (path, clearance) = route_asked(&nodes, &edges, &outlines(&rooms, &nodes), &ask, &fit, method, metric);
            (Some(path), Some(clearance))
        }
    };

    Ok(Some(ConnectivityResult {
        schema_version: SCHEMA_VERSION,
        revision,
        metric,
        method: method.id(),
        methods: routing::CATALOG,
        doors_pushed,
        levels: rooms.levels,
        nodes: (detail == Detail::Full).then_some(nodes),
        edges: (detail == Detail::Full).then_some(edges),
        components,
        isolated,
        counts,
        connections: connections_report,
        path,
        clearance,
    }))
}

/// What the zones and links did to the graph, written into the report a person reads: the
/// edges each zone added, the pairs a door already joined, and the links that were
/// redundant and so not applied.
fn record_what_applied(
    report: &mut ConnectionsReport,
    tally: &BTreeMap<String, (usize, usize)>,
    redundant_links: &std::collections::BTreeSet<String>,
) {
    for zone in &mut report.zones {
        let (added, redundant) = tally.get(&zone.id).copied().unwrap_or((0, 0));
        zone.edges = added;
        zone.redundant = redundant;
    }
    for link in &mut report.links {
        if redundant_links.contains(&link.id) {
            link.applied = false;
            link.reason = Some("a door, open zone or earlier link already joins these two rooms".to_string());
        }
    }
}

/// Rooms nothing reaches: those alone in their component.
fn isolated_rooms(nodes: &[Node], components: &[Component]) -> Vec<IsolatedRoom> {
    nodes
        .iter()
        .filter(|n| components[n.component].size == 1)
        .map(|n| IsolatedRoom {
            room: n.room.clone(),
            name: n.name.clone(),
            level_id: n.level_id.clone(),
            exits: n.exits,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(model: &str, id: &str, level: &str, x: f64, y: f64) -> Node {
        Node {
            room: RoomRef { model_id: model.to_string(), room_id: id.to_string() },
            name: id.to_string(),
            level_id: level.to_string(),
            centroid: Point2D { x, y },
            degree: 0,
            exits: 0,
            component: 0,
            height_mm: None,
            height_from_ceiling: false,
        }
    }

    fn rr(model: &str, id: &str) -> Option<RoomRef> {
        Some(RoomRef { model_id: model.to_string(), room_id: id.to_string() })
    }

    fn door(id: &str, from: Option<RoomRef>, to: Option<RoomRef>, at: Option<(f64, f64)>) -> DoorFact {
        DoorFact {
            door_id: id.to_string(),
            door_model_id: "m1".to_string(),
            from,
            to,
            point: at.map(|(x, y)| (Point2D { x, y }, PointSource::Insertion)),
            width_mm: None,
            height_mm: None,
        }
    }

    fn graph(nodes: Vec<Node>, doors: Vec<DoorFact>) -> (Vec<Node>, Vec<Edge>, Vec<Component>, Counts) {
        let Graph { mut nodes, edges, counts } = link(nodes, doors);
        let comps = components(&mut nodes, &edges);
        (nodes, edges, comps, counts)
    }

    fn ep(model: Option<&str>, id: &str) -> Endpoint {
        Endpoint { model_id: model.map(str::to_string), room_id: id.to_string(), at: None }
    }

    /// A--B--C in a row, D alone: the doors decide connectivity, D is isolated,
    /// and a route A to C goes through B.
    #[test]
    fn test_doors_join_rooms_and_a_room_without_one_is_isolated() {
        let (nodes, edges, comps, counts) = graph(
            vec![
                room("m1", "A", "L1", 0.0, 0.0),
                room("m1", "B", "L1", 10.0, 0.0),
                room("m1", "C", "L1", 20.0, 0.0),
                room("m1", "D", "L1", 0.0, 50.0),
            ],
            vec![
                door("d1", rr("m1", "A"), rr("m1", "B"), Some((5.0, 0.0))),
                door("d2", rr("m1", "B"), rr("m1", "C"), Some((15.0, 0.0))),
            ],
        );
        assert_eq!(counts.edges, 2);
        assert_eq!(comps.iter().map(|c| c.size).collect::<Vec<_>>(), vec![3, 1], "largest component first");
        let d = nodes.iter().find(|n| n.room.room_id == "D").unwrap();
        assert_eq!(comps[d.component].size, 1);

        let (a, c) = (
            resolve(&nodes, &ep(None, "A"), "start").unwrap(),
            resolve(&nodes, &ep(None, "C"), "end").unwrap(),
        );
        let path = route(&nodes, &edges, a, c, Metric::Distance);
        assert!(path.found);
        assert_eq!(path.rooms.iter().map(|r| r.room_id.as_str()).collect::<Vec<_>>(), vec!["A", "B", "C"]);
        assert!((path.distance_ft - 20.0).abs() < 1e-9, "centroid-door-centroid along a line is the line");
        assert_eq!(path.segments.len(), 1, "one level is one polyline");
    }

    /// No route is a finding with a reason naming both components, not an error.
    #[test]
    fn test_no_route_is_a_finding() {
        let (nodes, edges, _, _) =
            graph(vec![room("m1", "A", "L1", 0.0, 0.0), room("m1", "D", "L1", 0.0, 50.0)], vec![]);
        let path = route(&nodes, &edges, 0, 1, Metric::Distance);
        assert!(!path.found);
        assert!(path.reason.as_deref().unwrap().contains("component"));
        assert!(path.rooms.is_empty());
    }

    /// The shorter of two routes wins under `Distance`, the fewer-door one under
    /// `Hops` — which is the whole reason the metric is a parameter.
    #[test]
    fn test_metric_decides_between_a_short_detour_and_a_long_direct_door() {
        // A-B direct through a door far off the line; A-X-Y-B through three
        // doors that stay on it.
        let nodes = vec![
            room("m1", "A", "L1", 0.0, 0.0),
            room("m1", "B", "L1", 30.0, 0.0),
            room("m1", "X", "L1", 10.0, 0.0),
            room("m1", "Y", "L1", 20.0, 0.0),
        ];
        let doors = vec![
            door("far", rr("m1", "A"), rr("m1", "B"), Some((15.0, 100.0))),
            door("a-x", rr("m1", "A"), rr("m1", "X"), Some((5.0, 0.0))),
            door("x-y", rr("m1", "X"), rr("m1", "Y"), Some((15.0, 0.0))),
            door("y-b", rr("m1", "Y"), rr("m1", "B"), Some((25.0, 0.0))),
        ];
        let (nodes, edges, _, _) = graph(nodes, doors);
        let (a, b) = (
            resolve(&nodes, &ep(None, "A"), "start").unwrap(),
            resolve(&nodes, &ep(None, "B"), "end").unwrap(),
        );

        let by_distance = route(&nodes, &edges, a, b, Metric::Distance);
        assert_eq!(by_distance.steps.len(), 3, "three doors on the line beat one door 100 ft away");
        let by_hops = route(&nodes, &edges, a, b, Metric::Hops);
        assert_eq!(by_hops.steps.len(), 1);
        assert_eq!(by_hops.steps[0].door_id.as_deref(), Some("far"));
    }

    /// One resolved side is an exit, no side is unattached, a room outside the
    /// scope is counted, and none of the three becomes an edge.
    #[test]
    fn test_doors_that_are_not_edges_are_counted_not_dropped() {
        let (nodes, edges, _, counts) = graph(
            vec![room("m1", "A", "L1", 0.0, 0.0), room("m1", "B", "L1", 10.0, 0.0)],
            vec![
                door("ext", rr("m1", "A"), None, None),
                door("none", None, None, None),
                door("self", rr("m1", "A"), rr("m1", "A"), None),
                door("away", rr("m1", "A"), rr("m1", "elsewhere"), None),
                door("ok", rr("m1", "A"), rr("m1", "B"), None),
            ],
        );
        assert_eq!(
            counts,
            Counts { doors: 5, edges: 1, exits: 1, unattached: 1, same_room: 1, out_of_scope: 1 }
        );
        assert_eq!(nodes.iter().find(|n| n.room.room_id == "A").unwrap().exits, 1);
        assert_eq!(
            edges[0].point_source,
            PointSource::Midpoint,
            "a door with no point is placed between the rooms"
        );
        assert_eq!((edges[0].point.x, edges[0].point.y), (5.0, 0.0));
    }

    /// A room id is unique only within a model. Two models both holding `r1` are
    /// two rooms, a bare id is refused and says which models, and naming the
    /// model resolves it.
    #[test]
    fn test_a_bare_id_in_two_models_is_refused_not_guessed() {
        let (nodes, _, _, _) = graph(vec![room("m1", "r1", "L1", 0.0, 0.0), room("m2", "r1", "L1", 0.0, 0.0)], vec![]);
        let err = resolve(&nodes, &ep(None, "r1"), "start").unwrap_err();
        let ServiceError::Invalid(msg) = err else {
            panic!("expected Invalid")
        };
        assert!(msg.contains("m1") && msg.contains("m2"), "{msg}");
        assert!(resolve(&nodes, &ep(Some("m2"), "r1"), "start").is_ok());
        assert!(resolve(&nodes, &ep(None, "nope"), "end").is_err());
    }

    /// A door between rooms in different models joins them: the edge is between
    /// qualified refs, not bare ids.
    #[test]
    fn test_a_door_can_join_rooms_in_different_models() {
        let (nodes, edges, comps, _) = graph(
            vec![room("arch", "r1", "L1", 0.0, 0.0), room("fit", "r1", "L1", 10.0, 0.0)],
            vec![door("d", rr("arch", "r1"), rr("fit", "r1"), Some((5.0, 0.0)))],
        );
        assert_eq!(comps.len(), 1);
        assert_eq!(edges[0].a.model_id, "arch");
        assert_eq!(edges[0].b.model_id, "fit");
        assert_eq!(nodes.len(), 2);
    }

    /// A door between two levels splits the polyline where the level changes,
    /// so each zone gets its own slice.
    #[test]
    fn test_a_route_across_levels_splits_into_one_segment_per_level() {
        let (nodes, edges, _, _) = graph(
            vec![room("m1", "A", "L1", 0.0, 0.0), room("m1", "B", "L2", 10.0, 0.0)],
            vec![door("d", rr("m1", "A"), rr("m1", "B"), Some((5.0, 0.0)))],
        );
        let path = route(&nodes, &edges, 0, 1, Metric::Distance);
        assert_eq!(path.segments.len(), 2);
        assert_eq!(path.segments[0].level_id, "L1");
        assert_eq!(path.segments[0].points.len(), 2, "the centroid and the door");
        assert_eq!(path.segments[1].level_id, "L2");
        assert_eq!(path.segments[1].points.len(), 1);
    }

    /// Both ends or neither; a blank counts as absent.
    #[test]
    fn test_endpoints_need_both_ends_or_neither() {
        assert!(endpoints(None, None, None, None).unwrap().is_none());
        assert!(endpoints(Some(" "), None, Some(""), None).unwrap().is_none());
        assert!(endpoints(Some("a"), None, None, None).is_err());
        let (a, b) = endpoints(Some("a"), Some("m1"), Some("b"), Some("")).unwrap().unwrap();
        assert_eq!((a.model_id.as_deref(), b.model_id), (Some("m1"), None));
    }
    #[test]
    fn test_metric_parses_and_rejects() {
        assert_eq!(Metric::parse(None).unwrap(), Metric::Distance);
        assert_eq!(Metric::parse(Some("hops")).unwrap(), Metric::Hops);
        assert!(Metric::parse(Some("fastest")).is_err());
    }

    /// A route from a room to itself is found, free and one room long.
    #[test]
    fn test_a_route_to_the_same_room_is_trivial() {
        let (nodes, edges, _, _) = graph(vec![room("m1", "A", "L1", 0.0, 0.0)], vec![]);
        let path = route(&nodes, &edges, 0, 0, Metric::Distance);
        assert!(path.found);
        assert_eq!(path.rooms.len(), 1);
        assert_eq!(path.cost, 0.0);
    }

    // ---------- the read, over a real store ----------

    mod assembled {
        use std::collections::{BTreeMap, HashMap};

        use super::*;
        use crate::contract::{
            CustomValue, Loop, Model, Opening, Project, Room, RoomPayload, Snapshot, SUPPORTED_DOOR_SCHEMA,
            SUPPORTED_SCHEMA,
        };
        use crate::state::ProjectSettings;
        use crate::storage::MemStore;

        fn bundle() -> ProjectSettings {
            ProjectSettings {
                spaces: Default::default(),
                reference: BTreeMap::new(),
                hierarchy: vec![],
                builtin_properties: vec![],
                room_label: vec![],
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
            }
        }

        fn rect(id: &str, x0: f64, x1: f64) -> Room {
            Room {
                enclosure: None,
                id: id.to_string(),
                name: id.to_string(),
                level_id: "lvl1".to_string(),
                loops: vec![Loop {
                    points: vec![
                        Point2D { x: x0, y: 0.0 },
                        Point2D { x: x1, y: 0.0 },
                        Point2D { x: x1, y: 10.0 },
                        Point2D { x: x0, y: 10.0 },
                    ],
                }],
                properties: BTreeMap::new(),
            }
        }

        fn door(id: &str, from: Option<&str>, to: Option<&str>, fire: &str) -> Opening {
            let mut properties = BTreeMap::new();
            properties.insert("Fire Rating".into(), CustomValue { value: fire.to_string(), storage_type: None });
            Opening {
                id: id.to_string(),
                level_id: "lvl1".to_string(),
                loops: vec![],
                from_room: from.map(str::to_string),
                to_room: to.map(str::to_string),
                insertion_point: None,
                through_wall_normal: None,
                sub_components: Vec::new(),
                type_id: "t1".to_string(),
                type_name: "Single".to_string(),
                properties,
                type_properties: Default::default(),
            }
        }

        fn rooms_payload(rooms: Vec<Room>) -> RoomPayload {
            RoomPayload {
                schema_version: SUPPORTED_SCHEMA,
                project: Project { id: "p1".into(), name: "P".into() },
                model: Model { id: "m1".into(), name: "M".into(), source: "revit".into() },
                snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".into() },
                phase: Some("New Construction".into()),
                model_to_shared: None,
                room_boundary: Some(crate::contract::RoomBoundary::Centreline),
                levels: vec![
                    Level { id: "lvl1".into(), name: "Level 1".into(), elevation: 0.0 },
                    Level { id: "lvl2".into(), name: "Level 2".into(), elevation: 12.0 },
                    Level { id: "lvl3".into(), name: "Level 3".into(), elevation: 24.0 },
                ],
                rooms,
            }
        }

        fn empty_state() -> AppState {
            AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), bundle())]), None)
        }

        /// Three rooms in a row (`a | b | c`) plus a bay `d` away from them.
        fn state(doors: Vec<Opening>) -> AppState {
            state_of(
                vec![
                    rect("a", 0.0, 10.0),
                    rect("b", 10.0, 20.0),
                    rect("c", 20.0, 30.0),
                    rect("d", 40.0, 50.0),
                ],
                doors,
            )
        }

        /// A room on a named level.
        fn rect_on(id: &str, x0: f64, x1: f64, level: &str) -> Room {
            Room { level_id: level.to_string(), ..rect(id, x0, x1) }
        }

        fn state_of(rooms: Vec<Room>, doors: Vec<Opening>) -> AppState {
            let state = empty_state();
            state.set_snapshot(rooms_payload(rooms)).unwrap();
            state
                .set_door_snapshot(DoorPayload {
                    schema_version: SUPPORTED_DOOR_SCHEMA,
                    project: Project { id: "p1".into(), name: "P".into() },
                    model: Model { id: "m1".into(), name: "M".into(), source: "revit".into() },
                    snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".into() },
                    phase: Some("New Construction".into()),
                    model_to_shared: None,
                    levels: vec![],
                    doors,
                })
                .unwrap();
            state
        }

        /// Doors a-b and b-c (the second fire rated), and one leaving `a`.
        fn standard() -> Vec<Opening> {
            vec![
                door("ab", Some("a"), Some("b"), "none"),
                door("bc", Some("b"), Some("c"), "FRL60"),
                door("out", Some("a"), None, "none"),
            ]
        }

        #[test]
        fn test_the_read_builds_the_graph_and_lists_the_bay() {
            let s = state(standard());
            let result = assemble_connectivity(
                &s,
                "p1",
                &ConnectivityScope::default(),
                None,
                Metric::Distance,
                Method::Centroid,
                Detail::Full,
            )
            .unwrap()
            .expect("rooms were pushed");
            assert!(result.doors_pushed);
            assert_eq!(result.counts.edges, 2);
            assert_eq!(result.counts.exits, 1);
            assert_eq!(result.isolated.len(), 1);
            assert_eq!(result.isolated[0].room.room_id, "d");
            assert!(result.path.is_none());
            // A door with no point sits between its rooms' centroids, so the walk
            // from a to b is the 10 ft between them.
            let ab = result
                .edges
                .as_ref()
                .unwrap()
                .iter()
                .find(|e| e.door_id.as_deref() == Some("ab"))
                .unwrap();
            assert!((ab.length - 10.0).abs() < 1e-6, "{}", ab.length);
        }

        #[test]
        fn test_the_read_routes_and_reports_an_unreachable_bay() {
            let (a, c, d) = (ep(None, "a"), ep(None, "c"), ep(None, "d"));
            let s = state(standard());
            let ok = assemble_connectivity(
                &s,
                "p1",
                &ConnectivityScope::default(),
                Some((&a, &c)),
                Metric::Distance,
                Method::Centroid,
                Detail::Full,
            )
            .unwrap()
            .unwrap();
            let path = ok.path.unwrap();
            assert!(path.found);
            assert_eq!(path.rooms.len(), 3);
            assert!((path.distance_ft - 20.0).abs() < 1e-6);

            let bay = assemble_connectivity(
                &s,
                "p1",
                &ConnectivityScope::default(),
                Some((&a, &d)),
                Metric::Distance,
                Method::Centroid,
                Detail::Full,
            )
            .unwrap()
            .unwrap();
            assert!(!bay.path.unwrap().found, "the bay has no door, which is a finding and not an error");
        }

        /// The door filter removes a door from the graph, so a route that needed
        /// it stops existing, which is how "avoid fire doors" is asked.
        #[test]
        fn test_a_door_filter_removes_a_door_from_the_graph() {
            let known = std::collections::BTreeSet::new();
            let filter = RoomFilter::parse_query("Fire Rating=none", &known).unwrap();
            let scope = ConnectivityScope { door_filter: Some(&filter), ..Default::default() };
            let (a, c) = (ep(None, "a"), ep(None, "c"));
            let result = assemble_connectivity(
                &state(standard()),
                "p1",
                &scope,
                Some((&a, &c)),
                Metric::Distance,
                Method::Centroid,
                Detail::Full,
            )
            .unwrap()
            .unwrap();
            assert_eq!(result.counts.doors, 2, "the fire-rated door is not in the graph");
            assert!(!result.path.unwrap().found);
        }

        /// Rooms but no doors snapshot at all: every room is isolated and the
        /// response says no doors were pushed, rather than reporting a building
        /// with no doors.
        #[test]
        fn test_no_doors_pushed_is_stated() {
            let state = empty_state();
            state.set_snapshot(rooms_payload(vec![rect("a", 0.0, 10.0)])).unwrap();
            let result = assemble_connectivity(
                &state,
                "p1",
                &ConnectivityScope::default(),
                None,
                Metric::Distance,
                Method::Centroid,
                Detail::Full,
            )
            .unwrap()
            .unwrap();
            assert!(!result.doors_pushed);
            assert_eq!(result.isolated.len(), 1);
        }

        // ---------- open zones ----------

        fn zone_input(id: &str, rooms: &[&str]) -> crate::connections::ZoneInput {
            crate::connections::ZoneInput {
                id: id.to_string(),
                name: format!("Zone {id}"),
                kind: crate::connections::ZoneKind::Open,
                rooms: rooms
                    .iter()
                    .map(|r| crate::connections::RoomInput { room_id: r.to_string(), model_id: Some("m1".to_string()) })
                    .collect(),
                disconnects: vec![],
                hub: None,
                note: None,
            }
        }

        /// The standard rooms and doors, with `zones` authored for the project.
        /// Rooms a|b|c share walls in a row, `d` stands apart; only a-b has a door.
        fn zoned(tag: &str, zones: Vec<crate::connections::ZoneInput>) -> (AppState, std::path::PathBuf) {
            let dir = std::env::temp_dir().join(format!("roommate-conn-zones-{tag}-{}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok(); // a stale one from a run with the same process id would read as saved data
            std::fs::create_dir_all(&dir).unwrap();
            let state = state(vec![door("ab", Some("a"), Some("b"), "none")]).with_projects_dir(dir.clone());
            crate::connections::save(
                &state,
                &dir,
                "p1",
                crate::connections::SaveRequest {
                    base: String::new(),
                    zones,
                    links: vec![],
                    routes: None,
                    passages: None,
                    reaches: None,
                },
            )
            .unwrap();
            (state, dir)
        }

        fn read(state: &AppState, route: Option<(&Endpoint, &Endpoint)>) -> ConnectivityResult {
            assemble_connectivity(
                state,
                "p1",
                &ConnectivityScope::default(),
                route,
                Metric::Distance,
                Method::Centroid,
                Detail::Full,
            )
            .unwrap()
            .unwrap()
        }

        /// The point of the whole feature: a zone joins the rooms that share a
        /// wall, so a room no door reaches becomes routable, and the route says
        /// which hop was a door and which was the zone.
        #[test]
        fn test_a_zone_connects_rooms_that_share_a_wall() {
            let (s, dir) = zoned("joins", vec![zone_input("z1", &["b", "c"])]);
            let (a, c) = (ep(None, "a"), ep(None, "c"));
            let result = read(&s, Some((&a, &c)));

            let path = result.path.as_ref().unwrap();
            assert!(path.found, "{:?}", path.reason);
            let kinds: Vec<_> = path.steps.iter().map(|s| s.kind).collect();
            assert_eq!(kinds, vec![EdgeKind::Door, EdgeKind::Zone]);
            assert_eq!(path.steps[1].zone_id.as_deref(), Some("z1"));
            // The zone hop runs through the middle of the b|c wall, x = 20.
            assert!((path.steps[1].point.x - 20.0).abs() < 1e-6);
            // c is no longer isolated; d still is.
            assert_eq!(result.isolated.iter().map(|r| r.room.room_id.as_str()).collect::<Vec<_>>(), vec!["d"]);
            assert_eq!(result.counts.edges, 1, "counts stay a count of DOORS");
            let zone = &result.connections.zones[0];
            assert_eq!((zone.edges, zone.redundant), (1, 0));
            assert!(zone.reaches_doors && zone.unlinked.is_empty() && zone.stale.is_empty());
            assert!(!result.connections.pinned);
            std::fs::remove_dir_all(&dir).ok();
        }

        /// Members that share no wall cannot be joined by a zone: they are
        /// reported rather than connected by a made-up line.
        #[test]
        fn test_members_with_no_shared_wall_are_reported_unlinked() {
            let (s, dir) = zoned("unlinked", vec![zone_input("z1", &["c", "d"])]);
            let result = read(&s, None);
            let zone = &result.connections.zones[0];
            assert_eq!(zone.edges, 0);
            assert_eq!(zone.unlinked.len(), 2);
            assert!(!zone.reaches_doors, "neither c nor d has a door: the zone is an island");
            assert_eq!(result.isolated.len(), 2, "c and d are still unreachable");
            std::fs::remove_dir_all(&dir).ok();
        }

        /// A pair a door already joins is not joined twice, and the zone says it
        /// was redundant. A member that is not in the model is stale, not an error.
        #[test]
        fn test_a_zone_over_a_door_is_redundant_and_a_missing_member_is_stale() {
            let (s, dir) = zoned("redundant", vec![zone_input("z1", &["a", "b", "ghost"])]);
            let result = read(&s, None);
            let zone = &result.connections.zones[0];
            assert_eq!((zone.edges, zone.redundant), (0, 1));
            assert_eq!(zone.stale.len(), 1);
            assert_eq!(zone.stale[0].room_id, "ghost");
            assert_eq!(result.edges.as_ref().unwrap().len(), 1, "still one door edge and nothing added");
            std::fs::remove_dir_all(&dir).ok();
        }

        /// With no authored document the read is exactly the doors-only graph.
        #[test]
        fn test_without_connections_nothing_changes() {
            let result = read(&state(standard()), None);
            assert!(result.connections.zones.is_empty());
            assert_eq!(result.connections.taken_at, "");
            assert!(result.connections.error.is_none());
        }

        // ---------- vertical links ----------

        fn link_input(id: &str, a: &str, b: &str, cost: Option<f64>) -> crate::connections::LinkInput {
            crate::connections::LinkInput {
                id: id.to_string(),
                a: crate::connections::RoomInput { room_id: a.to_string(), model_id: Some("m1".to_string()) },
                b: crate::connections::RoomInput { room_id: b.to_string(), model_id: Some("m1".to_string()) },
                cost_ft: cost,
                note: None,
            }
        }

        /// Three storeys: `a | b` on level 1, `e | f` on level 2 and `g` on level
        /// 3, a door joining each pair on one storey and nothing between storeys.
        fn storeys(tag: &str, links: Vec<crate::connections::LinkInput>) -> (AppState, std::path::PathBuf) {
            let dir = std::env::temp_dir().join(format!("roommate-conn-links-{tag}-{}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok(); // a stale one from a run with the same process id would read as saved data
            std::fs::create_dir_all(&dir).unwrap();
            let rooms = vec![
                rect_on("a", 0.0, 10.0, "lvl1"),
                rect_on("b", 10.0, 20.0, "lvl1"),
                rect_on("e", 10.0, 20.0, "lvl2"),
                rect_on("f", 20.0, 30.0, "lvl2"),
                rect_on("g", 20.0, 30.0, "lvl3"),
            ];
            let doors = vec![
                door("ab", Some("a"), Some("b"), "none"),
                door("ef", Some("e"), Some("f"), "none"),
            ];
            let state = state_of(rooms, doors).with_projects_dir(dir.clone());
            crate::connections::save(
                &state,
                &dir,
                "p1",
                crate::connections::SaveRequest {
                    base: String::new(),
                    zones: vec![],
                    links,
                    routes: None,
                    passages: None,
                    reaches: None,
                },
            )
            .unwrap();
            (state, dir)
        }

        /// A link a person drew joins the storeys, the route says which hop was the
        /// level change, charges the link's own cost, and splits into one polyline
        /// per level so each viewer zone draws its own slice.
        #[test]
        fn test_a_vertical_link_joins_two_storeys_at_its_own_cost() {
            let (s, dir) = storeys("joins", vec![link_input("up1", "b", "e", Some(25.0))]);
            let (a, f) = (ep(None, "a"), ep(None, "f"));
            let result = read(&s, Some((&a, &f)));

            let path = result.path.as_ref().unwrap();
            assert!(path.found, "{:?}", path.reason);
            let kinds: Vec<_> = path.steps.iter().map(|s| s.kind).collect();
            assert_eq!(kinds, vec![EdgeKind::Door, EdgeKind::Vertical, EdgeKind::Door]);
            let hop = &path.steps[1];
            assert_eq!(hop.link_id.as_deref(), Some("up1"));
            assert!(hop.zone_id.is_none());
            assert!((hop.length - 25.0).abs() < 1e-9, "the stated cost, not a distance");
            assert_eq!(
                path.segments.iter().map(|s| s.level_id.as_str()).collect::<Vec<_>>(),
                vec!["lvl1", "lvl2"]
            );
            let report = &result.connections.links[0];
            assert!(report.applied && report.reason.is_none());
            assert_eq!(report.levels_between, 0, "floor to floor");
            std::fs::remove_dir_all(&dir).ok();
        }

        /// A room may be in an open zone AND at the end of a vertical link: the two
        /// records are independent, and a lift lobby that is part of an open
        /// landing is the ordinary case. Here `c` is in the zone `b | c` and is one
        /// end of the link to `e`, so a route from `a` takes a door, a zone hop
        /// and then the level change, in that order.
        #[test]
        fn test_a_room_can_be_in_an_open_zone_and_a_vertical_link() {
            let dir = std::env::temp_dir().join(format!("roommate-conn-both-{}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok(); // a stale one from a run with the same process id would read as saved data
            std::fs::create_dir_all(&dir).unwrap();
            let rooms = vec![
                rect_on("a", 0.0, 10.0, "lvl1"),
                rect_on("b", 10.0, 20.0, "lvl1"),
                rect_on("c", 20.0, 30.0, "lvl1"),
                rect_on("e", 20.0, 30.0, "lvl2"),
            ];
            let state = state_of(rooms, vec![door("ab", Some("a"), Some("b"), "none")]).with_projects_dir(dir.clone());
            crate::connections::save(
                &state,
                &dir,
                "p1",
                crate::connections::SaveRequest {
                    base: String::new(),
                    zones: vec![zone_input("landing", &["b", "c"])],
                    links: vec![link_input("up", "c", "e", Some(10.0))],
                    routes: None,
                    passages: None,
                    reaches: None,
                },
            )
            .unwrap();
            let (a, e) = (ep(None, "a"), ep(None, "e"));
            let result = read(&state, Some((&a, &e)));
            let path = result.path.as_ref().unwrap();
            assert!(path.found, "{:?}", path.reason);
            let kinds: Vec<_> = path.steps.iter().map(|s| s.kind).collect();
            assert_eq!(kinds, vec![EdgeKind::Door, EdgeKind::Zone, EdgeKind::Vertical]);
            assert!(result.connections.links[0].applied);
            assert!(result.connections.zones[0].stale.is_empty());
            std::fs::remove_dir_all(&dir).ok();
        }

        // ---------- widths ----------

        /// A door whose footprint is `width_mm + frame` wide, so its estimated clear
        /// opening is exactly `width_mm` at the default 150 mm allowance. Placed at the
        /// middle of the wall the two rooms share.
        fn sized(id: &str, from: &str, to: &str, clear_mm: f64) -> Opening {
            let w = (clear_mm + DEFAULT_FRAME_ALLOWANCE_MM) / MM_PER_FT;
            let mut o = door(id, Some(from), Some(to), "none");
            o.insertion_point = Some(Point2D { x: 10.0, y: 5.0 });
            o.loops = vec![Loop {
                points: vec![
                    Point2D { x: 10.0 - w / 2.0, y: 4.8 },
                    Point2D { x: 10.0 + w / 2.0, y: 4.8 },
                    Point2D { x: 10.0 + w / 2.0, y: 5.2 },
                    Point2D { x: 10.0 - w / 2.0, y: 5.2 },
                ],
            }];
            o
        }

        fn read_width(state: &AppState, from: &str, to: &str, width_mm: Option<f64>) -> ConnectivityResult {
            let (a, b) = (ep(None, from), ep(None, to));
            assemble_connectivity(
                state,
                "p1",
                &ConnectivityScope { clear_width_mm: width_mm, ..Default::default() },
                Some((&a, &b)),
                Metric::Distance,
                Method::Centroid,
                Detail::Full,
            )
            .unwrap()
            .unwrap()
        }

        /// A door narrower than the object is not a way through, and the answer says what
        /// WOULD fit and what limits it, instead of a bare "no route".
        #[test]
        fn test_a_door_too_narrow_blocks_the_route_and_the_answer_names_it() {
            let s = state(vec![sized("ab", "a", "b", 900.0), sized("bc", "b", "c", 600.0)]);
            let fits = read_width(&s, "a", "c", Some(550.0));
            assert!(fits.path.as_ref().unwrap().found);
            assert!((fits.clearance.as_ref().unwrap().narrowest.as_ref().unwrap().width_mm - 600.0).abs() < 1.0);

            let blocked = read_width(&s, "a", "c", Some(700.0));
            let path = blocked.path.as_ref().unwrap();
            assert!(!path.found);
            let reason = path.reason.as_deref().unwrap();
            assert!(
                reason.contains("700 mm") && reason.contains("600 mm") && reason.contains("door bc"),
                "{reason}"
            );
            let widest = blocked.clearance.as_ref().unwrap().widest_possible.as_ref().unwrap();
            assert_eq!(widest.door_id.as_deref(), Some("bc"));
        }

        /// With no width asked nothing changes, but the readout is still there: the widest
        /// object that could make the trip is worth knowing before guessing a width.
        #[test]
        fn test_without_a_width_the_route_is_unchanged_and_the_readout_says_what_fits() {
            let s = state(vec![sized("ab", "a", "b", 900.0), sized("bc", "b", "c", 600.0)]);
            let r = read_width(&s, "a", "c", None);
            assert!(r.path.as_ref().unwrap().found);
            let c = r.clearance.as_ref().unwrap();
            assert!(c.asked_mm.is_none());
            assert!((c.widest_possible.as_ref().unwrap().width_mm - 600.0).abs() < 1.0);
            assert_eq!(c.unchecked_hops, 0);
        }

        /// A wide enough alternative is taken, so the width shapes the route and not only
        /// whether one exists.
        #[test]
        fn test_a_width_sends_the_route_the_wide_way_round() {
            let s = state(vec![
                sized("ab", "a", "b", 600.0),
                sized("ac", "a", "c", 1200.0),
                sized("cb", "c", "b", 1200.0),
            ]);
            let direct: Vec<_> = read_width(&s, "a", "b", None)
                .path
                .unwrap()
                .rooms
                .iter()
                .map(|r| r.room_id.clone())
                .collect();
            assert_eq!(direct, vec!["a", "b"]);
            let wide = read_width(&s, "a", "b", Some(1000.0)).path.unwrap();
            assert_eq!(wide.rooms.iter().map(|r| r.room_id.as_str()).collect::<Vec<_>>(), vec!["a", "c", "b"]);
        }

        /// A door with no footprint has no width to check, so it passes and is counted.
        #[test]
        fn test_a_door_with_no_footprint_is_passable_and_counted_unchecked() {
            let s = state(vec![door("ab", Some("a"), Some("b"), "none")]);
            let r = read_width(&s, "a", "b", Some(5000.0));
            assert!(r.path.as_ref().unwrap().found);
            let c = r.clearance.unwrap();
            assert_eq!(c.unchecked_hops, 1);
            assert!(c.narrowest.is_none());
        }

        /// An open zone's opening is the wall its two rooms share: 10 ft is 3,048 mm.
        #[test]
        fn test_an_open_areas_wall_is_as_wide_as_the_wall_the_rooms_share() {
            let (s, dir) = zoned("width", vec![zone_input("z", &["b", "c"])]);
            assert!(read_width(&s, "b", "c", Some(3000.0)).path.unwrap().found);
            let blocked = read_width(&s, "b", "c", Some(3100.0));
            assert!(!blocked.path.as_ref().unwrap().found);
            let widest = blocked.clearance.unwrap().widest_possible.unwrap();
            assert_eq!(widest.kind, EdgeKind::Zone);
            assert!((widest.width_mm - 3048.0).abs() < 1.0, "{}", widest.width_mm);
            std::fs::remove_dir_all(&dir).ok();
        }

        /// A lift or stair has no width here, so a level change is passable and counted.
        #[test]
        fn test_a_level_change_is_not_checked_and_says_so() {
            let (s, dir) = storeys("width", vec![link_input("up", "b", "e", Some(10.0))]);
            let (a, f) = (ep(None, "a"), ep(None, "f"));
            let r = assemble_connectivity(
                &s,
                "p1",
                &ConnectivityScope { clear_width_mm: Some(1500.0), ..Default::default() },
                Some((&a, &f)),
                Metric::Distance,
                Method::Centroid,
                Detail::Full,
            )
            .unwrap()
            .unwrap();
            assert!(r.path.as_ref().unwrap().found, "{:?}", r.path.as_ref().unwrap().reason);
            assert_eq!(
                r.clearance.unwrap().unchecked_hops,
                3,
                "two doors without a footprint and the level change"
            );
            std::fs::remove_dir_all(&dir).ok();
        }

        #[test]
        fn test_width_parameters_are_validated() {
            assert_eq!(width_params(Some(0.0), None).unwrap(), (None, None), "0 is no check");
            assert_eq!(width_params(Some(900.0), Some(100.0)).unwrap(), (Some(900.0), Some(100.0)));
            assert!(width_params(Some(-1.0), None).is_err());
            assert!(width_params(Some(f64::NAN), None).is_err());
            assert!(width_params(None, Some(5000.0)).is_err());
        }

        // ---------- heights ----------

        /// A door whose type name states `height_mm` the way real ones do ("970 x 2040").
        fn sized_tall(id: &str, from: &str, to: &str, clear_mm: f64, height_mm: f64) -> Opening {
            let mut o = sized(id, from, to, clear_mm);
            o.type_name = format!("{} x {} TD01", clear_mm as u32, height_mm as u32);
            o
        }

        /// A room with a ceiling height in the property a real project uses, TWO SPACES and all.
        fn with_height(mut room: Room, mm: &str) -> Room {
            room.properties.insert(
                "Ceiling  Height".into(),
                CustomValue { value: mm.to_string(), storage_type: Some("String".into()) },
            );
            room
        }

        fn read_size(
            state: &AppState,
            from: &str,
            to: &str,
            width: Option<f64>,
            height: Option<f64>,
        ) -> ConnectivityResult {
            let (a, b) = (ep(None, from), ep(None, to));
            assemble_connectivity(
                state,
                "p1",
                &ConnectivityScope { clear_width_mm: width, clear_height_mm: height, ..Default::default() },
                Some((&a, &b)),
                Metric::Distance,
                Method::Centroid,
                Detail::Full,
            )
            .unwrap()
            .unwrap()
        }

        fn rooms_abc(heights: [Option<&str>; 3]) -> Vec<Room> {
            ["a", "b", "c"]
                .iter()
                .zip([(0.0, 10.0), (10.0, 20.0), (20.0, 30.0)])
                .zip(heights)
                .map(|((id, (x0, x1)), h)| {
                    let r = rect(id, x0, x1);
                    match h {
                        Some(h) => with_height(r, h),
                        None => r,
                    }
                })
                .collect()
        }

        #[test]
        fn test_the_height_in_a_door_type_name_is_read_and_a_low_door_blocks() {
            let s = state_of(
                rooms_abc([None, None, None]),
                vec![
                    sized_tall("ab", "a", "b", 900.0, 2040.0),
                    sized_tall("bc", "b", "c", 900.0, 1900.0),
                ],
            );
            let ok = read_size(&s, "a", "c", None, Some(1800.0));
            assert!(ok.path.as_ref().unwrap().found);
            let lowest = ok.clearance.as_ref().unwrap().lowest.as_ref().unwrap();
            assert_eq!((lowest.kind, lowest.door_id.as_deref()), (HeightKind::Door, Some("bc")));
            assert!((lowest.height_mm - 1900.0).abs() < 1e-9);

            let blocked = read_size(&s, "a", "c", None, Some(2000.0));
            let path = blocked.path.as_ref().unwrap();
            assert!(!path.found);
            let reason = path.reason.as_deref().unwrap();
            assert!(
                reason.contains("2000 mm tall") && reason.contains("1900 mm") && reason.contains("door bc"),
                "{reason}"
            );
            assert_eq!(blocked.clearance.unwrap().tallest_possible.unwrap().door_id.as_deref(), Some("bc"));
        }

        /// The point of checking rooms too: a doorway can be tall enough and the room
        /// behind it still too low (a plant room, a undercroft).
        #[test]
        fn test_a_room_too_low_blocks_the_route_and_is_named() {
            let s = state_of(
                rooms_abc([Some("2700"), Some("2100"), Some("2700")]),
                vec![
                    sized_tall("ab", "a", "b", 900.0, 2400.0),
                    sized_tall("bc", "b", "c", 900.0, 2400.0),
                ],
            );
            assert!(read_size(&s, "a", "c", None, Some(2000.0)).path.as_ref().unwrap().found);
            let blocked = read_size(&s, "a", "c", None, Some(2400.0));
            let reason = blocked.path.as_ref().unwrap().reason.clone().unwrap();
            assert!(reason.contains("room b") && reason.contains("2100 mm"), "{reason}");
            let tallest = blocked.clearance.unwrap().tallest_possible.unwrap();
            assert_eq!((tallest.kind, tallest.height_mm), (HeightKind::Room, 2100.0));
        }

        /// A detour with enough headroom is taken, so height shapes the route and not only
        /// whether one exists.
        #[test]
        fn test_a_height_sends_the_route_round_a_low_door() {
            let s = state_of(
                rooms_abc([None, None, None]),
                vec![
                    sized_tall("ab", "a", "b", 900.0, 1900.0),
                    sized_tall("ac", "a", "c", 900.0, 2400.0),
                    sized_tall("cb", "c", "b", 900.0, 2400.0),
                ],
            );
            let via: Vec<_> = read_size(&s, "a", "b", None, Some(2100.0))
                .path
                .unwrap()
                .rooms
                .iter()
                .map(|r| r.room_id.clone())
                .collect();
            assert_eq!(via, vec!["a", "c", "b"]);
        }

        /// A door or room with no height is passable and counted, never blocked.
        #[test]
        fn test_an_unknown_height_does_not_block_and_is_counted() {
            let s = state_of(rooms_abc([None, None, None]), vec![sized("ab", "a", "b", 900.0)]);
            let r = read_size(&s, "a", "b", None, Some(9000.0));
            assert!(r.path.as_ref().unwrap().found);
            let c = r.clearance.unwrap();
            assert!(c.lowest.is_none());
            assert_eq!(c.unchecked_height, 3, "the door and both rooms");
        }

        /// Each size fits SOME route and no single route fits both: said as that.
        #[test]
        fn test_each_size_fitting_some_route_but_not_together_is_said() {
            let s = state_of(
                rooms_abc([None, None, None]),
                vec![
                    sized_tall("ab", "a", "b", 600.0, 2400.0),
                    sized_tall("ac", "a", "c", 1200.0, 1900.0),
                    sized_tall("cb", "c", "b", 1200.0, 1900.0),
                ],
            );
            let r = read_size(&s, "a", "b", Some(1000.0), Some(2200.0));
            let path = r.path.as_ref().unwrap();
            assert!(!path.found);
            assert!(path.reason.as_deref().unwrap().contains("no single route fits both"), "{:?}", path.reason);
        }

        #[test]
        fn test_a_type_name_is_read_for_its_first_size_only() {
            assert_eq!(height_from_type_name("970 x 2040 TD01 (default single)"), Some(2040.0));
            assert_eq!(height_from_type_name("970/570 x 2040 TD56 200Wx800H VP"), Some(2040.0));
            assert_eq!(height_from_type_name("DWWH-003 820W x 2100H"), Some(2100.0));
            assert_eq!(height_from_type_name("820 X 2100"), Some(2100.0));
            assert_eq!(height_from_type_name("Mesh - Single"), None);
            assert_eq!(height_from_type_name("3500W (Placeholder)"), None);
            assert_eq!(height_from_type_name("Max 3"), None, "an x inside a word is not a size");
            assert_eq!(height_from_type_name("900 x 12"), None, "no door is 12 mm tall");
        }

        #[test]
        fn test_a_property_is_matched_ignoring_case_and_spacing_and_units_are_not_guessed() {
            let mut p = crate::contract::PropertyMap::new();
            p.insert("Ceiling  Height".into(), CustomValue { value: " 2700 ".into(), storage_type: None });
            assert_eq!(property_mm(&p, &["ceiling height"]), Some(2700.0));
            p.insert("Room Height".into(), CustomValue { value: "2.7".into(), storage_type: None });
            assert_eq!(
                property_mm(&p, &["room height"]),
                None,
                "2.7 is metres, not millimetres: unchecked, not guessed"
            );
            assert_eq!(property_mm(&p, &["no such"]), None);
        }

        /// A project whose bundle states its own sources.
        fn state_routing(rooms: Vec<Room>, doors: Vec<Opening>, routing: RoutingPolicy) -> AppState {
            let state = AppState::new(
                Box::new(MemStore::new()),
                HashMap::from([("p1".to_string(), ProjectSettings { routing, ..bundle() })]),
                None,
            );
            state.set_snapshot(rooms_payload(rooms)).unwrap();
            state
                .set_door_snapshot(DoorPayload {
                    schema_version: SUPPORTED_DOOR_SCHEMA,
                    project: Project { id: "p1".into(), name: "P".into() },
                    model: Model { id: "m1".into(), name: "M".into(), source: "revit".into() },
                    snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".into() },
                    phase: Some("New Construction".into()),
                    model_to_shared: None,
                    levels: vec![],
                    doors,
                })
                .unwrap();
            state
        }

        /// The room height comes from the property the PROJECT names, not a built-in one.
        #[test]
        fn test_the_room_height_property_is_the_projects_to_name() {
            let mut a = rect("a", 0.0, 10.0);
            a.properties
                .insert("Clear Height".into(), CustomValue { value: "1800".into(), storage_type: None });
            let mut b = rect("b", 10.0, 20.0);
            b.properties
                .insert("Clear Height".into(), CustomValue { value: "3000".into(), storage_type: None });
            let doors = vec![sized_tall("ab", "a", "b", 900.0, 2400.0)];

            // Default property: neither room has a "Ceiling Height", so nothing is checked.
            let by_default = state_routing(vec![a.clone(), b.clone()], doors.clone(), RoutingPolicy::default());
            assert!(read_size(&by_default, "a", "b", None, Some(2200.0)).path.as_ref().unwrap().found);

            // Named: room a is only 1800 mm tall, so a 2200 mm object cannot be in it.
            let named = RoutingPolicy { room_height_property: Some("clear height".into()), ..Default::default() };
            let s = state_routing(vec![a, b], doors, named);
            let r = read_size(&s, "a", "b", None, Some(2200.0));
            assert!(!r.path.as_ref().unwrap().found);
            assert!(r.path.unwrap().reason.unwrap().contains("room a"));
        }

        /// The door height properties are the project's, tried in order, before the name.
        #[test]
        fn test_door_height_properties_come_from_the_policy_and_beat_the_type_name() {
            let mut door = sized_tall("ab", "a", "b", 900.0, 2400.0);
            door.properties
                .insert("Rough Height".into(), CustomValue { value: "1950".into(), storage_type: None });
            let policy = RoutingPolicy { door_height_properties: vec!["rough height".into()], ..Default::default() };
            let s = state_routing(rooms_abc([None, None, None]), vec![door.clone()], policy);
            let r = read_size(&s, "a", "b", None, Some(2100.0));
            assert!(!r.path.as_ref().unwrap().found, "the property (1950) wins over the name (2400)");
            // Without the policy naming it, the property is not read and the name's 2400 passes.
            let plain = state_routing(rooms_abc([None, None, None]), vec![door], RoutingPolicy::default());
            assert!(read_size(&plain, "a", "b", None, Some(2100.0)).path.as_ref().unwrap().found);
        }

        #[test]
        fn test_reading_a_height_from_the_type_name_can_be_switched_off() {
            let doors = vec![sized_tall("ab", "a", "b", 900.0, 1900.0)];
            let on = state_routing(rooms_abc([None, None, None]), doors.clone(), RoutingPolicy::default());
            assert!(!read_size(&on, "a", "b", None, Some(2100.0)).path.as_ref().unwrap().found);
            let off = RoutingPolicy { door_height_from_type_name: false, ..Default::default() };
            let s = state_routing(rooms_abc([None, None, None]), doors, off);
            let r = read_size(&s, "a", "b", None, Some(2100.0));
            assert!(r.path.as_ref().unwrap().found, "no source left to say the door is low");
            assert_eq!(r.clearance.unwrap().unchecked_height, 3);
        }

        #[test]
        fn test_the_frame_allowance_comes_from_the_policy_and_a_request_overrides_it() {
            // A footprint of 1050 mm: 900 clear at the default 150 allowance, 950 at 100.
            let doors = vec![sized("ab", "a", "b", 900.0)];
            let policy = RoutingPolicy { door_frame_allowance_mm: Some(100.0), ..Default::default() };
            let s = state_routing(rooms_abc([None, None, None]), doors.clone(), policy);
            assert!(read_size(&s, "a", "b", Some(940.0), None).path.as_ref().unwrap().found);
            let default = state_routing(rooms_abc([None, None, None]), doors, RoutingPolicy::default());
            assert!(!read_size(&default, "a", "b", Some(940.0), None).path.as_ref().unwrap().found);
        }

        // ---------- heights from ceilings ----------

        /// A ceiling over `x0..x1` (the rooms are 10 ft deep) at `mm` above its level.
        fn ceiling(id: &str, x0: f64, x1: f64, mm: f64) -> crate::contract::Surface {
            crate::contract::Surface {
                id: id.to_string(),
                level_id: "lvl1".to_string(),
                height_offset: Some(mm / MM_PER_FT),
                polygons: vec![crate::contract::SurfacePolygon {
                    loops: vec![Loop {
                        points: vec![
                            Point2D { x: x0, y: 0.0 },
                            Point2D { x: x1, y: 0.0 },
                            Point2D { x: x1, y: 10.0 },
                            Point2D { x: x0, y: 10.0 },
                        ],
                    }],
                }],
                properties: Default::default(),
                type_properties: Default::default(),
                type_id: None,
                type_name: None,
            }
        }

        fn with_ceilings(state: AppState, ceilings: Vec<crate::contract::Surface>) -> AppState {
            state
                .set_element_snapshot(
                    crate::storage::SnapshotKind::Ceilings,
                    &crate::contract::CeilingPayload {
                        schema_version: crate::contract::SUPPORTED_CEILING_SCHEMA,
                        project: Project { id: "p1".into(), name: "P".into() },
                        model: Model { id: "m1".into(), name: "M".into(), source: "revit".into() },
                        snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".into() },
                        phase: Some("New Construction".into()),
                        model_to_shared: None,
                        levels: vec![Level { id: "lvl1".into(), name: "Level 1".into(), elevation: 0.0 }],
                        ceilings,
                    },
                )
                .unwrap();
            state
        }

        fn doors_abc() -> Vec<Opening> {
            vec![
                sized_tall("ab", "a", "b", 900.0, 2400.0),
                sized_tall("bc", "b", "c", 900.0, 2400.0),
            ]
        }

        /// A ceiling is where the ceiling really is: it beats the room's property, and the
        /// answer says the height came from it.
        #[test]
        fn test_a_ceiling_over_a_room_beats_the_rooms_property() {
            let s = with_ceilings(
                state_of(rooms_abc([Some("2700"), Some("2700"), Some("2700")]), doors_abc()),
                vec![ceiling("c1", 10.0, 20.0, 2100.0)],
            );
            let r = read_size(&s, "a", "c", None, Some(2400.0));
            let path = r.path.as_ref().unwrap();
            assert!(!path.found, "room b has a 2100 mm ceiling although its property says 2700");
            let reason = path.reason.as_deref().unwrap();
            assert!(reason.contains("room b (from its ceiling)") && reason.contains("2100 mm"), "{reason}");
            let tallest = r.clearance.unwrap().tallest_possible.unwrap();
            assert!(tallest.from_ceiling);
        }

        /// No ceiling over a room: the property is the fallback, and says so by not saying
        /// "from its ceiling".
        #[test]
        fn test_a_room_no_ceiling_covers_falls_back_to_its_property() {
            let s = with_ceilings(
                state_of(rooms_abc([Some("2700"), Some("2300"), Some("2000")]), doors_abc()),
                vec![ceiling("c1", 10.0, 20.0, 3000.0)],
            );
            // Room b's 3000 mm ceiling is what it reads, not its 2300 property, so 2400 fits it.
            assert!(read_size(&s, "a", "b", None, Some(2400.0)).path.as_ref().unwrap().found);
            // Room c has no ceiling over it, so its property (2000) answers, and says so by not
            // claiming a ceiling.
            let r = read_size(&s, "a", "c", None, Some(2400.0));
            let reason = r.path.as_ref().unwrap().reason.clone().unwrap();
            assert!(reason.contains("room c") && !reason.contains("from its ceiling"), "{reason}");
        }

        #[test]
        fn test_ceilings_can_be_switched_off_for_a_project() {
            let rooms = rooms_abc([Some("2700"), Some("2700"), Some("2700")]);
            let policy = RoutingPolicy { room_height_from_ceilings: false, ..Default::default() };
            let s = with_ceilings(state_routing(rooms, doors_abc(), policy), vec![ceiling("c1", 10.0, 20.0, 2100.0)]);
            assert!(
                read_size(&s, "a", "c", None, Some(2400.0)).path.as_ref().unwrap().found,
                "only the property is read"
            );
        }

        /// A strip of a neighbour's ceiling and a ceiling at or below its level are not what a
        /// room's height is.
        #[test]
        fn test_a_sliver_and_a_non_positive_offset_are_not_a_rooms_height() {
            let s = with_ceilings(
                state_of(rooms_abc([None, None, None]), doors_abc()),
                vec![
                    // 2 of room b's 10 ft: 20%, under the quarter that counts.
                    ceiling("sliver", 10.0, 12.0, 1500.0),
                    // a roof: below its level.
                    ceiling("roof", 10.0, 20.0, -800.0),
                ],
            );
            let r = read_size(&s, "a", "c", None, Some(2400.0));
            assert!(r.path.as_ref().unwrap().found);
            // The doors' heights come from their type names; the three rooms have no property and no
            // ceiling that counts.
            assert_eq!(r.clearance.unwrap().unchecked_height, 3);
        }

        #[test]
        fn test_the_lowest_ceiling_that_covers_the_room_is_its_height() {
            let s = with_ceilings(
                state_of(rooms_abc([None, None, None]), doors_abc()),
                vec![ceiling("high", 10.0, 20.0, 3000.0), ceiling("drop", 10.0, 15.0, 2200.0)],
            );
            let blocked = read_size(&s, "a", "c", None, Some(2400.0));
            assert!(!blocked.path.as_ref().unwrap().found);
            assert!((blocked.clearance.unwrap().tallest_possible.unwrap().height_mm - 2200.0).abs() < 1.0);
        }

        /// The height facts are only worked out when a height is asked, so a route without one
        /// reports none and the ceilings were not read for it.
        #[test]
        fn test_without_a_height_there_are_no_height_facts() {
            let s = with_ceilings(
                state_of(rooms_abc([Some("2700"), Some("2700"), Some("2700")]), doors_abc()),
                vec![ceiling("c1", 10.0, 20.0, 2100.0)],
            );
            let c = read_size(&s, "a", "c", None, None).clearance.unwrap();
            assert!(c.lowest.is_none() && c.tallest_possible.is_none());
            assert_eq!(c.unchecked_height, 0);
        }

        #[test]
        fn test_the_height_asked_is_validated() {
            assert_eq!(height_param(None).unwrap(), None);
            assert_eq!(height_param(Some(0.0)).unwrap(), None);
            assert_eq!(height_param(Some(2100.0)).unwrap(), Some(2100.0));
            assert!(height_param(Some(-1.0)).is_err());
            assert!(height_param(Some(f64::NAN)).is_err());
            assert!(height_param(Some(60_000.0)).is_err());
        }

        // ---------- disconnects and the hub ----------

        /// Three bays side by side along a corridor: `b1 | b2 | b3` below `cor`. Every
        /// bay touches the corridor AND its neighbours, which is the layout where an
        /// open zone of all four lets a route go through the bays.
        fn bays_state(tag: &str, zone: crate::connections::ZoneInput) -> (AppState, std::path::PathBuf) {
            fn at(id: &str, x0: f64, x1: f64, y0: f64, y1: f64) -> Room {
                Room {
                    loops: vec![Loop {
                        points: vec![
                            Point2D { x: x0, y: y0 },
                            Point2D { x: x1, y: y0 },
                            Point2D { x: x1, y: y1 },
                            Point2D { x: x0, y: y1 },
                        ],
                    }],
                    ..rect(id, 0.0, 1.0)
                }
            }
            let dir = std::env::temp_dir().join(format!("roommate-conn-bays-{tag}-{}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok(); // a stale one from a run with the same process id would read as saved data
            std::fs::create_dir_all(&dir).unwrap();
            let rooms = vec![
                at("b1", 0.0, 10.0, 0.0, 10.0),
                at("b2", 10.0, 20.0, 0.0, 10.0),
                at("b3", 20.0, 30.0, 0.0, 10.0),
                at("cor", 0.0, 30.0, 10.0, 20.0),
            ];
            let state = state_of(rooms, vec![]).with_projects_dir(dir.clone());
            crate::connections::save(
                &state,
                &dir,
                "p1",
                crate::connections::SaveRequest {
                    base: String::new(),
                    zones: vec![zone],
                    links: vec![],
                    routes: None,
                    passages: None,
                    reaches: None,
                },
            )
            .unwrap();
            (state, dir)
        }

        fn route_rooms(state: &AppState) -> Vec<String> {
            let (a, b) = (ep(None, "b1"), ep(None, "b3"));
            let path = read(state, Some((&a, &b))).path.unwrap();
            assert!(path.found, "{:?}", path.reason);
            path.rooms.iter().map(|r| r.room_id.clone()).collect()
        }

        fn cut(a: &str, b: &str) -> crate::connections::DisconnectInput {
            let room =
                |r: &str| crate::connections::RoomInput { room_id: r.to_string(), model_id: Some("m1".to_string()) };
            crate::connections::DisconnectInput { a: room(a), b: room(b) }
        }

        /// The problem as reported: with all four rooms in one open zone the shortest
        /// route from the first bay to the last goes THROUGH the middle bay.
        #[test]
        fn test_without_a_disconnect_a_route_goes_through_the_bays() {
            let (s, dir) = bays_state("none", zone_input("bays", &["b1", "b2", "b3", "cor"]));
            assert_eq!(route_rooms(&s), vec!["b1", "b2", "b3"]);
            std::fs::remove_dir_all(&dir).ok();
        }

        /// Closing the walls between neighbouring bays sends it along the corridor.
        #[test]
        fn test_disconnecting_the_bays_sends_the_route_along_the_corridor() {
            let mut zone = zone_input("bays", &["b1", "b2", "b3", "cor"]);
            zone.disconnects = vec![cut("b1", "b2"), cut("b3", "b2")];
            let (s, dir) = bays_state("cuts", zone);
            assert_eq!(route_rooms(&s), vec!["b1", "cor", "b3"]);
            let report = &read(&s, None).connections.zones[0];
            assert_eq!(report.closed, 2);
            assert!(report.unlinked.is_empty(), "every bay still opens onto the corridor");
            std::fs::remove_dir_all(&dir).ok();
        }

        /// The hub shortcut closes every wall two NON-hub members share, in one field.
        #[test]
        fn test_a_hub_connects_members_only_through_itself() {
            let mut zone = zone_input("bays", &["b1", "b2", "b3", "cor"]);
            zone.hub =
                Some(crate::connections::RoomInput { room_id: "cor".to_string(), model_id: Some("m1".to_string()) });
            let (s, dir) = bays_state("hub", zone);
            assert_eq!(route_rooms(&s), vec!["b1", "cor", "b3"]);
            let report = &read(&s, None).connections.zones[0];
            assert_eq!(report.closed, 2, "b1-b2 and b2-b3");
            assert_eq!(report.hub.as_ref().map(|h| h.room_id.as_str()), Some("cor"));
            assert!(!report.hub_stale);
            std::fs::remove_dir_all(&dir).ok();
        }

        /// A single cut still works beside a hub, to close the hub's own wall.
        #[test]
        fn test_a_member_cut_from_everything_is_reported_unlinked() {
            let mut zone = zone_input("bays", &["b1", "b2", "b3", "cor"]);
            zone.disconnects = vec![cut("b2", "b1"), cut("b2", "b3"), cut("b2", "cor")];
            let (s, dir) = bays_state("island", zone);
            let report = &read(&s, None).connections.zones[0];
            assert_eq!(report.unlinked.iter().map(|r| r.room_id.as_str()).collect::<Vec<_>>(), vec!["b2"]);
            std::fs::remove_dir_all(&dir).ok();
        }

        /// A hub that has left the model must NOT be applied: closing every wall would
        /// silently switch the zone off. It is said, and the zone works as if it had none.
        #[test]
        fn test_a_hub_that_left_the_model_is_reported_and_not_applied() {
            let mut zone = zone_input("bays", &["b1", "b2", "b3", "cor", "gone"]);
            zone.hub =
                Some(crate::connections::RoomInput { room_id: "gone".to_string(), model_id: Some("m1".to_string()) });
            let (s, dir) = bays_state("stale-hub", zone);
            let report = &read(&s, None).connections.zones[0];
            assert!(report.hub_stale && report.hub.is_none());
            assert_eq!(report.closed, 0);
            assert_eq!(route_rooms(&s), vec!["b1", "b2", "b3"]);
            std::fs::remove_dir_all(&dir).ok();
        }

        /// Without a stated cost the default applies.
        #[test]
        fn test_a_link_without_a_cost_uses_the_default() {
            let (s, dir) = storeys("default", vec![link_input("up1", "b", "e", None)]);
            let (a, f) = (ep(None, "a"), ep(None, "f"));
            let path = read(&s, Some((&a, &f))).path.unwrap();
            assert!((path.steps[1].length - crate::connections::DEFAULT_LEVEL_COST_FT).abs() < 1e-9);
            std::fs::remove_dir_all(&dir).ok();
        }

        /// Floor to floor is the person's job, one link per hop, so a route up two
        /// storeys pays for two hops and passes through the middle storey.
        #[test]
        fn test_a_route_up_two_storeys_takes_two_hops() {
            let (s, dir) = storeys(
                "two-hops",
                vec![
                    link_input("up1", "b", "e", Some(10.0)),
                    link_input("up2", "f", "g", Some(10.0)),
                ],
            );
            let (a, g) = (ep(None, "a"), ep(None, "g"));
            let path = read(&s, Some((&a, &g))).path.unwrap();
            assert!(path.found, "{:?}", path.reason);
            let hops = path.steps.iter().filter(|s| s.kind == EdgeKind::Vertical).count();
            assert_eq!(hops, 2);
            assert_eq!(
                path.segments.iter().map(|s| s.level_id.as_str()).collect::<Vec<_>>(),
                vec!["lvl1", "lvl2", "lvl3"]
            );
            std::fs::remove_dir_all(&dir).ok();
        }

        /// A link that jumps a storey is APPLIED (the person drew it) and REPORTED:
        /// `levels_between` says it skipped one.
        #[test]
        fn test_a_link_that_skips_a_storey_is_applied_and_flagged() {
            let (s, dir) = storeys("skips", vec![link_input("jump", "b", "g", None)]);
            let report = &read(&s, None).connections.links[0];
            assert!(report.applied);
            assert_eq!(report.levels_between, 1);
            std::fs::remove_dir_all(&dir).ok();
        }

        /// Two rooms on one storey, a room that is gone, and a pair already joined
        /// are each reported with a reason and never become an edge.
        #[test]
        fn test_a_link_that_cannot_apply_says_why() {
            let (s, dir) = storeys(
                "refused",
                vec![
                    link_input("same", "a", "b", None),
                    link_input("gone", "b", "ghost", None),
                    link_input("first", "b", "e", None),
                    link_input("again", "e", "b", None),
                ],
            );
            let result = read(&s, None);
            let by_id = |id: &str| result.connections.links.iter().find(|l| l.id == id).unwrap();
            assert!(!by_id("same").applied && by_id("same").reason.as_deref().unwrap().contains("same storey"));
            assert!(!by_id("gone").applied && by_id("gone").reason.as_deref().unwrap().contains("no longer"));
            assert!(by_id("first").applied);
            assert!(!by_id("again").applied && by_id("again").reason.as_deref().unwrap().contains("already"));
            assert_eq!(result.edges.as_ref().unwrap().iter().filter(|e| e.kind == EdgeKind::Vertical).count(), 1);
            std::fs::remove_dir_all(&dir).ok();
        }

        /// A linked room is no longer isolated: authored links lift the picker's
        /// refusal exactly as zones do.
        #[test]
        fn test_a_linked_room_stops_being_isolated() {
            let (s, dir) = storeys("isolated", vec![link_input("up1", "b", "g", None)]);
            let result = read(&s, None);
            let isolated: Vec<_> = result.isolated.iter().map(|r| r.room.room_id.as_str()).collect();
            assert!(!isolated.contains(&"g"), "{isolated:?}");
            std::fs::remove_dir_all(&dir).ok();
        }

        // ---------- routing methods ----------

        /// A room with its own rectangle, unlike `rect` which fixes y to 0..10.
        fn rect_xy(id: &str, x0: f64, y0: f64, x1: f64, y1: f64) -> Room {
            Room {
                loops: vec![Loop {
                    points: vec![
                        Point2D { x: x0, y: y0 },
                        Point2D { x: x1, y: y0 },
                        Point2D { x: x1, y: y1 },
                        Point2D { x: x0, y: y1 },
                    ],
                }],
                ..rect(id, 0.0, 1.0)
            }
        }

        fn door_at(id: &str, from: &str, to: &str, x: f64, y: f64) -> Opening {
            Opening {
                insertion_point: Some(Point2D { x, y }),
                ..door(id, Some(from), Some(to), "none")
            }
        }

        /// A big room `r` with two small rooms `a` and `b` below it, their doors a
        /// metre apart on `r`'s south wall. Going a to b the sensible walk is door to
        /// door; going via `r`'s centre is a detour into the middle of a room nobody
        /// needs to enter, which is the fault Liu and Zlatanova name in room-centre
        /// networks.
        fn detour() -> AppState {
            state_of(
                vec![
                    rect_xy("r", 0.0, 0.0, 10.0, 10.0),
                    rect_xy("a", 0.0, -10.0, 5.0, 0.0),
                    rect_xy("b", 5.0, -10.0, 10.0, 0.0),
                ],
                vec![door_at("ra", "r", "a", 4.5, 0.0), door_at("rb", "r", "b", 5.5, 0.0)],
            )
        }

        fn run(s: &AppState, method: Method, metric: Metric) -> ConnectivityResult {
            let (a, b) = (ep(None, "a"), ep(None, "b"));
            assemble_connectivity(s, "p1", &ConnectivityScope::default(), Some((&a, &b)), metric, method, Detail::Full)
                .unwrap()
                .unwrap()
        }

        /// The point of the method: the same route, found door to door, is much
        /// shorter than through the room's centre, passes the same doors in the same
        /// order, and is drawn through the doors and not the centre.
        #[test]
        fn test_door_to_door_is_shorter_than_through_the_room_centre() {
            let s = detour();
            let centre = run(&s, Method::Centroid, Metric::Distance);
            let direct = run(&s, Method::DoorToDoor, Metric::Distance);
            let (c, d) = (centre.path.as_ref().unwrap(), direct.path.as_ref().unwrap());
            assert!(c.found && d.found);

            assert!(c.distance_ft > 20.0, "through the centre of r: {}", c.distance_ft);
            assert!(d.distance_ft < 13.0 && d.distance_ft > 11.0, "door to door: {}", d.distance_ft);
            assert!(d.distance_ft < c.distance_ft * 0.7);

            let ids = |p: &PathResult| p.steps.iter().filter_map(|s| s.door_id.clone()).collect::<Vec<_>>();
            assert_eq!(ids(c), ids(d), "the same doors in the same order");
            assert_eq!((c.method, d.method), ("centroid", "door_to_door"));
            assert!(d.note.is_none(), "every room had an outline");

            // The polyline passes through both door points.
            let pts: Vec<(f64, f64)> = d.segments.iter().flat_map(|s| s.points.iter().map(|p| (p.x, p.y))).collect();
            assert!(pts.contains(&(4.5, 0.0)) && pts.contains(&(5.5, 0.0)), "{pts:?}");
            assert!(!pts.contains(&(5.0, 5.0)), "it does not visit the middle of r");
        }

        /// Step lengths plus the last walk are the total, so a reader can attribute
        /// every foot.
        #[test]
        fn test_door_to_door_step_lengths_account_for_the_whole_route() {
            let d = run(&detour(), Method::DoorToDoor, Metric::Distance).path.unwrap();
            let sum: f64 = d.steps.iter().map(|s| s.length).sum::<f64>() + d.arrival_ft;
            assert!((sum - d.distance_ft).abs() < 1e-9, "{sum} vs {}", d.distance_ft);
            assert_eq!(d.rooms.iter().map(|r| r.room_id.as_str()).collect::<Vec<_>>(), vec!["a", "r", "b"]);
        }

        /// The answer says which methods exist and which ran, so a picker needs no
        /// second request; the default is the recommended one.
        #[test]
        fn test_the_answer_lists_the_methods_and_the_one_used() {
            let s = detour();
            let result = run(&s, Method::Centroid, Metric::Distance);
            assert_eq!(result.method, "centroid");
            assert_eq!(result.methods.iter().map(|m| m.id).collect::<Vec<_>>(), vec!["door_to_door", "centroid"]);
            assert_eq!(Method::default(), Method::DoorToDoor);
        }

        /// The hops metric counts doors and has no geometry for a method to improve,
        /// so it takes the graph's own search and says why.
        #[test]
        fn test_the_hops_metric_ignores_the_method_and_says_so() {
            let d = run(&detour(), Method::DoorToDoor, Metric::Hops).path.unwrap();
            assert!(d.found);
            assert_eq!(d.method, "centroid");
            assert!(d.note.as_deref().unwrap().contains("hops"));
        }

        /// No route is the same finding under either method.
        #[test]
        fn test_door_to_door_reports_no_route_like_the_other_method() {
            let s = state_of(vec![rect_xy("a", 0.0, 0.0, 5.0, 5.0), rect_xy("b", 20.0, 0.0, 25.0, 5.0)], vec![]);
            let d = run(&s, Method::DoorToDoor, Metric::Distance).path.unwrap();
            assert!(!d.found);
            assert!(d.reason.as_deref().unwrap().contains("component"));
        }

        /// A route that uses an open zone and a vertical link works door to door
        /// too: the zone hop crosses the shared wall and the link is a break.
        #[test]
        fn test_door_to_door_crosses_zones_and_levels() {
            let (s, dir) = storeys("d2d", vec![link_input("up1", "b", "e", Some(25.0))]);
            let (a, f) = (ep(None, "a"), ep(None, "f"));
            let result = assemble_connectivity(
                &s,
                "p1",
                &ConnectivityScope::default(),
                Some((&a, &f)),
                Metric::Distance,
                Method::DoorToDoor,
                Detail::Full,
            )
            .unwrap()
            .unwrap();
            let path = result.path.unwrap();
            assert!(path.found, "{:?}", path.reason);
            let kinds: Vec<_> = path.steps.iter().map(|s| s.kind).collect();
            assert_eq!(kinds, vec![EdgeKind::Door, EdgeKind::Vertical, EdgeKind::Door]);
            assert!((path.steps[1].length - 25.0).abs() < 1e-6 || path.steps[1].length >= 25.0);
            assert_eq!(
                path.segments.iter().map(|s| s.level_id.as_str()).collect::<Vec<_>>(),
                vec!["lvl1", "lvl2"]
            );
            std::fs::remove_dir_all(&dir).ok();
        }

        // ---------- start and end points ----------

        fn run_at(s: &AppState, method: Method, from_at: Option<Point2D>, to_at: Option<Point2D>) -> PathResult {
            let a = Endpoint { at: from_at, ..ep(None, "a") };
            let b = Endpoint { at: to_at, ..ep(None, "b") };
            assemble_connectivity(
                s,
                "p1",
                &ConnectivityScope::default(),
                Some((&a, &b)),
                Metric::Distance,
                method,
                Detail::Full,
            )
            .unwrap()
            .unwrap()
            .path
            .unwrap()
        }

        fn pt(x: f64, y: f64) -> Point2D {
            Point2D { x, y }
        }

        /// A route that starts and ends beside its doors is much shorter than one
        /// from the room centres, under both methods, and the answer says where it
        /// started and ended. In `detour()` the doors are at (4.5, 0) and (5.5, 0).
        #[test]
        fn test_a_start_and_end_point_replace_the_room_centres() {
            let s = detour();
            for method in [Method::DoorToDoor, Method::Centroid] {
                let from_centres = run_at(&s, method, None, None);
                let near_doors = run_at(&s, method, Some(pt(4.0, -1.0)), Some(pt(6.0, -1.0)));
                assert!(near_doors.found && from_centres.found);
                // Door to door is only the walk from beside one door to beside the other;
                // the centre method still goes via the middle of `r`, so it gains less.
                let share = if method == Method::DoorToDoor { 0.5 } else { 0.7 };
                assert!(
                    near_doors.distance_ft < from_centres.distance_ft * share,
                    "{method:?}: {} vs {}",
                    near_doors.distance_ft,
                    from_centres.distance_ft
                );
                assert_eq!((near_doors.start.x, near_doors.start.y), (4.0, -1.0), "{method:?}");
                assert_eq!((near_doors.end.x, near_doors.end.y), (6.0, -1.0), "{method:?}");
            }
        }

        /// Door to door from beside the doors is the plain geometry: a metre-ish to
        /// the first door, a metre between doors, a metre-ish from the last.
        #[test]
        fn test_door_to_door_from_a_point_is_the_exact_walk() {
            let d = run_at(&detour(), Method::DoorToDoor, Some(pt(4.5, -1.0)), Some(pt(5.5, -1.0)));
            // (4.5,-1) -> door (4.5,0) -> door (5.5,0) -> (5.5,-1)
            assert!((d.distance_ft - 3.0).abs() < 1e-6, "{}", d.distance_ft);
            assert!(d.note.is_none());
        }

        /// A point outside its room is moved onto the room's outline, and the
        /// answer says so; it is not refused.
        #[test]
        fn test_a_point_outside_its_room_is_moved_onto_it_and_said() {
            // `a` is x 0..5, y -10..0; this point is far to its left.
            let d = run_at(&detour(), Method::DoorToDoor, Some(pt(-3.0, -5.0)), None);
            assert!(d.found);
            assert!((d.start.x - 0.0).abs() < 1e-6 && (d.start.y + 5.0).abs() < 1e-6, "{:?}", d.start);
            assert!(d.note.as_deref().unwrap().contains("start point was outside"), "{:?}", d.note);
        }

        /// Both ends in the same room: a straight line under the centre method, the
        /// exact walk under door to door, and no door crossed either way.
        #[test]
        fn test_both_points_in_one_room_need_no_door() {
            let s = detour();
            let (r1, r2) = (ep(None, "r"), ep(None, "r"));
            for method in [Method::DoorToDoor, Method::Centroid] {
                let r1 = Endpoint { at: Some(pt(1.0, 1.0)), ..r1.clone() };
                let r2 = Endpoint { at: Some(pt(4.0, 5.0)), ..r2.clone() };
                let path = assemble_connectivity(
                    &s,
                    "p1",
                    &ConnectivityScope::default(),
                    Some((&r1, &r2)),
                    Metric::Distance,
                    method,
                    Detail::Full,
                )
                .unwrap()
                .unwrap()
                .path
                .unwrap();
                assert!(path.found && path.steps.is_empty(), "{method:?}");
                assert!((path.distance_ft - 5.0).abs() < 1e-6, "{method:?}: {}", path.distance_ft);
            }
        }

        /// A point is two coordinates or none, finite, and needs a route to belong to.
        #[test]
        fn test_a_point_is_both_coordinates_or_neither() {
            assert!(point(None, None, "start").unwrap().is_none());
            assert_eq!(point(Some(1.0), Some(2.0), "start").unwrap().map(|p| (p.x, p.y)), Some((1.0, 2.0)));
            assert!(point(Some(1.0), None, "start").is_err());
            assert!(point(Some(f64::NAN), Some(2.0), "end").is_err());
            assert!(
                endpoints_at(None, None, Some(pt(1.0, 1.0)), None, None, None).is_err(),
                "a point without a route"
            );
            let (a, b) = endpoints_at(Some("a"), None, Some(pt(1.0, 1.0)), Some("b"), None, None).unwrap().unwrap();
            assert!(a.at.is_some() && b.at.is_none());
        }

        /// The summary leaves the graph out of the body; `full` puts it in.
        #[test]
        fn test_summary_omits_the_graph_and_full_includes_it() {
            let s = state(standard());
            let summary = assemble_connectivity(
                &s,
                "p1",
                &ConnectivityScope::default(),
                None,
                Metric::Distance,
                Method::Centroid,
                Detail::Summary,
            )
            .unwrap()
            .unwrap();
            let json = serde_json::to_value(&summary).unwrap();
            assert!(json.get("nodes").is_none() && json.get("edges").is_none());
            assert_eq!(json["isolated"].as_array().unwrap().len(), 1, "what a picker needs is still there");
            assert_eq!(json["counts"]["edges"], 2);
            assert!(Detail::parse(Some("everything")).is_err());
        }
        #[test]
        fn test_nothing_pushed_is_none() {
            let state = AppState::new(Box::new(MemStore::new()), HashMap::new(), None);
            assert!(assemble_connectivity(
                &state,
                "p1",
                &ConnectivityScope::default(),
                None,
                Metric::Distance,
                Method::Centroid,
                Detail::Full
            )
            .unwrap()
            .is_none());
        }

        // ---------- passages ----------

        fn passage_input(
            id: &str,
            a: &str,
            b: &str,
            from: (f64, f64),
            to: (f64, f64),
        ) -> crate::connections::PassageInput {
            let room =
                |r: &str| crate::connections::RoomInput { room_id: r.to_string(), model_id: Some("m1".to_string()) };
            crate::connections::PassageInput {
                id: id.to_string(),
                a: room(a),
                b: room(b),
                from: crate::connections::RoutePoint { x: from.0, y: from.1 },
                to: crate::connections::RoutePoint { x: to.0, y: to.1 },
                width_mm: None,
                height_mm: None,
                note: None,
            }
        }

        /// The standard rooms (`a | b | c`, a gap, `d`) and doors, with passages authored.
        fn passaged(tag: &str, rooms: Option<Vec<Room>>, passages: Vec<crate::connections::PassageInput>) -> AppState {
            let dir = std::env::temp_dir().join(format!("roommate-conn-passages-{tag}-{}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok(); // a stale one from a run with the same process id would read as saved data
            std::fs::create_dir_all(&dir).unwrap();
            let doors = standard();
            let state = match rooms {
                Some(rooms) => state_of(rooms, doors),
                None => state(doors),
            }
            .with_projects_dir(dir.clone());
            crate::connections::save(
                &state,
                &dir,
                "p1",
                crate::connections::SaveRequest {
                    base: String::new(),
                    zones: vec![],
                    links: vec![],
                    routes: None,
                    passages: Some(passages),
                    reaches: None,
                },
            )
            .unwrap();
            state
        }

        fn read_sized(
            state: &AppState,
            from: &str,
            to: &str,
            width: Option<f64>,
            height: Option<f64>,
            method: Method,
        ) -> ConnectivityResult {
            let (f, t) = (ep(None, from), ep(None, to));
            let scope = ConnectivityScope { clear_width_mm: width, clear_height_mm: height, ..Default::default() };
            assemble_connectivity(state, "p1", &scope, Some((&f, &t)), Metric::Distance, method, Detail::Full)
                .unwrap()
                .unwrap()
        }

        /// The point of the feature: `d` has no door and shares no wall with anything, and a
        /// line drawn across the gap makes it routable. The edge runs from where the line
        /// leaves `c` (x = 30) to where it enters `d` (x = 40), and its length is the walk
        /// from centre to crossing to crossing to centre: 5 + 10 + 5.
        #[test]
        fn test_a_passage_joins_rooms_that_share_no_wall() {
            let s = passaged("joins", None, vec![passage_input("gap", "c", "d", (25.0, 5.0), (45.0, 5.0))]);
            let result = read_sized(&s, "a", "d", None, None, Method::Centroid);
            let path = result.path.as_ref().unwrap();
            assert!(path.found, "{:?}", path.reason);
            let last = path.steps.last().unwrap();
            assert_eq!(last.kind, EdgeKind::Passage);
            assert_eq!(last.passage_id.as_deref(), Some("gap"));
            assert!((last.length - 20.0).abs() < 1e-6, "{}", last.length);
            // a -> b -> c is 20 ft of doors, then the passage.
            assert!((path.distance_ft - 40.0).abs() < 1e-6, "{}", path.distance_ft);
            let report = &result.connections.passages[0];
            assert!(report.applied, "{:?}", report.reason);
            assert!((report.from.unwrap().x - 30.0).abs() < 1e-6 && (report.to.unwrap().x - 40.0).abs() < 1e-6);
            assert!((report.span_mm.unwrap() - 10.0 * MM_PER_FT).abs() < 1e-6);
            assert!(result.isolated.is_empty(), "d is reachable now");
            // Drawn as one polyline through both crossings.
            let xs: Vec<f64> = path.segments.iter().flat_map(|s| s.points.iter().map(|p| p.x)).collect();
            assert!(xs.windows(2).any(|w| (w[0] - 30.0).abs() < 1e-6 && (w[1] - 40.0).abs() < 1e-6), "{xs:?}");
        }

        /// Walking the passage the other way draws the crossings in the other order.
        #[test]
        fn test_a_passage_is_walked_both_ways() {
            let s = passaged("both", None, vec![passage_input("gap", "c", "d", (25.0, 5.0), (45.0, 5.0))]);
            for method in [Method::Centroid, Method::DoorToDoor] {
                let path = read_sized(&s, "d", "c", None, None, method).path.unwrap();
                assert!(path.found, "{:?}", path.reason);
                let xs: Vec<f64> = path.segments.iter().flat_map(|s| s.points.iter().map(|p| p.x)).collect();
                let at = |x: f64| xs.iter().position(|v| (v - x).abs() < 1e-6);
                assert!(at(40.0).unwrap() < at(30.0).unwrap(), "{method:?} {xs:?}");
            }
        }

        /// The stated width and height are what the size checks read, and the blocked
        /// route's sentence names the passage.
        #[test]
        fn test_a_passages_width_and_height_are_checked() {
            let mut gap = passage_input("gap", "c", "d", (25.0, 5.0), (45.0, 5.0));
            gap.width_mm = Some(700.0);
            gap.height_mm = Some(2000.0);
            let s = passaged("sized", None, vec![gap]);
            assert!(read_sized(&s, "a", "d", Some(700.0), Some(2000.0), Method::Centroid).path.unwrap().found);
            let wide = read_sized(&s, "a", "d", Some(800.0), None, Method::Centroid);
            assert!(!wide.path.as_ref().unwrap().found);
            let limit = wide.clearance.as_ref().unwrap().widest_possible.as_ref().unwrap();
            assert_eq!((limit.kind, limit.passage_id.as_deref()), (EdgeKind::Passage, Some("gap")));
            assert!(wide.path.unwrap().reason.unwrap().contains("a passage gap"));
            let tall = read_sized(&s, "a", "d", None, Some(2100.0), Method::Centroid);
            assert!(!tall.path.as_ref().unwrap().found);
            assert_eq!(tall.clearance.unwrap().tallest_possible.unwrap().kind, HeightKind::Passage);
        }

        /// A passage with no width or height is not a barrier: it passes and is counted.
        #[test]
        fn test_a_passage_with_no_size_is_passable_and_counted_unchecked() {
            let s = passaged("unsized", None, vec![passage_input("gap", "c", "d", (25.0, 5.0), (45.0, 5.0))]);
            let r = read_sized(&s, "c", "d", Some(5000.0), Some(5000.0), Method::Centroid);
            assert!(r.path.as_ref().unwrap().found);
            let c = r.clearance.unwrap();
            assert_eq!(c.unchecked_hops, 1);
            assert!(c.narrowest.is_none());
        }

        /// A passage that cannot apply is reported with its reason, never dropped silently.
        #[test]
        fn test_a_passage_that_cannot_apply_says_why() {
            let rooms = vec![
                rect("a", 0.0, 10.0),
                rect_on("up", 0.0, 10.0, "lvl2"),
                rect("c", 20.0, 30.0),
                rect("d", 40.0, 50.0),
            ];
            let s = passaged(
                "reasons",
                Some(rooms),
                vec![
                    passage_input("storeys", "a", "up", (5.0, 5.0), (5.0, 6.0)),
                    passage_input("outside", "c", "d", (15.0, 5.0), (45.0, 5.0)),
                    passage_input("gone", "c", "missing", (25.0, 5.0), (45.0, 5.0)),
                ],
            );
            let result = read(&s, None);
            let by = |id: &str| result.connections.passages.iter().find(|p| p.id == id).unwrap();
            assert!(by("storeys").reason.as_deref().unwrap().contains("vertical link"));
            assert!(by("outside").reason.as_deref().unwrap().contains("starts outside"));
            assert!(by("gone").reason.as_deref().unwrap().contains("no longer among"));
            assert!(result.connections.passages.iter().all(|p| !p.applied));
            assert_eq!(result.edges.unwrap().iter().filter(|e| e.kind == EdgeKind::Passage).count(), 0);
        }

        /// A door and a passage between the same rooms both stay: the size check chooses.
        #[test]
        fn test_a_passage_beside_a_door_is_not_dropped_as_redundant() {
            let mut wide = passage_input("wide", "a", "b", (5.0, 5.0), (15.0, 5.0));
            wide.width_mm = Some(3000.0);
            let s = passaged("beside", None, vec![wide]);
            let result = read(&s, None);
            assert!(result.connections.passages[0].applied);
            assert_eq!(
                result.edges.unwrap().iter().filter(|e| e.a.room_id == "a" && e.b.room_id == "b").count(),
                2
            );
        }
    }
}
