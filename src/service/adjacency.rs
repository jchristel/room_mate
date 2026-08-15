//! Room-to-room adjacency graph — the **second** geometry-processing service,
//! after `areas`, and the point at which the Rust server's performance
//! advantage stops being potential (STRATEGY.md: "it becomes real only when
//! actual heavy geometry … is pushed server-side").
//!
//! Transport-agnostic like every `service` module: it imports `geo` and
//! `crate::contract`, never `axum`/`rmcp`. The HTTP handler and the MCP tool are
//! both thin adapters over `assemble_adjacency`.
//!
//! **What "adjacent" means here:** two rooms share a wall. Their outer
//! boundaries run parallel, sit within a wall-thickness gap of each other, and
//! overlap along that wall by more than a trivial length. Same level only.
//!
//! **This is not door connectivity, and doors now exist.** This module used to
//! say the extractor collected no doors; it does, and `/doors` serves them with
//! `from_room`/`to_room` on every one. That does not make this graph a
//! connectivity graph, and the distinction is now load-bearing rather than
//! hypothetical: two rooms can share a wall with no door in it, and a door can
//! connect two rooms sharing almost no wall. **It is a second edge set over the
//! same rooms, not a refinement of this one**
//! ([Entities](../../docs/STRATEGY-ENTITIES.md), "Deferred"), so this endpoint
//! keeps its meaning unchanged and connectivity gets its own when it is built.
//! Anyone wanting "which rooms are connected by a door" today can read it off
//! `/doors` directly — every door names both of its rooms.
//!
//! **The load-bearing subtlety: two boundary regimes.** Revit's
//! `SpatialElementBoundaryLocation` decides where a room's boundary sits, and
//! both settings occur in real models:
//!
//! - **centreline** — neighbouring rooms tile edge-to-edge; the shared
//!   boundaries are *coincident* and the gap is `0` up to export noise;
//! - **finish face** — rooms float inside their walls; the gap is roughly the
//!   wall thickness.
//!
//! The algorithm handles both, and `wall_max` is what spans them: at or near
//! `0` for a centreline model, just over the thickest partition for a
//! finish-face one. That is the real reason it is a per-request parameter
//! rather than a constant — the right value is a property of the model, not of
//! the code. Hence the acceptance band is **closed at zero**: `0` is a valid
//! tolerance, not a degenerate one.
//!
//! **The regime is no longer a guess.** An earlier version of this header said
//! nothing on the wire declared it; `contract::RoomBoundary` now rides the
//! upload envelope, so an *unrequested* `wall_max` resolves from the project's
//! `[areas] max_wall_thickness` through the declared regime — zero when every
//! level in scope is centreline (see `default_wall_max`). What stayed a
//! request parameter is the *question a caller asks*, which is a different
//! thing from the fact a model states.
//!
//! **On length** (CODING-CONVENTIONS "Module structure & length"): this file
//! runs past the ~500 non-test-line split trigger, but its real code is in the
//! same ballpark as `areas.rs` — the sibling geometry module, which is not
//! split. The excess over the raw count is rationale, not substance: the
//! tolerances, the two boundary regimes, and now the spatial index are the
//! things a future reader cannot recover from the code, so they are written
//! down. Splitting would scatter one algorithm across files to satisfy a line
//! count. If it does grow, the grid (`SpatialGrid`) is the clean seam — it is
//! the natural first `adjacency/` sibling.

use std::collections::{BTreeMap, HashMap, HashSet};

use geo::{Area, Centroid, Contains, Coord, LineString, Point, Polygon};
use serde::Serialize;

use crate::classify::TierValue;
use crate::contract::{Level, Point2D, Room, RoomBoundary, SUPPORTED_SCHEMA};
use crate::reference::ReferenceRecord;
use crate::settings::AreaPolicy;
use crate::state::AppState;

use super::rooms::{assemble_rooms, RoomScope, RoomsResult};
use super::ServiceError;

/// Hard ceiling on a requested `wall_max`, re-exported from the one place the
/// quantity is now declared. Beyond ~5 ft the query stops being a
/// wall-thickness question and starts bridging whole rooms, so it is rejected
/// loudly rather than clamped silently — a request that would wire the entire
/// building together is a mistake worth surfacing.
///
/// This module used to carry its own `WALL_MAX_FT` default alongside
/// `areas::MAX_WALL_FT` — the **same physical quantity in two constants**,
/// which is a drift risk, not a duplication of convenience. Both are gone: the
/// number is declared once per project as `[areas] max_wall_thickness`
/// (`settings::AreaPolicy`) and read here as the request default. The remaining
/// constant is the *range guard*, which is about what a caller may ask for
/// rather than about any one project's walls.
pub const WALL_MAX_LIMIT_FT: f64 = AreaPolicy::MAX_WALL_THICKNESS_LIMIT_FT;

/// Float-noise allowance added to `wall_max` when testing the gap. Its whole job
/// is to make a slider value of `0` still match a *centreline* model, whose
/// "coincident" edges disagree at the 1e-9 level because Revit exports are not
/// bit-identical across rooms (`areas` already banked exactly this — see its
/// `test_noise_level_gap_still_dissolves`). Tight on purpose, in the spirit of
/// `areas::COLLINEAR_EPS_FT`: it absorbs export noise and admits nothing a human
/// would call a gap.
const COINCIDENT_EPS_FT: f64 = 1e-6;

/// Angular tolerance for "parallel", in radians (~0.57°). Wide enough for float
/// noise and the slight non-orthogonality of a real model, tight enough that a
/// genuinely skewed wall is not matched to an orthogonal one. Compared against
/// `|u × v|` (i.e. `|sin θ|`), so antiparallel segments — the normal case, since
/// two rooms trace their shared wall in opposite winding directions — pass too.
const PARALLEL_EPS_RAD: f64 = 0.01;

/// Minimum **accumulated** overlap for a pair to count as a real relationship.
/// Suppresses the corner-touch artefact, where two rooms meet at a point (or
/// along a few inches of a door reveal) and are not meaningfully adjacent. This
/// gates the total, not each segment: an L-shaped junction legitimately reaches
/// it by summing two disjoint runs.
const MIN_SHARED_FT: f64 = 1.0;

// ================================ wire shape ================================

/// Wire result of `GET /projects/{id}/adjacency`. Node/edge, so the client does
/// no geometry at all — the browser draws a graph, it does not derive one.
#[derive(Serialize)]
pub struct AdjacencyResult {
    pub schema_version: u32,
    /// Content revision over *both* the scoped snapshot set and the effective
    /// `wall_max` (see [`adjacency_revision`]). Two responses at different
    /// tolerances are different content; a revision that ignored the tolerance
    /// would let the viewer skip its re-render after a slider change and look
    /// frozen.
    pub revision: String,
    /// The tolerance actually applied, echoed so a client that sent none can
    /// show the server's default on its slider without hardcoding the constant.
    pub wall_max: f64,
    /// The scoped level set (same shape `/rooms` and `/areas` return).
    pub levels: Vec<Level>,
    pub nodes: Vec<AdjacencyNode>,
    pub edges: Vec<AdjacencyEdge>,
}

