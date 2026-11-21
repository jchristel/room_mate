//! The walk over a grid laid on each room (`grid_walks`, `cover_rooms`): the lattice method's cells, the room coverage it adds up to and
//! the summary. The exact method is `routing::field`.

use super::*;

/// A cell waiting in the grid search, cheapest first.
#[derive(PartialEq)]
pub(super) struct Pending {
    pub(super) cost: f64,
    pub(super) cell: usize,
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
pub(super) fn grid_walks(
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
pub(super) struct Tally {
    pub(super) cells: usize,
    pub(super) covered: usize,
    pub(super) nearest: Option<i32>,
    pub(super) farthest: Option<i32>,
}

impl Tally {
    /// One sample: its walk in mm when it is within the limit.
    pub(super) fn add(&mut self, mm: Option<i32>) {
        self.cells += 1;
        if let Some(mm) = mm {
            self.covered += 1;
            self.nearest = Some(self.nearest.map_or(mm, |n| n.min(mm)));
            self.farthest = Some(self.farthest.map_or(mm, |f| f.max(mm)));
        }
    }
}

pub(super) struct Grid {
    pub(super) level_id: String,
    pub(super) origin: Point2D,
    pub(super) cols: usize,
    pub(super) rows: usize,
    pub(super) cells: Vec<i32>,
}

/// A room's bounding box from its outer ring, in feet: `(min_x, min_y, max_x, max_y)`.
pub(super) fn bounds_of(room: &crate::contract::Room) -> Option<(f64, f64, f64, f64)> {
    let ring = room.loops.first()?.points.as_slice();
    let first = ring.first()?;
    Some(ring.iter().fold((first.x, first.y, first.x, first.y), |(x0, y0, x1, y1), p| {
        (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y))
    }))
}

/// Merge a cell's candidate into what it holds: any distance beats "beyond", which
/// beats "no room", and a shorter distance beats a longer.
pub(super) fn better(held: i32, candidate: i32) -> i32 {
    match (held, candidate) {
        (NO_ROOM, c) => c,
        (BEYOND, c) if c >= 0 => c,
        (h, c) if h >= 0 && c >= 0 => h.min(c),
        (h, _) => h,
    }
}

/// One grid per storey in play, sized to the rooms on it, or the refusal when the lot would
/// be more cells than an answer may hold.
pub(super) fn grids_for(
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
pub(super) fn cover_rooms(
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
pub(super) fn summarise(rooms: &[RoomCoverage], spacing_mm: f64) -> Summary {
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
