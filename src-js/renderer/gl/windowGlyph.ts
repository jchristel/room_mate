// The window glyph, as triangles — all of the geometry and none of the GL, so
// the shape rules are testable without a canvas.
//
// A window draws as its footprint rectangle plus the window SYMBOL — a square
// of four panes above a sill line — set into the room the window opens **to**.
// It follows every rule `doorGlyph` established, and for the same reasons:
//
//  * everything is BAKED. The orientation is applied to the vertices as they
//    are written, never carried as a per-window uniform or matrix, so every
//    window's triangles go into the buffer the doors already fill and the scene
//    stays at one draw call per layer.
//  * payload data is Y-up, everything downstream of `flip()` is Y-down. The
//    normal is a DIRECTION, so flipping it is a sign change on y with no
//    translation — but it must still be flipped, or every symbol lands on the
//    wrong side of its wall, which is invisible on a north-south wall and
//    obvious on any other.
//  * the symbol is built in the window's own frame — `n` through the wall, `t`
//    along it — so no trigonometry appears below.
//
// WHAT DIFFERS FROM A DOOR, and why it is not a style choice.
//
// A door's arrow is symmetric about the wall: it says which way the door
// swings, and it reads the same drawn from either side. A window's symbol is
// **one-sided on purpose** — it sits entirely in the to-room, because that is
// the room the window serves and the whole question a reader asks of a window
// on a plan is which space it lights. An arrow would answer a question nobody
// asks about a window, and a symmetric symbol would waste half its ink in the
// space outside the building.
//
// The consequence is that a window with no direction gets no symbol, only its
// footprint — the same discipline the arrow follows. There is no side to put it
// on, and putting it on a guessed side is worse than leaving it off, because
// nothing downstream could tell a guess from a measurement. In a facade model
// that is not a rare case: 0 of 158 windows carried a room reference when it
// was measured, though they do carry a facing direction, so the symbol still
// draws — it simply points into whatever is on the far side of the wall.

import type { Extent, Loop, Point2D, WindowOpening } from "../types.js";
import { flip } from "../geometry.js";

/**
 * Below this, a footprint is not drawn as a rectangle (world units — feet).
 *
 * Shared value and shared reasoning with the door glyph: a sub-pixel rectangle
 * reads as a rendering fault rather than as an opening, and 0.15 ft is about
 * 45 mm — comfortably below any real window and comfortably above the zero-area
 * boxes this exists to catch.
 */
export const MIN_FOOTPRINT_EXTENT = 0.15;

/**
 * The size of a glyph with no footprint to take its size from (world units).
 * A placeholder, not a measurement, which is why it is a constant.
 */
export const FALLBACK_GLYPH_SIZE = 2.0;

/**
 * The largest footprint dimension the symbol will size itself from.
 *
 * The same cap the arrow takes, for the same reason: the scaling is right for
 * the windows that dominate a plan by count, and it is the rare very wide
 * opening — a shopfront, a curtain-wall bay — that would otherwise put a symbol
 * on the drawing larger than the rooms around it.
 */
export const MAX_SYMBOL_SOURCE_SIZE = 4.0;

/**
 * The minimum THROUGH-WALL depth of a window's click target (world units).
 * Identical to the door rule and for the identical reason — a footprint is as
 * deep as the wall and no deeper, which at a fitted view is a few pixels.
 */
export const MIN_PICK_DEPTH = 1.0;

// Symbol proportions, as fractions of the glyph's size along the wall. Tuned so
// the four panes are still four panes at the zoom a whole level is viewed at;
// below that the symbol reads as a filled square, which is the honest
// degradation — it still says "window here".
const STROKE = 0.05;
/** Half-length of the sill line, which is the widest part of the symbol. */
const SILL_HALF_WIDTH = 0.34;
/** How far off the wall the sill line sits. */
const SILL_OFFSET = 0.14;
/** Half-width of the pane square. */
const PANE_HALF = 0.24;
/** Centre of the pane square, measured through the wall from the window. */
const PANE_OFFSET = 0.48;

