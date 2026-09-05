// The FF&E glyph, as triangles — all of the geometry and none of the GL, so the
// shape rules are testable without a canvas.
//
// An item draws as a square marker at its insertion point, with a short tick
// showing which way it faces. It follows the rules `doorGlyph` established and
// `windowGlyph` inherited — everything baked into the vertices, one draw call
// per layer, payload Y-up flipped once on the way in — and departs from both in
// what it draws and why.
//
// WHAT DIFFERS FROM AN OPENING, and none of it is a style choice.
//
// **The footprint is the exception here, not the rule.** A door or a window is
// drawn from its footprint and falls back to a marker; every FF&E item measured
// so far has NO footprint at all, because `DataItem` carries no geometry until
// duHast's upstream change lands (`docs/PLAN-ffe.md` U1). So this glyph is
// built the other way round: the marker is the normal case and the rectangle is
// the upgrade. The moment items arrive with `loops`, the same code draws them
// without a change here.
//
// **There is no symbol, and adding one would be a guess.** A door's arc says
// "this way through"; a window's panes say "this room is lit". FF&E is nine
// Revit categories at once — furniture, casework, plumbing, electrical, generic
// models — and one symbol standing for all of them would say nothing, while a
// symbol per category would be nine drawing conventions invented here rather
// than read from the model. A square says "a placed object is here", which is
// exactly what the data supports.
//
// **The tick is an orientation, not a direction of travel.** An opening's
// normal points THROUGH a wall and has a meaning on each side; an item's facing
// is just which way it points, so the tick runs from the marker's centre
// outward and stops. It is drawn on the same terms as the door arrow: absent
// facing means no tick, never a guessed one. That is not a rare case — 44 of
// 644 House A items have no plan facing, because their local X points along Z.

import type { Extent, Item, Loop, Point2D } from "../types.js";
import { flip } from "../geometry.js";

/**
 * Below this, a footprint is not drawn as a rectangle (world units — feet).
 * Shared value and shared reasoning with the opening glyphs: a sub-pixel
 * rectangle reads as a rendering fault rather than as an object.
 */
export const MIN_FOOTPRINT_EXTENT = 0.15;

/**
 * The size of the marker drawn for an item with no usable footprint (world
 * units — feet), which today is every item.
 *
 * Smaller than the openings' `FALLBACK_GLYPH_SIZE` of 2.0 ft, and deliberately
 * so: an opening's fallback marker stands in for something that is rare, while
 * this one is drawn for hundreds of items on a level. At 2 ft a furnished floor
 * would be a field of overlapping squares. 1.2 ft is about 370 mm — big enough
 * to aim at, small enough that a run of desks reads as a run of desks.
 */
export const MARKER_SIZE = 1.2;

/**
 * The largest footprint dimension the tick will size itself from.
 * The cap the opening glyphs take, for the same reason: a very large item — a
 * bench run, a modular wall — would otherwise put a tick on the drawing longer
 * than the rooms around it.
 */
export const MAX_TICK_SOURCE_SIZE = 4.0;

/** The minimum extent of an item's click target (world units). A marker is
 *  already about this size; a real footprint may be far thinner in one axis. */
export const MIN_PICK_SIZE = 1.0;

/** Tick length, as a fraction of the glyph's size. */
const TICK_LENGTH = 0.7;
/** Tick thickness, same fraction basis. */
const TICK_STROKE = 0.09;
/** Marker outline thickness, same fraction basis. */
const MARKER_STROKE = 0.12;

/** What an item draws, in flipped space. Each triangle list is a flat
 *  `[x0,y0, x1,y1, x2,y2, …]` run of independent triangles. */
