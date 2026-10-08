//! The shortest walk between two points INSIDE one room.
//!
//! Transport-agnostic and pure: a room polygon in, lengths and points out. It is
//! the geometry under `service::routing`'s door-to-door method, and it exists
//! because walking from a room's centre to its door and on to the next room's
//! centre (the original method) is not a shortest path: it walks to the middle of
//! rooms nobody needs to enter, and bends at every centre.
//!
//! **The method is the visibility graph**, the classic exact answer to "shortest
//! path among polygonal obstacles" (Lozano-Perez and Wesley 1979; de Berg et al.,
//! *Computational Geometry*, ch. 15), and the one the door-to-door indoor
//! routing papers build on (Liu and Zlatanova 2011; Xu, Wei and Zlatanova 2016).
//! A shortest path in a polygon with holes is a polyline whose bends are only at
//! **reflex vertices**: corners where the free space turns more than 180 degrees,
//! which for the outer ring are the concave corners and for a hole (a column, a
//! shaft) are its convex corners. So the graph is those vertices, joined where the
//! straight line between them stays inside the room, and a query adds its two end
//! points. Straight line if the ends see each other; otherwise Dijkstra over that
//! small graph.
//!
//! **Exact for a room, not for a building.** A room's polygon is its boundary,
//! walls included, so nothing inside it is an obstacle but a hole. Furniture is
//! not modelled, and a person does not walk the geometric shortest path (they keep
//! off walls); both are stated limits, and wall clearance is the natural next
//! option (`docs/PLAN-connectivity.md`).
//!
//! **Numerics.** "The segment stays inside" is `covers` (the room's interior and
//! boundary), so a path that grazes a corner or runs along a wall is allowed, which
//! is what a shortest path does. A segment is never tested against a tolerance
//! band, so a query whose ends sit a hair outside the polygon is first `snap`ped
//! onto it; when even so no walk is found (a degenerate sliver) the caller falls
//! back to the straight line and says so.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use geo::{
    algorithm::orient::Direction, Closest, ClosestPoint, Coord, Distance, Euclidean, InteriorPoint, LineString, Orient,
    Point, Polygon,
};

use crate::contract::{Point2D, Room};

/// A corner must turn at least this much (cross product of the two edges, ft^2) to
/// count as reflex: a vertex on a straight wall never bends a shortest path, and
/// keeping those out keeps the graph small.
const REFLEX_EPS: f64 = 1e-9;

/// More reflex vertices than this and the visibility graph is not built: it is
/// quadratic in them, and a room this irregular is better walked in a straight
/// line (reported) than allowed to dominate a read. Far above any real room.
const MAX_REFLEX: usize = 160;

/// How far from the outline a point may be and still count as in the room, in feet:
/// rounding, not geometry (a micron).
const ON_OUTLINE: f64 = 1e-6;

/// A walk and where it bends.
#[derive(Debug, Clone)]
pub struct Walk {
    pub length: f64,
    /// Both ends and every bend between them, in order.
    pub points: Vec<Point2D>,
}

/// One room's polygon and the visibility graph over its reflex vertices.
pub struct RoomShape {
    poly: Polygon<f64>,
    /// Every edge of every ring, flattened once for the visibility test.
    edges: Vec<(Coord<f64>, Coord<f64>)>,
    reflex: Vec<Coord<f64>>,
    /// `links[i]` lists `(j, distance)` for each reflex vertex `j` that `i` sees.
    links: Vec<Vec<(usize, f64)>>,
    /// False when the graph was skipped (too many reflex vertices): `walk` is then
    /// the straight line, and callers can ask.
    exact: bool,
}

fn coord(p: Point2D) -> Coord<f64> {
    Coord { x: p.x, y: p.y }
}

fn point2d(c: Coord<f64>) -> Point2D {
    Point2D { x: c.x, y: c.y }
}

fn dist(a: Coord<f64>, b: Coord<f64>) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// A ring of points as a closed `LineString`, without a repeated closing point
/// counted twice. `None` for fewer than three distinct points.
fn ring(points: &[Point2D]) -> Option<LineString<f64>> {
    let mut coords: Vec<Coord<f64>> = points.iter().map(|p| coord(*p)).collect();
    if coords.len() >= 2 && coords.first() == coords.last() {
        coords.pop();
    }
    (coords.len() >= 3).then(|| LineString::from(coords))
}

