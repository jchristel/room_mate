//! How a route is computed once the graph of connections is known: the choice of
//! METHOD.
//!
//! The graph (which rooms connect, through what) is `service::connectivity`'s and
//! does not depend on the method. What does is where a person walks *inside* a
//! room, and so how long a route is and where it is drawn. There is more than one
//! published way to do that, so the method is a parameter, listed in `CATALOG` so a
//! caller (the viewer's picker, an MCP client) can discover them, with what each is
//! and where it comes from.
//!
//! **Adding a method is a variant of `Method`, an entry in `CATALOG`, and a
//! function here**; nothing in the graph, the HTTP route or the tool changes shape.
//!
//! ## The methods
//!
//! - **`centroid`**: the original. A room is a node at its centre; a route walks
//!   from one centre to a door to the next centre. The dual-graph / cell-centre
//!   family (Lee 2001; Lorenz, Ohlbach and Stoffel 2006). Cheap and always
//!   available, but it bends at every centre, and Liu and Zlatanova (2011) name
//!   exactly that as its fault: it walks to the middle of rooms nobody needs to
//!   enter and takes "unnecessary tortuous paths".
//! - **`door_to_door`**: the recommended one. Liu and Zlatanova (2011), "A
//!   'door-to-door' path-finding approach for indoor navigation" (ISPRS Gi4DM),
//!   invert the graph: the doors are the nodes and a room is the set of edges
//!   between its doors, so a route runs from one door to the next visible door, or
//!   by the shortest way round for two doors that cannot see each other. Inside a
//!   room that shortest way is computed exactly on the room's polygon, columns
//!   included, by a visibility graph over its reflex corners (`service::geodesic`;
//!   de Berg et al., *Computational Geometry*, ch. 15; the building block Xu, Wei
//!   and Zlatanova 2016 compare their subdivision against and find theirs
//!   compatible with). Two-level, as they describe it: the graph says which rooms,
//!   the room polygons say where.
//!
//! ## What the geometry covers, and does not
//!
//! The walk is the geometric shortest path inside the room's outline. It is not
//! where a person walks (people keep off walls), furniture is not modelled, and a
//! room whose outline is missing or too irregular falls back to the straight line
//! between its two points, counted in the result's `note` so it is never silent.
//! A route's distance is a walking estimate, not a measurement.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use serde::Serialize;

use crate::contract::{Point2D, Room};

use super::connectivity::{route as centroid_route, Edge, EdgeKind, Metric, Node, PathResult, Segment, Step};
use super::geodesic::RoomShape;
use super::ServiceError;

/// One way of computing where a route runs inside rooms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// Room centre to door to room centre.
    Centroid,
    /// Door to door, by the shortest walk inside each room. The default.
    #[default]
    DoorToDoor,
}

/// What a method is, for a picker or an agent choosing one.
#[derive(Debug, Serialize)]
pub struct MethodInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    /// The published source, or what the method is when it is the baseline.
    pub reference: &'static str,
}

pub const CATALOG: &[MethodInfo] = &[
    MethodInfo {
        id: "door_to_door",
        name: "Door to door (shortest in each room)",
        summary: "Routes from door to door, taking the exact shortest walk inside each room around its corners and columns. \
                  The shortest of the methods; the default.",
        reference: "Liu and Zlatanova 2011, 'A \"door-to-door\" path-finding approach for indoor navigation', ISPRS Gi4DM; \
                    visibility graph after Lozano-Perez and Wesley 1979 and de Berg et al., Computational Geometry, ch. 15",
    },
    MethodInfo {
        id: "centroid",
        name: "Room centres",
        summary: "Routes from the centre of each room to its door and on to the next centre. Fast and simple, but it bends \
                  at every room centre, so it overstates distances and draws detours.",
        reference: "Dual-graph / cell-centre networks (Lee 2001; Lorenz, Ohlbach and Stoffel 2006), the baseline Liu and \
                    Zlatanova 2011 improve on",
    },
];

impl Method {
    pub fn id(self) -> &'static str {
        match self {
            Method::Centroid => "centroid",
            Method::DoorToDoor => "door_to_door",
        }
    }

    pub fn parse(raw: Option<&str>) -> Result<Self, ServiceError> {
        match raw.map(str::trim) {
            None | Some("") => Ok(Method::default()),
            Some("door_to_door") => Ok(Method::DoorToDoor),
            Some("centroid") => Ok(Method::Centroid),
            Some(other) => Err(ServiceError::Invalid(format!(
                "method {other:?} is not one of: {}",
                CATALOG.iter().map(|m| m.id).collect::<Vec<_>>().join(", ")
            ))),
        }
    }
}

