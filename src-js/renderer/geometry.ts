// Pure geometry. No DOM, no GL, no globals — moved verbatim in behaviour from
// `static/index.html`'s "pure geometry helpers" block.
//
// THE COORDINATE RULE, because everything here depends on it: payload data is
// **Y-up**, and every function below returns **Y-down** ("flipped") values. The
// SVG viewBox, `zone.view`, the cull, the areas overlay and the pick all live in
// the flipped space. When the GL renderer lands it keeps the flip and inverts Y
// in the projection matrix instead — dropping the flip because GL clip space is
// Y-up would silently move five other subsystems.

import type { Extent, Loop, Point2D, Rect, Room, Size } from "./types.js";

/** Y-up -> Y-down. The one place the sign flips. */
export function flip(p: Point2D): Point2D {
  return { x: p.x, y: -p.y };
}

/** Extent of every loop of every room, or `null` when nothing is drawable. */
export function bounds(rooms: readonly Room[]): Extent | null {
  let minX = Infinity,
    minY = Infinity,
    maxX = -Infinity,
    maxY = -Infinity;
  for (const r of rooms) {
    for (const loop of r.loops ?? []) {
      for (const raw of loop.points) {
        const p = flip(raw);
        if (p.x < minX) minX = p.x;
        if (p.y < minY) minY = p.y;
        if (p.x > maxX) maxX = p.x;
        if (p.y > maxY) maxY = p.y;
      }
    }
  }
  if (!isFinite(minX)) return null;
  return { minX, minY, maxX, maxY };
}

export function loopBox(loop: Loop): Size {
  let minX = Infinity,
    minY = Infinity,
    maxX = -Infinity,
    maxY = -Infinity;
  for (const raw of loop.points) {
    const p = flip(raw);
    if (p.x < minX) minX = p.x;
    if (p.y < minY) minY = p.y;
    if (p.x > maxX) maxX = p.x;
    if (p.y > maxY) maxY = p.y;
  }
  return { w: maxX - minX, h: maxY - minY };
}

/**
 * A room's bounding box, computed from the OUTER loop's points and never from
 * `getBBox()` — which forces a synchronous layout and, at a few thousand rooms
 * on every pan frame, is the difference between a viewer and a slideshow.
 */
export function roomBBox(room: Room): Extent {
  let minX = Infinity,
    minY = Infinity,
    maxX = -Infinity,
    maxY = -Infinity;
  for (const raw of room.loops?.[0]?.points ?? []) {
    const p = flip(raw);
    if (p.x < minX) minX = p.x;
    if (p.y < minY) minY = p.y;
    if (p.x > maxX) maxX = p.x;
    if (p.y > maxY) maxY = p.y;
  }
  return { minX, minY, maxX, maxY };
}

/** The `points` attribute for an SVG `<polygon>`. */
export function pointsAttr(loop: Loop): string {
  return loop.points
    .map((raw) => {
      const p = flip(raw);
      return `${p.x},${p.y}`;
    })
    .join(" ");
}

/**
 * Area-weighted centroid of a ring, with a plain vertex average as the fallback
 * for a degenerate one. The fallback is load-bearing, not defensive: a
 * zero-area ring divides by zero and would place a label at NaN,NaN, which
 * renders nothing and looks like a missing label rather than bad geometry.
 */
export function centroid(loop: Loop): Point2D {
  const pts = loop.points.map(flip);
  let a = 0,
    cx = 0,
    cy = 0;
  for (let i = 0; i < pts.length; i++) {
    const p = pts[i]!;
    const q = pts[(i + 1) % pts.length]!;
    const cross = p.x * q.y - q.x * p.y;
    a += cross;
    cx += (p.x + q.x) * cross;
    cy += (p.y + q.y) * cross;
  }
  if (Math.abs(a) < 1e-9) {
    const n = pts.length;
    return {
      x: pts.reduce((s, p) => s + p.x, 0) / n,
      y: pts.reduce((s, p) => s + p.y, 0) / n,
    };
  }
  a *= 0.5;
  return { x: cx / (6 * a), y: cy / (6 * a) };
}

/**
 * Padded bounds framing a level's rooms. Both the on-screen refit and the SVG
 * export frame to this, so the two can never disagree about what "fitted"
 * means. `null` when the rooms have no drawable geometry.
 */
export function fittedBounds(rooms: readonly Room[]): Rect | null {
  const b = bounds(rooms);
  if (!b) return null;
  const w = b.maxX - b.minX;
  const h = b.maxY - b.minY;
  // `|| 1` catches a single-point level, where both extents are 0 and an
  // unpadded viewBox would have zero width.
  const pad = Math.max(w, h) * 0.04 || 1;
  return { x: b.minX - pad, y: b.minY - pad, w: w + 2 * pad, h: h + 2 * pad };
}
