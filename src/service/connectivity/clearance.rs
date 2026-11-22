//! What a route can carry: the width and height limits along it, the tightest door and the reason a path is blocked for the
//! object being moved (`clearance_of`, `blocked_reason`).

use super::*;

/// What an object has to get through: its width and height, and how much of a door's
/// footprint is frame. A dimension that is absent is not checked.
#[derive(Clone, Copy)]
pub(super) struct Fit {
    pub(super) width_mm: Option<f64>,
    pub(super) height_mm: Option<f64>,
    pub(super) frame_allowance_mm: f64,
}

/// Whether the object gets through this hop: the way through is wide enough, its door
/// (if any) is tall enough, and BOTH rooms it joins are tall enough. A room too low to
/// stand the object in closes every hop into it, the start and the end included: an
/// object that does not fit its end room has not arrived.
pub(super) fn passable(e: &Edge, nodes: &[Node], fit: &Fit) -> bool {
    let wide = fit.width_mm.is_none_or(|w| e.width_mm.is_none_or(|ew| ew >= w));
    let tall = fit.height_mm.is_none_or(|h| {
        e.height_mm.is_none_or(|eh| eh >= h)
            && nodes[e.ia].height_mm.is_none_or(|nh| nh >= h)
            && nodes[e.ib].height_mm.is_none_or(|nh| nh >= h)
    });
    wide && tall
}

/// The route between two rooms, and what its sizes say.
///
/// With a size asked the route is searched over the edges the object fits through, which
/// is what keeps both methods unchanged: they take an edge list and a node list, and a
/// shorter edge list is just a smaller building. The readout is read against the WHOLE
/// graph.
pub(super) fn route_asked(
    nodes: &[Node],
    edges: &[Edge],
    outlines: &[Option<&Room>],
    ask: &routing::Ask,
    fit: &Fit,
    method: Method,
    metric: Metric,
) -> (PathResult, Clearance) {
    let routable: Vec<Edge>;
    let searched: &[Edge] = if fit.width_mm.is_some() || fit.height_mm.is_some() {
        routable = edges.iter().filter(|e| passable(e, nodes, fit)).cloned().collect();
        &routable
    } else {
        edges
    };
    let mut path = routing::route(method, nodes, searched, outlines, ask, metric);
    let clearance = clearance_of(nodes, edges, &mut path, ask.from, ask.to, fit);
    (path, clearance)
}

/// The edge a route step took, matched back by what identifies it.
pub(super) fn edge_for_step<'a>(edges: &'a [Edge], step: &Step) -> Option<&'a Edge> {
    edges.iter().find(|e| {
        e.kind == step.kind
            && match e.kind {
                EdgeKind::Door => e.door_id == step.door_id && e.door_model_id == step.door_model_id,
                EdgeKind::Zone => {
                    e.zone_id == step.zone_id
                        && ((e.a == step.from && e.b == step.to) || (e.a == step.to && e.b == step.from))
                }
                EdgeKind::Vertical => e.link_id == step.link_id,
                EdgeKind::Passage => e.passage_id == step.passage_id,
            }
    })
}

pub(super) fn width_limit(e: &Edge) -> WidthLimit {
    WidthLimit {
        width_mm: e.width_mm.unwrap_or(0.0),
        kind: e.kind,
        door_id: e.door_id.clone(),
        door_model_id: e.door_model_id.clone(),
        zone_id: e.zone_id.clone(),
        passage_id: e.passage_id.clone(),
        from: e.a.clone(),
        to: e.b.clone(),
    }
}

/// Whether an edge has a height of its own to check: a door's, or a passage's as stated.
/// An open area's walls carry none (the rooms either side do) and nor does a level change.
pub(super) fn carries_height(kind: EdgeKind) -> bool {
    matches!(kind, EdgeKind::Door | EdgeKind::Passage)
}

pub(super) fn door_height_limit(e: &Edge) -> HeightLimit {
    HeightLimit {
        height_mm: e.height_mm.unwrap_or(0.0),
        kind: if e.kind == EdgeKind::Passage { HeightKind::Passage } else { HeightKind::Door },
        door_id: e.door_id.clone(),
        door_model_id: e.door_model_id.clone(),
        passage_id: e.passage_id.clone(),
        rooms: vec![e.a.clone(), e.b.clone()],
        from_ceiling: false,
    }
}

pub(super) fn room_height_limit(n: &Node) -> HeightLimit {
    HeightLimit {
        height_mm: n.height_mm.unwrap_or(0.0),
        kind: HeightKind::Room,
        door_id: None,
        door_model_id: None,
        passage_id: None,
        rooms: vec![n.room.clone()],
        from_ceiling: n.height_from_ceiling,
    }
}

