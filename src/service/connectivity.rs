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
//! ~500-line trigger, but about 250 of the lines are tests over one algorithm
//! (join, components, Dijkstra, resolve) that share a fixture. The seam if it
//! grows is `link`/`components`/`shortest`, which are already pure functions.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap};

use serde::Serialize;

use crate::contract::{DoorPayload, Level, Point2D};
use crate::state::AppState;

use super::adjacency::centroid_of;
use super::openings::{assemble_openings, OpeningKind, OpeningScope};
use super::room_locator::RoomRef;
use super::rooms::{assemble_rooms, RoomFilter, RoomScope};
use super::ServiceError;

pub const SCHEMA_VERSION: u32 = 1;

/// What "shortest" means.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    /// Approximate walking distance in feet. The default.
    #[default]
    Distance,
    /// Fewest doors.
    Hops,
}

impl Metric {
    pub fn parse(raw: Option<&str>) -> Result<Self, ServiceError> {
        match raw.map(str::trim) {
            None | Some("") | Some("distance") => Ok(Metric::Distance),
            Some("hops") => Ok(Metric::Hops),
            Some(other) => Err(ServiceError::Invalid(format!("metric {other:?} is not one of: distance, hops"))),
        }
    }
}

/// A room named by a caller. The model is optional because a room id is unique
/// only within one: a bare id is accepted when exactly one model has it, and
/// refused with the candidates listed when several do.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub model_id: Option<String>,
    pub room_id: String,
}

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
}

// ================================ wire shape ================================

#[derive(Serialize)]
pub struct ConnectivityResult {
    pub schema_version: u32,
    pub revision: String,
    pub metric: Metric,
    /// False when no doors snapshot exists for the project at all. Every room is
    /// then isolated, which says "no doors were pushed", not "no doors exist".
    pub doors_pushed: bool,
    pub levels: Vec<Level>,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Connected sets of rooms, largest first. `Node::component` indexes this.
    pub components: Vec<Component>,
    /// Rooms no door reaches: the worklist for authored connections. Not all of
    /// them are bays.
    pub isolated: Vec<IsolatedRoom>,
    pub counts: Counts,
    /// Present only when both `from` and `to` were asked for.
    pub path: Option<PathResult>,
}