/// Compute the route with the chosen method.
///
/// `rooms[i]` is node `i`'s room outline, when it has one. The hops metric has no
/// geometry to improve (it counts doors), so it always takes the graph's own
/// search, and says so.
pub(super) fn route(
    method: Method,
    nodes: &[Node],
    edges: &[Edge],
    rooms: &[Option<&Room>],
    from: usize,
    to: usize,
    metric: Metric,
) -> PathResult {
    match (method, metric) {
        (Method::DoorToDoor, Metric::Distance) => door_to_door(nodes, edges, rooms, from, to),
        (Method::DoorToDoor, Metric::Hops) => {
            let mut result = centroid_route(nodes, edges, from, to, metric);
            result.note = Some(
                "the hops metric counts doors and has no geometry, so the room-centre drawing was used".to_string(),
            );
            result
        }
        (Method::Centroid, _) => centroid_route(nodes, edges, from, to, metric),
    }
}

// =============================== door to door ===============================

/// A point where a route crosses between rooms or starts or ends, as seen from
/// ONE room. A door is two ports (one on each side), joined by the crossing.
struct Port {
    room: usize,
    at: Point2D,
    /// The port on the other side, what crossing costs, and which edge it is.
    twin: Option<(usize, f64, usize)>,
}

#[derive(Clone, Copy)]
enum Hop {
    Walk(usize),
    Cross(usize),
}

fn dist(a: Point2D, b: Point2D) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// Lazily built room shapes, one per room a search touches.
struct Shapes<'a> {
    rooms: &'a [Option<&'a Room>],
    cache: HashMap<usize, Option<RoomShape>>,
}

impl<'a> Shapes<'a> {
    fn new(rooms: &'a [Option<&'a Room>]) -> Self {
        Self { rooms, cache: HashMap::new() }
    }

    fn get(&mut self, node: usize) -> Option<&RoomShape> {
        let rooms = self.rooms;
        self.cache.entry(node).or_insert_with(|| rooms[node].and_then(RoomShape::new)).as_ref()
    }

    /// Where a route starts, ends or changes level in a room: its centre when the
    /// centre is in the room (so both methods start from the same place and a
    /// comparison between them is about the walk and nothing else), else a point
    /// surely inside it, as for an L or a C whose centre is out in the cold. The
    /// given centre again for a room with no outline.
    fn inside(&mut self, node: usize, centre: Point2D) -> Point2D {
        match self.get(node) {
            Some(shape) if shape.contains(centre) => centre,
            Some(shape) => shape.inside_point().unwrap_or(centre),
            None => centre,
        }
    }

    fn snap(&mut self, node: usize, p: Point2D) -> Point2D {
        self.get(node).map_or(p, |s| s.snap(p))
    }

    /// The walk between two points of one room: exact where the room has an
    /// outline, the straight line where it does not, and `false` in that case so
    /// the caller can count the fallback.
    fn walk(&mut self, node: usize, a: Point2D, b: Point2D) -> RoomWalk {
        match self.get(node).and_then(|s| s.walk(a, b)) {
            Some(w) => (w.length, w.points, true),
            None => (dist(a, b), vec![a, b], false),
        }
    }
}

#[derive(PartialEq)]
struct Frontier {
    /// Cost so far plus the heuristic: what the queue is ordered by.
    estimate: f64,
    /// Cost so far.
    cost: f64,
    port: usize,
}
impl Eq for Frontier {}
impl Ord for Frontier {
    fn cmp(&self, other: &Self) -> Ordering {
        other.estimate.total_cmp(&self.estimate).then(other.port.cmp(&self.port))
    }
}
impl PartialOrd for Frontier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A walk between two ports of one room: its length, its points, and whether it is
/// exact (false when the room had no usable outline and the line is straight).
type RoomWalk = (f64, Vec<Point2D>, bool);

/// Every port, and which ports each room holds.
struct Network {
    ports: Vec<Port>,
    by_room: Vec<Vec<usize>>,
}

/// What the search found: the cheapest cost to each port, how it was reached, and
/// every in-room walk it computed (so the path can be drawn without redoing them).
struct Solved {
    best: Vec<f64>,
    prev: Vec<Option<Hop>>,
    walks: HashMap<(usize, usize), RoomWalk>,
}