impl RoomShape {
    /// The room's polygon: `loops[0]` the outer ring and every other loop a hole.
    /// `None` for a room with no usable outline (an unplaced room).
    pub fn new(room: &Room) -> Option<Self> {
        let outer = ring(&room.loops.first()?.points)?;
        let holes: Vec<LineString<f64>> = room.loops.iter().skip(1).filter_map(|l| ring(&l.points)).collect();
        // Outer ring counter-clockwise and holes clockwise, so the free space is on
        // the LEFT of every edge and "reflex" is the same test on every ring.
        let poly = Polygon::new(outer, holes).orient(Direction::Default);

        let mut reflex = Vec::new();
        for r in std::iter::once(poly.exterior()).chain(poly.interiors()) {
            let cs: Vec<Coord<f64>> = r.coords().copied().collect();
            let n = cs.len().saturating_sub(1); // the ring repeats its first point last
            for i in 0..n {
                let (prev, cur, next) = (cs[(i + n - 1) % n], cs[i], cs[(i + 1) % n]);
                let cross = (cur.x - prev.x) * (next.y - cur.y) - (cur.y - prev.y) * (next.x - cur.x);
                if cross < -REFLEX_EPS {
                    reflex.push(cur);
                }
            }
        }

        let edges: Vec<(Coord<f64>, Coord<f64>)> = std::iter::once(poly.exterior())
            .chain(poly.interiors())
            .flat_map(|r| r.lines().map(|l| (l.start, l.end)).collect::<Vec<_>>())
            .collect();
        let mut shape = RoomShape { poly, edges, reflex, links: Vec::new(), exact: true };
        if shape.reflex.len() > MAX_REFLEX {
            shape.exact = false;
            shape.reflex.clear();
        }
        shape.links = vec![Vec::new(); shape.reflex.len()];
        for i in 0..shape.reflex.len() {
            for j in (i + 1)..shape.reflex.len() {
                if shape.sees(shape.reflex[i], shape.reflex[j]) {
                    let d = dist(shape.reflex[i], shape.reflex[j]);
                    shape.links[i].push((j, d));
                    shape.links[j].push((i, d));
                }
            }
        }
        Some(shape)
    }

    /// Whether the straight segment between two points stays within the room,
    /// boundary included.
    ///
    /// **Not a single "covers" predicate, and that is deliberate.** A door's point is
    /// snapped onto the outline and so sits on it to within rounding, and an exact
    /// covers test then says "outside" for a segment that plainly runs along or into
    /// the room, which turned most real routes into straight-line fallbacks. Instead
    /// the segment is cut where it meets any edge (touching, crossing or running
    /// along it), and each piece between cuts must have its middle inside the room
    /// or within `ON_OUTLINE` of it. That accepts a path that grazes a corner or
    /// follows a wall, refuses one that leaves the room between two touches, and is
    /// a plain pass over the edges, cheap enough to run for every pair of corners.
    fn sees(&self, a: Coord<f64>, b: Coord<f64>) -> bool {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let len2 = dx * dx + dy * dy;
        if len2 < 1e-18 {
            return true;
        }
        let len = len2.sqrt();
        let mut cuts = vec![0.0_f64, 1.0];
        for (p, q) in &self.edges {
            let (sx, sy) = (q.x - p.x, q.y - p.y);
            let (qpx, qpy) = (p.x - a.x, p.y - a.y);
            let denom = dx * sy - dy * sx;
            if denom.abs() > 1e-12 * len * (sx.hypot(sy)).max(1e-12) {
                let t = (qpx * sy - qpy * sx) / denom;
                let u = (qpx * dy - qpy * dx) / denom;
                if (-1e-9..=1.0 + 1e-9).contains(&t) && (-1e-9..=1.0 + 1e-9).contains(&u) {
                    cuts.push(t.clamp(0.0, 1.0));
                }
            } else if (qpx * dy - qpy * dx).abs() / len <= ON_OUTLINE {
                // Parallel and on the same line: the edge runs along the segment.
                for end in [(qpx, qpy), (q.x - a.x, q.y - a.y)] {
                    cuts.push(((end.0 * dx + end.1 * dy) / len2).clamp(0.0, 1.0));
                }
            }
        }
        cuts.sort_by(f64::total_cmp);
        cuts.windows(2).all(|w| {
            if (w[1] - w[0]) * len < 1e-7 {
                return true;
            }
            let m = (w[0] + w[1]) / 2.0;
            Euclidean.distance(&Point::new(a.x + dx * m, a.y + dy * m), &self.poly) <= ON_OUTLINE
        })
    }

    /// Whether a point is in the room, outline included (to rounding).
    pub fn contains(&self, p: Point2D) -> bool {
        Euclidean.distance(&Point::new(p.x, p.y), &self.poly) <= ON_OUTLINE
    }

    /// A point guaranteed to lie inside the room. A room's centroid can fall
    /// outside an L or a C, which is why a route cannot always start from it.
    pub fn inside_point(&self) -> Option<Point2D> {
        self.poly.interior_point().map(|p| Point2D { x: p.x(), y: p.y() })
    }

