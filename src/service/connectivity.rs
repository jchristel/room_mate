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
//! ~500-line trigger, but more than 300 of the lines are tests over one algorithm
//! (join, components, Dijkstra, resolve) that share a fixture. The seam if it
//! grows is `link`/`components`/`shortest`, which are already pure functions.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap};

use serde::Serialize;

use crate::connections;
use crate::contract::{DoorPayload, Level, Point2D, Room};
use crate::state::AppState;

use super::adjacency::{centroid_of, compute_adjacency_indexed, default_wall_max};
use super::openings::{assemble_openings, OpeningKind, OpeningScope};
use super::room_locator::RoomRef;
use super::rooms::{assemble_rooms, RoomFilter, RoomScope};
use super::routing::{self, Method};
use super::ServiceError;

pub const SCHEMA_VERSION: u32 = 1;

/// How much of the graph a read returns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Detail {
    /// Components, isolated rooms, counts and the route; no nodes or edges.
    #[default]
    Summary,
    /// Everything, including every node and edge.
    Full,
}

impl Detail {
    pub fn parse(raw: Option<&str>) -> Result<Self, ServiceError> {
        match raw.map(str::trim) {
            None | Some("") | Some("summary") => Ok(Detail::Summary),
            Some("full") => Ok(Detail::Full),
            Some(other) => Err(ServiceError::Invalid(format!("detail {other:?} is not one of: summary, full"))),
        }
    }
}

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
    /// Where in the room the route starts or ends, in the project's local frame.
    /// Absent means the room's centre. A point outside the room is moved onto its
    /// outline and the answer says so.
    pub at: Option<Point2D>,
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
    /// The method the route was asked to use, and every method on offer, so a
    /// picker needs no second request. See `service::routing`.
    pub method: &'static str,
    pub methods: &'static [routing::MethodInfo],
    /// False when no doors snapshot exists for the project at all. Every room is
    /// then isolated, which says "no doors were pushed", not "no doors exist".
    pub doors_pushed: bool,
    pub levels: Vec<Level>,
    /// The graph itself, present only for `Detail::Full`. The summary is what a
    /// picker or an agent needs (components, isolated rooms, counts, the route),
    /// and on a large project the graph is over a megabyte.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodes: Option<Vec<Node>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edges: Option<Vec<Edge>>,
    /// Connected sets of rooms, largest first. `Node::component` indexes this.
    pub components: Vec<Component>,
    /// Rooms no door reaches: the worklist for authored connections. Not all of
    /// them are bays.
    pub isolated: Vec<IsolatedRoom>,
    pub counts: Counts,
    /// What the authored connections did to this graph.
    pub connections: ConnectionsReport,
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
    /// The middle of the longest wall two members of an open zone share.
    SharedWall,
    /// A change of storey has no plan position, so the point is the lower
    /// room's own centre and the route is drawn as a break, not a line.
    Vertical,
}

/// What joins the two rooms of an edge.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// A door, from the model.
    Door,
    /// An open zone, from the user.
    Zone,
    /// A change of storey inside a vertical zone, from the user.
    Vertical,
}

