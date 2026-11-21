//! Shortest paths over the graph: Dijkstra by the chosen metric (`shortest`), a route between two endpoints and the segments it
//! walks (`route`, `segments_of`). The in-room walk is `routing`.

use super::*;

#[derive(PartialEq)]
pub(super) struct Frontier {
    pub(super) cost: f64,
    pub(super) node: usize,
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

pub(super) fn weight(edge: &Edge, metric: Metric) -> f64 {
    match metric {
        Metric::Distance => edge.length,
        Metric::Hops => 1.0,
    }
}

/// Cheapest route between two node indices, as the edge indices walked in order.
/// `None` when they are in different components.
pub(super) fn shortest(nodes: &[Node], edges: &[Edge], from: usize, to: usize, metric: Metric) -> Option<Vec<usize>> {
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
pub(in crate::service) fn resolve(nodes: &[Node], endpoint: &Endpoint, which: &str) -> Result<usize, ServiceError> {
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
pub(super) fn segments_of(nodes: &[Node], edges: &[Edge], start: usize, walked: &[usize]) -> Vec<Segment> {
    let mut points: Vec<(String, Point2D)> = vec![(nodes[start].level_id.clone(), nodes[start].centroid)];
    let mut at = start;
    for &k in walked {
        let e = &edges[k];
        let next = if e.ia == at { e.ib } else { e.ia };
        // The door sits with the room we are leaving; the next room's own
        // centroid starts the next run if it is on another level. A passage has
        // two crossings, met in the order of travel, both on this storey.
        for p in e.crossing_from(e.ia == at) {
            points.push((nodes[at].level_id.clone(), p));
        }
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

pub(in crate::service) fn route(nodes: &[Node], edges: &[Edge], from: usize, to: usize, metric: Metric) -> PathResult {
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
            method: crate::service::routing::Method::Centroid.id(),
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
            passage_id: e.passage_id.clone(),
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
        method: crate::service::routing::Method::Centroid.id(),
        note: None,
        rooms,
        steps,
        segments: segments_of(nodes, edges, from, &walked),
    }
}

// ============================== open zones and links ==============================