    /// Whether the visibility graph was built, rather than skipped for a room with
    /// too many reflex corners.
    pub fn is_exact(&self) -> bool {
        self.exact
    }

    /// The point itself when it is in the room, else the nearest point on the
    /// room's outline. A door's point sits in the WALL, which a finish-face room
    /// stops short of, so it is brought to the room before a walk starts from it.
    pub fn snap(&self, p: Point2D) -> Point2D {
        let pt = Point::new(p.x, p.y);
        match self.poly.closest_point(&pt) {
            Closest::Intersection(q) | Closest::SinglePoint(q) => Point2D { x: q.x(), y: q.y() },
            Closest::Indeterminate => p,
        }
    }

    /// The shortest walk from `a` to `b` inside the room. `None` when no walk
    /// exists, which for a sound polygon means an end lies outside it (`snap`
    /// first).
    pub fn walk(&self, a: Point2D, b: Point2D) -> Option<Walk> {
        let (ca, cb) = (coord(a), coord(b));
        if self.sees(ca, cb) {
            return Some(Walk { length: dist(ca, cb), points: vec![a, b] });
        }
        if !self.exact {
            return None;
        }
        // Dijkstra over the reflex graph plus the two ends.
        let n = self.reflex.len();
        let (src, dst) = (n, n + 1);
        let from_a: Vec<(usize, f64)> = (0..n)
            .filter(|&i| self.sees(ca, self.reflex[i]))
            .map(|i| (i, dist(ca, self.reflex[i])))
            .collect();
        let to_b: std::collections::BTreeMap<usize, f64> = (0..n)
            .filter(|&i| self.sees(cb, self.reflex[i]))
            .map(|i| (i, dist(cb, self.reflex[i])))
            .collect();

        let mut best = vec![f64::INFINITY; n + 2];
        let mut via: Vec<Option<usize>> = vec![None; n + 2];
        let mut heap = BinaryHeap::new();
        best[src] = 0.0;
        heap.push(Frontier { cost: 0.0, node: src });
        while let Some(Frontier { cost, node }) = heap.pop() {
            if node == dst {
                break;
            }
            if cost > best[node] {
                continue;
            }
            let mut relax = |next: usize, w: f64, best: &mut Vec<f64>, via: &mut Vec<Option<usize>>| {
                if cost + w < best[next] {
                    best[next] = cost + w;
                    via[next] = Some(node);
                    heap.push(Frontier { cost: cost + w, node: next });
                }
            };
            if node == src {
                for &(i, w) in &from_a {
                    relax(i, w, &mut best, &mut via);
                }
            } else {
                for &(j, w) in &self.links[node] {
                    relax(j, w, &mut best, &mut via);
                }
                if let Some(&w) = to_b.get(&node) {
                    relax(dst, w, &mut best, &mut via);
                }
            }
        }
        if !best[dst].is_finite() {
            return None;
        }
        let mut chain = vec![dst];
        while let Some(prev) = via[*chain.last().unwrap()] {
            chain.push(prev);
        }
        chain.reverse();
        let points = chain
            .into_iter()
            .map(|k| match k {
                k if k == src => a,
                k if k == dst => b,
                k => point2d(self.reflex[k]),
            })
            .collect();
        Some(Walk { length: best[dst], points })
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Loop;
    use std::collections::BTreeMap;

    fn p(x: f64, y: f64) -> Point2D {
        Point2D { x, y }
    }

    fn room(loops: Vec<Vec<(f64, f64)>>) -> Room {
        Room {
            enclosure: None,
            id: "r".to_string(),
            name: "r".to_string(),
            level_id: "l".to_string(),
            loops: loops
                .into_iter()
                .map(|l| Loop { points: l.into_iter().map(|(x, y)| p(x, y)).collect() })
                .collect(),
            properties: BTreeMap::new(),
        }
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(f64, f64)> {
        vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    }

    /// An L: a 10 x 4 arm along the bottom and a 4 x 10 arm up the left side.
    fn ell() -> Room {
        room(vec![vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 4.0),
            (4.0, 4.0),
            (4.0, 10.0),
            (0.0, 10.0),
        ]])
    }

    #[test]
    fn test_in_a_convex_room_the_walk_is_the_straight_line() {
        let shape = RoomShape::new(&room(vec![rect(0.0, 0.0, 10.0, 6.0)])).unwrap();
        let w = shape.walk(p(1.0, 1.0), p(9.0, 5.0)).unwrap();
        assert!((w.length - (8.0f64.hypot(4.0))).abs() < 1e-9);
        assert_eq!(w.points.len(), 2);
    }