#[derive(Serialize, Clone)]
pub struct Edge {
    pub a: RoomRef,
    pub b: RoomRef,
    pub kind: EdgeKind,
    /// The door, for a door edge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub door_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub door_model_id: Option<String>,
    /// The open zone, for a zone edge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_id: Option<String>,
    /// The vertical link, for a level change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_id: Option<String>,
    pub point: Point2D,
    pub point_source: PointSource,
    /// Approximate walking distance through the door, in feet.
    pub length: f64,
    #[serde(skip)]
    pub(super) ia: usize,
    #[serde(skip)]
    pub(super) ib: usize,
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
    /// The walk inside the LAST room, from its final door to the end point. Each
    /// step's length already includes the walk inside the room it leaves, so the
    /// steps plus this sum to `distance_ft` under the door-to-door method; it is 0
    /// under room centres, whose step lengths are centre to door to centre.
    pub arrival_ft: f64,
    /// Where the route starts and ends in the plan: each room's centre, or the
    /// point asked for (moved onto the room if it lay outside it).
    pub start: Point2D,
    pub end: Point2D,
    /// The method that produced this route: what was asked for, unless it could
    /// not apply (see `note`).
    pub method: &'static str,
    /// Anything about how the route was computed that a reader should know, such as
    /// rooms whose outline was too irregular to walk exactly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
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
    pub kind: EdgeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub door_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub door_model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_id: Option<String>,
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
                    kind: EdgeKind::Door,
                    door_id: Some(door.door_id),
                    door_model_id: Some(door.door_model_id),
                    zone_id: None,
                    link_id: None,
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

pub(super) fn route(nodes: &[Node], edges: &[Edge], from: usize, to: usize, metric: Metric) -> PathResult {
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
            arrival_ft: 0.0,
            start: nodes[from].centroid,
            end: nodes[to].centroid,
            method: super::routing::Method::Centroid.id(),
            note: None,
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
            kind: e.kind,
            door_id: e.door_id.clone(),
            door_model_id: e.door_model_id.clone(),
            zone_id: e.zone_id.clone(),
            link_id: e.link_id.clone(),
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
        arrival_ft: 0.0,
        start: nodes[from].centroid,
        end: nodes[to].centroid,
        method: super::routing::Method::Centroid.id(),
        note: None,
        rooms,
        steps,
        segments: segments_of(nodes, edges, from, &walked),
    }
}

// ============================== open zones and links ==============================

/// One wall two members of an open zone share, ready to become an edge.
struct ZoneEdgeFact {
    zone_id: String,
    ia: usize,
    ib: usize,
    point: Point2D,
}

/// What the authored connections did to this read, so a person can see the
/// effect of what they drew.
#[derive(Serialize)]
pub struct ConnectionsReport {
    /// The version of the document applied; empty when nothing is authored.
    pub taken_at: String,
    /// Whether a milestone's own version was used. Always false for now: a
    /// milestone view applies today's connections, and says so, so a historical
    /// route is not read as historical.
    pub pinned: bool,
    /// Set when the document could not be read. The route still computes, from
    /// doors alone, because an unreadable file must not take routing down.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub zones: Vec<ZoneReport>,
    pub links: Vec<LinkReport>,
}

#[derive(Serialize)]
pub struct ZoneReport {
    pub id: String,
    pub name: String,
    pub members: usize,
    /// Members that are no longer among the rooms in scope: renumbered, deleted,
    /// or in another building. A reported state, never an error.
    pub stale: Vec<RoomRef>,
    /// Edges this zone added to the graph.
    pub edges: usize,
    /// Pairs the zone covers that a door (or an earlier zone) already joined.
    pub redundant: usize,
    /// Members sharing a wall with no other member: the zone cannot connect them.
    pub unlinked: Vec<RoomRef>,
    /// Whether any member has a door to somewhere. False means the whole zone is
    /// still an island, which is the thing a person most needs to be told.
    pub reaches_doors: bool,
}

/// What one vertical link did. A link is a person's statement, so it is applied
/// whenever it can be and **reported when it cannot**, never silently dropped.
#[derive(Serialize)]
pub struct LinkReport {
    pub id: String,
    pub a: RoomRef,
    pub b: RoomRef,
    /// The walking-equivalent cost used for this hop.
    pub cost_ft: f64,
    /// Whether it became an edge.
    pub applied: bool,
    /// Why not, when it did not: a room no longer in scope, both rooms on one
    /// storey, or a pair a door or earlier link already joined.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// How many storeys with rooms lie strictly between the two rooms. A link is
    /// meant to be floor to floor, so anything above 0 is worth a second look; it
    /// is reported, not refused, because the person drew it.
    pub levels_between: usize,
}

/// One link, ready to become an edge.
struct LinkFact {
    link_id: String,
    ia: usize,
    ib: usize,
    cost: f64,
}

