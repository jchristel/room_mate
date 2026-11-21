//! The edges a person authored on top of the doors: open zones, vertical links and passages become edges between rooms
//! (`authored_edges`), a passage's crossings read from the outlines at read time (`passage_fact`).

use super::*;

/// One wall two members of an open zone share, ready to become an edge.
pub(super) struct ZoneEdgeFact {
    pub(super) zone_id: String,
    pub(super) ia: usize,
    pub(super) ib: usize,
    pub(super) point: Point2D,
    /// The shared wall's length in mm: how wide the opening is.
    pub(super) width_mm: f64,
}

/// One link, ready to become an edge.
pub(super) struct LinkFact {
    pub(super) link_id: String,
    pub(super) ia: usize,
    pub(super) ib: usize,
    pub(super) cost: f64,
}

/// One passage, ready to become an edge: both crossings already found.
pub(super) struct PassageFact {
    pub(super) passage_id: String,
    pub(super) ia: usize,
    pub(super) ib: usize,
    pub(super) at_a: Point2D,
    pub(super) at_b: Point2D,
    pub(super) width_mm: Option<f64>,
    pub(super) height_mm: Option<f64>,
}

/// Turn zone facts into edges, skipping any pair a door or an earlier zone
/// already joined. Returns, per zone, `(added, redundant)`.
pub(super) fn add_zone_edges(
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
            passage_id: None,
            point: fact.point,
            far_point: None,
            point_source: PointSource::SharedWall,
            width_mm: Some(fact.width_mm),
            height_mm: None,
            length: dist(ca, fact.point) + dist(fact.point, cb),
            ia: fact.ia,
            ib: fact.ib,
        });
    }
    tally
}

/// Turn passage facts into edges. **No pair is skipped for being joined already**,
/// unlike a zone or a link: a passage is drawn on purpose, and a door's estimated
/// width beside the one a person stated is exactly the case where both should stay
/// in the graph, the size check choosing between them.
pub(super) fn add_passage_edges(nodes: &mut [Node], edges: &mut Vec<Edge>, facts: Vec<PassageFact>) {
    for fact in facts {
        let (ca, cb) = (nodes[fact.ia].centroid, nodes[fact.ib].centroid);
        nodes[fact.ia].degree += 1;
        nodes[fact.ib].degree += 1;
        let mut edge = Edge {
            a: nodes[fact.ia].room.clone(),
            b: nodes[fact.ib].room.clone(),
            kind: EdgeKind::Passage,
            door_id: None,
            door_model_id: None,
            zone_id: None,
            link_id: None,
            passage_id: Some(fact.passage_id),
            point: fact.at_a,
            far_point: Some(fact.at_b),
            point_source: PointSource::Passage,
            width_mm: fact.width_mm,
            height_mm: fact.height_mm,
            length: 0.0,
            ia: fact.ia,
            ib: fact.ib,
        };
        edge.length = edge.length_between(ca, cb);
        edges.push(edge);
    }
}

/// Turn link facts into edges. A change of storey has no plan distance, so the
/// edge's length is the link's cost and its point is the first room's own centre
/// (the route is drawn as a break there, not a line). Returns the ids that were
/// redundant with an edge already in the graph.
pub(super) fn add_link_edges(
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
            passage_id: None,
            point: nodes[fact.ia].centroid,
            far_point: None,
            point_source: PointSource::Vertical,
            width_mm: None,
            height_mm: None,
            length: fact.cost,
            ia: fact.ia,
            ib: fact.ib,
        });
    }
    redundant
}

