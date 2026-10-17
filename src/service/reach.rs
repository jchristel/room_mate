//! How far a walk of a given length reaches from one or more start points, laid out
//! as a grid over the floor plan: a heat map of walking distance.
//!
//! Transport-agnostic like every `service` module: the HTTP handler and the MCP tool
//! are thin adapters over `assemble_reach`.
//!
//! **The question is "within a walk of L from these points, what is covered?"** A hose
//! reel's 30 m, a travel-distance limit from an exit, a walk to the nearest extinguisher
//! and a catchment are all the same read with different sources and limit. So the distance is the walk of `service::routing`'s
//! door-to-door method run from every source at once (`routing::field`), over the
//! very graph a route is searched over (`connectivity::build_graph`): doors, open
//! zones and passages. A cell's distance is the cheapest of "the walk to one of its
//! room's ports, then the exact walk inside the room from that port to the cell".
//! Where the port sees the cell that is a straight line, which is almost every cell,
//! and where it does not (an L, a column) it is the visibility-graph walk.
//!
//! **One floor plate at a time.** A vertical link is not crossed: a walk
//! measured on a plate does not continue by lift. The levels in the answer are the sources' own plus any a door joins
//! to them, so a stair door that opens onto another storey's room is followed.
//!
//! **Cells are decided by their centre being inside a room's outline.** The outline is
//! the room's boundary, walls included, so a cell in a wall belongs to no room (`-1`)
//! and a cell in two overlapping rooms takes the nearer. A room too small to hold a
//! cell centre is not dropped from the coverage: its inside point is the one sample,
//! so a cupboard still gets a verdict (it is not drawn, having no cell).
//!
//! **Three states per cell, and they are different facts.** `-1`: no room here.
//! `-2`: a room, and the walk is longer than the limit or never gets here (a room no
//! door reaches is `-2`, which is a finding about what is reachable and not an error).
//! Otherwise the walk in whole millimetres. "Beyond" is deliberately not a number:
//! the search stops at the limit, so a distance past it was never measured.
//!
//! **`ignore_cuts`** treats every open zone's closed walls as open. A cut is a
//! statement about where a person routes, and a coverage question may not be bound by it. The
//! building's own walls, and the doors, are unchanged.
//!
//! **Two methods, chosen per read.** `exact` (the default) evaluates each cell by the
//! exact in-room walk from the room's ports. `grid` runs Dijkstra over the cells
//! themselves, eight neighbours a step, inside each room and seeded from the same
//! port distances: the lattice approximation grid methods use, kept as a choice
//! because it is the method a reader may know and compare against. It is
//! never shorter than `exact` inside a room (a lattice path cannot beat the straight
//! line) and runs longer between cells (the lattice's own error is up to about 8%, and
//! a port seeds its nearest cell, not its exact point; on RHH's GROUND the median was
//! 0.2% and the worst 14%), so it can leave a cell at the
//! limit's edge uncovered that `exact` covers, and its values change with the spacing.
//! Only `exact` is a distance a route would also measure.
//!
//! **Not modelled, and stated:** furniture, and any allowance beyond the walk itself,
//! which is the caller's to add to the limit. The geometric shortest path hugs corners,
//! so a distance is the least walk that can reach the cell.
//!
//! **Prior art.** Nothing here is copied from a paper or a package; each piece is a
//! standard technique, named so a reader can find the source:
//! - The door-to-door distance model (distances held per room, the search run over
//!   doors) is the indoor-space model compared in Liu et al., "Indoor Spatial Queries:
//!   Modeling, Indexing, and Processing" (arXiv 2010.03910) and used by Shao et al.,
//!   "VIP-Tree: An Effective Index for Indoor Spatial Queries" (PVLDB vol. 10).
//!   Cutting the search off at the limit is their indoor range query.
//! - The walk inside a room is the visibility-graph shortest path among polygonal
//!   obstacles (Lozano-Perez and Wesley, CACM 1979; surveyed in Mitchell, "Geometric
//!   Shortest Paths and Network Optimization", Handbook of Computational Geometry).
//! - Starting the search from every source at once is Dijkstra (1959) with a
//!   multi-source frontier.
//! - The per-cell field is the distance map that grid methods approximate with the
//!   Fast Marching Method (Sethian, PNAS 1996); this evaluates the exact in-room walk
//!   instead, so a cell's value does not depend on the spacing.
//!
//! Ideas only; no text or code from those works is reproduced.

use std::cmp::Ordering;
use std::collections::{BTreeSet, BinaryHeap, HashMap};

use serde::Serialize;

use crate::contract::{Level, Point2D};
use crate::state::AppState;

use super::connectivity::{self, build_graph, outlines, ConnectivityScope, Endpoint};
use super::room_locator::RoomRef;
use super::routing::{self, Source};
use super::ServiceError;

pub const SCHEMA_VERSION: u32 = 1;

const MM_PER_FT: f64 = 304.8;

/// A cell's distance when it is in no room.
pub const NO_ROOM: i32 = -1;
/// A cell's distance when it is in a room the walk does not reach within the limit.
pub const BEYOND: i32 = -2;