/** What a window draws, in flipped space. Each triangle list is a flat
 *  `[x0,y0, x1,y1, x2,y2, …]` run of independent triangles. */
export interface WindowGlyph {
  /** The footprint rectangle. Empty when there is no usable box. */
  rect: number[];
  /** The window symbol, set into the to-room. Empty when the window has no
   *  plan direction and so has no side to be set into. */
  symbol: number[];
  /** The "window exists, nothing else known" marker. Only ever non-empty when
   *  both of the above are empty. */
  cross: number[];
  /** The click target's bounding box, in flipped space — a candidate filter
   *  rather than the answer. */
  pick: Extent;
  /**
   * The click target as a ring, flipped — what the hit test runs against.
   *
   * **The footprint's ring, NOT the symbol's.** The symbol is an annotation
   * sitting in a room; making it clickable would mean a window claiming a patch
   * of floor a foot into the space, and clicking that room near its window
   * would select the window instead. A reader aims at the opening.
   */
  pickRing: Point2D[];
}

function tri(out: number[], ax: number, ay: number, bx: number, by: number, cx: number, cy: number): void {
  out.push(ax, ay, bx, by, cx, cy);
}

function quad(
  out: number[],
  ax: number, ay: number,
  bx: number, by: number,
  cx: number, cy: number,
  dx: number, dy: number,
): void {
  tri(out, ax, ay, bx, by, cx, cy);
  tri(out, ax, ay, cx, cy, dx, dy);
}

/** Fan-triangulate a convex ring. The footprint is a rectangle, so a fan is
 *  exact. */
function fanTriangulate(points: readonly Point2D[]): number[] {
  const out: number[] = [];
  if (points.length < 3) return out;
  const a = flip(points[0]!);
  for (let i = 1; i < points.length - 1; i++) {
    const b = flip(points[i]!);
    const c = flip(points[i + 1]!);
    tri(out, a.x, a.y, b.x, b.y, c.x, c.y);
  }
  return out;
}

function extentOf(points: readonly Point2D[]): Extent {
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const raw of points) {
    const p = flip(raw);
    if (p.x < minX) minX = p.x;
    if (p.y < minY) minY = p.y;
    if (p.x > maxX) maxX = p.x;
    if (p.y > maxY) maxY = p.y;
  }
  return { minX, minY, maxX, maxY };
}

function extentOfFlipped(points: readonly Point2D[]): Extent {
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const p of points) {
    if (p.x < minX) minX = p.x;
    if (p.y < minY) minY = p.y;
    if (p.x > maxX) maxX = p.x;
    if (p.y > maxY) maxY = p.y;
  }
  return { minX, minY, maxX, maxY };
}

function cornersOf(e: Extent): Point2D[] {
  return [
    { x: e.minX, y: e.minY },
    { x: e.maxX, y: e.minY },
    { x: e.maxX, y: e.maxY },
    { x: e.minX, y: e.maxY },
  ];
}

function squareAround(c: Point2D, size: number): Extent {
  const h = size / 2;
  return { minX: c.x - h, minY: c.y - h, maxX: c.x + h, maxY: c.y + h };
}

/**
 * The footprint's ring, padded to a minimum depth so it can be clicked. Points
 * in, points out — both already flipped. The door rule verbatim: with a
 * direction the padding happens in the window's own frame, so a window in a
 * diagonal wall grows perpendicular to itself rather than into a bounding box
 * several times its size.
 */