/// One room as a graph node. `classification` and the joined reference records
/// are carried **through** from the room assembly rather than re-fetched: the
/// graph colours by department, and making the client cross-reference a second
/// request to do that would be a worse version of what `/rooms` already solved.
#[derive(Serialize)]
pub struct AdjacencyNode {
    pub room_id: String,
    pub name: String,
    pub level_id: String,
    /// Area centroid of the outer loop — the layout's seed position, and what
    /// lets the graph re-centre on a room without a second geometry fetch. A
    /// room too degenerate for a centroid (an unplaced room) falls back to the
    /// mean of whatever points it has, and to the origin when it has none.
    pub centroid: Point2D,
    pub classification: Vec<TierValue>,
    /// Every joined reference record for this room, keyed by source name and
    /// `#[serde(flatten)]`ed exactly as `RoomResponse.reference` is — so a
    /// node and a room carry their sources in the same wire shape, and one
    /// client-side accessor reads both. A source with no record for this room
    /// is simply absent (an unmatched key is a signal, not an error), and an
    /// empty map flattens to no keys at all.
    #[serde(flatten)]
    pub reference: BTreeMap<String, ReferenceRecord>,
}

/// One undirected adjacency. Emitted once per pair with `a < b` by room id, so
/// the payload is deterministic and the revision means something.
///
/// `level_id` sits on the **edge**, not only on the nodes, even though every
/// edge is same-level today: a future cross-level edge (a riser, a stair core)
/// then differs only in its field values, not in its type. Same reasoning as
/// leaving room for a `kind`/`connection` discriminator when door connectivity
/// arrives — an edge is a pair plus metadata, so both are additive.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct AdjacencyEdge {
    pub a: String,
    pub b: String,
    pub level_id: String,
    /// Accumulated shared-wall length in model units (feet). Summed across
    /// disjoint runs, so an L-shaped junction reports both of its walls.
    pub shared_length: f64,
}

// ============================== the algorithm ==============================

/// One directed edge segment of a room's outer loop, in model coordinates.
#[derive(Clone, Copy)]
struct Seg {
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
}

/// A room prepared for pairing: its outer polygon (for the occlusion test), its
/// bounding box (to reject candidates cheaply), and its boundary segments.
struct Prepared<'a> {
    room: &'a Room,
    poly: Option<Polygon<f64>>,
    bbox: (f64, f64, f64, f64), // minx, miny, maxx, maxy
    segs: Vec<Seg>,
}

/// The accepted overlap of one segment pair, expressed in the *reference*
/// segment's frame: `[lo, hi]` along its axis, and the perpendicular gap.
struct Overlap {
    lo: f64,
    hi: f64,
    gap: f64,
}

/// A distinct shared wall between one pair of rooms: a direction, a point on it,
/// and the 1-D intervals along it that the two rooms were found to share.
///
/// This grouping is why the shared length is not simply a sum over segment
/// pairs. A Revit room boundary is split at every bounding element, so one
/// physical wall with a door in it arrives as *several* collinear segments on
/// each side; naively adding every accepted pair's overlap would count the same
/// stretch of wall two or three times. Intervals are merged within a run, and
/// runs are kept apart when they are genuinely different walls — which is also
/// exactly what makes an L-shaped junction sum correctly instead of collapsing.
struct WallRun {
    ux: f64,
    uy: f64,
    ox: f64,
    oy: f64,
    intervals: Vec<(f64, f64)>,
}

/// Explode a room's **outer** loop into segments. `loops[0]` only: interior
/// loops are the room's own holes (a column, a shaft) and cannot bound a
/// neighbour — the same construction `areas::room_outer_polygon` uses, for the
/// same reason. A loop that repeats its first point last is accepted either way;
/// the contract does not promise one form.
fn outer_segments(room: &Room) -> Vec<Seg> {
    let Some(outer) = room.loops.first() else {
        return Vec::new();
    };
    let pts = &outer.points;
    let n = if pts.len() >= 2 && pts[0].x == pts[pts.len() - 1].x && pts[0].y == pts[pts.len() - 1].y {
        pts.len() - 1
    } else {
        pts.len()
    };
    if n < 3 {
        return Vec::new(); // unplaced room: a node with no edges, not an error
    }
    (0..n)
        .map(|i| {
            let a = &pts[i];
            let b = &pts[(i + 1) % n];
            Seg { ax: a.x, ay: a.y, bx: b.x, by: b.y }
        })
        .collect()
}

fn outer_polygon(room: &Room) -> Option<Polygon<f64>> {
    let outer = room.loops.first()?;
    if outer.points.len() < 3 {
        return None;
    }
    let ring: Vec<Coord<f64>> = outer.points.iter().map(|p| Coord { x: p.x, y: p.y }).collect();
    Some(Polygon::new(LineString::from(ring), vec![]))
}

fn prepare(room: &Room) -> Prepared<'_> {
    let segs = outer_segments(room);
    let mut bbox = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for s in &segs {
        bbox.0 = bbox.0.min(s.ax).min(s.bx);
        bbox.1 = bbox.1.min(s.ay).min(s.by);
        bbox.2 = bbox.2.max(s.ax).max(s.bx);
        bbox.3 = bbox.3.max(s.ay).max(s.by);
    }
    Prepared { room, poly: outer_polygon(room), bbox, segs }
}

/// Unit direction and length of a segment; `None` for a zero-length one.
fn unit(s: &Seg) -> Option<(f64, f64, f64)> {
    let (dx, dy) = (s.bx - s.ax, s.by - s.ay);
    let len = dx.hypot(dy);
    if len <= COINCIDENT_EPS_FT {
        return None;
    }
    Some((dx / len, dy / len, len))
}

/// Test one segment pair. `s` is the reference: the result is expressed in its
/// frame, `[lo, hi]` measured along its own direction from its start point.
///
/// Three conditions, in the order that rejects fastest:
/// 1. **parallel** within `PARALLEL_EPS_RAD`;
/// 2. **overlapping** when `t` is projected onto `s`'s axis;
/// 3. **separated** by no more than `wall_max` — measured *across the overlap*,
///    not at `t`'s endpoints. That distinction matters: two segments parallel to
///    within the tolerance can still drift by `len·sin(eps)` over a long run, so
///    sampling the gap where they actually face each other is the only reading
///    that means anything.
fn overlap_of(s: &Seg, t: &Seg, wall_max: f64) -> Option<Overlap> {
    let (ux, uy, slen) = unit(s)?;
    let (vx, vy, _) = unit(t)?;

    // |u × v| == |sin θ|. Antiparallel passes too, which is the normal case:
    // two rooms trace their shared wall in opposite directions.
    if (ux * vy - uy * vx).abs() > PARALLEL_EPS_RAD.sin() {
        return None;
    }

    // `t`'s endpoints in `s`'s frame: `along` on its axis, `perp` across it.
    let along = |x: f64, y: f64| (x - s.ax) * ux + (y - s.ay) * uy;
    let perp = |x: f64, y: f64| ux * (y - s.ay) - uy * (x - s.ax);
    let (ta, tb) = (along(t.ax, t.ay), along(t.bx, t.by));
    let (pa, pb) = (perp(t.ax, t.ay), perp(t.bx, t.by));

    let lo = ta.min(tb).max(0.0);
    let hi = ta.max(tb).min(slen);
    if hi - lo <= COINCIDENT_EPS_FT {
        return None; // touching at a point, or not facing each other at all
    }

    // The gap varies linearly along the axis when the two are only near-parallel;
    // take the worst reading inside the overlap, so a pair that is close at one
    // end and far at the other is rejected rather than averaged into acceptance.
    let span = tb - ta;
    let gap_at = |x: f64| {
        if span.abs() <= COINCIDENT_EPS_FT {
            pa
        } else {
            pa + (pb - pa) * (x - ta) / span
        }
    };
    let gap = gap_at(lo).abs().max(gap_at(hi).abs());
    if gap > wall_max + COINCIDENT_EPS_FT {
        return None;
    }

    // Signed midpoint gap is kept (not the absolute one) because the occlusion
    // test needs to know which side of `s` the neighbour is on.
    Some(Overlap { lo, hi, gap: gap_at((lo + hi) / 2.0) })
}