/// The largest walk asked for, in mm. A kilometre is far past any walk asked of a floor plate and keeps a
/// typo from asking the search to cover a campus.
const MAX_LIMIT_MM: f64 = 1_000_000.0;
/// The smallest and largest grid spacing, in mm. Finer than this is a cell smaller than
/// a wall's thickness, and coarser a cell bigger than most rooms.
const MIN_SPACING_MM: f64 = 100.0;
const MAX_SPACING_MM: f64 = 10_000.0;
/// The most cells one answer may hold, so a fine spacing over a large project is
/// refused with the number it would have been and not allowed to build a response of
/// hundreds of megabytes.
const MAX_CELLS: usize = 600_000;
/// The most sources one request may name.
pub const MAX_SOURCES: usize = 500;

/// How a cell's walk is measured. See the module header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// The exact walk from the room's ports to the cell.
    #[default]
    Exact,
    /// Dijkstra over the cells of each room, seeded from the room's ports.
    Grid,
}

impl Method {
    /// The caller's word for it; absent is `exact`.
    pub fn parse(s: Option<&str>) -> Result<Self, ServiceError> {
        match s.map(str::trim).filter(|s| !s.is_empty()) {
            None | Some("exact") => Ok(Self::Exact),
            Some("grid") => Ok(Self::Grid),
            Some(other) => Err(ServiceError::Invalid(format!("method must be `exact` or `grid`, not `{other}`"))),
        }
    }
}

/// What narrows a reach read.
#[derive(Default)]
pub struct ReachScope<'a> {
    /// How each cell's walk is measured.
    pub method: Method,
    pub building: Option<&'a str>,
    pub milestone: Option<&'a str>,
    /// Treat closed walls inside open zones as open. See the module header.
    pub ignore_cuts: bool,
}

/// The two numbers a caller sends, checked. Shared by the HTTP handler and the MCP
/// tool so the two cannot disagree on what is valid. Both in millimetres.
pub fn params(limit_mm: Option<f64>, spacing_mm: Option<f64>) -> Result<(f64, f64), ServiceError> {
    let limit = limit_mm
        .ok_or_else(|| ServiceError::Invalid("limit_mm is required: the length of walk to measure".to_string()))?;
    if !limit.is_finite() || limit <= 0.0 || limit > MAX_LIMIT_MM {
        return Err(ServiceError::Invalid(format!(
            "limit_mm must be above 0 and at most {MAX_LIMIT_MM}, not {limit}"
        )));
    }
    let spacing = spacing_mm.unwrap_or(1_000.0);
    if !spacing.is_finite() || !(MIN_SPACING_MM..=MAX_SPACING_MM).contains(&spacing) {
        return Err(ServiceError::Invalid(format!(
            "spacing_mm must be between {MIN_SPACING_MM} and {MAX_SPACING_MM}, not {spacing}"
        )));
    }
    Ok((limit, spacing))
}

/// One source as a caller names it: a room, the model when the id exists in several, and
/// where in the room the start point is (both coordinates or neither; neither is the room's
/// inside point). Shared by the HTTP handler and the MCP tool so they cannot disagree.
pub fn source(room_id: &str, model_id: Option<&str>, x: Option<f64>, y: Option<f64>) -> Result<Endpoint, ServiceError> {
    let room_id = room_id.trim();
    if room_id.is_empty() {
        return Err(ServiceError::Invalid("a source needs a room_id".to_string()));
    }
    Ok(Endpoint {
        model_id: model_id.map(str::trim).filter(|m| !m.is_empty()).map(str::to_string),
        room_id: room_id.to_string(),
        at: connectivity::point(x, y, "source")?,
    })
}

// ================================ wire shape ================================