function paddedPickRing(flippedRing: readonly Point2D[], unit: Point2D | null): Point2D[] {
  if (!unit) {
    const e = extentOfFlipped(flippedRing);
    const growX = Math.max(0, MIN_PICK_DEPTH - (e.maxX - e.minX)) / 2;
    const growY = Math.max(0, MIN_PICK_DEPTH - (e.maxY - e.minY)) / 2;
    return cornersOf({
      minX: e.minX - growX, maxX: e.maxX + growX,
      minY: e.minY - growY, maxY: e.maxY + growY,
    });
  }

  const tx = -unit.y;
  const ty = unit.x;
  let tMin = Infinity, tMax = -Infinity, nMin = Infinity, nMax = -Infinity;
  for (const p of flippedRing) {
    const a = p.x * tx + p.y * ty;
    const b = p.x * unit.x + p.y * unit.y;
    if (a < tMin) tMin = a;
    if (a > tMax) tMax = a;
    if (b < nMin) nMin = b;
    if (b > nMax) nMax = b;
  }
  const grow = Math.max(0, MIN_PICK_DEPTH - (nMax - nMin)) / 2;
  nMin -= grow;
  nMax += grow;

  return ([
    [tMin, nMin], [tMax, nMin], [tMax, nMax], [tMin, nMax],
  ] as const).map(([a, b]) => ({
    x: tx * a + unit.x * b,
    y: ty * a + unit.y * b,
  }));
}

/**
 * The window symbol, in the window's own frame, set into the to-room.
 *
 * `c` is the centre of the opening and `n` the unit normal, BOTH already
 * flipped. `size` is the glyph's extent along the wall, which every proportion
 * above is a fraction of — so a wide window gets a wide symbol without that
 * being a special case.
 *
 * **`n` points from the from-room toward the to-room**, which is what puts the
 * symbol on the served side rather than outside the building. It is the same
 * vector the door arrow points along, used for a different purpose: the door
 * says "this way through", the window says "this room".
 *
 * Six bars: the sill, and the five that make a four-pane square (its four sides
 * plus a mullion and a transom, drawn as one cross). Written as bars rather
 * than as an outline with a stroke width because a stroked path would be a
 * second draw path — these are the same triangles everything else here emits.
 */
function symbolTriangles(c: Point2D, n: Point2D, size: number): number[] {
  // The tangent is the normal rotated a quarter turn. The only rotation in the
  // file, and it is exact rather than trigonometric.
  const tx = -n.y;
  const ty = n.x;

  // A point in the window's frame: `a` along the wall, `b` through it.
  const at = (a: number, b: number): [number, number] => [
    c.x + tx * a * size + n.x * b * size,
    c.y + ty * a * size + n.y * b * size,
  ];

  // One bar of the symbol, given its centre-line span in the window's frame.
  // `half` is measured perpendicular to the run, so a horizontal and a vertical
  // bar are the same call with the arguments swapped.
  const bar = (out: number[], a0: number, b0: number, a1: number, b1: number, half: number): void => {
    // The run direction in frame coordinates, normalised so `half` is a true
    // half-thickness rather than a fraction of the bar's own length.
    const da = a1 - a0;
    const db = b1 - b0;
    const len = Math.hypot(da, db);
    if (len < 1e-9) return;
    const pa = (-db / len) * half;
    const pb = (da / len) * half;
    const [x0, y0] = at(a0 - pa, b0 - pb);
    const [x1, y1] = at(a1 - pa, b1 - pb);
    const [x2, y2] = at(a1 + pa, b1 + pb);
    const [x3, y3] = at(a0 + pa, b0 + pb);
    quad(out, x0, y0, x1, y1, x2, y2, x3, y3);
  };

  const out: number[] = [];
  const h = STROKE / 2;

  // The sill: a single line along the wall, nearest the opening. It is the part
  // that reads at the smallest zoom, which is why it is the widest element.
  bar(out, -SILL_HALF_WIDTH, SILL_OFFSET, SILL_HALF_WIDTH, SILL_OFFSET, h);

  // The pane square, centred further into the room.
  const lo = PANE_OFFSET - PANE_HALF;
  const hi = PANE_OFFSET + PANE_HALF;
  bar(out, -PANE_HALF, lo, PANE_HALF, lo, h); // near side
  bar(out, -PANE_HALF, hi, PANE_HALF, hi, h); // far side
  bar(out, -PANE_HALF, lo, -PANE_HALF, hi, h); // left
  bar(out, PANE_HALF, lo, PANE_HALF, hi, h); // right

  // The cross that makes it four panes rather than one.
  bar(out, -PANE_HALF, PANE_OFFSET, PANE_HALF, PANE_OFFSET, h);
  bar(out, 0, lo, 0, hi, h);

  return out;
}

