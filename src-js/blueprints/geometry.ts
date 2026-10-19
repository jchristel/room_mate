// Plan geometry for the blueprint canvas: the outline's edges and where a recipe puts an
// item. A port of `src/service/blueprint_geometry.rs`'s `edges`, `Edge::point` and
// `depth_at`, because the canvas draws an edit before the server has seen it.
//
// **Two copies of "where does this item go" is how a preview and a result disagree**
// (plan decision 2), so this file does ONLY what editing a blueprint in its OWN room needs.
// Placing a blueprint in ANOTHER room is the server's resolver, never this file.
//
// Millimetres throughout; the outline is wound counter-clockwise, so an edge's left-hand
// normal points into the room.

import type { BlueprintItem, Point } from "./types.js";

export type Pt = readonly [number, number];

export interface Edge {
  a: Pt;
  b: Pt;
  /** Unit direction from `a` to `b`. */
  t: Pt;
  /** Unit normal pointing into the room. */
  n: Pt;
  len: number;
}

export const pt = (p: Point): Pt => [p.x, p.y];

export function edges(ring: readonly Pt[]): Edge[] {
  return ring.map((a, i) => {
    const b = ring[(i + 1) % ring.length] as Pt;
    const len = Math.hypot(b[0] - a[0], b[1] - a[1]);
    const t: Pt = len === 0 ? [1, 0] : [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
    return { a, b, t, n: [-t[1], t[0]], len };
  });
}

/** The point `along` mm from the edge's start and `off` mm into the room. Not clamped. */
export function pointOn(e: Edge, along: number, off: number): Pt {
  return [e.a[0] + e.t[0] * along + e.n[0] * off, e.a[1] + e.t[1] * along + e.n[1] * off];
}

/** How far the room extends from edge `i`, at `foot`, along its inward normal: the distance to the
 *  next wall in front, or the edge's own length when nothing is. */
export function depthAt(ring: readonly Pt[], i: number, foot: Pt): number {
  const es = edges(ring);
  const e = es[i];
  if (!e) return 0;
  const origin: Pt = [foot[0] + e.n[0] * 0.5, foot[1] + e.n[1] * 0.5];
  let best = Infinity;
  es.forEach((f, j) => {
    if (j === i) return;
    const d: Pt = [f.b[0] - f.a[0], f.b[1] - f.a[1]];
    const den = e.n[0] * d[1] - e.n[1] * d[0];
    if (Math.abs(den) < 1e-9) return;
    const w: Pt = [f.a[0] - origin[0], f.a[1] - origin[1]];
    const s = (w[0] * d[1] - w[1] * d[0]) / den;
    const u = (w[0] * e.n[1] - w[1] * e.n[0]) / den;
    if (s > 0 && u >= -1e-6 && u <= 1 + 1e-6) best = Math.min(best, s);
  });
  return Number.isFinite(best) ? best : e.len;
}

export interface Frame {
  centre: Pt;
  /** Radians: the edge's inward normal turned by the item's facing. */
  angle: number;
}

/** Where an item is in its own blueprint's frame, or null if its edge is not in the outline. */
export function frameOf(item: BlueprintItem, ring: readonly Pt[]): Frame | null {
  const e = edges(ring)[item.placement.edge];
  if (!e) return null;
  return {
    centre: pointOn(e, item.placement.along_mm, item.placement.off_mm),
    angle: Math.atan2(e.n[1], e.n[0]) + (item.placement.facing_deg * Math.PI) / 180,
  };
}

/** The four corners of an item's footprint, counter-clockwise. `size` is along the facing, then across. */
export function corners(frame: Frame, size: readonly [number, number]): Pt[] {
  const f: Pt = [Math.cos(frame.angle), Math.sin(frame.angle)];
  const v: Pt = [-f[1], f[0]];
  const [hu, hv] = [size[0] / 2, size[1] / 2];
  return (
    [
      [1, 1],
      [-1, 1],
      [-1, -1],
      [1, -1],
    ] as const
  ).map(([sx, sy]) => [frame.centre[0] + sx * hu * f[0] + sy * hv * v[0], frame.centre[1] + sx * hu * f[1] + sy * hv * v[1]] as Pt);
}

export function pointInPolygon(p: Pt, poly: readonly Pt[]): boolean {
  let inside = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const [a, b] = [poly[i] as Pt, poly[j] as Pt];
    if (a[1] > p[1] !== b[1] > p[1] && p[0] < ((b[0] - a[0]) * (p[1] - a[1])) / (b[1] - a[1]) + a[0]) inside = !inside;
  }
  return inside;
}

export interface Bounds {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

export function boundsOf(points: readonly Pt[]): Bounds {
  const xs = points.map((p) => p[0]);
  const ys = points.map((p) => p[1]);
  return { minX: Math.min(...xs), minY: Math.min(...ys), maxX: Math.max(...xs), maxY: Math.max(...ys) };
}