#[derive(Serialize, Clone)]
pub struct Node {
    #[serde(flatten)]
    pub room: RoomRef,
    pub name: String,
    pub level_id: String,
    pub centroid: Point2D,
    /// Doors joining this room to another room in scope.
    pub degree: usize,
    /// Doors with this room on exactly one side and nothing on the other: the
    /// way out of the building, or into a model that holds no rooms.
    pub exits: usize,
    pub component: usize,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum PointSource {
    /// The door's own insertion point.
    Insertion,
    /// The middle of the door's footprint, for a door with no insertion point.
    Footprint,
    /// Neither existed, so the point is halfway between the two room centroids.
    /// The route through it is straight; say so rather than pretend otherwise.
    Midpoint,
}

#[derive(Serialize, Clone)]
pub struct Edge {
    pub a: RoomRef,
    pub b: RoomRef,
    pub door_id: String,
    pub door_model_id: String,
    pub point: Point2D,
    pub point_source: PointSource,
    /// Approximate walking distance through the door, in feet.
    pub length: f64,
    #[serde(skip)]
    ia: usize,
    #[serde(skip)]
    ib: usize,
}

#[derive(Serialize)]
pub struct Component {
    pub size: usize,
}

#[derive(Serialize)]
pub struct IsolatedRoom {
    #[serde(flatten)]
    pub room: RoomRef,
    pub name: String,
    pub level_id: String,
    /// Doors that leave this room for outside. An isolated room with an exit is
    /// a room whose only door goes outdoors, which is not a bay.
    pub exits: usize,
}

#[derive(Serialize, Default, Debug, PartialEq, Eq)]
pub struct Counts {
    pub doors: usize,
    pub edges: usize,
    /// Doors naming exactly one room.
    pub exits: usize,
    /// Doors naming no room.
    pub unattached: usize,
    /// Doors naming the same room on both sides.
    pub same_room: usize,
    /// Doors naming a room that is not in the requested scope.
    pub out_of_scope: usize,
}

#[derive(Serialize)]
pub struct PathResult {
    pub found: bool,
    /// Why not, when `found` is false.
    pub reason: Option<String>,
    pub from: RoomRef,
    pub to: RoomRef,
    /// Total weight in the request's metric.
    pub cost: f64,
    /// Approximate walking distance in feet, whatever the metric.
    pub distance_ft: f64,
    pub rooms: Vec<RoomRef>,
    pub steps: Vec<Step>,
    /// The route as polylines, one per run of points on one level, so the
    /// viewer draws each zone's slice without asking again.
    pub segments: Vec<Segment>,
}

#[derive(Serialize)]
pub struct Step {
    pub from: RoomRef,
    pub to: RoomRef,
    pub door_id: String,
    pub door_model_id: String,
    pub point: Point2D,
    pub length: f64,
}

#[derive(Serialize, Debug)]
pub struct Segment {
    pub level_id: String,
    pub points: Vec<Point2D>,
}

// ============================== the graph ==============================

/// One door, reduced to what the graph needs.
struct DoorFact {
    door_id: String,
    door_model_id: String,
    from: Option<RoomRef>,
    to: Option<RoomRef>,
    /// The door's own point, when it has one.
    point: Option<(Point2D, PointSource)>,
}

struct Graph {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    counts: Counts,
}

fn dist(a: Point2D, b: Point2D) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// Join the doors onto the rooms. Pure, so the whole decision table is testable
/// without a store.
fn link(mut nodes: Vec<Node>, doors: Vec<DoorFact>) -> Graph {
    nodes.sort_by(|x, y| x.room.cmp(&y.room));
    let index: BTreeMap<RoomRef, usize> = nodes.iter().enumerate().map(|(i, n)| (n.room.clone(), i)).collect();
    let mut counts = Counts { doors: doors.len(), ..Default::default() };
    let mut edges = Vec::new();

    for door in doors {
        match (door.from, door.to) {
            (None, None) => counts.unattached += 1,
            (Some(only), None) | (None, Some(only)) => match index.get(&only) {
                Some(&i) => {
                    nodes[i].exits += 1;
                    counts.exits += 1;
                }
                None => counts.out_of_scope += 1,
            },
            (Some(a), Some(b)) if a == b => counts.same_room += 1,
            (Some(a), Some(b)) => {
                let (Some(&ia), Some(&ib)) = (index.get(&a), index.get(&b)) else {
                    counts.out_of_scope += 1;
                    continue;
                };
                let (ca, cb) = (nodes[ia].centroid, nodes[ib].centroid);
                let (point, point_source) = door
                    .point
                    .unwrap_or((Point2D { x: (ca.x + cb.x) / 2.0, y: (ca.y + cb.y) / 2.0 }, PointSource::Midpoint));
                nodes[ia].degree += 1;
                nodes[ib].degree += 1;
                edges.push(Edge {
                    a,
                    b,
                    door_id: door.door_id,
                    door_model_id: door.door_model_id,
                    point,
                    point_source,
                    length: dist(ca, point) + dist(point, cb),
                    ia,
                    ib,
                });
            }
        }
    }
    counts.edges = edges.len();
    Graph { nodes, edges, counts }
}

/// Label each node with its connected component, largest component first.
/// Returns the component sizes in that order.
fn components(nodes: &mut [Node], edges: &[Edge]) -> Vec<Component> {
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut parent: Vec<usize> = (0..nodes.len()).collect();
    for e in edges {
        let (ra, rb) = (find(&mut parent, e.ia), find(&mut parent, e.ib));
        if ra != rb {
            parent[ra] = rb;
        }
    }
    let mut size: BTreeMap<usize, usize> = BTreeMap::new();
    let roots: Vec<usize> = (0..nodes.len()).map(|i| find(&mut parent, i)).collect();
    for r in &roots {
        *size.entry(*r).or_default() += 1;
    }
    // Largest first; the lowest-sorting room breaks a tie, so the numbering is a
    // function of the data and not of hash order.
    let mut first_room: BTreeMap<usize, usize> = BTreeMap::new();
    for (i, r) in roots.iter().enumerate() {
        first_room.entry(*r).or_insert(i);
    }
    let mut order: Vec<usize> = size.keys().copied().collect();
    order.sort_by(|a, b| size[b].cmp(&size[a]).then(first_room[a].cmp(&first_room[b])));
    let rank: BTreeMap<usize, usize> = order.iter().enumerate().map(|(i, r)| (*r, i)).collect();
    for (i, node) in nodes.iter_mut().enumerate() {
        node.component = rank[&roots[i]];
    }
    order.iter().map(|r| Component { size: size[r] }).collect()
}

#[derive(PartialEq)]
struct Frontier {
    cost: f64,
    node: usize,
}
impl Eq for Frontier {}
impl Ord for Frontier {
    // Reversed: `BinaryHeap` is a max-heap and Dijkstra wants the cheapest.
    fn cmp(&self, other: &Self) -> Ordering {
        other.cost.total_cmp(&self.cost).then(other.node.cmp(&self.node))
    }
}
impl PartialOrd for Frontier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn weight(edge: &Edge, metric: Metric) -> f64 {
    match metric {
        Metric::Distance => edge.length,
        Metric::Hops => 1.0,
    }
}

/// Cheapest route between two node indices, as the edge indices walked in order.
/// `None` when they are in different components.
fn shortest(nodes: &[Node], edges: &[Edge], from: usize, to: usize, metric: Metric) -> Option<Vec<usize>> {
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (k, e) in edges.iter().enumerate() {
        adj[e.ia].push(k);
        adj[e.ib].push(k);
    }
    let mut best = vec![f64::INFINITY; nodes.len()];
    let mut via: Vec<Option<usize>> = vec![None; nodes.len()];
    let mut heap = BinaryHeap::new();
    best[from] = 0.0;
    heap.push(Frontier { cost: 0.0, node: from });
    while let Some(Frontier { cost, node }) = heap.pop() {
        if node == to {
            break;
        }
        if cost > best[node] {
            continue;
        }
        for &k in &adj[node] {
            let e = &edges[k];
            let next = if e.ia == node { e.ib } else { e.ia };
            let c = cost + weight(e, metric);
            if c < best[next] {
                best[next] = c;
                via[next] = Some(k);
                heap.push(Frontier { cost: c, node: next });
            }
        }
    }
    if from != to && via[to].is_none() {
        return None;
    }
    let mut walked = Vec::new();
    let mut at = to;
    while at != from {
        let k = via[at].expect("a reached node has a predecessor");
        walked.push(k);
        at = if edges[k].ia == at { edges[k].ib } else { edges[k].ia };
    }
    walked.reverse();
    Some(walked)
}

/// Resolve a caller's room to a node, refusing to guess between models.
fn resolve(nodes: &[Node], endpoint: &Endpoint, which: &str) -> Result<usize, ServiceError> {
    let hits: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            n.room.room_id == endpoint.room_id && endpoint.model_id.as_ref().is_none_or(|m| *m == n.room.model_id)
        })
        .map(|(i, _)| i)
        .collect();
    match hits.as_slice() {
        [one] => Ok(*one),
        [] => Err(ServiceError::Invalid(format!(
            "{which} room {:?}{} is not among the rooms in scope",
            endpoint.room_id,
            endpoint.model_id.as_ref().map(|m| format!(" in model {m:?}")).unwrap_or_default()
        ))),
        many => Err(ServiceError::Invalid(format!(
            "{which} room id {:?} exists in {} models ({}); name the model",
            endpoint.room_id,
            many.len(),
            many.iter().map(|&i| nodes[i].room.model_id.as_str()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

/// The route as one polyline per run of points on one level. A door joining two
/// levels splits the route there, which is also how a vertical hop will be
/// drawn: as a break, not a line.
fn segments_of(nodes: &[Node], edges: &[Edge], start: usize, walked: &[usize]) -> Vec<Segment> {
    let mut points: Vec<(String, Point2D)> = vec![(nodes[start].level_id.clone(), nodes[start].centroid)];
    let mut at = start;
    for &k in walked {
        let e = &edges[k];
        let next = if e.ia == at { e.ib } else { e.ia };
        // The door sits with the room we are leaving; the next room's own
        // centroid starts the next run if it is on another level.
        points.push((nodes[at].level_id.clone(), e.point));
        points.push((nodes[next].level_id.clone(), nodes[next].centroid));
        at = next;
    }
    let mut out: Vec<Segment> = Vec::new();
    for (level, p) in points {
        match out.last_mut() {
            Some(seg) if seg.level_id == level => seg.points.push(p),
            _ => out.push(Segment { level_id: level, points: vec![p] }),
        }
    }
    out
}

fn route(nodes: &[Node], edges: &[Edge], from: usize, to: usize, metric: Metric) -> PathResult {
    let (from_ref, to_ref) = (nodes[from].room.clone(), nodes[to].room.clone());
    let Some(walked) = shortest(nodes, edges, from, to, metric) else {
        return PathResult {
            found: false,
            reason: Some(format!(
                "no route through doors: the first room is in component {} and the second in component {}. \
                 This is a limit of a doors-only graph, not necessarily of the building",
                nodes[from].component, nodes[to].component
            )),
            from: from_ref,
            to: to_ref,
            cost: 0.0,
            distance_ft: 0.0,
            rooms: vec![],
            steps: vec![],
            segments: vec![],
        };
    };
    let mut rooms = vec![from_ref.clone()];
    let mut steps = Vec::new();
    let mut at = from;
    for &k in &walked {
        let e = &edges[k];
        let next = if e.ia == at { e.ib } else { e.ia };
        steps.push(Step {
            from: nodes[at].room.clone(),
            to: nodes[next].room.clone(),
            door_id: e.door_id.clone(),
            door_model_id: e.door_model_id.clone(),
            point: e.point,
            length: e.length,
        });
        rooms.push(nodes[next].room.clone());
        at = next;
    }
    PathResult {
        found: true,
        reason: None,
        from: from_ref,
        to: to_ref,
        cost: walked.iter().map(|&k| weight(&edges[k], metric)).sum(),
        distance_ft: walked.iter().map(|&k| edges[k].length).sum(),
        rooms,
        steps,
        segments: segments_of(nodes, edges, from, &walked),
    }
}

// ================================ the read ================================

/// Both ends of a route or neither: a route needs two rooms, and one alone is a
/// request that would silently answer a different question than the caller
/// asked. Blank strings count as absent, so an empty form field is "not given".
/// Shared by the HTTP handler and the MCP tool so the two cannot disagree.
pub fn endpoints(
    from: Option<&str>,
    from_model: Option<&str>,
    to: Option<&str>,
    to_model: Option<&str>,
) -> Result<Option<(Endpoint, Endpoint)>, ServiceError> {
    fn blank(s: Option<&str>) -> Option<&str> {
        s.map(str::trim).filter(|s| !s.is_empty())
    }
    let endpoint = |room: &str, model: Option<&str>| Endpoint {
        model_id: blank(model).map(str::to_string),
        room_id: room.to_string(),
    };
    match (blank(from), blank(to)) {
        (None, None) => Ok(None),
        (Some(a), Some(b)) => Ok(Some((endpoint(a, from_model), endpoint(b, to_model)))),
        _ => Err(ServiceError::Invalid("a route needs both `from` and `to`".to_string())),
    }
}

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
) -> Result<Option<ConnectivityResult>, ServiceError> {
    let room_scope = RoomScope {
        project: Some(project),
        building: scope.building,
        milestone: scope.milestone,
        filter: None,
    };
    let Some(rooms) = assemble_rooms(state, &room_scope)? else {
        return Ok(None);
    };

    let door_scope = OpeningScope {
        project: Some(project),
        milestone: scope.milestone,
        filter: scope.door_filter,
        ..Default::default()
    };
    let assembled = assemble_openings::<DoorPayload>(state, OpeningKind::Doors, &door_scope)?;
    let doors_pushed = assembled.is_some();

    let nodes: Vec<Node> = rooms
        .rooms
        .iter()
        .map(|r| Node {
            room: RoomRef { model_id: r.model_id.clone(), room_id: r.room.id.clone() },
            name: r.room.name.clone(),
            level_id: r.room.level_id.clone(),
            centroid: centroid_of(&r.room),
            degree: 0,
            exits: 0,
            component: 0,
        })
        .collect();

    let mut revision = rooms.revision.clone();
    let facts: Vec<DoorFact> = match &assembled {
        Some(a) => {
            revision = format!("{revision}.{}", a.revision);
            a.openings
                .iter()
                .map(|d| DoorFact {
                    door_id: d.door.id.clone(),
                    door_model_id: d.model_id.clone(),
                    from: d.room_origin.from_room.room().cloned(),
                    to: d.room_origin.to_room.room().cloned(),
                    point: door_point(&d.door),
                })
                .collect()
        }
        None => Vec::new(),
    };

    let Graph { mut nodes, edges, counts } = link(nodes, facts);
    let components = components(&mut nodes, &edges);

    let isolated: Vec<IsolatedRoom> = nodes
        .iter()
        .filter(|n| components[n.component].size == 1)
        .map(|n| IsolatedRoom {
            room: n.room.clone(),
            name: n.name.clone(),
            level_id: n.level_id.clone(),
            exits: n.exits,
        })
        .collect();

    let path = match route_between {
        None => None,
        Some((from, to)) => {
            let (i, j) = (resolve(&nodes, from, "start")?, resolve(&nodes, to, "end")?);
            Some(route(&nodes, &edges, i, j, metric))
        }
    };

    Ok(Some(ConnectivityResult {
        schema_version: SCHEMA_VERSION,
        revision,
        metric,
        doors_pushed,
        levels: rooms.levels,
        nodes,
        edges,
        components,
        isolated,
        counts,
        path,
    }))
}

/// A door's own position: its insertion point, else the middle of its
/// footprint. `None` leaves the graph to place it between the two rooms.
fn door_point(door: &crate::contract::Opening) -> Option<(Point2D, PointSource)> {
    if let Some(p) = door.insertion_point {
        return Some((p, PointSource::Insertion));
    }
    let pts = door.loops.first().map(|l| l.points.as_slice()).unwrap_or(&[]);
    if pts.is_empty() {
        return None;
    }
    let n = pts.len() as f64;
    Some((
        Point2D {
            x: pts.iter().map(|p| p.x).sum::<f64>() / n,
            y: pts.iter().map(|p| p.y).sum::<f64>() / n,
        },
        PointSource::Footprint,
    ))
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
        }
    }

    fn graph(nodes: Vec<Node>, doors: Vec<DoorFact>) -> (Vec<Node>, Vec<Edge>, Vec<Component>, Counts) {
        let Graph { mut nodes, edges, counts } = link(nodes, doors);
        let comps = components(&mut nodes, &edges);
        (nodes, edges, comps, counts)
    }

    fn ep(model: Option<&str>, id: &str) -> Endpoint {
        Endpoint { model_id: model.map(str::to_string), room_id: id.to_string() }
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
        assert_eq!(by_hops.steps[0].door_id, "far");
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
                comparison_key: None,
                comparison_properties: vec![],
                areas: Default::default(),
                doors: Default::default(),
                windows: Default::default(),
                ffe: Default::default(),
                hierarchy_exclusions: vec![],
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
                levels: vec![Level { id: "lvl1".into(), name: "Level 1".into(), elevation: 0.0 }],
                rooms,
            }
        }

        fn empty_state() -> AppState {
            AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), bundle())]), None)
        }

        /// Three rooms in a row (`a | b | c`) plus a bay `d` away from them.
        fn state(doors: Vec<Opening>) -> AppState {
            let state = empty_state();
            state
                .set_snapshot(rooms_payload(vec![
                    rect("a", 0.0, 10.0),
                    rect("b", 10.0, 20.0),
                    rect("c", 20.0, 30.0),
                    rect("d", 40.0, 50.0),
                ]))
                .unwrap();
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
            let result = assemble_connectivity(&s, "p1", &ConnectivityScope::default(), None, Metric::Distance)
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
            let ab = result.edges.iter().find(|e| e.door_id == "ab").unwrap();
            assert!((ab.length - 10.0).abs() < 1e-6, "{}", ab.length);
        }

        #[test]
        fn test_the_read_routes_and_reports_an_unreachable_bay() {
            let (a, c, d) = (ep(None, "a"), ep(None, "c"), ep(None, "d"));
            let s = state(standard());
            let ok = assemble_connectivity(&s, "p1", &ConnectivityScope::default(), Some((&a, &c)), Metric::Distance)
                .unwrap()
                .unwrap();
            let path = ok.path.unwrap();
            assert!(path.found);
            assert_eq!(path.rooms.len(), 3);
            assert!((path.distance_ft - 20.0).abs() < 1e-6);

            let bay = assemble_connectivity(&s, "p1", &ConnectivityScope::default(), Some((&a, &d)), Metric::Distance)
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
            let result = assemble_connectivity(&state(standard()), "p1", &scope, Some((&a, &c)), Metric::Distance)
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
            let result = assemble_connectivity(&state, "p1", &ConnectivityScope::default(), None, Metric::Distance)
                .unwrap()
                .unwrap();
            assert!(!result.doors_pushed);
            assert_eq!(result.isolated.len(), 1);
        }

        #[test]
        fn test_nothing_pushed_is_none() {
            let state = AppState::new(Box::new(MemStore::new()), HashMap::new(), None);
            assert!(assemble_connectivity(&state, "p1", &ConnectivityScope::default(), None, Metric::Distance)
                .unwrap()
                .is_none());
        }
    }
}