#[derive(Serialize)]
pub struct ReachResult {
    pub schema_version: u32,
    pub revision: String,
    /// The walk measured, in mm, and the grid spacing.
    pub limit_mm: f64,
    pub spacing_mm: f64,
    pub ignore_cuts: bool,
    pub method: Method,
    /// Where each source started: the room, and the point inside it the walk began from.
    pub sources: Vec<ReachSource>,
    /// One grid per storey the walk can reach. See `ReachLevel`.
    pub levels: Vec<ReachLevel>,
    /// Every room on those storeys with an outline, and how much of it is within reach.
    pub rooms: Vec<RoomCoverage>,
    pub summary: Summary,
    /// Anything about how the answer was made that a reader should know, such as walks
    /// that fell back to a straight line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Serialize)]
pub struct ReachSource {
    #[serde(flatten)]
    pub room: RoomRef,
    pub name: String,
    pub level_id: String,
    pub at: Point2D,
}

/// One storey's grid, row by row from the lowest y, each row from the lowest x.
///
/// Cell `(col, row)` has its centre at `origin + ((col + 0.5), (row + 0.5)) * spacing_ft`,
/// in the plan's own frame (feet, y up), and its value is at `distance_mm[row * cols + col]`:
/// `-1` no room, `-2` a room beyond the limit, else the walk in whole mm. The origin is a
/// whole multiple of the spacing, so two reads at one spacing share cell edges.
#[derive(Serialize)]
pub struct ReachLevel {
    pub level_id: String,
    pub name: String,
    pub origin: Point2D,
    pub spacing_ft: f64,
    pub cols: usize,
    pub rows: usize,
    pub distance_mm: Vec<i32>,
}

#[derive(Serialize)]
pub struct RoomCoverage {
    #[serde(flatten)]
    pub room: RoomRef,
    pub name: String,
    pub level_id: String,
    /// Samples taken in the room: its cells, or one point for a room too small to hold one.
    pub cells: usize,
    /// How many of them are within the limit.
    pub covered: usize,
    /// `covered / cells`.
    pub fraction: f64,
    /// Whether any door, zone or passage lets the walk into this room at all.
    pub reached: bool,
    /// The shortest and longest walk, in mm, to a covered sample. Absent when none is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nearest_mm: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub farthest_mm: Option<i32>,
}

#[derive(Serialize, Default)]
pub struct Summary {
    /// Cells in rooms, and how many are within the limit.
    pub cells: usize,
    pub covered: usize,
    pub fraction: f64,
    /// The same, as plan area in square metres (cells times spacing squared).
    pub area_sqm: f64,
    pub covered_sqm: f64,
    /// Rooms with a sample, how many are fully covered, partly, and not at all.
    pub rooms: usize,
    pub rooms_full: usize,
    pub rooms_partial: usize,
    pub rooms_none: usize,
}

// ================================ the read ================================

/// A cell waiting in the grid search, cheapest first.
#[derive(PartialEq)]
struct Pending {
    cost: f64,
    cell: usize,
}
impl Eq for Pending {}
impl Ord for Pending {
    fn cmp(&self, other: &Self) -> Ordering {
        other.cost.total_cmp(&self.cost).then(other.cell.cmp(&self.cell))
    }
}
impl PartialOrd for Pending {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The `grid` method for one room: Dijkstra over its cells, eight neighbours a step,
/// seeded from the room's ports (`(point, walk to it)`, as `routing::field` leaves them).
///
/// `cells` are the room's `(col, row, centre)`. A diagonal step needs both of the cells it
/// squeezes between, so a walk cannot cut a wall's corner. A port lies on the outline and
/// seeds the cells in the 3 by 3 around it; a port with none (a room thinner than a cell)
/// seeds the nearest cell. Returns each cell's walk in feet, or `None` past the limit.
fn grid_walks(
    ports: &[(Point2D, f64)],
    cells: &[(usize, usize, Point2D)],
    origin: Point2D,
    spacing_ft: f64,
    limit_ft: f64,
) -> Vec<Option<f64>> {
    if cells.is_empty() || ports.is_empty() {
        return vec![None; cells.len()];
    }
    let at: HashMap<(usize, usize), usize> = cells.iter().enumerate().map(|(k, c)| ((c.0, c.1), k)).collect();
    let mut best = vec![f64::INFINITY; cells.len()];
    let mut heap = BinaryHeap::new();
    let offer = |best: &mut [f64], heap: &mut BinaryHeap<Pending>, k: usize, cost: f64| {
        if cost <= limit_ft && cost < best[k] {
            best[k] = cost;
            heap.push(Pending { cost, cell: k });
        }
    };
    for &(p, d0) in ports {
        let (pc, pr) = (
            ((p.x - origin.x) / spacing_ft).floor() as i64,
            ((p.y - origin.y) / spacing_ft).floor() as i64,
        );
        let mut seeded = false;
        for dr in -1..=1i64 {
            for dc in -1..=1i64 {
                let (Ok(c), Ok(r)) = (usize::try_from(pc + dc), usize::try_from(pr + dr)) else {
                    continue;
                };
                if let Some(&k) = at.get(&(c, r)) {
                    seeded = true;
                    offer(&mut best, &mut heap, k, d0 + (cells[k].2.x - p.x).hypot(cells[k].2.y - p.y));
                }
            }
        }
        if !seeded {
            let nearest = (0..cells.len()).min_by(|&a, &b| {
                let da = (cells[a].2.x - p.x).hypot(cells[a].2.y - p.y);
                let db = (cells[b].2.x - p.x).hypot(cells[b].2.y - p.y);
                da.total_cmp(&db)
            });
            if let Some(k) = nearest {
                offer(&mut best, &mut heap, k, d0 + (cells[k].2.x - p.x).hypot(cells[k].2.y - p.y));
            }
        }
    }
    while let Some(Pending { cost, cell }) = heap.pop() {
        if cost > best[cell] {
            continue;
        }
        let (col, row) = (cells[cell].0, cells[cell].1);
        for (dc, dr) in [
            (-1i64, -1i64),
            (0, -1),
            (1, -1),
            (-1, 0),
            (1, 0),
            (-1, 1),
            (0, 1),
            (1, 1),
        ] {
            let (Ok(c), Ok(r)) = (usize::try_from(col as i64 + dc), usize::try_from(row as i64 + dr)) else {
                continue;
            };
            let Some(&k) = at.get(&(c, r)) else { continue };
            if dc != 0 && dr != 0 && !(at.contains_key(&(c, row)) && at.contains_key(&(col, r))) {
                continue;
            }
            let step = if dc != 0 && dr != 0 {
                spacing_ft * std::f64::consts::SQRT_2
            } else {
                spacing_ft
            };
            offer(&mut best, &mut heap, k, cost + step);
        }
    }
    best.into_iter().map(|d| d.is_finite().then_some(d)).collect()
}

/// What the samples of one room add up to.
#[derive(Default)]
struct Tally {
    cells: usize,
    covered: usize,
    nearest: Option<i32>,
    farthest: Option<i32>,
}

impl Tally {
    /// One sample: its walk in mm when it is within the limit.
    fn add(&mut self, mm: Option<i32>) {
        self.cells += 1;
        if let Some(mm) = mm {
            self.covered += 1;
            self.nearest = Some(self.nearest.map_or(mm, |n| n.min(mm)));
            self.farthest = Some(self.farthest.map_or(mm, |f| f.max(mm)));
        }
    }
}

struct Grid {
    level_id: String,
    origin: Point2D,
    cols: usize,
    rows: usize,
    cells: Vec<i32>,
}

/// A room's bounding box from its outer ring, in feet: `(min_x, min_y, max_x, max_y)`.
fn bounds_of(room: &crate::contract::Room) -> Option<(f64, f64, f64, f64)> {
    let ring = room.loops.first()?.points.as_slice();
    let first = ring.first()?;
    Some(ring.iter().fold((first.x, first.y, first.x, first.y), |(x0, y0, x1, y1), p| {
        (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y))
    }))
}

/// Merge a cell's candidate into what it holds: any distance beats "beyond", which
/// beats "no room", and a shorter distance beats a longer.
fn better(held: i32, candidate: i32) -> i32 {
    match (held, candidate) {
        (NO_ROOM, c) => c,
        (BEYOND, c) if c >= 0 => c,
        (h, c) if h >= 0 && c >= 0 => h.min(c),
        (h, _) => h,
    }
}

/// One grid per storey in play, sized to the rooms on it, or the refusal when the lot would
/// be more cells than an answer may hold.
fn grids_for(
    level_ids: &BTreeSet<&str>,
    nodes: &[connectivity::Node],
    outlines: &[Option<&crate::contract::Room>],
    spacing_ft: f64,
    spacing_mm: f64,
) -> Result<Vec<Grid>, ServiceError> {
    let mut grids: Vec<Grid> = Vec::new();
    let mut total_cells = 0usize;
    for level_id in level_ids {
        let boxes: Vec<_> = (0..nodes.len())
            .filter(|&i| nodes[i].level_id == *level_id)
            .filter_map(|i| outlines[i].and_then(bounds_of))
            .collect();
        let Some(first) = boxes.first() else { continue };
        let (x0, y0, x1, y1) = boxes
            .iter()
            .fold(*first, |(a, b, c, d), (e, f, g, h)| (a.min(*e), b.min(*f), c.max(*g), d.max(*h)));
        let origin = Point2D {
            x: (x0 / spacing_ft).floor() * spacing_ft,
            y: (y0 / spacing_ft).floor() * spacing_ft,
        };
        let cols = (((x1 - origin.x) / spacing_ft).ceil() as usize).max(1);
        let rows = (((y1 - origin.y) / spacing_ft).ceil() as usize).max(1);
        total_cells += cols * rows;
        grids.push(Grid {
            level_id: (*level_id).to_string(),
            origin,
            cols,
            rows,
            cells: vec![NO_ROOM; cols * rows],
        });
    }
    if total_cells > MAX_CELLS {
        return Err(ServiceError::Invalid(format!(
            "a {spacing_mm:.0} mm grid over these storeys would have {total_cells} cells, more than the {MAX_CELLS} \
             allowed: raise spacing_mm"
        )));
    }
    Ok(grids)
}

/// Sample every room of the grids' storeys: write each cell's value into its grid and
/// return the room's coverage. A room is sampled where its cell centres fall inside its
/// outline, scanning only its own bounding box so the cost follows the area of the rooms
/// and not of the grid.
fn cover_rooms(
    nodes: &[connectivity::Node],
    outlines: &[Option<&crate::contract::Room>],
    field: &mut routing::Field<'_>,
    grids: &mut [Grid],
    limit_ft: f64,
    spacing_ft: f64,
    method: Method,
) -> Vec<RoomCoverage> {
    let to_mm = |ft: f64| (ft * MM_PER_FT).round() as i32;
    let mut out: Vec<RoomCoverage> = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let Some(grid) = grids.iter_mut().find(|g| g.level_id == node.level_id) else {
            continue;
        };
        let Some(room) = outlines[i] else { continue };
        let Some((rx0, ry0, rx1, ry1)) = bounds_of(room) else {
            continue;
        };
        let reached = !field.ports[i].is_empty();
        let ports = field.ports[i].clone();

        // The walk from the nearest source to a point of this room, in feet, or `None`
        // when it is past the limit or the room is not reached.
        let walk_to = |field: &mut routing::Field<'_>, p: Point2D| -> Option<f64> {
            if !reached {
                return None;
            }
            let mut best = f64::INFINITY;
            for &(port, d0) in &ports {
                if d0 >= best {
                    break; // nearest first, so nothing later can be cheaper
                }
                let straight = (port.x - p.x).hypot(port.y - p.y);
                if d0 + straight >= best {
                    continue;
                }
                best = best.min(d0 + field.walk_length(i, port, p));
            }
            (best <= limit_ft).then_some(best)
        };

