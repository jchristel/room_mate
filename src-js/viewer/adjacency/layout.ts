// The radial layout: rings by hop count, angle simulated, nothing else.
//
// Pure — no canvas, no DOM, no clock — so the rules below are tested rather than
// eyeballed. Each was earned by a picture that went visibly wrong, and a port
// that "tidied" a constant or a force would break the picture with nothing to
// catch it. `layout.test.ts` pins them; change one there first.
//
// ---------------------------------------------------------------------------
// Why the layout simulates ANGLE ONLY
// ---------------------------------------------------------------------------
// A general 2-D force-directed layout would let ring position drift, and ring
// position is the whole message: ring N means "N walls away from the room you
// selected". So each node is pinned to the radius of its hop count and only its
// ANGLE is simulated. That makes the layout a set of independent 1-D problems
// (one per ring), which is both far more stable than free 2-D — no tangling, no
// nodes flung off-screen, no need for a cooling schedule to look sane — and
// cheap enough that settling takes a fraction of a second. It is also why no
// graph library fits: they all simulate free 2-D.
//
// ---------------------------------------------------------------------------
// Why each ring uses the WHOLE circle
// ---------------------------------------------------------------------------
// The first version pulled every node toward the circular mean of its
// neighbours one ring in — including ring 1, whose only neighbour is the focus.
// The focus sits at the centre, where an angle is meaningless (it is stored as
// 0), so every ring-1 node was sprung toward angle 0 and the entire graph
// collapsed into a fan on the right-hand side, with only a capped repulsion
// holding the dots apart. The rings were four times longer than the picture
// drawn on them. Two changes fix it:
//
//   - **Ring 1 is not attracted to anything.** There is nothing at the centre to
//     sit beside. Its target spacing is `2π/n`, i.e. even over the full circle,
//     which is also the seeded state — so ring 1 starts correct and stays there.
//   - **Deeper rings keep the parent spring**, because "sits next to its
//     neighbour" is real information there, and their spread force degrades to a
//     collision guard (`2π/n` capped at what a label needs). They still fill the
//     circle, but they get it from their parents being spread rather than from a
//     force of their own — so a room's children stay visibly its children.
//
// Nodes are also SEEDED in plan order (their real bearing from the focus), so a
// ring's cyclic order matches the way the rooms sit around it on the floor plan.
// The angles are even, not the bearings themselves: two rooms in the same
// direction would seed on top of each other, and two nodes at an identical angle
// cannot be separated by the spread force (it pushes both the same way).

import type { GraphView, ViewNode } from "./view.js";

export const TAU = Math.PI * 2;
export const NODE_R = 7; // node dot radius, CSS px
export const FOCUS_R = 10; // the selected node reads larger
// Angular spring toward the neighbours in the ring below, an even-spacing force
// between same-ring nodes, and enough damping that the whole thing is visibly
// still within a second.
export const ATTRACT = 0.06;
export const SPREAD = 0.35;
export const DAMPING = 0.82;
/** Below this peak angular velocity (radians/frame) the picture is not visibly
 *  moving, so the loop stops. A force sim left running is a permanent 60fps
 *  repaint of a static image — on the same page as a 5,000-room plan, that is
 *  not a cost worth paying for nothing. */
export const SETTLE_EPS = 0.0006;
/** Margin inside the canvas for the outermost ring's labels, CSS px. */
const LABEL_MARGIN = 28;

/** A laid-out node. `x`/`y` are filled by `positions` for drawing and picking. */
export interface Placed {
  node: ViewNode;
  hop: number;
  angle: number;
  vel: number;
  x: number;
  y: number;
  r: number;
}

export interface PlacedEdge {
  a: Placed;
  b: Placed;
  weight: number;
}

export interface Layout {
  placed: Placed[];
  /** View edges with both ends placed, resolved once per layout instead of
   *  filtered every frame: at depth 1 on a big level that is a handful of edges
   *  out of thousands, and the frame loop should not keep rediscovering it. */
  edges: PlacedEdge[];
  /** The heaviest of those — the thickness scale's top end, so it means
   *  something at every depth and in both granularities. An aggregated area
   *  edge is the sum of its rooms' and would otherwise flatten every room edge. */
  maxWeight: number;
  /** The deepest ring actually present, which is what scales the rings. */
  maxHop: number;
}

