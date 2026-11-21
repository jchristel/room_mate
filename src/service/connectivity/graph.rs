//! Building the graph: doors become edges between room nodes (`link`), authored edges are added on top (`build_graph`), the components are
//! found and the room outlines read. Routing over it is `paths`; `reach` and `routing` share this graph, never a second one.

use super::*;

/// One door, reduced to what the graph needs.
pub(in crate::service) struct DoorFact {
    pub(in crate::service) door_id: String,
    pub(in crate::service) door_model_id: String,
    pub(in crate::service) from: Option<RoomRef>,
    pub(in crate::service) to: Option<RoomRef>,
    /// The door's own point, when it has one.
    pub(in crate::service) point: Option<(Point2D, PointSource)>,
    /// The estimated clear opening, from the footprint. `None` when there is none.
    pub(in crate::service) width_mm: Option<f64>,
    /// The estimated height, from a property or the type name. `None` when unknown.
    pub(in crate::service) height_mm: Option<f64>,
}

pub(in crate::service) struct Graph {
    pub(in crate::service) nodes: Vec<Node>,
    pub(in crate::service) edges: Vec<Edge>,
    pub(in crate::service) counts: Counts,
}

pub(in crate::service) fn dist(a: Point2D, b: Point2D) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// Join the doors onto the rooms. Pure, so the whole decision table is testable
/// without a store.
pub(in crate::service) fn link(mut nodes: Vec<Node>, doors: Vec<DoorFact>) -> Graph {
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
                    passage_id: None,
                    point,
                    far_point: None,
                    point_source,
                    width_mm: door.width_mm,
                    height_mm: door.height_mm,
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
pub(in crate::service) fn components(nodes: &mut [Node], edges: &[Edge]) -> Vec<Component> {
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

/// The graph a read is made from: rooms, doors, and whatever somebody authored, joined
/// into nodes and edges. Shared by the route read and `service::reach`, so a distance
/// field is measured over exactly the graph a route is.
pub(in crate::service) struct GraphParts {
    pub rooms: crate::service::rooms::RoomsResult,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub counts: Counts,
    pub revision: String,
    pub doors_pushed: bool,
    pub connections: ConnectionsReport,
    pub frame_allowance_mm: f64,
}

/// Build the graph for one project, scoped like `/rooms`. `Ok(None)` when no rooms have
/// ever been pushed.
pub(in crate::service) fn build_graph(
    state: &AppState,
    project: &str,
    scope: &ConnectivityScope<'_>,
) -> Result<Option<GraphParts>, ServiceError> {
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
    // Where the size checks read their data from is the project's to name; a request may
    // override the two it can name itself.
    let policy = state.settings().settings_for(project).map(|b| b.routing.clone()).unwrap_or_default();
    let frame_allowance_mm = scope.frame_allowance_mm.unwrap_or_else(|| policy.frame_allowance_mm());
    let height_key = norm(scope.height_property.unwrap_or_else(|| policy.room_height_property()));
    let door_height_properties: Vec<String> = policy.door_height_properties().iter().map(|p| norm(p)).collect();

    // A room's height is read from the ceilings over it first, and only when a height was
    // asked: reading them is a whole extra pass over the project and nothing else needs it.
    let from_ceilings = if scope.clear_height_mm.is_some() && policy.room_height_from_ceilings {
        ceiling_heights(state, project, scope.milestone)?
    } else {
        BTreeMap::new()
    };
    let nodes = room_nodes(&rooms, &from_ceilings, &height_key);

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
                    width_mm: door_clear_width_mm(&d.door, frame_allowance_mm),
                    height_mm: door_height_mm(&d.door, &door_height_properties, policy.door_height_from_type_name),
                })
                .collect()
        }
        None => Vec::new(),
    };

    let Graph { mut nodes, mut edges, counts } = link(nodes, facts);
    // Door degrees, taken before zones add theirs: a zone whose members have none
    // is an island, and the report says so.
    let door_degree: Vec<usize> = nodes.iter().map(|n| n.degree).collect();
    let (zone_facts, link_facts, passage_facts, mut connections_report) =
        authored_edges(state, project, &rooms, &nodes, &door_degree, scope.ignore_cuts);
    let tally = add_zone_edges(&mut nodes, &mut edges, zone_facts);
    let redundant_links = add_link_edges(&mut nodes, &mut edges, link_facts);
    add_passage_edges(&mut nodes, &mut edges, passage_facts);
    record_what_applied(&mut connections_report, &tally, &redundant_links);

    Ok(Some(GraphParts {
        rooms,
        nodes,
        edges,
        counts,
        revision,
        doors_pushed,
        connections: connections_report,
        frame_allowance_mm,
    }))
}

/// Each node's room outline, for the methods that walk inside rooms.
pub(in crate::service) fn outlines<'a>(
    rooms: &'a crate::service::rooms::RoomsResult,
    nodes: &[Node],
) -> Vec<Option<&'a Room>> {
    let by_ref: BTreeMap<RoomRef, &Room> = rooms
        .rooms
        .iter()
        .map(|r| (RoomRef { model_id: r.model_id.clone(), room_id: r.room.id.clone() }, &r.room))
        .collect();
    nodes.iter().map(|n| by_ref.get(&n.room).copied()).collect()
}

/// A door's estimated clear opening, in mm: the longer side of its footprint (an oriented
/// rectangle, so its width along the wall) less the frame allowance. `None` for a footprint
/// that is not a four-point rectangle, because guessing a width from anything else would
/// turn "not known" into a number.
pub(in crate::service) fn door_clear_width_mm(door: &crate::contract::Opening, frame_allowance_mm: f64) -> Option<f64> {
    let pts = &door.loops.first()?.points;
    if pts.len() != 4 {
        return None;
    }
    let overall = dist(pts[0], pts[1]).max(dist(pts[1], pts[2])) * MM_PER_FT;
    (overall > 0.0).then_some((overall - frame_allowance_mm).max(0.0))
}

/// A door's own position: its insertion point, else the middle of its
/// footprint. `None` leaves the graph to place it between the two rooms.
pub(in crate::service) fn door_point(door: &crate::contract::Opening) -> Option<(Point2D, PointSource)> {
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
