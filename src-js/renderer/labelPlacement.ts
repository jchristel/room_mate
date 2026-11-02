// Where a room's label goes, and how big it may be, from the room's REAL outline.
//
// It used to go at the area centroid and be sized by the bounding box. For a T or
// L shape the centroid can lie in the notch, outside the room, and the bounding box
// is far wider than the arm the text sits in, so the label spilled over neighbours.
//
// **One function, both painters.** The GL labels and the SVG export call this and
// nothing else decides where a label is, so the file cannot disagree with the screen.
//
// The rule: start at the POLE OF INACCESSIBILITY (the point inside the room, holes
// excluded, farthest from every edge -- the thickest part), centre the text on the
// open stretch of the room around it, and shrink the font until the text block
// lies inside the outline. A circle of radius `d` around the pole is wholly inside,
// so a small enough font always fits: a label is never dropped, as it never was.
//
// Y-down ("flipped") space, like everything in `geometry.ts`.

import { flip } from "./geometry.js";
import type { Point2D, Room } from "./types.js";

/** The mono glyph's width over its height. */
export const GLYPH_ASPECT = 0.6;
/** A line's height over the font size, as the painters set stacked lines. */
const LINE_HEIGHT = 1.2;

export interface LabelPlacement {
  /** The centre of the text block. */
  x: number;
  y: number;
  fontSize: number;
}

type Ring = readonly Point2D[];

function rings(room: Room): Ring[] {
  return (room.loops ?? []).filter((l) => l.points.length >= 3).map((l) => l.points.map(flip));
}

/** Signed distance from `p` to the polygon (rings together, even-odd): positive inside. */
function signedDistance(p: Point2D, all: readonly Ring[]): number {
  let inside = false;
  let best = Infinity;
  for (const ring of all) {
    for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
      const a = ring[i]!;
      const b = ring[j]!;
      if (a.y > p.y !== b.y > p.y && p.x < ((b.x - a.x) * (p.y - a.y)) / (b.y - a.y) + a.x) inside = !inside;
      const d = segmentDistanceSq(p, a, b);
      if (d < best) best = d;
    }
  }
  const dist = Math.sqrt(best);
  return inside ? dist : -dist;
}

function segmentDistanceSq(p: Point2D, a: Point2D, b: Point2D): number {
  let x = a.x;
  let y = a.y;
  let dx = b.x - x;
  let dy = b.y - y;
  if (dx !== 0 || dy !== 0) {
    const t = ((p.x - x) * dx + (p.y - y) * dy) / (dx * dx + dy * dy);
    if (t > 1) {
      x = b.x;
      y = b.y;
    } else if (t > 0) {
      x += dx * t;
      y += dy * t;
    }
  }
  dx = p.x - x;
  dy = p.y - y;
  return dx * dx + dy * dy;
}

/** The pole of inaccessibility of the rings, to `precision`, by quadtree search. */
export function poleOfInaccessibility(all: readonly Ring[], precision: number): { x: number; y: number; d: number } {
  const outer = all[0]!;
  let minX = Infinity,
    minY = Infinity,
    maxX = -Infinity,
    maxY = -Infinity;
  for (const p of outer) {
    if (p.x < minX) minX = p.x;
    if (p.y < minY) minY = p.y;
    if (p.x > maxX) maxX = p.x;
    if (p.y > maxY) maxY = p.y;
  }
  const size = Math.min(maxX - minX, maxY - minY);
  let h = size / 2;
  if (size === 0) return { x: minX, y: minY, d: 0 };

  interface Cell {
    x: number;
    y: number;
    h: number;
    d: number;
    max: number;
  }
  const cell = (x: number, y: number, half: number): Cell => {
    const d = signedDistance({ x, y }, all);
    return { x, y, h: half, d, max: d + half * Math.SQRT2 };
  };

  const queue: Cell[] = [];
  for (let x = minX; x < maxX; x += size) for (let y = minY; y < maxY; y += size) queue.push(cell(x + h, y + h, h));

  // The bounding-box centre is a good first guess for a convex room.
  let best = cell((minX + maxX) / 2, (minY + maxY) / 2, 0);
  for (const c of queue) if (c.d > best.d) best = c;

  while (queue.length) {
    queue.sort((a, b) => a.max - b.max);
    const c = queue.pop()!;
    if (c.d > best.d) best = c;
    if (c.max - best.d <= precision) continue;
    h = c.h / 2;
    queue.push(cell(c.x - h, c.y - h, h), cell(c.x + h, c.y - h, h), cell(c.x - h, c.y + h, h), cell(c.x + h, c.y + h, h));
  }
  return { x: best.x, y: best.y, d: best.d };
}

/**
 * The stretch of the horizontal line `y` that is inside the room and contains `x`:
 * `[left, right]`, or `null` when `x` is not inside at that height.
 */