        let mut tally = Tally::default();
        let col0 = (((rx0 - grid.origin.x) / spacing_ft).floor().max(0.0)) as usize;
        let col1 = ((((rx1 - grid.origin.x) / spacing_ft).ceil()) as usize).min(grid.cols);
        let row0 = (((ry0 - grid.origin.y) / spacing_ft).floor().max(0.0)) as usize;
        let row1 = ((((ry1 - grid.origin.y) / spacing_ft).ceil()) as usize).min(grid.rows);
        let mut inside: Vec<(usize, usize, Point2D)> = Vec::new();
        for row in row0..row1 {
            for col in col0..col1 {
                let centre = Point2D {
                    x: grid.origin.x + (col as f64 + 0.5) * spacing_ft,
                    y: grid.origin.y + (row as f64 + 0.5) * spacing_ft,
                };
                if field.contains(i, centre) {
                    inside.push((col, row, centre));
                }
            }
        }
        let founds: Vec<Option<f64>> = match method {
            Method::Exact => inside.iter().map(|&(_, _, centre)| walk_to(field, centre)).collect(),
            Method::Grid => grid_walks(&ports, &inside, grid.origin, spacing_ft, limit_ft),
        };
        for (&(col, row, _), found) in inside.iter().zip(founds) {
            tally.add(found.map(to_mm));
            let at = row * grid.cols + col;
            grid.cells[at] = better(grid.cells[at], found.map_or(BEYOND, to_mm));
        }
        if tally.cells == 0 {
            // Too small to hold a cell centre: one sample at its inside point, so a
            // cupboard is not silently left out of the coverage.
            if let Some(p) = field.inside_point(i) {
                let found = walk_to(field, p);
                tally.add(found.map(to_mm));
            }
        }
        let Tally { cells, covered, nearest, farthest } = tally;
        if cells == 0 {
            continue;
        }
        out.push(RoomCoverage {
            room: node.room.clone(),
            name: node.name.clone(),
            level_id: node.level_id.clone(),
            cells,
            covered,
            fraction: covered as f64 / cells as f64,
            reached,
            nearest_mm: nearest,
            farthest_mm: farthest,
        });
    }
    out
}