/** Shortest angular difference b - a, wrapped to (-π, π]. Everything angular
 *  goes through this: without it a node at 359° and one at 1° look 358° apart
 *  and shove each other the wrong way round the ring. */
export function angleDelta(a: number, b: number): number {
  let d = (b - a) % TAU;
  if (d > Math.PI) d -= TAU;
  if (d <= -Math.PI) d += TAU;
  return d;
}

/** The counter-clockwise gap from a to b, in [0, 2π). What "the space between
 *  me and the next node" means, and not `|angleDelta|` — two nodes 350° apart
 *  the short way are 10° apart, but the gap on one side of each is still 350°. */
export function gapCCW(a: number, b: number): number {
  return (((b - a) % TAU) + TAU) % TAU;
}

/** Keep an accumulating angle inside (-π, π]. The spread force sorts each ring
 *  by angle to find every node's two neighbours, and a ring whose angles have
 *  drifted past 2π sorts into the wrong order. */
export function normAngle(a: number): number {
  return angleDelta(0, a);
}

/**
 * Hop count from the focus, breadth-first, capped at `depth` (Infinity is
 * legal: every ring the graph has). The cap is not a performance guard — it is
 * the readability default. The full graph of a hospital level is a hairball and
 * must never be what opens, but asking for it is a legitimate question.
 */
export function reachable(view: GraphView, focusId: string | null, depth: number): Map<string, number> {
  const hops = new Map<string, number>();
  if (!focusId || !view.nodesById.has(focusId)) return hops;
  hops.set(focusId, 0);
  let frontier = [focusId];
  for (let h = 1; h <= depth; h++) {
    const next: string[] = [];
    for (const id of frontier) {
      for (const nb of view.neighbours.get(id) ?? []) {
        if (hops.has(nb.id)) continue;
        hops.set(nb.id, h);
        next.push(nb.id);
      }
    }
    frontier = next;
    // Also what terminates an unbounded `depth`: the graph is finite, so the
    // frontier always empties.
    if (!frontier.length) break;
  }
  return hops;
}

/** The seeded layout: every reachable node on its ring, at even angles in plan
 *  order, with velocity zero. Ring 1 is already at its equilibrium here; deeper
 *  rings relax toward their parents from it. */
export function seed(view: GraphView, focusId: string | null, depth: number): Layout {
  const hops = reachable(view, focusId, depth);
  const focus = focusId ? view.nodesById.get(focusId) : undefined;
  const byHop = new Map<number, ViewNode[]>();
  // Bearing from the focus on the actual floor plan. Precomputed rather than
  // called from the comparator, so the sort cost stays n log n and a node's key
  // cannot change mid-sort.
  const bearing = new Map<string, number>();
  for (const [id, hop] of hops) {
    const node = view.nodesById.get(id)!;
    let ring = byHop.get(hop);
    if (!ring) byHop.set(hop, (ring = []));
    ring.push(node);
    const c = node.centroid;
    const o = focus?.centroid;
    bearing.set(id, c && o ? Math.atan2(c.y - o.y, c.x - o.x) : 0);
  }

  const placed: Placed[] = [];
  let maxHop = 0;
  for (const [hop, ring] of byHop) {
    maxHop = Math.max(maxHop, hop);
    // Plan order, with the id as a tie-break so a ring of co-located rooms (or
    // one with no geometry at all) still opens the same way twice.
    ring.sort((a, b) => bearing.get(a.id)! - bearing.get(b.id)! || (a.id < b.id ? -1 : 1));
    ring.forEach((node, i) => {
      placed.push({
        node,
        hop,
        angle: hop === 0 ? 0 : normAngle((i / ring.length) * TAU),
        vel: 0,
        x: 0,
        y: 0,
        r: hop === 0 ? FOCUS_R : NODE_R,
      });
    });
  }

  const index = new Map(placed.map((p) => [p.node.id, p]));
  const edges: PlacedEdge[] = [];
  let maxWeight = 1;
  for (const e of view.edges) {
    const a = index.get(e.a);
    const b = index.get(e.b);
    if (!a || !b) continue;
    edges.push({ a, b, weight: e.weight });
    maxWeight = Math.max(maxWeight, e.weight);
  }
  return { placed, edges, maxWeight, maxHop };
}