/** A fixed-size X — "this window exists and nothing else about it is known".
 *  Deliberately the same marker a door in that state gets: the reader is being
 *  told the same thing, and two different unknown-markers would imply two
 *  different unknowns. */
function crossTriangles(c: Point2D, size: number): number[] {
  const out: number[] = [];
  const h = size / 2;
  const t = size * 0.08;
  quad(out, c.x - h, c.y - h + t, c.x - h + t, c.y - h, c.x + h, c.y + h - t, c.x + h - t, c.y + h);
  quad(out, c.x - h, c.y + h - t, c.x - h + t, c.y + h, c.x + h, c.y - h + t, c.x + h - t, c.y - h);
  return out;
}

/**
 * Build one window's glyph, or `null` when it cannot be placed at all.
 *
 * `null` means no footprint AND no insertion point — there is nowhere to draw
 * it, and drawing it at the origin would claim a position it does not have.
 *
 * **Draw what the data supports, never invent the part that is missing.** The
 * cases fall out of two independent questions rather than being enumerated:
 *
 * | footprint | direction | glyph |
 * |---|---|---|
 * | usable | present | rectangle + symbol in the to-room |
 * | usable | missing | rectangle only |
 * | too small / absent | present | symbol only |
 * | too small / absent | missing | cross |
 *
 * The second row is the one that matters here, and it is more common for
 * windows than the equivalent is for doors: without a direction there is no
 * side, and a symbol drawn on a guessed side would be indistinguishable from a
 * measured one while being wrong half the time.
 */
export function buildWindowGlyph(window: WindowOpening): WindowGlyph | null {
  const outer: readonly Point2D[] | undefined = (window.loops as Loop[] | undefined)?.[0]?.points;
  const box = outer && outer.length >= 3 ? extentOf(outer) : null;
  const usableBox =
    box !== null &&
    Math.max(box.maxX - box.minX, box.maxY - box.minY) >= MIN_FOOTPRINT_EXTENT;

  // The centre comes from the footprint when there is one, because that is
  // where the window visibly is; the insertion point is the fallback and the
  // only answer for a window with no geometry.
  const raw = window.insertion_point;
  const centre: Point2D | null = usableBox
    ? { x: (box.minX + box.maxX) / 2, y: (box.minY + box.maxY) / 2 }
    : raw
      ? flip(raw)
      : box
        ? { x: (box.minX + box.maxX) / 2, y: (box.minY + box.maxY) / 2 }
        : null;
  if (!centre) return null;

  const n = window.through_wall_normal;
  // A zero-length normal is not a direction. Normalising defensively is cheaper
  // than a NaN reaching a vertex buffer, where it silently discards the draw.
  const len = n ? Math.hypot(n.x, n.y) : 0;
  const unit: Point2D | null = n && len > 1e-9 ? flip({ x: n.x / len, y: n.y / len }) : null;

  const rect = usableBox ? fanTriangulate(outer!) : [];
  const size = usableBox
    ? Math.min(Math.max(box.maxX - box.minX, box.maxY - box.minY), MAX_SYMBOL_SOURCE_SIZE)
    : FALLBACK_GLYPH_SIZE;
  const symbol = unit ? symbolTriangles(centre, unit, size) : [];
  const cross = rect.length === 0 && symbol.length === 0 ? crossTriangles(centre, FALLBACK_GLYPH_SIZE) : [];

  // The pick target follows the OPENING, not the symbol — see `pickRing`. A
  // window with no footprint falls back to a square around the centre, sized so
  // it can be hit rather than sized to cover the symbol.
  const pickRing = usableBox
    ? paddedPickRing(outer!.map(flip), unit)
    : cornersOf(squareAround(centre, FALLBACK_GLYPH_SIZE));

  return { rect, symbol, cross, pick: extentOfFlipped(pickRing), pickRing };
}