/// Turn zone facts into edges, skipping any pair a door or an earlier zone
/// already joined. Returns, per zone, `(added, redundant)`.
fn add_zone_edges(
    nodes: &mut [Node],
    edges: &mut Vec<Edge>,
    facts: Vec<ZoneEdgeFact>,
) -> BTreeMap<String, (usize, usize)> {
    let key = |a: usize, b: usize| (a.min(b), a.max(b));
    let mut joined: std::collections::BTreeSet<(usize, usize)> = edges.iter().map(|e| key(e.ia, e.ib)).collect();
    let mut tally: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for fact in facts {
        let entry = tally.entry(fact.zone_id.clone()).or_default();
        if !joined.insert(key(fact.ia, fact.ib)) {
            entry.1 += 1;
            continue;
        }
        entry.0 += 1;
        let (ca, cb) = (nodes[fact.ia].centroid, nodes[fact.ib].centroid);
        nodes[fact.ia].degree += 1;
        nodes[fact.ib].degree += 1;
        edges.push(Edge {
            a: nodes[fact.ia].room.clone(),
            b: nodes[fact.ib].room.clone(),
            kind: EdgeKind::Zone,
            door_id: None,
            door_model_id: None,
            zone_id: Some(fact.zone_id),
            link_id: None,
            point: fact.point,
            point_source: PointSource::SharedWall,
            length: dist(ca, fact.point) + dist(fact.point, cb),
            ia: fact.ia,
            ib: fact.ib,
        });
    }
    tally
}

/// Turn link facts into edges. A change of storey has no plan distance, so the
/// edge's length is the link's cost and its point is the first room's own centre
/// (the route is drawn as a break there, not a line). Returns the ids that were
/// redundant with an edge already in the graph.
fn add_link_edges(
    nodes: &mut [Node],
    edges: &mut Vec<Edge>,
    facts: Vec<LinkFact>,
) -> std::collections::BTreeSet<String> {
    let key = |a: usize, b: usize| (a.min(b), a.max(b));
    let mut joined: std::collections::BTreeSet<(usize, usize)> = edges.iter().map(|e| key(e.ia, e.ib)).collect();
    let mut redundant = std::collections::BTreeSet::new();
    for fact in facts {
        if !joined.insert(key(fact.ia, fact.ib)) {
            redundant.insert(fact.link_id);
            continue;
        }
        nodes[fact.ia].degree += 1;
        nodes[fact.ib].degree += 1;
        edges.push(Edge {
            a: nodes[fact.ia].room.clone(),
            b: nodes[fact.ib].room.clone(),
            kind: EdgeKind::Vertical,
            door_id: None,
            door_model_id: None,
            zone_id: None,
            link_id: Some(fact.link_id),
            point: nodes[fact.ia].centroid,
            point_source: PointSource::Vertical,
            length: fact.cost,
            ia: fact.ia,
            ib: fact.ib,
        });
    }
    redundant
}