/// What the rooms' coverage adds up to.
fn summarise(rooms: &[RoomCoverage], spacing_mm: f64) -> Summary {
    let mut summary = Summary::default();
    for r in rooms {
        summary.cells += r.cells;
        summary.covered += r.covered;
        summary.rooms += 1;
        match r.covered {
            0 => summary.rooms_none += 1,
            c if c == r.cells => summary.rooms_full += 1,
            _ => summary.rooms_partial += 1,
        }
    }
    summary.fraction = if summary.cells == 0 {
        0.0
    } else {
        summary.covered as f64 / summary.cells as f64
    };
    let cell_sqm = (spacing_mm / 1000.0).powi(2);
    summary.area_sqm = summary.cells as f64 * cell_sqm;
    summary.covered_sqm = summary.covered as f64 * cell_sqm;
    summary
}

/// The distance field from `sources`, laid on a grid over every storey it reaches.
///
/// `Ok(None)` when no rooms have ever been pushed (the adapter's 204). An unreachable
/// room is a result (`reached: false`, `-2` cells), never an error; only a source that
/// is not a room, or an ambiguous id, is the caller's fault.
pub fn assemble_reach(
    state: &AppState,
    project: &str,
    scope: &ReachScope<'_>,
    sources: &[Endpoint],
    limit_mm: f64,
    spacing_mm: f64,
) -> Result<Option<ReachResult>, ServiceError> {
    if sources.is_empty() {
        return Err(ServiceError::Invalid("a reach needs at least one source room".to_string()));
    }
    if sources.len() > MAX_SOURCES {
        return Err(ServiceError::Invalid(format!(
            "{} sources is more than the {MAX_SOURCES} allowed",
            sources.len()
        )));
    }
    let graph_scope = ConnectivityScope {
        building: scope.building,
        milestone: scope.milestone,
        ignore_cuts: scope.ignore_cuts,
        ..Default::default()
    };
    let Some(graph) = build_graph(state, project, &graph_scope)? else {
        return Ok(None);
    };
    let nodes = &graph.nodes;

    let mut picked = Vec::with_capacity(sources.len());
    for (k, s) in sources.iter().enumerate() {
        let node = connectivity::resolve(nodes, s, &format!("source {}", k + 1))?;
        picked.push(Source { node, at: s.at });
    }
    let (limit_ft, spacing_ft) = (limit_mm / MM_PER_FT, spacing_mm / MM_PER_FT);

    let room_outlines = outlines(&graph.rooms, nodes);
    let mut field = routing::field(nodes, &graph.edges, &room_outlines, &picked, limit_ft);

    // The storeys in play: the sources', and any a reached room lies on.
    let mut level_ids: BTreeSet<&str> = picked.iter().map(|s| nodes[s.node].level_id.as_str()).collect();
    for (i, ports) in field.ports.iter().enumerate() {
        if !ports.is_empty() {
            level_ids.insert(nodes[i].level_id.as_str());
        }
    }

    let mut grids = grids_for(&level_ids, nodes, &room_outlines, spacing_ft, spacing_mm)?;
    let rooms = cover_rooms(nodes, &room_outlines, &mut field, &mut grids, limit_ft, spacing_ft, scope.method);
    let summary = summarise(&rooms, spacing_mm);

    let level_info: Vec<&Level> = graph.rooms.levels.iter().collect();
    let levels = grids
        .into_iter()
        .map(|g| ReachLevel {
            name: level_info
                .iter()
                .find(|l| l.id == g.level_id)
                .map_or_else(|| g.level_id.clone(), |l| l.name.clone()),
            level_id: g.level_id,
            origin: g.origin,
            spacing_ft,
            cols: g.cols,
            rows: g.rows,
            distance_mm: g.cells,
        })
        .collect();
    let sources_out = picked
        .iter()
        .zip(&field.starts)
        .map(|(s, at)| ReachSource {
            room: nodes[s.node].room.clone(),
            name: nodes[s.node].name.clone(),
            level_id: nodes[s.node].level_id.clone(),
            at: *at,
        })
        .collect();
    let note = (field.fallbacks > 0).then(|| {
        format!(
            "{} walks inside a room fell back to a straight line because the room has no usable outline, so those \
             distances are not exact",
            field.fallbacks
        )
    });

    Ok(Some(ReachResult {
        schema_version: SCHEMA_VERSION,
        revision: format!("{}.{}", graph.revision, graph.connections.taken_at),
        limit_mm,
        spacing_mm,
        ignore_cuts: scope.ignore_cuts,
        method: scope.method,
        sources: sources_out,
        levels,
        rooms,
        summary,
        note,
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use super::*;
    use crate::connections;
    use crate::contract::{
        DoorPayload, Level, Loop, Model, Opening, Project, Room, RoomPayload, Snapshot, SUPPORTED_DOOR_SCHEMA,
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
            routing: Default::default(),
        }
    }

    /// A 10 ft by 10 ft room whose left edge is `x0`.
    fn square(id: &str, x0: f64) -> Room {
        Room {
            enclosure: None,
            id: id.to_string(),
            name: id.to_string(),
            level_id: "lvl1".to_string(),
            loops: vec![Loop {
                points: vec![
                    Point2D { x: x0, y: 0.0 },
                    Point2D { x: x0 + 10.0, y: 0.0 },
                    Point2D { x: x0 + 10.0, y: 10.0 },
                    Point2D { x: x0, y: 10.0 },
                ],
            }],
            properties: BTreeMap::new(),
        }
    }

    fn door(id: &str, from: &str, to: &str) -> Opening {
        Opening {
            id: id.to_string(),
            level_id: "lvl1".to_string(),
            loops: vec![],
            from_room: Some(from.to_string()),
            to_room: Some(to.to_string()),
            insertion_point: None,
            through_wall_normal: None,
            type_id: "t1".to_string(),
            type_name: "Single".to_string(),
            properties: Default::default(),
            type_properties: Default::default(),
        }
    }

    /// `a | b | c` side by side, then a gap, then `d`; doors a-b and b-c.
    fn state(doors: Vec<Opening>) -> AppState {
        let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), bundle())]), None);
        state
            .set_snapshot(RoomPayload {
                schema_version: SUPPORTED_SCHEMA,
                project: Project { id: "p1".into(), name: "P".into() },
                model: Model { id: "m1".into(), name: "M".into(), source: "revit".into() },
                snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".into() },
                phase: Some("New Construction".into()),
                model_to_shared: None,
                room_boundary: Some(crate::contract::RoomBoundary::Centreline),
                levels: vec![Level { id: "lvl1".into(), name: "Level 1".into(), elevation: 0.0 }],
                rooms: vec![
                    square("a", 0.0),
                    square("b", 10.0),
                    square("c", 20.0),
                    square("d", 40.0),
                ],
            })
            .unwrap();
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

    fn standard() -> AppState {
        state(vec![door("ab", "a", "b"), door("bc", "b", "c")])
    }

    fn source(room: &str, at: Option<(f64, f64)>) -> Endpoint {
        Endpoint { model_id: None, room_id: room.to_string(), at: at.map(|(x, y)| Point2D { x, y }) }
    }

    /// One foot, so a distance in feet reads as the number it is.
    const FT_MM: f64 = 304.8;

    fn read(state: &AppState, sources: &[Endpoint], limit_ft: f64, scope: &ReachScope<'_>) -> ReachResult {
        assemble_reach(state, "p1", scope, sources, limit_ft * FT_MM, FT_MM).unwrap().unwrap()
    }

    fn room_of<'a>(r: &'a ReachResult, id: &str) -> &'a RoomCoverage {
        r.rooms.iter().find(|c| c.room.room_id == id).unwrap()
    }

    /// The value of the cell containing a plan point, in mm.
    fn at(r: &ReachResult, x: f64, y: f64) -> i32 {
        let level = &r.levels[0];
        let col = ((x - level.origin.x) / level.spacing_ft).floor() as usize;
        let row = ((y - level.origin.y) / level.spacing_ft).floor() as usize;
        level.distance_mm[row * level.cols + col]
    }

    /// The point of the whole module: a cell's value is the walk to it, through the door,
    /// and not the straight line. From (5, 5) in `a`, the a-b door sits at (10, 5), so a
    /// cell centred (12.5, 5.5) is 5 ft to the door and 2.55 ft beyond it.
    #[test]
    fn test_a_cell_is_the_walk_through_the_door() {
        let r = read(&standard(), &[source("a", Some((5.0, 5.0)))], 30.0, &ReachScope::default());
        let want = (5.0 + (2.5f64).hypot(0.5)) * FT_MM;
        assert!((at(&r, 12.5, 5.5) as f64 - want).abs() < 2.0, "{} vs {want}", at(&r, 12.5, 5.5));
        // Inside the source's own room it is the straight line from the source.
        let own = ((0.5f64 - 5.0).hypot(5.5 - 5.0)) * FT_MM;
        assert!((at(&r, 0.5, 5.5) as f64 - own).abs() < 2.0);
    }

    /// The `grid` method answers the same question by a different route: the same cells are
    /// in a room or not, nothing is nearer than the exact walk allows, and nothing is much
    /// further than the lattice's own error (8% in the room, a cell's width at the port).
    #[test]
    fn test_the_grid_method_agrees_with_the_exact_one_within_the_lattice_error() {
        let src = [source("a", Some((5.0, 5.0)))];
        let exact = read(&standard(), &src, 30.0, &ReachScope::default());
        let grid = read(&standard(), &src, 30.0, &ReachScope { method: Method::Grid, ..Default::default() });
        assert_eq!(grid.method, Method::Grid);
        let (e, g) = (&exact.levels[0].distance_mm, &grid.levels[0].distance_mm);
        assert_eq!(e.len(), g.len());
        let mut compared = 0;
        for (e, g) in e.iter().zip(g) {
            if *e == NO_ROOM || *g == NO_ROOM {
                assert_eq!(e, g, "which cells are in a room does not depend on the method");
            } else if *e >= 0 && *g >= 0 {
                compared += 1;
                let (e, g) = (*e as f64, *g as f64);
                assert!(g >= e - 1.5 * FT_MM, "a lattice walk cannot be shorter: {g} vs {e}");
                assert!(g <= e * 1.09 + 2.0 * FT_MM, "lattice error is bounded: {g} vs {e}");
            }
        }
        assert!(compared > 100, "most cells were compared, not {compared}");
    }

    #[test]
    fn test_the_method_is_parsed_and_an_unknown_one_refused() {
        assert_eq!(Method::parse(None).unwrap(), Method::Exact);
        assert_eq!(Method::parse(Some(" grid ")).unwrap(), Method::Grid);
        assert!(Method::parse(Some("fast")).is_err());
    }

    /// Past the limit is `-2`, a state and not a number, and a cell in no room is `-1`.
    #[test]
    fn test_beyond_the_limit_and_outside_every_room_are_different_states() {
        let r = read(&standard(), &[source("a", Some((5.0, 5.0)))], 15.0, &ReachScope::default());
        assert_eq!(at(&r, 25.5, 5.5), BEYOND, "c is behind two doors and 15.7 ft away");
        assert_eq!(at(&r, 35.5, 5.5), NO_ROOM, "the gap between c and d is no room");
        assert!(at(&r, 12.5, 5.5) >= 0);
    }

    /// Coverage is per room, and the summary counts rooms fully, partly and not covered.
    #[test]
    fn test_coverage_is_reported_per_room() {
        let r = read(&standard(), &[source("a", Some((5.0, 5.0)))], 15.0, &ReachScope::default());
        assert_eq!(room_of(&r, "a").fraction, 1.0);
        let b = room_of(&r, "b");
        assert!(b.fraction > 0.0 && b.fraction < 1.0, "{}", b.fraction);
        assert!(b.nearest_mm.unwrap() < b.farthest_mm.unwrap());
        assert!(b.farthest_mm.unwrap() as f64 <= 15.0 * FT_MM + 1.0);
        assert!(room_of(&r, "c").reached, "a door lets the walk in, even if no cell is close enough");
        assert_eq!(room_of(&r, "c").covered, 0);
        let s = &r.summary;
        assert_eq!((s.rooms_full, s.rooms_partial), (1, 1));
        assert_eq!(s.rooms_none, 2, "c is too far and d has no door");
        assert_eq!(s.cells, r.rooms.iter().map(|c| c.cells).sum::<usize>());
        assert!((s.fraction - s.covered as f64 / s.cells as f64).abs() < 1e-12);
    }

    /// A room no door, zone or passage reaches is `reached: false`: a finding about what
    /// can be reached, which is what a coverage check exists to surface.
    #[test]
    fn test_a_room_nothing_reaches_is_reported_unreached() {
        let r = read(&standard(), &[source("a", None)], 100.0, &ReachScope::default());
        let d = room_of(&r, "d");
        assert!(!d.reached);
        assert_eq!((d.covered, d.fraction), (0, 0.0));
        assert_eq!(at(&r, 45.5, 5.5), BEYOND);
    }

    /// With two sources a cell takes the nearer.
    #[test]
    fn test_several_sources_take_the_nearest() {
        let one = read(&standard(), &[source("a", Some((5.0, 5.0)))], 100.0, &ReachScope::default());
        let two = read(
            &standard(),
            &[source("a", Some((5.0, 5.0))), source("c", Some((25.0, 5.0)))],
            100.0,
            &ReachScope::default(),
        );
        assert!(at(&two, 25.5, 5.5) < at(&one, 25.5, 5.5));
        assert_eq!(two.sources.len(), 2);
    }

    /// A source point outside its room is moved onto the room, and the answer says where.
    #[test]
    fn test_a_source_outside_its_room_is_moved_in() {
        let r = read(&standard(), &[source("a", Some((-5.0, 5.0)))], 30.0, &ReachScope::default());
        assert!(r.sources[0].at.x.abs() < 1e-6, "{:?}", r.sources[0].at);
    }

    /// A zone's closed wall stops the walk, and `ignore_cuts` opens it again. Rooms b and c
    /// share a wall, there is no door between them, and the zone cuts it.
    #[test]
    fn test_ignore_cuts_opens_a_zones_closed_walls() {
        let dir = std::env::temp_dir().join(format!("roommate-reach-cuts-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok(); // a stale one from a run with the same process id would read as saved data
        std::fs::create_dir_all(&dir).unwrap();
        let state = state(vec![door("ab", "a", "b")]).with_projects_dir(dir.clone());
        let room = |r: &str| connections::RoomInput { room_id: r.to_string(), model_id: Some("m1".to_string()) };
        connections::save(
            &state,
            &dir,
            "p1",
            connections::SaveRequest {
                base: String::new(),
                zones: vec![connections::ZoneInput {
                    id: "z".to_string(),
                    name: "Z".to_string(),
                    kind: connections::ZoneKind::Open,
                    rooms: vec![room("b"), room("c")],
                    disconnects: vec![connections::DisconnectInput { a: room("b"), b: room("c") }],
                    hub: None,
                    note: None,
                }],
                links: vec![],
                routes: None,
                passages: None,
                reaches: None,
            },
        )
        .unwrap();
        let from = [source("a", Some((5.0, 5.0)))];
        let kept = read(&state, &from, 100.0, &ReachScope::default());
        assert!(!room_of(&kept, "c").reached, "the cut keeps the walk out of c");
        let ignored = read(&state, &from, 100.0, &ReachScope { ignore_cuts: true, ..Default::default() });
        assert!(room_of(&ignored, "c").reached, "ignoring the cut opens the b-c wall");
        assert!(ignored.ignore_cuts && !kept.ignore_cuts);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_the_request_is_validated() {
        assert!(params(Some(30_000.0), Some(500.0)).is_ok());
        assert_eq!(params(Some(30_000.0), None).unwrap().1, 1_000.0, "1 m by default");
        assert!(params(None, None).is_err(), "a reach needs a length");
        for bad in [0.0, -1.0, f64::NAN, 2_000_000.0] {
            assert!(params(Some(bad), None).is_err(), "limit {bad}");
        }
        for bad in [50.0, 20_000.0, f64::NAN] {
            assert!(params(Some(30_000.0), Some(bad)).is_err(), "spacing {bad}");
        }
        let s = standard();
        assert!(
            assemble_reach(&s, "p1", &ReachScope::default(), &[], 1000.0, 1000.0).is_err(),
            "no source"
        );
        assert!(
            assemble_reach(&s, "p1", &ReachScope::default(), &[source("nope", None)], 1000.0, 1000.0).is_err(),
            "a source that is not a room is the caller's fault"
        );
    }

    #[test]
    fn test_an_overlapping_cell_takes_the_better_value() {
        assert_eq!(better(NO_ROOM, BEYOND), BEYOND);
        assert_eq!(better(BEYOND, 500), 500);
        assert_eq!(better(500, BEYOND), 500);
        assert_eq!(better(500, 300), 300);
        assert_eq!(better(300, 500), 300);
        assert_eq!(better(BEYOND, NO_ROOM), BEYOND);
    }

    /// Two reads at one spacing share cell edges, so a grid can be compared between them.
    #[test]
    fn test_the_grid_origin_is_a_multiple_of_the_spacing() {
        let r = read(&standard(), &[source("a", None)], 100.0, &ReachScope::default());
        let l = &r.levels[0];
        assert!((l.origin.x / l.spacing_ft).fract().abs() < 1e-9 && (l.origin.y / l.spacing_ft).fract().abs() < 1e-9);
        assert_eq!(l.distance_mm.len(), l.cols * l.rows);
    }
}