/// The route from the start room's inside point to the end room's, door to door.
///
/// **Ports are the nodes.** Every edge of the graph becomes two ports, one in each
/// of its rooms; a door's ports sit where the door's point meets each room's
/// outline, and are joined by the short crossing through the wall. Within a room
/// every pair of its ports is joined by the exact walk between them, found only
/// when the search reaches the room (rooms with many doors are not paid for unless
/// the route goes through them). A level change has no plan position, so its two
/// ports sit at each room's inside point and are joined at the link's stated cost.
fn door_to_door(nodes: &[Node], edges: &[Edge], rooms: &[Option<&Room>], from: usize, to: usize) -> PathResult {
    let mut shapes = Shapes::new(rooms);
    let net = build_network(nodes, edges, &mut shapes, from, to);
    let solved = solve(&net, edges, &mut shapes);
    if !solved.best[1].is_finite() {
        return PathResult {
            found: false,
            reason: Some(format!(
                "no route through the connections: the first room is in component {} and the second in component {}. \
                 This is a limit of what is connected, not necessarily of the building",
                nodes[from].component, nodes[to].component
            )),
            from: nodes[from].room.clone(),
            to: nodes[to].room.clone(),
            cost: 0.0,
            distance_ft: 0.0,
            arrival_ft: 0.0,
            method: Method::DoorToDoor.id(),
            note: None,
            rooms: vec![],
            steps: vec![],
            segments: vec![],
        };
    }
    assemble(nodes, edges, &net, &solved, from, to)
}

/// Turn the graph's edges into ports. Port 0 is the start and port 1 the end.
fn build_network(nodes: &[Node], edges: &[Edge], shapes: &mut Shapes<'_>, from: usize, to: usize) -> Network {
    let mut ports: Vec<Port> = Vec::new();
    ports.push(Port { room: from, at: shapes.inside(from, nodes[from].centroid), twin: None });
    ports.push(Port { room: to, at: shapes.inside(to, nodes[to].centroid), twin: None });
    for (k, e) in edges.iter().enumerate() {
        let (at_a, at_b, crossing) = match e.kind {
            EdgeKind::Door | EdgeKind::Zone => {
                let (a, b) = (shapes.snap(e.ia, e.point), shapes.snap(e.ib, e.point));
                (a, b, dist(a, e.point) + dist(e.point, b))
            }
            EdgeKind::Vertical => (
                shapes.inside(e.ia, nodes[e.ia].centroid),
                shapes.inside(e.ib, nodes[e.ib].centroid),
                e.length,
            ),
        };
        let first = ports.len();
        ports.push(Port { room: e.ia, at: at_a, twin: Some((first + 1, crossing, k)) });
        ports.push(Port { room: e.ib, at: at_b, twin: Some((first, crossing, k)) });
    }
    let mut by_room: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (i, p) in ports.iter().enumerate() {
        by_room[p.room].push(i);
    }
    Network { ports, by_room }
}

/// A* over ports, with the straight-line distance to the end as the estimate.
///
/// That is safe (never over-estimates) as long as no edge is cheaper than the plan
/// distance it spans: true of every walk and crossing, and checked for level
/// changes, whose two ends need not be stacked. When it is not, the estimate is
/// zero and this is plain Dijkstra. A walk between two ports of a room is computed
/// once.
fn solve(net: &Network, edges: &[Edge], shapes: &mut Shapes<'_>) -> Solved {
    let ports = &net.ports;
    let goal = ports[1].at;
    let safe = edges.iter().enumerate().all(|(k, e)| {
        e.kind != EdgeKind::Vertical
            || ports
                .iter()
                .any(|p| matches!(p.twin, Some((t, c, kk)) if kk == k && dist(p.at, ports[t].at) <= c + 1e-9))
    });
    let estimate = |port: usize| if safe { dist(ports[port].at, goal) } else { 0.0 };

    let mut walks: HashMap<(usize, usize), RoomWalk> = HashMap::new();
    let mut best = vec![f64::INFINITY; ports.len()];
    let mut prev: Vec<Option<Hop>> = vec![None; ports.len()];
    let mut heap = BinaryHeap::new();
    best[0] = 0.0;
    heap.push(Frontier { estimate: estimate(0), cost: 0.0, port: 0 });
    while let Some(Frontier { cost, port: x, .. }) = heap.pop() {
        if x == 1 {
            break;
        }
        if cost > best[x] {
            continue;
        }
        let room = ports[x].room;
        for &y in &net.by_room[room] {
            if y == x {
                continue;
            }
            let key = (x.min(y), x.max(y));
            let len = walks.entry(key).or_insert_with(|| shapes.walk(room, ports[key.0].at, ports[key.1].at)).0;
            if cost + len < best[y] {
                best[y] = cost + len;
                prev[y] = Some(Hop::Walk(x));
                heap.push(Frontier { estimate: cost + len + estimate(y), cost: cost + len, port: y });
            }
        }
        if let Some((twin, crossing, _)) = ports[x].twin
            && cost + crossing < best[twin]
        {
            best[twin] = cost + crossing;
            prev[twin] = Some(Hop::Cross(x));
            heap.push(Frontier { estimate: cost + crossing + estimate(twin), cost: cost + crossing, port: twin });
        }
    }
    Solved { best, prev, walks }
}

