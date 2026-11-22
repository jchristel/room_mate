//! The distance field: the router's own port network walked from several sources at once (`field`), so a cell's distance and a route's
//! length agree. `reach` reads it; nothing here is a second graph.

use super::*;

/// Where a distance field starts: a room, and a point in it (the room's inside
/// point when absent).
pub(in crate::service) struct Source {
    pub node: usize,
    pub at: Option<Point2D>,
}

/// The cheapest walk, in feet, from the nearest source to every port within a limit.
///
/// **The same network a route is searched over, from several starts at once.** Every
/// edge becomes two ports joined by its crossing and every pair of a room's ports by
/// the exact walk inside it (`crossing_ports`, `Shapes::walk`), so a distance here and
/// a route's length agree. The search is plain Dijkstra, not A*: there is no single goal
/// to aim at. It stops expanding at `limit_ft`, so a room the limit cannot reach costs
/// nothing but its ports.
///
/// **Level changes are not crossed.** A walk measured on one plate does not continue by lift, and a field is about
/// one floor plate; a vertical link is skipped, which is a statement about the question
/// and not a limit of the data.
pub(in crate::service) struct Field<'a> {
    pub(super) shapes: Shapes<'a>,
    /// Per node: where its reached ports are and the cheapest walk to each, nearest
    /// first. A source's own port is among them at 0.
    pub ports: Vec<Vec<(Point2D, f64)>>,
    /// Where each source really starts: its point moved into the room.
    pub starts: Vec<Point2D>,
    /// Walks between two ports that fell back to a straight line (a room with no
    /// usable outline), so the answer can say its distances are not all exact.
    pub fallbacks: usize,
}

impl Field<'_> {
    /// The walk, in feet, between two points of one room.
    pub fn walk_length(&mut self, node: usize, a: Point2D, b: Point2D) -> f64 {
        let (len, _, exact) = self.shapes.walk(node, a, b);
        if !exact {
            self.fallbacks += 1;
        }
        len
    }

    /// Whether a point is in the room, outline included.
    pub fn contains(&mut self, node: usize, p: Point2D) -> bool {
        self.shapes.get(node).is_some_and(|s| s.contains(p))
    }

    /// A point surely inside the room, for a room too small to hold a grid point.
    pub fn inside_point(&mut self, node: usize) -> Option<Point2D> {
        self.shapes.get(node).and_then(|s| s.inside_point())
    }
}

/// Run the search. `rooms[i]` is node `i`'s outline, as for `route`.
pub(in crate::service) fn field<'a>(
    nodes: &[Node],
    edges: &[Edge],
    rooms: &'a [Option<&'a Room>],
    sources: &[Source],
    limit_ft: f64,
) -> Field<'a> {
    let mut shapes = Shapes::new(rooms);
    let mut ports: Vec<Port> = Vec::new();
    let mut starts = Vec::new();
    for s in sources {
        let at = match s.at {
            Some(p) => shapes.snap(s.node, p),
            None => shapes.inside(s.node, nodes[s.node].centroid),
        };
        starts.push(at);
        ports.push(Port { room: s.node, at, twin: None });
    }
    let source_ports = ports.len();
    for (k, e) in edges.iter().enumerate() {
        if e.kind == EdgeKind::Vertical {
            continue;
        }
        let (at_a, at_b, crossing) = crossing_ports(nodes, &mut shapes, e);
        let first = ports.len();
        ports.push(Port { room: e.ia, at: at_a, twin: Some((first + 1, crossing, k)) });
        ports.push(Port { room: e.ib, at: at_b, twin: Some((first, crossing, k)) });
    }
    let mut by_room: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (i, p) in ports.iter().enumerate() {
        by_room[p.room].push(i);
    }

    let mut walks: HashMap<(usize, usize), f64> = HashMap::new();
    let mut fallbacks = 0usize;
    let mut best = vec![f64::INFINITY; ports.len()];
    let mut heap = BinaryHeap::new();
    for (i, b) in best.iter_mut().enumerate().take(source_ports) {
        *b = 0.0;
        heap.push(Frontier { estimate: 0.0, cost: 0.0, port: i });
    }
    while let Some(Frontier { cost, port: x, .. }) = heap.pop() {
        if cost > best[x] {
            continue;
        }
        let room = ports[x].room;
        for &y in &by_room[room] {
            if y == x {
                continue;
            }
            let key = (x.min(y), x.max(y));
            let len = *walks.entry(key).or_insert_with(|| {
                let (len, _, exact) = shapes.walk(room, ports[key.0].at, ports[key.1].at);
                if !exact {
                    fallbacks += 1;
                }
                len
            });
            let c = cost + len;
            if c <= limit_ft && c < best[y] {
                best[y] = c;
                heap.push(Frontier { estimate: c, cost: c, port: y });
            }
        }
        if let Some((twin, crossing, _)) = ports[x].twin {
            let c = cost + crossing;
            if c <= limit_ft && c < best[twin] {
                best[twin] = c;
                heap.push(Frontier { estimate: c, cost: c, port: twin });
            }
        }
    }

    let mut reached: Vec<Vec<(Point2D, f64)>> = vec![Vec::new(); nodes.len()];
    for (i, p) in ports.iter().enumerate() {
        if best[i].is_finite() {
            reached[p.room].push((p.at, best[i]));
        }
    }
    for r in &mut reached {
        r.sort_by(|a, b| a.1.total_cmp(&b.1));
    }
    Field { shapes, ports: reached, starts, fallbacks }
}
