//! What the connectivity read answers: the graph's nodes and edges, the components and isolated rooms, a path and the steps it walks, the
//! limits (width, height) along it, and the reports on authored zones, links and passages.

use super::*;

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
    /// What the route's widths say, present whenever a route was asked for.
    pub clearance: Option<Clearance>,
}

/// One hop's width, and what it is.
#[derive(Serialize, Clone)]
pub struct WidthLimit {
    /// An ESTIMATE for a door (footprint less the frame allowance) and the shared wall's
    /// length for an open zone.
    pub width_mm: f64,
    pub kind: EdgeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub door_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub door_model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passage_id: Option<String>,
    pub from: RoomRef,
    pub to: RoomRef,
}

/// Whether an object of a given width can make the trip, and what limits it.
///
/// Widths are checked for **doors** (the footprint less a frame allowance) and **open
/// zones** (the length of the wall the two rooms share). **Not checked, and counted in
/// `unchecked_hops`:** a level change (a lift or stair has no width here) and a door with no
/// usable footprint. **Not checked at all:** the width of a corridor INSIDE a room; the
/// route hugs walls, so a corridor narrower than the object still passes. A route that
/// reports itself clear has cleared the doors and openings only.
#[derive(Serialize)]
pub struct Clearance {
    /// The width asked for, or absent when none was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asked_mm: Option<f64>,
    /// The frame allowance the door widths were reduced by.
    pub frame_allowance_mm: f64,
    /// The narrowest checked hop on the route found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub narrowest: Option<WidthLimit>,
    /// Hops of the route found that could not be checked.
    pub unchecked_hops: usize,
    /// The widest object that could make the trip by SOME route, and the hop that limits
    /// it. Absent when no checked hop limits it, or when no route exists at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub widest_possible: Option<WidthLimit>,
    /// The height asked for, or absent when none was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asked_height_mm: Option<f64>,
    /// The lowest checked door or room on the route found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lowest: Option<HeightLimit>,
    /// The tallest object that could make the trip by SOME route, and the door or room
    /// that limits it. Absent when nothing checked limits it, or no route exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tallest_possible: Option<HeightLimit>,
    /// Doors and rooms of the route found whose height is unknown, so were not checked.
    pub unchecked_height: usize,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum HeightKind {
    Door,
    Room,
    Passage,
}

/// One door's or room's height, and which it is.
#[derive(Serialize, Clone)]
pub struct HeightLimit {
    /// An ESTIMATE for a door (its property, else its type name) and the room's
    /// height property for a room.
    pub height_mm: f64,
    pub kind: HeightKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub door_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub door_model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passage_id: Option<String>,
    /// The room, or the two rooms a door or passage joins.
    pub rooms: Vec<RoomRef>,
    /// For a room: the height is a ceiling's and not the room's own property.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub from_ceiling: bool,
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
    /// The room's clear height in mm: the lowest ceiling over it, else its height
    /// property; absent when neither is readable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_mm: Option<f64>,
    /// `height_mm` came from a ceiling and not from the room's property.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub height_from_ceiling: bool,
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
    /// Where a passage's line leaves its first room; `Edge::far_point` is where it
    /// enters the second.
    Passage,
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
    /// A straight line a person drew between two rooms on one storey.
    Passage,
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
    /// The passage, for a drawn line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passage_id: Option<String>,
    /// Where the route crosses. For a passage this is the point on room `a`'s
    /// outline, and `far_point` the one on room `b`'s.
    pub point: Point2D,
    /// The second crossing, for a passage only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub far_point: Option<Point2D>,
    pub point_source: PointSource,
    /// How wide the way through is, in mm: a door's estimated clear opening, or an open
    /// zone's shared wall. `None` when it cannot be known (a level change, a door with
    /// no footprint), which is passable and counted as unchecked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_mm: Option<f64>,
    /// A door's estimated height in mm. `None` for an open zone (its rooms carry the
    /// height), a level change, and a door whose height is unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_mm: Option<f64>,
    /// Approximate walking distance through the door, in feet.
    pub length: f64,
    #[serde(skip)]
    pub(in crate::service) ia: usize,
    #[serde(skip)]
    pub(in crate::service) ib: usize,
}

impl Edge {
    /// The walk through this edge between two room centres, in feet. The two-point
    /// shape of a passage is why this is not `dist(ca, point) + dist(point, cb)` at
    /// every call site.
    pub(in crate::service) fn length_between(&self, ca: Point2D, cb: Point2D) -> f64 {
        match self.far_point {
            Some(far) => dist(ca, self.point) + dist(self.point, far) + dist(far, cb),
            None => dist(ca, self.point) + dist(self.point, cb),
        }
    }

    /// The crossing points in the order a walker leaving `from_a` meets them.
    pub(in crate::service) fn crossing_from(&self, from_a: bool) -> Vec<Point2D> {
        match (self.far_point, from_a) {
            (Some(far), true) => vec![self.point, far],
            (Some(far), false) => vec![far, self.point],
            (None, _) => vec![self.point],
        }
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passage_id: Option<String>,
    pub point: Point2D,
    pub length: f64,
}

#[derive(Serialize, Debug)]
pub struct Segment {
    pub level_id: String,
    pub points: Vec<Point2D>,
}

// ============================== the graph ==============================

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
    pub passages: Vec<PassageReport>,
}

/// What one passage did. Applied whenever it can be and **reported when it cannot**,
/// never silently dropped, like a link.
#[derive(Serialize)]
pub struct PassageReport {
    pub id: String,
    pub a: RoomRef,
    pub b: RoomRef,
    pub applied: bool,
    /// Why not, when it did not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Where the line leaves the first room and enters the second, when it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Point2D>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Point2D>,
    /// The length of the line between the two crossings, in mm: what the opening
    /// measures along its own direction, which is not its width.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_mm: Option<f64>,
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
    /// Counted AFTER any disconnects, so a bay cut from every neighbour is listed.
    pub unlinked: Vec<RoomRef>,
    /// Shared walls this zone left closed, by a disconnect or by its hub.
    pub closed: usize,
    /// The room members connect only through, when the zone has one that is in scope.
    pub hub: Option<RoomRef>,
    /// The zone names a hub that is no longer in scope, so the rule is NOT applied
    /// (closing every wall would silently disable the zone).
    pub hub_stale: bool,
    /// Disconnects naming a room that is no longer in scope.
    pub stale_disconnects: usize,
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