/// Read the chain of hops back from the end port and build the route: the rooms
/// passed, a step for each crossing (carrying the walk before it), and the drawn
/// polyline grouped by level.
fn assemble(nodes: &[Node], edges: &[Edge], net: &Network, solved: &Solved, from: usize, to: usize) -> PathResult {
    let ports = &net.ports;
    let mut chain: Vec<(usize, Hop)> = Vec::new();
    let mut at = 1;
    while at != 0 {
        let hop = solved.prev[at].expect("a reached port has a predecessor");
        chain.push((at, hop));
        at = match hop {
            Hop::Walk(x) | Hop::Cross(x) => x,
        };
    }
    chain.reverse();

    let (from_ref, to_ref) = (nodes[from].room.clone(), nodes[to].room.clone());
    let mut room_path = vec![from_ref.clone()];
    let mut steps: Vec<Step> = Vec::new();
    let mut drawn: Vec<(String, Point2D)> = vec![(nodes[from].level_id.clone(), ports[0].at)];
    let mut pending = 0.0;
    let mut fallbacks = 0usize;
    for (arrived, hop) in chain {
        match hop {
            Hop::Walk(x) => {
                let key = (x.min(arrived), x.max(arrived));
                let (len, pts, exact) = &solved.walks[&key];
                if !exact {
                    fallbacks += 1;
                }
                let ordered: Vec<Point2D> = if key.0 == x { pts.clone() } else { pts.iter().rev().copied().collect() };
                for p in ordered.into_iter().skip(1) {
                    drawn.push((nodes[ports[x].room].level_id.clone(), p));
                }
                pending += len;
            }
            Hop::Cross(x) => {
                let (twin, crossing, k) = ports[x].twin.expect("a crossing has a twin");
                let e = &edges[k];
                let (here, there) = (ports[x].room, ports[twin].room);
                if e.kind != EdgeKind::Vertical {
                    drawn.push((nodes[here].level_id.clone(), e.point));
                }
                drawn.push((nodes[there].level_id.clone(), ports[twin].at));
                steps.push(Step {
                    from: nodes[here].room.clone(),
                    to: nodes[there].room.clone(),
                    kind: e.kind,
                    door_id: e.door_id.clone(),
                    door_model_id: e.door_model_id.clone(),
                    zone_id: e.zone_id.clone(),
                    link_id: e.link_id.clone(),
                    point: e.point,
                    length: pending + crossing,
                });
                pending = 0.0;
                room_path.push(nodes[there].room.clone());
            }
        }
    }

    let mut segments: Vec<Segment> = Vec::new();
    for (level, p) in drawn {
        match segments.last_mut() {
            Some(seg) if seg.level_id == level => seg.points.push(p),
            _ => segments.push(Segment { level_id: level, points: vec![p] }),
        }
    }
    let total = solved.best[1];
    PathResult {
        found: true,
        reason: None,
        from: from_ref,
        to: to_ref,
        cost: total,
        distance_ft: total,
        arrival_ft: pending,
        method: Method::DoorToDoor.id(),
        note: (fallbacks > 0).then(|| {
            format!(
                "{fallbacks} walk{} inside rooms used a straight line because the room has no usable outline, \
                 so those stretches are not shortest-path",
                if fallbacks == 1 { "" } else { "s" }
            )
        }),
        rooms: room_path,
        steps,
        segments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_the_catalog_names_every_method_once_and_parse_agrees_with_it() {
        let ids: Vec<_> = CATALOG.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["door_to_door", "centroid"]);
        for m in CATALOG {
            assert_eq!(Method::parse(Some(m.id)).unwrap().id(), m.id);
            assert!(!m.reference.is_empty() && !m.summary.is_empty());
        }
        assert_eq!(Method::parse(None).unwrap(), Method::DoorToDoor, "the recommended method is the default");
        assert_eq!(Method::parse(Some(" ")).unwrap(), Method::DoorToDoor);
        let err = Method::parse(Some("teleport")).unwrap_err();
        assert!(matches!(err, ServiceError::Invalid(m) if m.contains("door_to_door") && m.contains("centroid")));
    }
}