/// Read the project's authored connections and turn them into edges, with a
/// report of what each did.
///
/// **Open zones:** for every zone, the members still in scope are run through the
/// same wall-sharing algorithm `/adjacency` uses, **restricted to those
/// members**, so a route through a zone follows its floor plan (room to room
/// across the wall they share) and never cuts a straight line through whatever
/// lies between.
///
/// **Vertical links:** each is one edge between its two rooms, charged its own
/// cost, applied only when both rooms are in scope and on different storeys.
fn authored_edges(
    state: &AppState,
    project: &str,
    rooms: &super::rooms::RoomsResult,
    nodes: &[Node],
    door_degree: &[usize],
) -> (Vec<ZoneEdgeFact>, Vec<LinkFact>, ConnectionsReport) {
    let (document, error) = match state.projects_dir() {
        None => (connections::ConnectionsDocument::empty(), None),
        Some(dir) => match connections::load(dir, project) {
            Ok(doc) => (doc, None),
            Err(e) => (connections::ConnectionsDocument::empty(), Some(format!("{e:?}"))),
        },
    };
    let mut report = ConnectionsReport {
        taken_at: document.taken_at.clone(),
        pinned: false,
        error,
        zones: Vec::new(),
        links: Vec::new(),
    };
    if document.zones.is_empty() && document.links.is_empty() {
        return (Vec::new(), Vec::new(), report);
    }

    let index: BTreeMap<&RoomRef, usize> = nodes.iter().enumerate().map(|(i, n)| (&n.room, i)).collect();

    let mut zone_facts = Vec::new();
    if !document.zones.is_empty() {
        let policy = state.settings().settings_for(project).map(|b| b.areas.clone()).unwrap_or_default();
        let wall_max = default_wall_max(&policy, rooms);
        let room_of: BTreeMap<RoomRef, &Room> = rooms
            .rooms
            .iter()
            .map(|r| (RoomRef { model_id: r.model_id.clone(), room_id: r.room.id.clone() }, &r.room))
            .collect();
        for zone in &document.zones {
            let mut present: Vec<(usize, &Room, &RoomRef)> = Vec::new();
            let mut stale = Vec::new();
            for member in &zone.rooms {
                match (index.get(member), room_of.get(member)) {
                    (Some(&i), Some(&room)) => present.push((i, room, member)),
                    _ => stale.push(member.clone()),
                }
            }
            let slice: Vec<&Room> = present.iter().map(|(_, r, _)| *r).collect();
            let mut linked = std::collections::BTreeSet::new();
            for pair in compute_adjacency_indexed(&slice, wall_max) {
                linked.insert(pair.ia);
                linked.insert(pair.ib);
                zone_facts.push(ZoneEdgeFact {
                    zone_id: zone.id.clone(),
                    ia: present[pair.ia].0,
                    ib: present[pair.ib].0,
                    point: pair.midpoint,
                });
            }
            report.zones.push(ZoneReport {
                id: zone.id.clone(),
                name: zone.name.clone(),
                members: zone.rooms.len(),
                stale,
                edges: 0,
                redundant: 0,
                unlinked: present
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| !linked.contains(k))
                    .map(|(_, (_, _, m))| (*m).clone())
                    .collect(),
                reaches_doors: present.iter().any(|(i, _, _)| door_degree[*i] > 0),
            });
        }
    }

    // Storeys that hold rooms, by elevation, so "how many lie between" counts only
    // storeys a person could have meant.
    let used: std::collections::BTreeSet<&str> = nodes.iter().map(|n| n.level_id.as_str()).collect();
    let elevation: BTreeMap<&str, f64> = rooms.levels.iter().map(|l| (l.id.as_str(), l.elevation)).collect();
    let mut link_facts = Vec::new();
    for link in &document.links {
        let cost = link.cost_ft.unwrap_or(connections::DEFAULT_LEVEL_COST_FT);
        let mut entry = LinkReport {
            id: link.id.clone(),
            a: link.a.clone(),
            b: link.b.clone(),
            cost_ft: cost,
            applied: false,
            reason: None,
            levels_between: 0,
        };
        match (index.get(&link.a), index.get(&link.b)) {
            (Some(&ia), Some(&ib)) if nodes[ia].level_id == nodes[ib].level_id => {
                entry.reason = Some("both rooms are on the same storey, so it joins no storeys".to_string());
            }
            (Some(&ia), Some(&ib)) => {
                let (ea, eb) = (
                    elevation.get(nodes[ia].level_id.as_str()).copied().unwrap_or(0.0),
                    elevation.get(nodes[ib].level_id.as_str()).copied().unwrap_or(0.0),
                );
                let (lo, hi) = (ea.min(eb), ea.max(eb));
                entry.levels_between =
                    used.iter().filter(|l| elevation.get(*l).is_some_and(|e| *e > lo && *e < hi)).count();
                link_facts.push(LinkFact { link_id: link.id.clone(), ia, ib, cost });
                entry.applied = true;
            }
            _ => entry.reason = Some("a room is no longer among the rooms in scope".to_string()),
        }
        report.links.push(entry);
    }
    (zone_facts, link_facts, report)
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
    endpoints_at(from, from_model, None, to, to_model, None)
}