/** Ring radius, scaled to the deepest ring PRESENT rather than the depth
 *  requested: a room whose graph runs out after two hops should not be drawn
 *  inside the innermost third of the panel because the control says 6. */
export function ringRadius(hop: number, maxHop: number, width: number, height: number): number {
  const usable = Math.min(width, height) / 2 - LABEL_MARGIN;
  return (Math.max(usable, 40) / Math.max(maxHop, 1)) * hop;
}

/** Minimum angular separation that keeps two dots (and their labels) from
 *  colliding at this radius — so an inner ring spreads its nodes further apart
 *  in angle than an outer one, which is correct. */
function labelSep(radius: number): number {
  return (NODE_R * 5) / Math.max(radius, 1);
}

/**
 * One simulation step, in place. Returns the peak angular velocity so the
 * caller can decide whether the picture is still moving. `radius` is the ring
 * radius for a hop, which the spacing force needs and only the canvas knows.
 */
export function step(
  placed: Placed[],
  neighbours: GraphView["neighbours"],
  radius: (hop: number) => number,
): number {
  const byHop = new Map<number, Placed[]>();
  for (const p of placed) {
    let ring = byHop.get(p.hop);
    if (!ring) byHop.set(p.hop, (ring = []));
    ring.push(p);
  }
  const index = new Map(placed.map((p) => [p.node.id, p]));
  let peak = 0;

  for (const [hop, ring] of byHop) {
    if (hop === 0) continue; // the focus is anchored at the centre by definition
    const n = ring.length;
    const even = Math.min(TAU / n, Math.PI);
    // Ring 1 spreads over the whole circle (`even`); deeper rings only need
    // enough room not to collide, so their spacing target is whichever of the
    // two is smaller — a ring of 40 nodes cannot give each one a label's worth
    // of angle, and demanding it would just fight the parent spring forever.
    const ideal = hop === 1 ? even : Math.min(even, labelSep(radius(hop)));

    // Sorted by angle, so every node's two angular neighbours are its
    // neighbours in this array. That makes the spread force O(n log n) rather
    // than all-pairs O(n²) — which is what lets the depth control offer "All"
    // on a level with thousands of rooms.
    const sorted = ring.slice().sort((a, b) => a.angle - b.angle);

    for (let i = 0; i < n; i++) {
      const p = sorted[i]!;
      let force = 0;

      // Spring toward the circular mean of this node's neighbours one hop in,
      // weighted by shared wall length: a room joined by a 4m wall is pulled to
      // sit beside its neighbour harder than one joined by a 200mm reveal.
      // Skipped for ring 1 — see the header.
      if (hop > 1) {
        let sx = 0;
        let sy = 0;
        for (const nb of neighbours.get(p.node.id) ?? []) {
          const other = index.get(nb.id);
          if (!other || other.hop !== hop - 1) continue;
          const w = Math.max(nb.weight, 0.1);
          sx += Math.cos(other.angle) * w;
          sy += Math.sin(other.angle) * w;
        }
        if (sx !== 0 || sy !== 0) force += ATTRACT * angleDelta(p.angle, Math.atan2(sy, sx));
      }

      // Even-spacing force: pushed away from whichever angular neighbour is
      // closer than `ideal`, toward the roomier side. At rest every gap in the
      // ring is `ideal`, so a ring left to itself ends up evenly distributed
      // rather than piled up under one parent.
      if (n > 1) {
        const prev = sorted[(i - 1 + n) % n]!;
        const next = sorted[(i + 1) % n]!;
        const gapIn = gapCCW(prev.angle, p.angle);
        const gapOut = gapCCW(p.angle, next.angle);
        force += SPREAD * (Math.max(0, ideal - gapIn) - Math.max(0, ideal - gapOut));
      }

      p.vel = (p.vel + force) * DAMPING;
      peak = Math.max(peak, Math.abs(p.vel));
    }
  }

  for (const p of placed) p.angle = normAngle(p.angle + p.vel);
  return peak;
}

/** Screen positions for the current angles, centred in a `width`×`height` box. */
export function positions(layout: Layout, width: number, height: number): void {
  const cx = width / 2;
  const cy = height / 2;
  for (const p of layout.placed) {
    const r = ringRadius(p.hop, layout.maxHop, width, height);
    p.x = cx + Math.cos(p.angle) * r;
    p.y = cy + Math.sin(p.angle) * r;
  }
}