export interface ItemGlyph {
  /** The footprint rectangle, when the item has one. Empty until U1 ships. */
  rect: number[];
  /** The square marker. Empty when a real footprint was drawn instead. */
  marker: number[];
  /** The orientation tick. Empty when the item has no plan facing. */
  tick: number[];
  /** The click target's bounding box, in flipped space — a candidate filter
   *  rather than the answer. */
  pick: Extent;
  /** The click target as a ring, flipped — what the hit test runs against. */
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

/** Fan-triangulate a convex ring. A footprint is a rectangle, so a fan is
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

/** Grow an extent to at least `MIN_PICK_SIZE` on each axis, so a thin footprint
 *  can still be aimed at. */
function padded(e: Extent): Extent {
  const growX = Math.max(0, MIN_PICK_SIZE - (e.maxX - e.minX)) / 2;
  const growY = Math.max(0, MIN_PICK_SIZE - (e.maxY - e.minY)) / 2;
  return {
    minX: e.minX - growX, maxX: e.maxX + growX,
    minY: e.minY - growY, maxY: e.maxY + growY,
  };
}

/**
 * A hollow square centred on `c`, rotated to the item's own frame when it has
 * one.
 *
 * Hollow rather than filled, and that is the one visual decision here worth
 * defending. A filled square at this density reads as a blocked-out area — a
 * furnished room would look like a room with a hole in it — while an outline
 * reads as an object sitting on a floor whose fill and colour stay visible
 * underneath. It is also what makes a run of adjacent desks legible as separate
 * items rather than one solid band.
 *
 * `unit`, when present, is the item's facing in flipped space; the square turns
 * with it, so a desk at 30 degrees looks like a desk at 30 degrees rather than
 * an axis-aligned box around one.
 */
function markerTriangles(c: Point2D, size: number, unit: Point2D | null): number[] {
  const ux = unit ? unit.x : 1;
  const uy = unit ? unit.y : 0;
  // The tangent is the facing rotated a quarter turn. The only rotation in the
  // file, and it is exact rather than trigonometric.
  const tx = -uy;
  const ty = ux;

  // A point in the item's own frame: `a` across, `b` along the facing.
  const at = (a: number, b: number): [number, number] => [
    c.x + tx * a * size + ux * b * size,
    c.y + ty * a * size + uy * b * size,
  ];

  const bar = (out: number[], a0: number, b0: number, a1: number, b1: number, half: number): void => {
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
  const h = MARKER_STROKE / 2;
  const e = 0.5; // half-extent of the square, in frame units
  bar(out, -e, -e, e, -e, h);
  bar(out, -e, e, e, e, h);
  bar(out, -e, -e, -e, e, h);
  bar(out, e, -e, e, e, h);
  return out;
}

/**
 * A bar from the marker's edge outward along the facing.
 *
 * It starts at the edge rather than at the centre so it reads as *pointing*
 * rather than as a line through the object, and so it is still visible when the
 * marker is small on screen.
 */
function tickTriangles(c: Point2D, unit: Point2D, size: number): number[] {
  const out: number[] = [];
  const px = -unit.y * (TICK_STROKE / 2) * size;
  const py = unit.x * (TICK_STROKE / 2) * size;
  const x0 = c.x + unit.x * 0.5 * size;
  const y0 = c.y + unit.y * 0.5 * size;
  const x1 = c.x + unit.x * (0.5 + TICK_LENGTH) * size;
  const y1 = c.y + unit.y * (0.5 + TICK_LENGTH) * size;
  quad(out, x0 - px, y0 - py, x1 - px, y1 - py, x1 + px, y1 + py, x0 + px, y0 + py);
  return out;
}

/**
 * Build one item's glyph, or `null` when it cannot be placed at all.
 *
 * `null` means no footprint AND no insertion point — there is nowhere to draw
 * it, and drawing it at the origin would claim a position it does not have.
 * Every pushed item has an insertion point today (duHast refuses an instance
 * without a `LocationPoint`), so this is the guard for a snapshot on disk rather
 * than for anything a producer can currently send.
 *
 * **Draw what the data supports, never invent the part that is missing.** Two
 * independent questions, four outcomes:
 *
 * | footprint | facing | glyph |
 * |---|---|---|
 * | usable | present | rectangle + tick |
 * | usable | missing | rectangle only |
 * | too small / absent | present | marker + tick |
 * | too small / absent | missing | marker only |
 *
 * There is no "nothing is known" cross, unlike the opening glyphs. An item with
 * no facing and no footprint still has a position, and the marker already says
 * exactly what is known — that something is here. A second unknown-marker would
 * be a distinction with nothing behind it.
 */
export function buildItemGlyph(item: Item): ItemGlyph | null {
  const outer: readonly Point2D[] | undefined = (item.loops as Loop[] | undefined)?.[0]?.points;
  const box = outer && outer.length >= 3 ? extentOf(outer) : null;
  const usableBox =
    box !== null &&
    Math.max(box.maxX - box.minX, box.maxY - box.minY) >= MIN_FOOTPRINT_EXTENT;

  // The centre comes from the footprint when there is one, because that is
  // where the item visibly is; the insertion point is the fallback and today it
  // is the only answer.
  const raw = item.insertion_point;
  const centre: Point2D | null = usableBox
    ? { x: (box.minX + box.maxX) / 2, y: (box.minY + box.maxY) / 2 }
    : raw
      ? flip(raw)
      : box
        ? { x: (box.minX + box.maxX) / 2, y: (box.minY + box.maxY) / 2 }
        : null;
  if (!centre) return null;

  const f = item.facing;
  // A zero-length facing is not a direction. Normalising defensively is cheaper
  // than a NaN reaching a vertex buffer, where it silently discards the draw.
  const len = f ? Math.hypot(f.x, f.y) : 0;
  const unit: Point2D | null = f && len > 1e-9 ? flip({ x: f.x / len, y: f.y / len }) : null;

  const rect = usableBox ? fanTriangulate(outer!) : [];
  const size = usableBox
    ? Math.min(Math.max(box.maxX - box.minX, box.maxY - box.minY), MAX_TICK_SOURCE_SIZE)
    : MARKER_SIZE;
  const marker = usableBox ? [] : markerTriangles(centre, MARKER_SIZE, unit);
  const tick = unit ? tickTriangles(centre, unit, size) : [];

  const pickRing = cornersOf(
    padded(usableBox ? extentOfFlipped(outer!.map(flip)) : {
      minX: centre.x - MARKER_SIZE / 2, maxX: centre.x + MARKER_SIZE / 2,
      minY: centre.y - MARKER_SIZE / 2, maxY: centre.y + MARKER_SIZE / 2,
    }),
  );

  return { rect, marker, tick, pick: extentOfFlipped(pickRing), pickRing };
}