/// One open zone's edges and its report: the members still in scope, run through the
/// wall-sharing algorithm restricted to them, **less the walls the zone keeps
/// closed**.
///
/// A wall is closed when its pair is one of the zone's `disconnects`, or when the zone
/// has a `hub` and neither room is it. A hub that is no longer in scope is NOT
/// applied and is reported (`hub_stale`): applying it would close every wall and turn
/// the zone off without a word. `unlinked` is read after the closing, so a member cut
/// from all its neighbours is listed as the island it now is.
pub(super) fn zone_edges(
    zone: &connections::OpenZone,
    index: &BTreeMap<&RoomRef, usize>,
    room_of: &BTreeMap<RoomRef, &Room>,
    wall_max: f64,
    door_degree: &[usize],
    ignore_cuts: bool,
) -> (Vec<ZoneEdgeFact>, ZoneReport) {
    let mut present: Vec<(usize, &Room, &RoomRef)> = Vec::new();
    let mut stale = Vec::new();
    for member in &zone.rooms {
        match (index.get(member), room_of.get(member)) {
            (Some(&i), Some(&room)) => present.push((i, room, member)),
            _ => stale.push(member.clone()),
        }
    }
    let slice: Vec<&Room> = present.iter().map(|(_, r, _)| *r).collect();
    let in_scope = |r: &RoomRef| present.iter().any(|(_, _, m)| *m == r);
    // Ignoring cuts is "this zone has none", so the report says the hub and the cuts are
    // not in play, rather than reporting a hub that was not applied as stale.
    let hub = zone.hub.as_ref().filter(|h| !ignore_cuts && in_scope(h));
    let hub_stale = !ignore_cuts && zone.hub.is_some() && hub.is_none();
    let disconnects: &[connections::Disconnect] = if ignore_cuts { &[] } else { &zone.disconnects };
    let stale_disconnects = disconnects.iter().filter(|d| !in_scope(&d.a) || !in_scope(&d.b)).count();
    let cuts: std::collections::BTreeSet<(&RoomRef, &RoomRef)> =
        disconnects.iter().map(|d| if d.a <= d.b { (&d.a, &d.b) } else { (&d.b, &d.a) }).collect();

    let mut facts = Vec::new();
    let mut linked = std::collections::BTreeSet::new();
    let mut closed = 0usize;
    for pair in compute_adjacency_indexed(&slice, wall_max) {
        let (ra, rb) = (present[pair.ia].2, present[pair.ib].2);
        let key = if ra <= rb { (ra, rb) } else { (rb, ra) };
        if cuts.contains(&key) || hub.is_some_and(|h| ra != h && rb != h) {
            closed += 1;
            continue;
        }
        linked.insert(pair.ia);
        linked.insert(pair.ib);
        facts.push(ZoneEdgeFact {
            zone_id: zone.id.clone(),
            ia: present[pair.ia].0,
            ib: present[pair.ib].0,
            point: pair.midpoint,
            width_mm: pair.shared_length * MM_PER_FT,
        });
    }
    let report = ZoneReport {
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
        closed,
        hub: hub.cloned(),
        hub_stale,
        stale_disconnects,
        reaches_doors: present.iter().any(|(i, _, _)| door_degree[*i] > 0),
    };
    (facts, report)
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
pub(super) fn authored_edges(
    state: &AppState,
    project: &str,
    rooms: &crate::service::rooms::RoomsResult,
    nodes: &[Node],
    door_degree: &[usize],
    ignore_cuts: bool,
) -> (Vec<ZoneEdgeFact>, Vec<LinkFact>, Vec<PassageFact>, ConnectionsReport) {
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
        passages: Vec::new(),
    };
    if document.zones.is_empty() && document.links.is_empty() && document.passages.is_empty() {
        return (Vec::new(), Vec::new(), Vec::new(), report);
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
            let (facts, zone_report) = zone_edges(zone, &index, &room_of, wall_max, door_degree, ignore_cuts);
            zone_facts.extend(facts);
            report.zones.push(zone_report);
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

    let mut passage_facts = Vec::new();
    if !document.passages.is_empty() {
        let room_of: BTreeMap<RoomRef, &Room> = rooms
            .rooms
            .iter()
            .map(|r| (RoomRef { model_id: r.model_id.clone(), room_id: r.room.id.clone() }, &r.room))
            .collect();
        for passage in &document.passages {
            let mut entry = PassageReport {
                id: passage.id.clone(),
                a: passage.a.clone(),
                b: passage.b.clone(),
                applied: false,
                reason: None,
                from: None,
                to: None,
                span_mm: None,
            };
            match passage_fact(passage, &index, &room_of, nodes) {
                Ok(fact) => {
                    entry.applied = true;
                    entry.from = Some(fact.at_a);
                    entry.to = Some(fact.at_b);
                    entry.span_mm = Some(dist(fact.at_a, fact.at_b) * MM_PER_FT);
                    passage_facts.push(fact);
                }
                Err(reason) => entry.reason = Some(reason),
            }
            report.passages.push(entry);
        }
    }
    (zone_facts, link_facts, passage_facts, report)
}

/// The two points where a straight line leaves its first room and enters its second.
/// Distances below this along the line are one point, not two.
pub(super) const PASSAGE_EPS: f64 = 1e-9;

/// Where the segment `p`→`q` crosses the segment `c`→`d`, as the fraction of `p`→`q`.
/// `None` for parallel lines (a line running along a wall crosses nothing) and for
/// segments that do not meet.
pub(super) fn crossing_t(p: Point2D, q: Point2D, c: Point2D, d: Point2D) -> Option<f64> {
    let (rx, ry) = (q.x - p.x, q.y - p.y);
    let (sx, sy) = (d.x - c.x, d.y - c.y);
    let denom = rx * sy - ry * sx;
    if denom.abs() < 1e-12 {
        return None;
    }
    let t = ((c.x - p.x) * sy - (c.y - p.y) * sx) / denom;
    let u = ((c.x - p.x) * ry - (c.y - p.y) * rx) / denom;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some(t)
}

/// Every fraction along `p`→`q` at which the line crosses a room's outer ring.
pub(super) fn ring_crossings(poly: &geo::Polygon<f64>, p: Point2D, q: Point2D) -> Vec<f64> {
    poly.exterior()
        .lines()
        .filter_map(|l| crossing_t(p, q, Point2D { x: l.start.x, y: l.start.y }, Point2D { x: l.end.x, y: l.end.y }))
        .collect()
}

/// A passage turned into the edge's facts, or the sentence that says why it is not one.
///
/// The line starts inside room `a` and ends inside `b`. It leaves `a` at the FIRST
/// crossing of `a`'s outline and enters `b` at the LAST crossing of `b`'s, so a line
/// that passes through other rooms in between, or across a gap, still joins the two it
/// names. Holes are ignored, as for a room's outline everywhere here: a column does not
/// close a passage.
pub(super) fn passage_fact(
    passage: &connections::Passage,
    index: &BTreeMap<&RoomRef, usize>,
    room_of: &BTreeMap<RoomRef, &Room>,
    nodes: &[Node],
) -> Result<PassageFact, String> {
    use geo::Intersects;
    let (Some(&ia), Some(&ib)) = (index.get(&passage.a), index.get(&passage.b)) else {
        return Err("a room is no longer among the rooms in scope".to_string());
    };
    if nodes[ia].level_id != nodes[ib].level_id {
        return Err("the rooms are on different storeys: join them with a vertical link".to_string());
    }
    let (Some(oa), Some(ob)) = (
        room_of.get(&passage.a).and_then(|r| outline_of(r)),
        room_of.get(&passage.b).and_then(|r| outline_of(r)),
    ) else {
        return Err("a room has no outline to cross".to_string());
    };
    let (p, q) = (
        Point2D { x: passage.from.x, y: passage.from.y },
        Point2D { x: passage.to.x, y: passage.to.y },
    );
    let at = |t: f64| Point2D { x: p.x + (q.x - p.x) * t, y: p.y + (q.y - p.y) * t };
    let inside = |poly: &geo::Polygon<f64>, pt: Point2D| poly.intersects(&geo::Point::new(pt.x, pt.y));
    if !inside(&oa, p) {
        return Err("the line starts outside the first room: the room has moved, or was drawn from another".to_string());
    }
    if !inside(&ob, q) {
        return Err("the line ends outside the second room: the room has moved, or was drawn from another".to_string());
    }
    let Some(leave) = ring_crossings(&oa, p, q).into_iter().filter(|t| *t > PASSAGE_EPS).reduce(f64::min) else {
        return Err("the line never leaves the first room".to_string());
    };
    let Some(enter) = ring_crossings(&ob, p, q).into_iter().filter(|t| *t < 1.0 - PASSAGE_EPS).reduce(f64::max) else {
        return Err("the line never enters the second room".to_string());
    };
    if enter + PASSAGE_EPS < leave {
        return Err(
            "the line enters the second room before it leaves the first: the rooms overlap along it".to_string()
        );
    }
    Ok(PassageFact {
        passage_id: passage.id.clone(),
        ia,
        ib,
        at_a: at(leave),
        at_b: at(enter),
        width_mm: passage.width_mm,
        height_mm: passage.height_mm,
    })
}

// ================================ the read ================================
