// Editing the blueprint's box (its reference outline) as pure functions over a ring of nodes.
//
// **Only the box's SHAPE is decided here.** Where the items go once the box has changed is the
// server's (`blueprint_reshape`, behind `POST .../blueprints/reshape`): an item keeps its place in
// the room and is written again as a recipe against the new edges. This file turns the person's
// gestures (drag a node, double-click a line, double-click a node, align a side with a wall) into
// the ring to send, and squares the blueprint up, which is the one thing that needs no server
// because a recipe is a distance along an edge and a turn of the whole outline changes none.
//
// Millimetres, counter-clockwise, Y up: the same plan frame as `geometry.ts`.

import type { Pt } from "./geometry.js";

/** A box has at least a rectangle's four nodes (the server refuses fewer). */
export const MIN_NODES = 4;

/** Snap a dragged node so the lines it ends are horizontal or vertical: for each of its two
 *  neighbours, take that neighbour's x when the pointer is within `tol` of it (the line is then
 *  vertical) and its y likewise (horizontal). Each axis snaps to the nearest neighbour. */
export function snapNode(ring: readonly Pt[], i: number, p: Pt, tol: number): Pt {
  const n = ring.length;
  const neighbours = [ring[(i + n - 1) % n], ring[(i + 1) % n]].filter((q): q is Pt => !!q);
  let [x, y] = p;
  let bx = tol;
  let by = tol;
  for (const q of neighbours) {
    if (Math.abs(p[0] - q[0]) < bx) {
      bx = Math.abs(p[0] - q[0]);
      x = q[0];
    }
    if (Math.abs(p[1] - q[1]) < by) {
      by = Math.abs(p[1] - q[1]);
      y = q[1];
    }
  }
  return [x, y];
}

export const moveNode = (ring: readonly Pt[], i: number, to: Pt): Pt[] => ring.map((p, j) => (j === i ? to : p));

/** The point of segment `a`-`b` nearest `p`, and how far `p` is from it. */
export function nearestOnSegment(p: Pt, a: Pt, b: Pt): { at: Pt; dist: number } {
  const [dx, dy] = [b[0] - a[0], b[1] - a[1]];
  const l2 = dx * dx + dy * dy;
  const t = l2 === 0 ? 0 : Math.min(1, Math.max(0, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2));
  const at: Pt = [a[0] + dx * t, a[1] + dy * t];
  return { at, dist: Math.hypot(p[0] - at[0], p[1] - at[1]) };
}

/** Put a node on edge `edge` at the point of it nearest `p`. */
export function insertNode(ring: readonly Pt[], edge: number, p: Pt): Pt[] {
  const a = ring[edge];
  const b = ring[(edge + 1) % ring.length];
  if (!a || !b) return [...ring];
  const { at } = nearestOnSegment(p, a, b);
  return [...ring.slice(0, edge + 1), at, ...ring.slice(edge + 1)];
}

/** Take node `i` away, or null when that would leave fewer than the minimum. */
export const removeNode = (ring: readonly Pt[], i: number): Pt[] | null =>
  ring.length <= MIN_NODES ? null : ring.filter((_, j) => j !== i);

/** Move both ends of box edge `edge` onto the line through `a` and `b` (a wall of the real room), each
 *  where it lands when dropped straight onto that line. The neighbouring sides follow their shared nodes. */
export function alignEdge(ring: readonly Pt[], edge: number, a: Pt, b: Pt): Pt[] {
  const [dx, dy] = [b[0] - a[0], b[1] - a[1]];
  const l2 = dx * dx + dy * dy;
  if (l2 === 0) return [...ring];
  const onto = (p: Pt): Pt => {
    const t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2;
    return [a[0] + dx * t, a[1] + dy * t];
  };
  const j = (edge + 1) % ring.length;
  return ring.map((p, k) => (k === edge || k === j ? onto(p) : p));
}

/** The direction the walls mostly run in, degrees within (-45, 45]: the length-weighted mean of the
 *  edge angles taken modulo a quarter turn (the server's `dominant_angle_deg`). */
export function dominantAngle(ring: readonly Pt[]): number {
  let s = 0;
  let c = 0;
  ring.forEach((a, i) => {
    const b = ring[(i + 1) % ring.length] as Pt;
    const len = Math.hypot(b[0] - a[0], b[1] - a[1]);
    const t = 4 * Math.atan2(b[1] - a[1], b[0] - a[0]);
    s += len * Math.sin(t);
    c += len * Math.cos(t);
  });
  if (Math.hypot(s, c) < 1e-9) return 0;
  const deg = (Math.atan2(s, c) * 180) / Math.PI / 4;
  return Math.abs(deg) < 0.001 ? 0 : deg;
}

export function rotatePoint(p: Pt, deg: number): Pt {
  const [sin, cos] = [Math.sin((deg * Math.PI) / 180), Math.cos((deg * Math.PI) / 180)];
  return [p[0] * cos - p[1] * sin, p[0] * sin + p[1] * cos];
}