/// The size readout for a route, and the sentence a blocked route gets.
///
/// \`edges\` is the WHOLE graph, so the narrowest and lowest points of the route found and
/// the widest and tallest object that could make the trip by any route are read against
/// everything there is, not just what the asked size left. A route that failed only
/// because of the size has its \`reason\` rewritten to say what would fit and what limits
/// it: "no route" with no way to see why is the answer this readout exists to avoid.
pub(super) fn clearance_of(
    nodes: &[Node],
    edges: &[Edge],
    path: &mut PathResult,
    from: usize,
    to: usize,
    fit: &Fit,
) -> Clearance {
    let mut narrowest: Option<WidthLimit> = None;
    let mut unchecked_hops = 0;
    let mut lowest: Option<HeightLimit> = None;
    let mut unchecked_height = 0;
    let mut lower = |candidate: HeightLimit| {
        if lowest.as_ref().is_none_or(|l| candidate.height_mm < l.height_mm) {
            lowest = Some(candidate);
        }
    };
    if path.found {
        for step in &path.steps {
            let edge = edge_for_step(edges, step);
            match edge {
                Some(e) if e.width_mm.is_some() => {
                    if narrowest.as_ref().is_none_or(|n| e.width_mm.unwrap_or(f64::MAX) < n.width_mm) {
                        narrowest = Some(width_limit(e));
                    }
                }
                _ => unchecked_hops += 1,
            }
            // Only a DOOR or a PASSAGE has a height of its own: an open area's walls carry
            // none, the rooms on either side do, and a level change has none to check.
            if let Some(e) = edge.filter(|e| carries_height(e.kind) && fit.height_mm.is_some()) {
                if e.height_mm.is_some() {
                    lower(door_height_limit(e));
                } else {
                    unchecked_height += 1;
                }
            }
        }
        for r in path.rooms.iter().filter(|_| fit.height_mm.is_some()) {
            if let Some(n) = nodes.iter().find(|n| n.room == *r) {
                if n.height_mm.is_some() {
                    lower(room_height_limit(n));
                } else {
                    unchecked_height += 1;
                }
            }
        }
    }

    let widest_possible = widest_between(nodes, edges, from, to).map(|k| width_limit(&edges[k]));
    // Heights are read (from ceilings, which is a whole extra read) only when one is asked,
    // so without one there is nothing to report and nothing was paid for.
    let tallest_possible = fit.height_mm.and_then(|_| tallest_between(nodes, edges, from, to));

    // Blocked by size, not by the building: the rooms ARE joined, so say what limits it.
    let asked = fit.width_mm.is_some() || fit.height_mm.is_some();
    if !path.found && asked && nodes[from].component == nodes[to].component {
        path.reason = Some(blocked_reason(nodes, fit, widest_possible.as_ref(), tallest_possible.as_ref()));
    }

    Clearance {
        asked_mm: fit.width_mm,
        frame_allowance_mm: fit.frame_allowance_mm,
        narrowest,
        unchecked_hops,
        widest_possible,
        asked_height_mm: fit.height_mm,
        lowest,
        tallest_possible,
        unchecked_height,
    }
}

/// The sentence for a trip no route of the asked size makes: what the widest and tallest
/// that would fit are, and what limits each. When each dimension fits SOME route but no
/// single route fits both, it says that, which is a different finding.
pub(super) fn blocked_reason(
    nodes: &[Node],
    fit: &Fit,
    widest: Option<&WidthLimit>,
    tallest: Option<&HeightLimit>,
) -> String {
    let name = |r: &RoomRef| nodes.iter().find(|n| n.room == *r).map_or(r.room_id.clone(), |n| n.name.clone());
    let mut asked = Vec::new();
    let mut clauses = Vec::new();
    if let Some(w) = fit.width_mm {
        asked.push(format!("{w:.0} mm wide"));
        if let Some(l) = widest.filter(|l| l.width_mm < w) {
            let what = match l.kind {
                EdgeKind::Door => format!("door {}", l.door_id.as_deref().unwrap_or("?")),
                EdgeKind::Zone => "an open area's shared wall".to_string(),
                EdgeKind::Vertical => "a level change".to_string(),
                EdgeKind::Passage => format!("a passage {}", l.passage_id.as_deref().unwrap_or("?")),
            };
            clauses.push(format!(
                "the widest that fits is about {:.0} mm, limited by {what} between {} and {}",
                l.width_mm,
                name(&l.from),
                name(&l.to)
            ));
        }
    }
    if let Some(h) = fit.height_mm {
        asked.push(format!("{h:.0} mm tall"));
        if let Some(l) = tallest.filter(|l| l.height_mm < h) {
            let what = match l.kind {
                HeightKind::Door => format!(
                    "door {} between {} and {}",
                    l.door_id.as_deref().unwrap_or("?"),
                    name(&l.rooms[0]),
                    name(&l.rooms[1])
                ),
                HeightKind::Passage => format!(
                    "passage {} between {} and {}",
                    l.passage_id.as_deref().unwrap_or("?"),
                    name(&l.rooms[0]),
                    name(&l.rooms[1])
                ),
                HeightKind::Room => {
                    format!("room {}{}", name(&l.rooms[0]), if l.from_ceiling { " (from its ceiling)" } else { "" })
                }
            };
            clauses.push(format!("the tallest that fits is about {:.0} mm, limited by {what}", l.height_mm));
        }
    }
    if clauses.is_empty() {
        clauses.push("each size fits some route, but no single route fits both".to_string());
    }
    format!(
        "no route for an object {}: {}. Door widths are estimates (footprint less {:.0} mm for the frame), door heights \
         are read from a height property or the type name, and room heights from the room's height property",
        asked.join(" and "),
        clauses.join("; "),
        fit.frame_allowance_mm
    )
}