/// A uniform grid over room bounding boxes — the spatial index the handover
/// named for when the naive O(n²) pairing bites (it does, measured: ~8–22s on a
/// 5,000-room level). Each room is registered into every cell its bbox covers,
/// which serves both hot paths: candidate-pair generation (query a room's
/// reach-expanded bbox, get only the rooms that could possibly touch it) and the
/// occlusion test (query one point's cell, not every other room on the level).
struct SpatialGrid {
    cell: f64,
    cells: HashMap<(i64, i64), Vec<usize>>,
}

impl SpatialGrid {
    /// Cell size is the mean room bbox side across the level, so a typical room
    /// spans about one cell — small enough that a cell holds few rooms, large
    /// enough that a room doesn't smear across dozens. Floored to a positive
    /// value so degenerate input can't divide by zero.
    fn cell_size(prepared: &[Prepared<'_>]) -> f64 {
        let mut sum = 0.0;
        let mut n = 0.0;
        for p in prepared {
            if p.segs.is_empty() {
                continue;
            }
            sum += (p.bbox.2 - p.bbox.0) + (p.bbox.3 - p.bbox.1);
            n += 2.0; // width + height contribute one side each
        }
        if n == 0.0 {
            return 1.0;
        }
        (sum / n).max(1e-3)
    }

    fn build(prepared: &[Prepared<'_>], cell: f64) -> Self {
        let mut cells: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, p) in prepared.iter().enumerate() {
            if p.segs.is_empty() {
                continue; // unplaced room: never a neighbour, never an occluder
            }
            for key in Self::span(cell, p.bbox) {
                cells.entry(key).or_default().push(i);
            }
        }
        Self { cell, cells }
    }

    /// Every cell coordinate a bbox overlaps.
    fn span(cell: f64, bbox: (f64, f64, f64, f64)) -> impl Iterator<Item = (i64, i64)> {
        let cx0 = (bbox.0 / cell).floor() as i64;
        let cy0 = (bbox.1 / cell).floor() as i64;
        let cx1 = (bbox.2 / cell).floor() as i64;
        let cy1 = (bbox.3 / cell).floor() as i64;
        (cx0..=cx1).flat_map(move |x| (cy0..=cy1).map(move |y| (x, y)))
    }

    /// Room indices registered in the cell containing `(x, y)`. A room whose
    /// polygon contains the point is guaranteed here: the point lies in the
    /// room's bbox, so the room registered this cell.
    fn at_point(&self, x: f64, y: f64) -> &[usize] {
        let key = ((x / self.cell).floor() as i64, (y / self.cell).floor() as i64);
        self.cells.get(&key).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// Does a third room sit *between* these two segments?
///
/// Nothing in the parallel/gap/overlap tests rules this out, and with a
/// finish-face tolerance of ~450mm any room narrower than that — a duct shaft, a
/// riser, a service void, all routinely modelled as rooms in a hospital — would
/// otherwise let its two neighbours match straight through it. That is a likelier
/// first-contact failure than bridging a corridor, and it gets worse the higher
/// the slider goes.
///
/// Skipped entirely when the gap is coincident: a zero-width gap can contain
/// nothing, which also means the whole centreline regime never pays for this.
///
/// Candidate occluders come from the grid cell at the gap midpoint, not a scan
/// of every room — the two pair indices are excluded so a room is never treated
/// as sitting between its own segments.
fn is_occluded(s: &Seg, ov: &Overlap, grid: &SpatialGrid, prepared: &[Prepared<'_>], i: usize, j: usize) -> bool {
    if ov.gap.abs() <= COINCIDENT_EPS_FT {
        return false;
    }
    let Some((ux, uy, _)) = unit(s) else { return false };
    // Midpoint of the gap: along the overlap's centre, then half-way across.
    let mid = (ov.lo + ov.hi) / 2.0;
    let px = s.ax + mid * ux + (ov.gap / 2.0) * -uy;
    let py = s.ay + mid * uy + (ov.gap / 2.0) * ux;
    let point = Point::new(px, py);
    grid.at_point(px, py).iter().any(|&k| {
        if k == i || k == j {
            return false;
        }
        let o = &prepared[k];
        // Bounding box first: rejects almost every candidate in four comparisons
        // before the point-in-polygon, which is the expensive half.
        px >= o.bbox.0
            && px <= o.bbox.2
            && py >= o.bbox.1
            && py <= o.bbox.3
            && o.poly.as_ref().is_some_and(|p| p.contains(&point))
    })
}

/// Record an accepted overlap against the right shared wall, creating a new run
/// when this is a wall the pair has not met before (a different orientation, or
/// the same orientation at a different offset — an L-junction is the first case,
/// two parallel walls of a U-shaped neighbour the second).
fn record(runs: &mut Vec<WallRun>, s: &Seg, ov: &Overlap, wall_max: f64) {
    let Some((ux, uy, _)) = unit(s) else { return };
    let (sx, sy) = (s.ax + ov.lo * ux, s.ay + ov.lo * uy);
    let (ex, ey) = (s.ax + ov.hi * ux, s.ay + ov.hi * uy);

    for run in runs.iter_mut() {
        let parallel = (run.ux * uy - run.uy * ux).abs() <= PARALLEL_EPS_RAD.sin();
        let offset = (run.ux * (sy - run.oy) - run.uy * (sx - run.ox)).abs();
        if parallel && offset <= wall_max + COINCIDENT_EPS_FT {
            let a = run.ux * (sx - run.ox) + run.uy * (sy - run.oy);
            let b = run.ux * (ex - run.ox) + run.uy * (ey - run.oy);
            run.intervals.push((a.min(b), a.max(b)));
            return;
        }
    }
    runs.push(WallRun { ux, uy, ox: sx, oy: sy, intervals: vec![(0.0, ov.hi - ov.lo)] });
}

/// Total shared length across every run, with each run's intervals merged first
/// so a wall split into several boundary segments is counted once.
fn total_length(runs: &[WallRun]) -> f64 {
    let mut total = 0.0;
    for run in runs {
        let mut iv = run.intervals.clone();
        iv.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut cur: Option<(f64, f64)> = None;
        for (lo, hi) in iv {
            match cur {
                Some((clo, chi)) if lo <= chi + COINCIDENT_EPS_FT => cur = Some((clo, chi.max(hi))),
                Some((clo, chi)) => {
                    total += chi - clo;
                    cur = Some((lo, hi));
                }
                None => cur = Some((lo, hi)),
            }
        }
        if let Some((clo, chi)) = cur {
            total += chi - clo;
        }
    }
    total
}

/// Two rooms that are the *same* room, contributed twice by linked models.
///
/// `assemble_rooms` dedups levels across linked models but concatenates rooms,
/// so an ARCH and a coordination model covering one floor deliver each room
/// twice: coincident outlines, zero gap, full-perimeter overlap — i.e. the
/// strongest possible edge between what is really one room. Suppressing the pair
/// is the honest answer; reporting it would put a maximum-weight edge at the
/// centre of every graph and let the layout make it look like a finding.
///
/// Identified by equal bounding boxes *and* equal area: two genuinely distinct
/// neighbours always differ in bbox (they sit side by side), so this cannot
/// swallow a real adjacency.
fn is_same_room(a: &Prepared<'_>, b: &Prepared<'_>) -> bool {
    let bbox_eq = (a.bbox.0 - b.bbox.0).abs() <= COINCIDENT_EPS_FT
        && (a.bbox.1 - b.bbox.1).abs() <= COINCIDENT_EPS_FT
        && (a.bbox.2 - b.bbox.2).abs() <= COINCIDENT_EPS_FT
        && (a.bbox.3 - b.bbox.3).abs() <= COINCIDENT_EPS_FT;
    if !bbox_eq {
        return false;
    }
    match (&a.poly, &b.poly) {
        (Some(pa), Some(pb)) => (pa.unsigned_area() - pb.unsigned_area()).abs() <= COINCIDENT_EPS_FT,
        _ => false,
    }
}

/// The pure algorithm: every shared-wall pair among `rooms`, at tolerance
/// `wall_max`. Rooms are partitioned by `level_id` first — adjacency is
/// single-level by decision, and pairing across floors would be both wrong and
/// quadratically expensive.
///
/// A uniform [`SpatialGrid`] over room bounding boxes turns pairing from O(n²)
/// into roughly O(n·k) for k rooms per cell — measured on the 5,000-room
/// `big-plate` level, the fetch dropped from ~22s to well under a second. The
/// grid was the handover's prescribed fix once measurement showed the naive loop
/// biting, and measurement did (STRATEGY.md "measure before optimising" — this
/// is the after). Rayon is deliberately still not reached for: the grid removed
/// the quadratic term, so threading a now-near-linear pass would trade
/// determinism for little (and a few hundred rooms can be slower threaded).
pub fn compute_adjacency(rooms: &[&Room], wall_max: f64) -> Vec<AdjacencyEdge> {
    let mut by_level: BTreeMap<&str, Vec<Prepared<'_>>> = BTreeMap::new();
    for room in rooms {
        by_level.entry(room.level_id.as_str()).or_default().push(prepare(room));
    }

    let mut edges = Vec::new();
    for (level_id, prepared) in &by_level {
        edges.extend(level_edges(level_id, prepared, wall_max));
    }
    // Deterministic order: the revision is only meaningful if the payload is.
    edges.sort_by(|x, y| (&x.level_id, &x.a, &x.b).cmp(&(&y.level_id, &y.a, &y.b)));
    edges
}

fn level_edges(level_id: &str, prepared: &[Prepared<'_>], wall_max: f64) -> Vec<AdjacencyEdge> {
    // The gap can only be spanned within `wall_max` of a bbox, so grow each box
    // by the tolerance before testing for overlap — a cheap, correct rejection.
    let reach = wall_max + COINCIDENT_EPS_FT;

    let grid = SpatialGrid::build(prepared, SpatialGrid::cell_size(prepared));

    // Candidate pairs from the grid, deduped: a large room spanning several
    // cells finds the same neighbour through more than one, so the set collapses
    // them. Each room queries its reach-expanded bbox, so any room within the
    // tolerance shares at least one queried cell — nothing within reach is missed.
    let mut pairs: HashSet<(usize, usize)> = HashSet::new();
    for (i, a) in prepared.iter().enumerate() {
        if a.segs.is_empty() {
            continue;
        }
        let query = (a.bbox.0 - reach, a.bbox.1 - reach, a.bbox.2 + reach, a.bbox.3 + reach);
        for key in SpatialGrid::span(grid.cell, query) {
            let Some(bucket) = grid.cells.get(&key) else { continue };
            for &j in bucket {
                if j != i {
                    pairs.insert((i.min(j), i.max(j)));
                }
            }
        }
    }

    let mut edges = Vec::new();
    for (i, j) in pairs {
        let (a, b) = (&prepared[i], &prepared[j]);
        // The grid over-includes (a shared cell is not a guaranteed reach), so
        // the exact bbox+reach test still runs — cheap, and it keeps the segment
        // loops off pairs that only shared a cell corner.
        if a.bbox.0 - reach > b.bbox.2 || b.bbox.0 - reach > a.bbox.2 {
            continue;
        }
        if a.bbox.1 - reach > b.bbox.3 || b.bbox.1 - reach > a.bbox.3 {
            continue;
        }
        if is_same_room(a, b) {
            continue;
        }

        let mut runs: Vec<WallRun> = Vec::new();
        for s in &a.segs {
            for t in &b.segs {
                if let Some(ov) = overlap_of(s, t, wall_max)
                    && !is_occluded(s, &ov, &grid, prepared, i, j)
                {
                    record(&mut runs, s, &ov, wall_max);
                }
            }
        }

        let shared_length = total_length(&runs);
        if shared_length >= MIN_SHARED_FT {
            // Stable `a < b` ordering by room id, so the pair has one
            // representation regardless of iteration order.
            let (ra, rb) = if a.room.id <= b.room.id {
                (&a.room.id, &b.room.id)
            } else {
                (&b.room.id, &a.room.id)
            };
            edges.push(AdjacencyEdge { a: ra.clone(), b: rb.clone(), level_id: level_id.to_string(), shared_length });
        }
    }
    edges
}

// ============================== endpoint side ==============================

/// Validate a *requested* tolerance, returning it unchanged, or fail with
/// caller-addressable text. `Ok(None)` means nothing was asked for — the caller
/// then applies its own default (see `default_wall_max`).
///
/// Lives in the service, not the adapters, so the HTTP handler and the MCP tool
/// cannot drift on what a valid tolerance is — each maps `ServiceError::Invalid`
/// to its own convention (HTTP 400, MCP `invalid_params`).
///
/// **Zero is valid.** It is the centreline setting, and the earlier draft of
/// this endpoint would have rejected it as "non-positive" — which would have
/// broken the more common of the two boundary regimes. Rejected instead:
/// non-finite, negative, and anything past [`WALL_MAX_LIMIT_FT`]. Loud over a
/// silent clamp: a tolerance that would wire the whole building together is a
/// mistake the caller should hear about.
///
/// **The guard is separate from the default on purpose**, and returns
/// `Option<f64>` rather than resolving one: `assemble_adjacency` cannot know its
/// default until the rooms are assembled, because it depends on which regime the
/// contributing models declare — and a caller-fault tolerance should cost a
/// rejection, not a multi-second room merge first. A `resolve_wall_max` wrapper
/// (this plus a passed-in default) used to sit here for the one-call form; it
/// ended up with no caller once the guard moved ahead of the merge, so it is
/// gone rather than kept warm.
///
/// Note that a *requested* zero and a *declared-centreline* zero are different
/// facts arriving at the same number: the first is a caller asking a question,
/// the second is the project stating one. Only the first travels through
/// `requested`; that is why `AreaPolicy::max_wall_thickness` may not be zero
/// while this parameter may.
pub fn check_wall_max(requested: Option<f64>) -> Result<Option<f64>, ServiceError> {
    let Some(v) = requested else { return Ok(None) };
    if !v.is_finite() {
        return Err(ServiceError::Invalid("wall_max must be a finite number".to_string()));
    }
    if v < 0.0 {
        return Err(ServiceError::Invalid(
            "wall_max must not be negative (0 is valid — it is the wall-centreline case)".to_string(),
        ));
    }
    if v > WALL_MAX_LIMIT_FT {
        return Err(ServiceError::Invalid(format!(
            "wall_max {v} ft exceeds the {WALL_MAX_LIMIT_FT} ft limit; a gap that wide bridges rooms, not walls"
        )));
    }
    Ok(Some(v))
}

/// The tolerance an *unrequested* `wall_max` resolves to: the project's
/// declared wall thickness, or zero when every level in scope is centreline.
///
/// "Every level", not "any level": one finish-face level in a mixed scope still
/// needs its walls spanned, and running that level at zero would report its
/// rooms as touching nothing at all — a graph that lies by omission, which is
/// worse than a graph that over-connects. A scope with no levels (nothing
/// matched) falls to the declared thickness; there is no evidence for the
/// narrower reading, so it is not taken.
fn default_wall_max(policy: &AreaPolicy, rooms: &RoomsResult) -> f64 {
    let all_centreline =
        !rooms.boundary_by_level.is_empty() && rooms.boundary_by_level.values().all(|b| *b == RoomBoundary::Centreline);
    policy.wall_gap_ft(if all_centreline { RoomBoundary::Centreline } else { RoomBoundary::FinishFace })
}

/// Content revision for an adjacency response: the scoped room revision (which
/// already tracks exactly which snapshot each contributing model provides —
/// `rooms::scoped_revision`) hashed together with the effective tolerance.
///
/// Deriving it rather than recomputing it is deliberate. `assemble_rooms`
/// already hands back the snapshot-set revision, so there is no reason to widen
/// `rooms.rs`'s API — and folding `wall_max` in is what makes a slider drag
/// register as new content, which is the whole reason this field exists.
fn adjacency_revision(rooms_revision: &str, wall_max: f64) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rooms_revision.hash(&mut hasher);
    // `to_bits` because f64 is not Hash: two tolerances that differ at all are
    // different content, and bit equality is exactly the question being asked.
    wall_max.to_bits().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Area centroid of a room's outer loop. Falls back to the mean of its points
/// for geometry too degenerate to have one (an unplaced room), and to the origin
/// for a room with no points at all — a node still needs a position, and a
/// missing one is a diagnostic signal, not an error.
fn centroid_of(room: &Room) -> Point2D {
    if let Some(c) = outer_polygon(room).and_then(|p| p.centroid()) {
        return Point2D { x: c.x(), y: c.y() };
    }
    let pts = room.loops.first().map(|l| l.points.as_slice()).unwrap_or(&[]);
    if pts.is_empty() {
        return Point2D { x: 0.0, y: 0.0 };
    }
    let n = pts.len() as f64;
    Point2D {
        x: pts.iter().map(|p| p.x).sum::<f64>() / n,
        y: pts.iter().map(|p| p.y).sum::<f64>() / n,
    }
}

/// Assemble the adjacency result for one project, scoped like `/rooms` and
/// `/areas`. Reuses `assemble_rooms` for the scoped, classified, level-deduped
/// room set, so the graph is built over exactly the rooms the plan is drawing —
/// milestone pins and building filters included, for free.
///
/// `?filter=` is deliberately **not** part of the scope: a filtered room set
/// would silently drop neighbours, and a graph that omits what a room touches is
/// worse than no graph.
///
/// `Ok(None)` when nothing has ever been pushed (the adapter's 204), mirroring
/// `assemble_rooms` and `assemble_areas`.
pub fn assemble_adjacency(
    state: &AppState,
    project: &str,
    building: Option<&str>,
    milestone: Option<&str>,
    wall_max: Option<f64>,
) -> Result<Option<AdjacencyResult>, ServiceError> {
    // Guard the request before doing any work — a bad tolerance should not cost
    // a full room merge first.
    let requested = check_wall_max(wall_max)?;
    let policy = state.settings().settings_for(project).map(|b| b.areas.clone()).unwrap_or_default();

    let scope = RoomScope { project: Some(project), building, milestone, ..Default::default() };
    let Some(rooms) = assemble_rooms(state, &scope)? else {
        return Ok(None);
    };

    // With no explicit request, the default is the project's declared wall
    // thickness — narrowed to zero when *every* contributing model declares the
    // centreline regime, where neighbours already touch and any positive
    // tolerance only invites bridging. This is the payoff of the declared
    // regime on this endpoint: the slider's honest starting point is now
    // derived rather than guessed. A model that declares nothing resolves to
    // finish face, so a single undeclared model keeps the wider default — the
    // conservative direction.
    let wall_max = requested.unwrap_or_else(|| default_wall_max(&policy, &rooms));

    let nodes: Vec<AdjacencyNode> = rooms
        .rooms
        .iter()
        .map(|r| AdjacencyNode {
            room_id: r.room.id.clone(),
            name: r.room.name.clone(),
            level_id: r.room.level_id.clone(),
            centroid: centroid_of(&r.room),
            classification: r.classification.clone(),
            reference: r.reference.clone(),
        })
        .collect();

    let refs: Vec<&Room> = rooms.rooms.iter().map(|r| &r.room).collect();
    let edges = compute_adjacency(&refs, wall_max);

    Ok(Some(AdjacencyResult {
        schema_version: SUPPORTED_SCHEMA,
        revision: adjacency_revision(&rooms.revision, wall_max),
        wall_max,
        levels: rooms.levels,
        nodes,
        edges,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Loop;

    /// The wall gap a project that declares no `[areas]` policy runs at — the
    /// stand-in these tests use wherever they mean "the default tolerance".
    /// Named locally so the tests read as "the default" rather than repeating
    /// where the number now lives.
    const DEFAULT_WALL_FT: f64 = AreaPolicy::DEFAULT_MAX_WALL_THICKNESS_FT;

    /// Build a room from one or more loops of `(x, y)` corners — same helper
    /// shape `areas.rs`'s tests use, so the two geometry modules read alike.
    fn room(id: &str, loops: &[&[(f64, f64)]]) -> Room {
        Room {
            id: id.to_string(),
            name: id.to_string(),
            level_id: "L1".to_string(),
            loops: loops
                .iter()
                .map(|pts| Loop { points: pts.iter().map(|&(x, y)| Point2D { x, y }).collect() })
                .collect(),
            properties: Default::default(),
        }
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(f64, f64)> {
        vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    }

    fn edges_of(rooms: &[Room], wall_max: f64) -> Vec<AdjacencyEdge> {
        let refs: Vec<&Room> = rooms.iter().collect();
        compute_adjacency(&refs, wall_max)
    }

    fn find<'a>(edges: &'a [AdjacencyEdge], a: &str, b: &str) -> Option<&'a AdjacencyEdge> {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        edges.iter().find(|e| e.a == lo && e.b == hi)
    }

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "expected ~{b}, got {a}");
    }

    // ---- the two boundary regimes ----

    /// **Centreline regime**: neighbours tile edge-to-edge, so the shared
    /// boundaries are coincident and the gap is zero. This must work at
    /// `wall_max = 0` — it is the more common of the two regimes, and the one an
    /// earlier "reject non-positive tolerance" rule would have made unreachable.
    #[test]
    fn test_centreline_coincident_edges_at_zero_tolerance() {
        let rooms = vec![
            room("a", &[&rect(0.0, 0.0, 10.0, 10.0)]),
            room("b", &[&rect(10.0, 0.0, 20.0, 10.0)]),
        ];
        let e = edges_of(&rooms, 0.0);
        assert_eq!(e.len(), 1, "coincident edges are adjacent at zero tolerance");
        approx(find(&e, "a", "b").unwrap().shared_length, 10.0);
    }

    /// The same pair with export noise on the shared edge (Revit exports are not
    /// bit-identical across rooms). `COINCIDENT_EPS_FT` is what keeps a
    /// zero-tolerance request working here.
    #[test]
    fn test_centreline_survives_float_noise() {
        let a = rect(0.0, 0.0, 10.0, 10.0);
        let b = vec![(10.0 + 1e-9, 0.0), (20.0, 0.0), (20.0, 10.0), (10.0 - 1e-9, 10.0)];
        let e = edges_of(&[room("a", &[&a]), room("b", &[&b])], 0.0);
        assert_eq!(e.len(), 1, "noise-level disagreement is still one shared wall");
    }

    /// **Finish-face regime**: the rooms are offset by a wall, so the gap is
    /// real. It matches once the tolerance clears the wall, and not before —
    /// which is precisely what the viewer's slider is for.
    #[test]
    fn test_finish_face_gap_needs_the_tolerance() {
        let rooms = vec![
            room("a", &[&rect(0.0, 0.0, 10.0, 10.0)]),
            room("b", &[&rect(10.5, 0.0, 20.0, 10.0)]), // 0.5 ft of wall between
        ];
        assert!(edges_of(&rooms, 0.0).is_empty(), "a real wall gap is not adjacency at zero tolerance");
        assert!(edges_of(&rooms, 0.4).is_empty(), "still short of the wall thickness");
        let e = edges_of(&rooms, 0.6);
        assert_eq!(e.len(), 1, "tolerance clears the wall -> adjacent");
        approx(find(&e, "a", "b").unwrap().shared_length, 10.0);
    }

    // ---- rejections ----

    /// Two rooms meeting at a single corner point share no length, so they are
    /// not adjacent. `MIN_SHARED_FT` is what says so.
    #[test]
    fn test_corner_touch_is_not_adjacency() {
        let rooms = vec![
            room("a", &[&rect(0.0, 0.0, 10.0, 10.0)]),
            room("b", &[&rect(10.0, 10.0, 20.0, 20.0)]),
        ];
        assert!(edges_of(&rooms, DEFAULT_WALL_FT).is_empty(), "a corner touch is not a shared wall");
    }

    /// A short shared stretch — a door reveal, a jog in a wall — stays below the
    /// minimum and is dropped, while the same geometry with a real wall length
    /// is kept. Locks the threshold's purpose, not just its value.
    #[test]
    fn test_sub_minimum_overlap_is_dropped() {
        let short = vec![
            room("a", &[&rect(0.0, 0.0, 10.0, 10.0)]),
            room("b", &[&rect(10.0, 9.5, 20.0, 19.5)]), // 0.5 ft of shared edge
        ];
        assert!(edges_of(&short, 0.0).is_empty(), "0.5 ft of overlap is below MIN_SHARED_FT");

        let long = vec![
            room("a", &[&rect(0.0, 0.0, 10.0, 10.0)]),
            room("b", &[&rect(10.0, 8.0, 20.0, 18.0)]), // 2 ft of shared edge
        ];
        assert_eq!(edges_of(&long, 0.0).len(), 1, "2 ft clears it");
    }

    /// Rooms on opposite sides of a corridor must not be wired together. The
    /// corridor is wider than any wall tolerance, so the gap test alone rejects
    /// this — the case the tolerance's upper bound exists to protect.
    #[test]
    fn test_corridor_is_not_bridged() {
        let rooms = vec![
            room("west", &[&rect(0.0, 0.0, 10.0, 10.0)]),
            room("corridor", &[&rect(10.0, 0.0, 18.0, 10.0)]),
            room("east", &[&rect(18.0, 0.0, 28.0, 10.0)]),
        ];
        let e = edges_of(&rooms, DEFAULT_WALL_FT);
        assert!(find(&e, "west", "corridor").is_some());
        assert!(find(&e, "corridor", "east").is_some());
        assert!(find(&e, "west", "east").is_none(), "an 8 ft corridor is not a wall");
    }

    /// **The occlusion case.** A service shaft narrower than the tolerance sits
    /// between two rooms; without the third-room test its neighbours would match
    /// straight through it. Hospital models are full of these, which is why this
    /// is a test and not a footnote.
    #[test]
    fn test_thin_room_is_not_bridged_through() {
        let rooms = vec![
            room("a", &[&rect(0.0, 0.0, 10.0, 10.0)]),
            room("shaft", &[&rect(10.0, 0.0, 10.8, 10.0)]), // 0.8 ft wide
            room("b", &[&rect(10.8, 0.0, 20.0, 10.0)]),
        ];
        let e = edges_of(&rooms, DEFAULT_WALL_FT); // 1.5 ft — wide enough to span the shaft
        assert!(find(&e, "a", "shaft").is_some(), "each neighbour still touches the shaft");
        assert!(find(&e, "shaft", "b").is_some());
        assert!(find(&e, "a", "b").is_none(), "must not match through the shaft");
    }

    /// The occlusion test must not reject a legitimate pair just because some
    /// unrelated room exists elsewhere on the level.
    #[test]
    fn test_occlusion_does_not_reject_unrelated_rooms() {
        let rooms = vec![
            room("a", &[&rect(0.0, 0.0, 10.0, 10.0)]),
            room("b", &[&rect(10.5, 0.0, 20.0, 10.0)]),
            room("far", &[&rect(100.0, 100.0, 110.0, 110.0)]),
        ];
        assert!(find(&edges_of(&rooms, 1.0), "a", "b").is_some());
    }

    // ---- accumulation ----

    /// An L-shaped neighbour wraps two sides of a room: two disjoint runs, on
    /// perpendicular walls, which must **sum** rather than either being lost or
    /// being merged into one interval.
    #[test]
    fn test_l_shaped_junction_sums_disjoint_runs() {
        // `a` is the inner square; `l` wraps its right side and its top.
        let a = rect(0.0, 0.0, 10.0, 10.0);
        let l = vec![
            (10.0, 0.0),
            (20.0, 0.0),
            (20.0, 20.0),
            (0.0, 20.0),
            (0.0, 10.0),
            (10.0, 10.0),
        ];
        let e = edges_of(&[room("a", &[&a]), room("l", &[&l])], 0.0);
        assert_eq!(e.len(), 1);
        approx(find(&e, "a", "l").unwrap().shared_length, 20.0); // 10 right + 10 top
    }

    /// A wall split into several boundary segments — which is what Revit
    /// produces wherever a bounding element changes, e.g. a door in the middle
    /// of a partition — must be counted **once**, not once per segment pair.
    /// Without interval merging this pair reports well over its true length.
    #[test]
    fn test_split_boundary_segments_are_not_double_counted() {
        let a = rect(0.0, 0.0, 10.0, 10.0);
        // `b`'s facing side is split into three collinear segments.
        let b = vec![
            (10.0, 0.0),
            (20.0, 0.0),
            (20.0, 10.0),
            (10.0, 10.0),
            (10.0, 6.0),
            (10.0, 3.0),
        ];
        let e = edges_of(&[room("a", &[&a]), room("b", &[&b])], 0.0);
        assert_eq!(e.len(), 1);
        approx(find(&e, "a", "b").unwrap().shared_length, 10.0);
    }

    // ---- degenerate and duplicate input ----

    /// An unplaced room (fewer than three points in its outer loop) is a node
    /// with no edges — a diagnostic signal, never an error or a panic.
    #[test]
    fn test_unplaced_room_yields_no_edges() {
        let rooms = vec![
            room("a", &[&rect(0.0, 0.0, 10.0, 10.0)]),
            room("ghost", &[&[(5.0, 5.0)]]),
        ];
        let e = edges_of(&rooms, DEFAULT_WALL_FT);
        assert!(e.is_empty(), "an unplaced room touches nothing");
    }

    /// The same room delivered twice by two linked models is suppressed rather
    /// than reported as a maximum-weight edge to itself.
    #[test]
    fn test_duplicate_room_from_linked_models_is_suppressed() {
        let r = rect(0.0, 0.0, 10.0, 10.0);
        let mut twin = room("arch-1", &[&r]);
        twin.id = "mep-1".to_string(); // same geometry, different model's id
        let e = edges_of(&[room("arch-1", &[&r]), twin], 0.0);
        assert!(e.is_empty(), "one room contributed twice is not two adjacent rooms");
    }

    /// Adjacency never crosses a level, even where the geometry would match
    /// perfectly — the single-level decision, enforced by partitioning.
    #[test]
    fn test_levels_are_independent() {
        let mut upper = room("b", &[&rect(10.0, 0.0, 20.0, 10.0)]);
        upper.level_id = "L2".to_string();
        assert!(edges_of(&[room("a", &[&rect(0.0, 0.0, 10.0, 10.0)]), upper], 0.0).is_empty());
    }

    /// A closed loop (first point repeated last) describes the same room as an
    /// open one — the contract does not promise either form.
    #[test]
    fn test_closed_and_open_loops_agree() {
        let open = vec![
            room("a", &[&rect(0.0, 0.0, 10.0, 10.0)]),
            room("b", &[&rect(10.0, 0.0, 20.0, 10.0)]),
        ];
        let closed_a = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0)];
        let closed = vec![room("a", &[&closed_a]), room("b", &[&rect(10.0, 0.0, 20.0, 10.0)])];
        approx(
            find(&edges_of(&open, 0.0), "a", "b").unwrap().shared_length,
            find(&edges_of(&closed, 0.0), "a", "b").unwrap().shared_length,
        );
    }

    /// Edges come back in a stable order with `a < b` inside each pair, so the
    /// payload — and therefore the revision — is deterministic.
    #[test]
    fn test_edges_are_deterministic_and_ordered() {
        let rooms = vec![
            room("z", &[&rect(0.0, 0.0, 10.0, 10.0)]),
            room("m", &[&rect(10.0, 0.0, 20.0, 10.0)]),
            room("a", &[&rect(20.0, 0.0, 30.0, 10.0)]),
        ];
        let e = edges_of(&rooms, 0.0);
        assert_eq!(e.len(), 2);
        for edge in &e {
            assert!(edge.a < edge.b, "each pair is ordered a < b");
        }
        let sorted: Vec<(&str, &str)> = e.iter().map(|x| (x.a.as_str(), x.b.as_str())).collect();
        let mut expect = sorted.clone();
        expect.sort_unstable();
        assert_eq!(sorted, expect, "edges come back sorted");
    }

    /// The spatial index must not change the answer, only the speed. A dense
    /// grid of rooms — where a room spans several cells and pairs are found
    /// through more than one — is exactly where a grid bug (a missed neighbour, a
    /// double-counted pair, an occluder queried from the wrong cell) would show.
    /// Every interior room has four orthogonal neighbours and no diagonal one, a
    /// fact independent of the grid, so it pins correctness rather than
    /// re-deriving the implementation.
    #[test]
    fn test_grid_matches_brute_force_on_a_dense_layout() {
        // 6x6 of unit-gap-free 10x10 rooms: centreline tiling, 36 rooms.
        let mut rooms = Vec::new();
        for gx in 0..6 {
            for gy in 0..6 {
                let (x, y) = (gx as f64 * 10.0, gy as f64 * 10.0);
                rooms.push(room(&format!("r{gx}{gy}"), &[&rect(x, y, x + 10.0, y + 10.0)]));
            }
        }
        let edges = edges_of(&rooms, 0.0);

        // Horizontal + vertical adjacencies in a 6x6 grid: 2 * 6 * 5 = 60.
        assert_eq!(edges.len(), 60, "every orthogonal neighbour pair, no diagonals");
        for e in &edges {
            approx(e.shared_length, 10.0);
        }
        // A corner room touches exactly two neighbours; a centre room four.
        let degree = |id: &str| edges.iter().filter(|e| e.a == id || e.b == id).count();
        assert_eq!(degree("r00"), 2, "corner room: two neighbours");
        assert_eq!(degree("r22"), 4, "interior room: four neighbours, no diagonal");
    }

    // ---- tolerance validation ----

    /// A valid request passes through untouched, and **zero passes** — it is the
    /// centreline setting, not a degenerate one, and rejecting it as
    /// "non-positive" would break the more common of the two boundary regimes.
    /// An absent request is `None`, not a resolved default: what an unrequested
    /// tolerance becomes is `default_wall_max`'s job (tested separately), and
    /// keeping the two apart is what lets the guard run before the room merge.
    #[test]
    fn test_check_wall_max_passes_valid_requests_including_zero() {
        assert!(check_wall_max(None).unwrap().is_none(), "no request is not a default");
        approx(check_wall_max(Some(0.0)).unwrap().unwrap(), 0.0); // the centreline case
        approx(check_wall_max(Some(1.2)).unwrap().unwrap(), 1.2);
        approx(check_wall_max(Some(DEFAULT_WALL_FT)).unwrap().unwrap(), DEFAULT_WALL_FT);
        approx(check_wall_max(Some(WALL_MAX_LIMIT_FT)).unwrap().unwrap(), WALL_MAX_LIMIT_FT);
    }

    /// Loud over a silent clamp, and the message is the part that matters: a
    /// caller who sent a tolerance that would wire the whole building together
    /// has to be able to tell that from a graph that legitimately found nothing.
    #[test]
    fn test_check_wall_max_rejects_loudly() {
        for bad in [-0.1, f64::NAN, f64::INFINITY, WALL_MAX_LIMIT_FT + 0.1] {
            match check_wall_max(Some(bad)) {
                Err(ServiceError::Invalid(msg)) => assert!(!msg.is_empty(), "the message is the useful part"),
                other => panic!("expected Invalid for {bad}, got {other:?}", other = other.map(|_| "Ok")),
            }
        }
    }

    /// The revision moves with the tolerance, not just with the data — the
    /// property that stops the viewer looking frozen after a slider drag.
    #[test]
    fn test_revision_tracks_both_data_and_tolerance() {
        let a = adjacency_revision("rev-1", 1.5);
        assert_eq!(a, adjacency_revision("rev-1", 1.5), "stable while nothing changes");
        assert_ne!(a, adjacency_revision("rev-1", 1.6), "a tolerance change is new content");
        assert_ne!(a, adjacency_revision("rev-2", 1.5), "a push is new content");
    }

    // ---- endpoint (assemble_adjacency end-to-end over AppState) ----

    mod endpoint {
        use super::*;
        use crate::contract::{Level, Model, Project, RoomPayload, Snapshot};
        use crate::state::{AppState, ProjectSettings};
        use crate::storage::MemStore;
        use std::collections::HashMap;

        fn bundle() -> ProjectSettings {
            ProjectSettings {
                reference: std::collections::BTreeMap::new(),
                hierarchy: vec![],
                builtin_properties: vec![],
                room_label: vec!["$name".to_string()],
                milestones: vec![],
                comparison_key: None,
                comparison_properties: vec![],
                areas: Default::default(),
                doors: Default::default(),
                hierarchy_exclusions: vec![],
            }
        }

        fn state_with(rooms: Vec<Room>) -> AppState {
            let registry = HashMap::from([("p1".to_string(), bundle())]);
            let state = AppState::new(Box::new(MemStore::new()), registry, None);
            state
                .set_snapshot(RoomPayload {
                    schema_version: SUPPORTED_SCHEMA,
                    project: Project { id: "p1".to_string(), name: "P".to_string() },
                    model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                    snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
                    phase: None,
                    model_to_shared: None,
                    room_boundary: None,
                    levels: vec![Level { id: "L1".to_string(), name: "Level 1".to_string(), elevation: 0.0 }],
                    rooms,
                })
                .unwrap();
            state
        }

        /// End-to-end: the endpoint scopes via `assemble_rooms`, emits one node
        /// per room with a centroid, and one edge for the shared wall.
        #[test]
        fn test_assemble_adjacency_happy_path() {
            let state = state_with(vec![
                room("a", &[&rect(0.0, 0.0, 10.0, 10.0)]),
                room("b", &[&rect(10.0, 0.0, 20.0, 10.0)]),
            ]);
            let r = assemble_adjacency(&state, "p1", None, None, Some(0.0)).unwrap().expect("store has data");

            assert_eq!(r.schema_version, SUPPORTED_SCHEMA);
            assert_eq!(r.levels.len(), 1);
            assert_eq!(r.nodes.len(), 2);
            assert_eq!(r.edges.len(), 1);
            approx(r.wall_max, 0.0);
            // Centroid of the left 10x10 room is its middle.
            let a = r.nodes.iter().find(|n| n.room_id == "a").unwrap();
            approx(a.centroid.x, 5.0);
            approx(a.centroid.y, 5.0);
        }

        /// The response revision is stable across idle requests and moves when
        /// `wall_max` does — both halves of what the viewer's change detection
        /// depends on.
        #[test]
        fn test_assemble_adjacency_revision_behaviour() {
            let state = state_with(vec![
                room("a", &[&rect(0.0, 0.0, 10.0, 10.0)]),
                room("b", &[&rect(10.0, 0.0, 20.0, 10.0)]),
            ]);
            let get = |w: f64| assemble_adjacency(&state, "p1", None, None, Some(w)).unwrap().unwrap().revision;
            assert_eq!(get(1.5), get(1.5), "idle requests are byte-identical");
            assert_ne!(get(1.5), get(0.5), "a tolerance change is a new revision");
        }

        /// With no `?wall_max=`, the default now comes from the **declared
        /// regime** rather than a hardcoded constant: zero when every level in
        /// scope is centreline, the project's declared thickness otherwise. An
        /// explicit request still overrides both — the slider is a question the
        /// caller is asking, not a policy the project is stating.
        #[test]
        fn test_default_wall_max_follows_the_declared_regime() {
            let with_boundary = |declared: Option<RoomBoundary>, thickness: f64| {
                let bundle = ProjectSettings {
                    areas: AreaPolicy { max_wall_thickness: thickness, ..Default::default() },
                    ..bundle()
                };
                let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), bundle)]), None);
                state
                    .set_snapshot(RoomPayload {
                        schema_version: SUPPORTED_SCHEMA,
                        project: Project { id: "p1".to_string(), name: "P".to_string() },
                        model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                        snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
                        phase: None,
                        model_to_shared: None,
                        room_boundary: declared,
                        levels: vec![Level { id: "L1".to_string(), name: "Level 1".to_string(), elevation: 0.0 }],
                        rooms: vec![room("a", &[&rect(0.0, 0.0, 10.0, 10.0)])],
                    })
                    .unwrap();
                state
            };