    /// Around the inside corner of an L, the walk bends exactly at the corner, and
    /// is longer than the straight line that would cross the wall.
    #[test]
    fn test_around_an_l_the_walk_bends_at_the_corner() {
        let shape = RoomShape::new(&ell()).unwrap();
        let w = shape.walk(p(9.0, 1.0), p(1.0, 9.0)).unwrap();
        // (9,1) -> corner (4,4) -> (1,9)
        let expected = 5.0f64.hypot(3.0) + 3.0f64.hypot(5.0);
        assert!((w.length - expected).abs() < 1e-9, "{} vs {expected}", w.length);
        assert_eq!(w.points.len(), 3);
        assert_eq!((w.points[1].x, w.points[1].y), (4.0, 4.0));
        assert!(w.length > 8.0f64.hypot(8.0), "longer than the line through the wall");
    }

    /// A column in the middle of a room is a hole: the walk goes round its corner.
    #[test]
    fn test_a_column_is_walked_around() {
        let shape = RoomShape::new(&room(vec![rect(0.0, 0.0, 10.0, 4.0), rect(4.0, 1.0, 6.0, 3.0)])).unwrap();
        let w = shape.walk(p(0.5, 2.0), p(9.5, 2.0)).unwrap();
        // (0.5,2) -> (4,3) -> (6,3) -> (9.5,2), or the mirror below.
        let expected = 2.0 * 3.5f64.hypot(1.0) + 2.0;
        assert!((w.length - expected).abs() < 1e-9, "{} vs {expected}", w.length);
        assert_eq!(w.points.len(), 4);
    }

    #[test]
    fn test_a_walk_is_the_same_length_either_way_and_never_shorter_than_the_line() {
        let shape = RoomShape::new(&ell()).unwrap();
        let (a, b) = (p(9.0, 1.0), p(1.0, 9.0));
        let (ab, ba) = (shape.walk(a, b).unwrap(), shape.walk(b, a).unwrap());
        assert!((ab.length - ba.length).abs() < 1e-9);
        assert!(ab.length >= (8.0f64).hypot(8.0));
    }

    /// A door's point sits in the wall, outside a finish-face room. Snapping brings
    /// it to the outline, leaves a point already inside alone, and makes the walk
    /// from it possible.
    #[test]
    fn test_a_point_in_the_wall_is_brought_to_the_room() {
        let shape = RoomShape::new(&room(vec![rect(0.0, 0.0, 10.0, 6.0)])).unwrap();
        let outside = shape.snap(p(10.5, 3.0));
        assert!((outside.x - 10.0).abs() < 1e-9 && (outside.y - 3.0).abs() < 1e-9);
        let inside = shape.snap(p(3.0, 3.0));
        assert_eq!((inside.x, inside.y), (3.0, 3.0));
        assert!(shape.walk(outside, p(1.0, 3.0)).is_some());
    }

    /// The centroid of an L-shaped room can lie outside it; the interior point
    /// cannot.
    #[test]
    fn test_the_interior_point_is_inside_even_where_the_centroid_is_not() {
        let c_shape = room(vec![vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 2.0),
            (2.0, 2.0),
            (2.0, 8.0),
            (10.0, 8.0),
            (10.0, 10.0),
            (0.0, 10.0),
        ]]);
        let shape = RoomShape::new(&c_shape).unwrap();
        let q = shape.inside_point().unwrap();
        assert!(shape.sees(coord(q), coord(q)));
        let again = shape.snap(q);
        assert!((again.x - q.x).abs() < 1e-9 && (again.y - q.y).abs() < 1e-9, "already inside, so unmoved");
        // The mean of the outline is at (~4.4, 5): outside the spine at x in 0..2 only
        // by luck of this shape; the property is that the interior point is inside.
        assert!(shape.walk(q, p(1.0, 1.0)).is_some());
    }

    #[test]
    fn test_contains_says_whether_a_point_is_in_the_room() {
        let shape = RoomShape::new(&ell()).unwrap();
        assert!(shape.contains(p(2.0, 8.0)), "in the up-arm");
        assert!(shape.contains(p(10.0, 2.0)), "on the outline counts");
        assert!(!shape.contains(p(8.0, 8.0)), "in the notch the L leaves out");
    }

    #[test]
    fn test_a_room_without_an_outline_has_no_shape() {
        assert!(RoomShape::new(&room(vec![])).is_none());
        assert!(RoomShape::new(&room(vec![vec![(0.0, 0.0), (1.0, 1.0)]])).is_none());
    }

    /// A closing point repeated at the end of a loop is accepted either way, as in
    /// `service::adjacency`.
    #[test]
    fn test_a_repeated_closing_point_is_accepted() {
        let mut open = rect(0.0, 0.0, 4.0, 4.0);
        open.push(open[0]);
        let shape = RoomShape::new(&room(vec![open])).unwrap();
        assert!(shape.walk(p(1.0, 1.0), p(3.0, 3.0)).is_some());
    }
}