/// A point as two optional coordinates: both or neither, and finite. A half point
/// is refused rather than guessed, as is a half route.
pub fn point(x: Option<f64>, y: Option<f64>, which: &str) -> Result<Option<Point2D>, ServiceError> {
    match (x, y) {
        (None, None) => Ok(None),
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Ok(Some(Point2D { x, y })),
        (Some(_), Some(_)) => Err(ServiceError::Invalid(format!("the {which} point must be finite numbers"))),
        _ => Err(ServiceError::Invalid(format!("the {which} point needs both its x and its y"))),
    }
}

/// `endpoints`, with an optional point inside each room.
pub fn endpoints_at(
    from: Option<&str>,
    from_model: Option<&str>,
    from_at: Option<Point2D>,
    to: Option<&str>,
    to_model: Option<&str>,
    to_at: Option<Point2D>,
) -> Result<Option<(Endpoint, Endpoint)>, ServiceError> {
    fn blank(s: Option<&str>) -> Option<&str> {
        s.map(str::trim).filter(|s| !s.is_empty())
    }
    let endpoint = |room: &str, model: Option<&str>, at: Option<Point2D>| Endpoint {
        model_id: blank(model).map(str::to_string),
        room_id: room.to_string(),
        at,
    };
    match (blank(from), blank(to)) {
        (None, None) => {
            if from_at.is_some() || to_at.is_some() {
                return Err(ServiceError::Invalid("a point needs the room it is in: give `from` and `to`".to_string()));
            }
            Ok(None)
        }
        (Some(a), Some(b)) => Ok(Some((endpoint(a, from_model, from_at), endpoint(b, to_model, to_at)))),
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
    method: Method,
    detail: Detail,
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

    let Graph { mut nodes, mut edges, counts } = link(nodes, facts);
    // Door degrees, taken before zones add theirs: a zone whose members have none
    // is an island, and the report says so.
    let door_degree: Vec<usize> = nodes.iter().map(|n| n.degree).collect();
    let (zone_facts, link_facts, mut connections_report) = authored_edges(state, project, &rooms, &nodes, &door_degree);
    let tally = add_zone_edges(&mut nodes, &mut edges, zone_facts);
    for zone in &mut connections_report.zones {
        let (added, redundant) = tally.get(&zone.id).copied().unwrap_or((0, 0));
        zone.edges = added;
        zone.redundant = redundant;
    }
    let redundant_links = add_link_edges(&mut nodes, &mut edges, link_facts);
    for link in &mut connections_report.links {
        if redundant_links.contains(&link.id) {
            link.applied = false;
            link.reason = Some("a door, open zone or earlier link already joins these two rooms".to_string());
        }
    }
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
            let ask = routing::Ask { from: i, to: j, from_at: from.at, to_at: to.at };
            Some(routing::route(method, &nodes, &edges, &outlines(&rooms, &nodes), &ask, metric))
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
    }))
}

/// Each node's room outline, for the methods that walk inside rooms.
fn outlines<'a>(rooms: &'a super::rooms::RoomsResult, nodes: &[Node]) -> Vec<Option<&'a Room>> {
    let by_ref: BTreeMap<RoomRef, &Room> = rooms
        .rooms
        .iter()
        .map(|r| (RoomRef { model_id: r.model_id.clone(), room_id: r.room.id.clone() }, &r.room))
        .collect();
    nodes.iter().map(|n| by_ref.get(&n.room).copied()).collect()
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
                note: None,
            }
        }

        /// The standard rooms and doors, with `zones` authored for the project.
        /// Rooms a|b|c share walls in a row, `d` stands apart; only a-b has a door.
        fn zoned(tag: &str, zones: Vec<crate::connections::ZoneInput>) -> (AppState, std::path::PathBuf) {
            let dir = std::env::temp_dir().join(format!("roommate-conn-zones-{tag}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let state = state(vec![door("ab", Some("a"), Some("b"), "none")]).with_projects_dir(dir.clone());
            crate::connections::save(
                &state,
                &dir,
                "p1",
                crate::connections::SaveRequest { base: String::new(), zones, links: vec![] },
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
                crate::connections::SaveRequest { base: String::new(), zones: vec![], links },
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
    }
}