/// The widest object that can get from \`from\` to \`to\`: the largest width such that the
/// rooms are still joined using only hops at least that wide. Returns the index of the
/// hop that limits it, or \`None\` when no hop limits it (the rooms are joined by hops of
/// unknown width alone) or the rooms are not joined at all.
///
/// Edges are added widest first (those of unknown width first of all, since they never
/// block) until the two rooms meet; the edge that joins them is the bottleneck.
pub(super) fn widest_between(nodes: &[Node], edges: &[Edge], from: usize, to: usize) -> Option<usize> {
    if from == to {
        return None;
    }
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut order: Vec<usize> = (0..edges.len()).collect();
    order.sort_by(|&a, &b| match (edges[a].width_mm, edges[b].width_mm) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(x), Some(y)) => y.total_cmp(&x),
    });
    let mut parent: Vec<usize> = (0..nodes.len()).collect();
    for k in order {
        let (ra, rb) = (find(&mut parent, edges[k].ia), find(&mut parent, edges[k].ib));
        if ra != rb {
            parent[ra] = rb;
        }
        if find(&mut parent, from) == find(&mut parent, to) {
            return edges[k].width_mm.map(|_| k);
        }
    }
    None
}

/// The nodes and edges of a route from \`from\` to \`to\` using only doors and rooms at least
/// \`t\` mm tall (an unknown height never blocks), or \`None\` when there is none.
pub(super) fn route_at_height(
    nodes: &[Node],
    edges: &[Edge],
    from: usize,
    to: usize,
    t: f64,
) -> Option<(Vec<usize>, Vec<usize>)> {
    let ok = |h: Option<f64>| h.is_none_or(|h| h >= t);
    if !ok(nodes[from].height_mm) {
        return None;
    }
    let mut via: Vec<Option<usize>> = vec![None; nodes.len()];
    let mut seen = vec![false; nodes.len()];
    let mut queue = std::collections::VecDeque::from([from]);
    seen[from] = true;
    while let Some(n) = queue.pop_front() {
        if n == to {
            break;
        }
        for (k, e) in edges.iter().enumerate() {
            let next = if e.ia == n {
                e.ib
            } else if e.ib == n {
                e.ia
            } else {
                continue;
            };
            if !seen[next] && ok(e.height_mm) && ok(nodes[next].height_mm) {
                seen[next] = true;
                via[next] = Some(k);
                queue.push_back(next);
            }
        }
    }
    if !seen[to] {
        return None;
    }
    let (mut ns, mut es, mut at) = (vec![to], Vec::new(), to);
    while at != from {
        let k = via[at].expect("a reached node has a predecessor");
        es.push(k);
        at = if edges[k].ia == at { edges[k].ib } else { edges[k].ia };
        ns.push(at);
    }
    Some((ns, es))
}

/// The tallest object that can get from \`from\` to \`to\`, and the door or room that limits
/// it. Tries each height that exists in the building from the top down; the first at which
/// the rooms are joined is the answer, and the lowest checked element of that route is the
/// limit. \`None\` when no height limits it (nothing on the way has a known height) or when
/// the rooms are not joined at all.
pub(super) fn tallest_between(nodes: &[Node], edges: &[Edge], from: usize, to: usize) -> Option<HeightLimit> {
    if from == to {
        return None;
    }
    let mut heights: Vec<f64> = nodes
        .iter()
        .filter_map(|n| n.height_mm)
        .chain(edges.iter().filter_map(|e| e.height_mm))
        .collect();
    heights.sort_by(|a, b| b.total_cmp(a));
    heights.dedup();
    for t in heights {
        let Some((ns, es)) = route_at_height(nodes, edges, from, to, t) else {
            continue;
        };
        let door = es
            .iter()
            .filter(|&&k| carries_height(edges[k].kind) && edges[k].height_mm.is_some())
            .min_by(|&&a, &&b| {
                edges[a].height_mm.unwrap_or(f64::MAX).total_cmp(&edges[b].height_mm.unwrap_or(f64::MAX))
            });
        let room = ns.iter().filter(|&&i| nodes[i].height_mm.is_some()).min_by(|&&a, &&b| {
            nodes[a].height_mm.unwrap_or(f64::MAX).total_cmp(&nodes[b].height_mm.unwrap_or(f64::MAX))
        });
        return match (door, room) {
            (Some(&d), Some(&r)) if edges[d].height_mm <= nodes[r].height_mm => Some(door_height_limit(&edges[d])),
            (_, Some(&r)) => Some(room_height_limit(&nodes[r])),
            (Some(&d), None) => Some(door_height_limit(&edges[d])),
            (None, None) => None,
        };
    }
    None
}