            let centreline = with_boundary(Some(RoomBoundary::Centreline), 0.4);
            approx(assemble_adjacency(&centreline, "p1", None, None, None).unwrap().unwrap().wall_max, 0.0);

            let finish_face = with_boundary(Some(RoomBoundary::FinishFace), 0.4);
            approx(assemble_adjacency(&finish_face, "p1", None, None, None).unwrap().unwrap().wall_max, 0.4);

            // Undeclared reads as finish face — the conservative direction.
            let undeclared = with_boundary(None, 0.4);
            approx(assemble_adjacency(&undeclared, "p1", None, None, None).unwrap().unwrap().wall_max, 0.4);

            // And an explicit request wins over any of it.
            approx(
                assemble_adjacency(&centreline, "p1", None, None, Some(1.0)).unwrap().unwrap().wall_max,
                1.0,
            );
        }

        /// An out-of-range tolerance is a caller fault carrying a message, not a
        /// clamped result — and it fails before any geometry is touched.
        #[test]
        fn test_assemble_adjacency_rejects_bad_tolerance() {
            let state = state_with(vec![room("a", &[&rect(0.0, 0.0, 10.0, 10.0)])]);
            assert!(matches!(
                assemble_adjacency(&state, "p1", None, None, Some(99.0)),
                Err(ServiceError::Invalid(_))
            ));
        }

        /// Nothing pushed -> None (the adapter's 204), mirroring
        /// `assemble_rooms` and `assemble_areas`.
        #[test]
        fn test_assemble_adjacency_empty_store_is_none() {
            let registry = HashMap::from([("p1".to_string(), bundle())]);
            let state = AppState::new(Box::new(MemStore::new()), registry, None);
            assert!(assemble_adjacency(&state, "p1", None, None, None).unwrap().is_none());
        }
    }
}