function openStretch(x: number, y: number, all: readonly Ring[]): [number, number] | null {
  const xs: number[] = [];
  for (const ring of all) {
    for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
      const a = ring[i]!;
      const b = ring[j]!;
      if (a.y > y !== b.y > y) xs.push(a.x + ((y - a.y) / (b.y - a.y)) * (b.x - a.x));
    }
  }
  xs.sort((a, b) => a - b);
  // Crossings pair up (even-odd): [0,1] is inside, [2,3] is inside...
  for (let i = 0; i + 1 < xs.length; i += 2) {
    if (xs[i]! <= x && x <= xs[i + 1]!) return [xs[i]!, xs[i + 1]!];
  }
  return null;
}

/**
 * Where `room`'s label goes. `longest` is the character count of its longest line and
 * `lines` how many there are (stacked lines are 0.7 of the first, as the painters
 * draw them); `baseFont` is the level's ceiling on the font. `null` when the room has
 * no usable outline.
 */
export function placeLabel(room: Room, longest: number, lines: number, baseFont: number): LabelPlacement | null {
  // Remembered per room object and per question: the search is the cost (about a
  // quarter of a millisecond a room), and a repaint asks the same thing again for every
  // room -- a layer toggle, a zone change -- while the rooms themselves have not moved.
  const key = `${longest}|${lines}|${baseFont}`;
  const known = remembered.get(room);
  if (known?.key === key) return known.placement;
  const placement = compute(room, longest, lines, baseFont);
  remembered.set(room, { key, placement });
  return placement;
}

const remembered = new WeakMap<Room, { key: string; placement: LabelPlacement | null }>();

function compute(room: Room, longest: number, lines: number, baseFont: number): LabelPlacement | null {
  const all = rings(room);
  if (all.length === 0) return null;
  const outer = all[0]!;
  let minX = Infinity,
    minY = Infinity,
    maxX = -Infinity,
    maxY = -Infinity;
  for (const p of outer) {
    if (p.x < minX) minX = p.x;
    if (p.y < minY) minY = p.y;
    if (p.x > maxX) maxX = p.x;
    if (p.y > maxY) maxY = p.y;
  }
  const extent = Math.max(maxX - minX, maxY - minY);
  if (!(extent > 0)) return null;

  const pole = poleOfInaccessibility(all, extent * 0.005);
  const stack = 1 + 0.7 * (lines - 1);
  const blockWidth = (font: number) => font * GLYPH_ASPECT * Math.max(longest, 1);
  const blockHeight = (font: number) => font * LINE_HEIGHT * stack;

  let font = baseFont;
  // 0.9 per step from the ceiling down; 60 steps is a factor of 500, and the circle
  // of radius `pole.d` guarantees a fit well before that.
  for (let step = 0; step < 60; step++, font *= 0.9) {
    const w = blockWidth(font) * 1.05;
    const h = blockHeight(font) * 1.05;
    // The block's top, middle and bottom lines must all have room around the pole;
    // their common stretch is where the text is centred.
    let left = -Infinity;
    let right = Infinity;
    let ok = true;
    for (const dy of [-h / 2, 0, h / 2]) {
      const s = openStretch(pole.x, pole.y + dy, all);
      if (!s) {
        ok = false;
        break;
      }
      left = Math.max(left, s[0]);
      right = Math.min(right, s[1]);
    }
    if (!ok || right - left < w) continue;
    // Centre on the open stretch, but never so far from the pole that a corner
    // leaves the room: the block must still clear every edge at its new centre.
    const cx = (left + right) / 2;
    if (clears(cx, pole.y, w, h, all)) return { x: cx, y: pole.y, fontSize: font };
    if (clears(pole.x, pole.y, w, h, all)) return { x: pole.x, y: pole.y, fontSize: font };
  }
  // Never reached for a real room: the block's half-diagonal within the inscribed circle.
  const diagonal = Math.hypot(GLYPH_ASPECT * Math.max(longest, 1), LINE_HEIGHT * stack);
  return { x: pole.x, y: pole.y, fontSize: Math.max((2 * pole.d) / diagonal, 1e-6) };
}

/** Whether the w x h block centred on (x, y) lies inside the room: its four corners and
 *  edge midpoints are inside, and no outline vertex falls within it. */
function clears(x: number, y: number, w: number, h: number, all: readonly Ring[]): boolean {
  const l = x - w / 2,
    r = x + w / 2,
    t = y - h / 2,
    b = y + h / 2;
  for (const q of [
    { x: l, y: t },
    { x: r, y: t },
    { x: l, y: b },
    { x: r, y: b },
    { x, y: t },
    { x, y: b },
    { x: l, y },
    { x: r, y },
  ]) {
    if (signedDistance(q, all) < 0) return false;
  }
  for (const ring of all) {
    for (const p of ring) if (p.x > l && p.x < r && p.y > t && p.y < b) return false;
  }
  return true;
}
