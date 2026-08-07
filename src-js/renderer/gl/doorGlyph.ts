// The door direction glyph, as triangles — all of the geometry and none of the
// GL, so the shape rules are testable without a canvas.
//
// A door draws as its footprint rectangle with an arrow running through the
// wall along the direction the door faces. Everything here is BAKED: the
// orientation is applied to the vertices as they are written, never carried as
// a per-door uniform or matrix. That is what lets every door's triangles go
// into the same buffer the rooms already fill, and so what keeps the scene at
// one draw call. It is also the same discipline the room geometry follows, so
// it costs no new rule.
//
// THE COORDINATE RULE, which is the easiest thing to get wrong here: payload
// data is Y-up, everything downstream of `flip()` is Y-down. The normal is a
// DIRECTION, so flipping it is a sign change on y with no translation — but it
// must still be flipped. Skip it and every arrow is mirrored vertically, which
// is invisible on a door in a north-south wall and obvious on any other, i.e.
// exactly the kind of bug that survives a casual look at one screenshot.
//
// The arrow is built in the door's own frame — `n` through the wall, `t` along
// it — so no trig appears below. That is the entire reason the exporter sends
// the normal rather than the wall tangent: with the tangent, every consumer
// would have to rotate 90 degrees and then decide the sign of that rotation.

import type { Door, Extent, Point2D } from "../types.js";
import { flip } from "../geometry.js";

/**
 * Below this, a footprint is not drawn as a rectangle (world units — feet).
 *
 * A door bbox smaller than this is not a real footprint, and a sub-pixel
 * rectangle reads as a rendering fault rather than as a door. The arrow still
 * carries the useful half of the information, so the rectangle is dropped and
 * the arrow kept.
 *
 * 0.15 ft is about 45 mm: comfortably below any real door leaf (the narrowest
 * in the House A sample is ~1.8 ft) and comfortably above the zero-area boxes
 * this exists to catch. It is deliberately NOT scaled to the current zoom —
 * the glyph is baked once per door set, so a zoom-dependent threshold would
 * mean rebuilding geometry on every wheel event.
 */
export const MIN_FOOTPRINT_EXTENT = 0.15;

/**
 * The size of a glyph that has no footprint to take its size from (world
 * units). A placeholder, not a measurement — which is why it is a constant and
 * not derived from anything.
 */
export const FALLBACK_GLYPH_SIZE = 2.0;

/**
 * The largest footprint dimension the arrow will size itself from (world
 * units).
 *
 * The arrow scales with the opening so a wide door gets a long arrow and a
 * narrow one a short arrow. Unclamped that reads badly at the extremes: House
 * A's garage panel-lift door is 17.75 ft across, which produced a ~19 ft arrow
 * that dominated the whole plan and looked like an annotation rather than a
 * door marking.
 *
 * A cap rather than a fixed size, because the scaling is right for the doors
 * that dominate a plan by count — it is only the rare very wide opening that
 * needs reining in. 4 ft is a wide-ish single leaf, so every ordinary door
 * keeps its proportional arrow and only the outliers are clamped.
 */
export const MAX_ARROW_SOURCE_SIZE = 4.0;

/**
 * The minimum THROUGH-WALL depth of a door's click target (world units).
 *
 * A door footprint is a sliver: it is as deep as the wall and no deeper. On the
 * House A plan at a fitted view that measures **4 px** for a typical door and
 * **1.2 px** for the garage panel-lift door, and an exact hit test against 1.2
 * px is a target nobody can hit. Picking then silently resolves to the room
 * underneath, so the door looks unselectable rather than hard to select.
 *
 * So the target is padded through the wall — only through it. The extent ALONG
 * the wall is left exactly as drawn, because that is the dimension a user aims
 * with, and the padding is applied in the door's own frame so a door in a
 * diagonal wall grows perpendicular to itself rather than into a bounding box
 * several times its size.
 *
 * 1 ft is about a wall's thickness again, so the target reaches roughly 3 in
 * into the rooms on either side: enough to catch an aimed click, small enough
 * that clicking a room near its wall still selects the room.
 *
 * Deliberately in WORLD units and not in pixels. The glyph is baked once per
 * door set, so a pixel-constant target would have to be rebuilt on every zoom —
 * the cost the whole bake-once design exists to avoid.
 */
export const MIN_PICK_DEPTH = 1.0;

/** Arrow proportions, as fractions of the glyph's size along the wall. Tuned
 *  so the head reads as a head at the zoom a whole level is viewed at. */
const ARROW_HALF_LENGTH = 0.55;
const SHAFT_HALF_WIDTH = 0.055;
const HEAD_LENGTH = 0.3;
const HEAD_HALF_WIDTH = 0.17;
const CHEVRON_LENGTH = 0.2;
const CHEVRON_HALF_WIDTH = 0.17;
const CHEVRON_THICKNESS = 0.055;

/** What a door draws, in flipped space. Each triangle list is a flat
 *  `[x0,y0, x1,y1, x2,y2, …]` run of independent triangles. */
export interface DoorGlyph {
  /** The footprint rectangle. Empty when there is no usable box. */
  rect: number[];
  /** The direction arrow. Empty when the door has no plan direction. */
  arrow: number[];
  /** The "door exists, nothing else known" marker. Only ever non-empty when
   *  BOTH of the above are empty. */
  cross: number[];
  /**
   * The click target's bounding box, in flipped space — the spatial index's
   * key, and a candidate filter rather than the answer.
   *
   * **Defined by what was drawn, not by the data that was available.** A door
   * with no footprint still has to be clickable, and its target is a fixed
   * square around the insertion point sized to the glyph it actually got.
   * Deriving this from `loops` instead would make precisely the doors that
   * most need explaining the ones that cannot be inspected.
   */
  pick: Extent;

  /**
   * The click target as a ring, flipped — what the hit test actually runs
   * against.
   *
   * Carried alongside the box because a door footprint is a rectangle in the
   * WALL's frame, not in the world's: a door in a diagonal wall has an
   * axis-aligned bounding box far larger than the door. Picking on the box
   * would let such a door swallow clicks meant for the room around it, and
   * "clicking the room sometimes selects a door" is a bug that only shows up on
   * the plans that have diagonal walls.
   *
   * For a degraded glyph this is the square's four corners, so the caller has
   * one uniform test and no special case.
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
 *  exact and earcut would be four times the work for the same four points. */
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

function squareAround(c: Point2D, size: number): Extent {
  const h = size / 2;
  return { minX: c.x - h, minY: c.y - h, maxX: c.x + h, maxY: c.y + h };
}

/**
 * The footprint's ring, padded to a minimum depth so it can actually be
 * clicked. Points in, points out — both already flipped.
 *
 * With a direction the padding happens in the DOOR'S frame: the ring is
 * projected onto the tangent and the normal, the through-wall span is grown to
 * `MIN_PICK_DEPTH` if it falls short, and the box is rebuilt from that basis.
 * Because the basis is orthonormal, rebuilding is the projection run backwards
 * and no trigonometry appears.
 *
 * Without a direction there is no frame to pad in, so the axis-aligned box is
 * grown on whichever axis is short. That over-covers for a diagonal door — but
 * a door with no normal has no arrow either, and a target slightly too generous
 * beats a target measured in single pixels.
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

function cornersOf(e: Extent): Point2D[] {
  return [
    { x: e.minX, y: e.minY },
    { x: e.maxX, y: e.minY },
    { x: e.maxX, y: e.maxY },
    { x: e.minX, y: e.maxY },
  ];
}

/** Extent of points that are ALREADY flipped — unlike `extentOf`, which flips
 *  as it reads. Two functions rather than a boolean, because a flag here is a
 *  double-flip waiting to happen. */
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

/**
 * The arrow, in the door's own frame.
 *
 * `c` is the centre and `n` the unit normal, BOTH already flipped. `size` is
 * the glyph's extent along the wall, which every proportion above is a fraction
 * of — so a wide opening gets a long arrow and a narrow one a short arrow,
 * without either being a special case.
 */
function arrowTriangles(c: Point2D, n: Point2D, size: number): number[] {
  // The tangent is the normal rotated a quarter turn. This is the only place a
  // rotation appears at all, and it is exact rather than trigonometric.
  const tx = -n.y;
  const ty = n.x;

  // A point in the door's frame: `a` along the wall, `b` through it.
  const at = (a: number, b: number): [number, number] => [
    c.x + tx * a * size + n.x * b * size,
    c.y + ty * a * size + n.y * b * size,
  ];

  const out: number[] = [];
  const tip = ARROW_HALF_LENGTH;
  const tail = -ARROW_HALF_LENGTH;
  const neck = tip - HEAD_LENGTH;

  // Shaft: tail to the base of the head.
  const [s0x, s0y] = at(-SHAFT_HALF_WIDTH, tail);
  const [s1x, s1y] = at(SHAFT_HALF_WIDTH, tail);
  const [s2x, s2y] = at(SHAFT_HALF_WIDTH, neck);
  const [s3x, s3y] = at(-SHAFT_HALF_WIDTH, neck);
  quad(out, s0x, s0y, s1x, s1y, s2x, s2y, s3x, s3y);

  // Head: the pointed end, and the only part that says which way.
  const [h0x, h0y] = at(-HEAD_HALF_WIDTH, neck);
  const [h1x, h1y] = at(HEAD_HALF_WIDTH, neck);
  const [h2x, h2y] = at(0, tip);
  tri(out, h0x, h0y, h1x, h1y, h2x, h2y);

  // Chevron tail: two arms sweeping back from the shaft, pointing the same way
  // as the head. A CHEVRON RATHER THAN A CIRCLE, deliberately: a circle needs a
  // triangle fan or a template to scale and translate, where this is two quads
  // that read as direction just as clearly.
  for (const sign of [-1, 1]) {
    const [a0x, a0y] = at(sign * CHEVRON_HALF_WIDTH, tail);
    const [a1x, a1y] = at(sign * (CHEVRON_HALF_WIDTH - CHEVRON_THICKNESS), tail);
    const [a2x, a2y] = at(0, tail + CHEVRON_LENGTH);
    const [a3x, a3y] = at(0, tail + CHEVRON_LENGTH - CHEVRON_THICKNESS);
    quad(out, a0x, a0y, a1x, a1y, a3x, a3y, a2x, a2y);
  }

  return out;
}

/** A fixed-size X. The marker for "this door exists and nothing else about it
 *  is known" — deliberately not an arrow (there is nothing to point) and
 *  deliberately a fixed size (a placeholder, not a measurement). */
function crossTriangles(c: Point2D, size: number): number[] {
  const out: number[] = [];
  const h = size / 2;
  const t = size * 0.08;
  // Two bars at 45 degrees. Written out rather than rotated, because at exactly
  // 45 degrees the offsets are the same number on both axes.
  quad(out, c.x - h, c.y - h + t, c.x - h + t, c.y - h, c.x + h, c.y + h - t, c.x + h - t, c.y + h);
  quad(out, c.x - h, c.y + h - t, c.x - h + t, c.y + h, c.x + h, c.y - h + t, c.x + h - t, c.y - h);
  return out;
}

/**
 * Build one door's glyph, or `null` when the door cannot be placed at all.
 *
 * `null` means no footprint AND no insertion point — there is nowhere to draw
 * it. On the House A export that is no doors at all, because every door carries
 * one or the other; it stays possible because a snapshot pushed before
 * `insertion_point` existed has neither for a geometry-less door.
 *
 * **Draw what the data supports, never invent the part that is missing.** The
 * four cases fall out of two independent questions rather than being enumerated:
 *
 * | footprint | direction | glyph |
 * |---|---|---|
 * | usable | present | rectangle + arrow |
 * | usable | missing | rectangle only |
 * | too small / absent | present | arrow only |
 * | too small / absent | missing | cross |
 *
 * The row that matters is the third: a guessed arrow is worse than no arrow,
 * because nothing downstream can tell it from a measured one.
 */
export function buildDoorGlyph(door: Door): DoorGlyph | null {
  const outer = door.loops?.[0]?.points;
  const box = outer && outer.length >= 3 ? extentOf(outer) : null;
  const usableBox =
    box !== null &&
    Math.max(box.maxX - box.minX, box.maxY - box.minY) >= MIN_FOOTPRINT_EXTENT;

  // The centre comes from the footprint when there is one, because that is
  // where the door visibly is; the insertion point is the fallback and the only
  // answer for a door with no geometry.
  const raw = door.insertion_point;
  const centre: Point2D | null = usableBox
    ? { x: (box.minX + box.maxX) / 2, y: (box.minY + box.maxY) / 2 }
    : raw
      ? flip(raw)
      : box
        ? { x: (box.minX + box.maxX) / 2, y: (box.minY + box.maxY) / 2 }
        : null;
  if (!centre) return null;

  const n = door.through_wall_normal;
  // A zero-length normal is not a direction. It should not arrive — the
  // producer drops those — but normalising defensively is cheaper than a NaN
  // reaching a vertex buffer, where it silently discards the whole draw.
  const len = n ? Math.hypot(n.x, n.y) : 0;
  const unit: Point2D | null = n && len > 1e-9 ? flip({ x: n.x / len, y: n.y / len }) : null;

  const rect = usableBox ? fanTriangulate(outer!) : [];
  // Size the arrow off the footprint when there is one so it fits the opening
  // it describes, and off the fallback constant when there is not. Clamped, so
  // a very wide opening does not turn its arrow into the loudest thing on the
  // drawing — see `MAX_ARROW_SOURCE_SIZE`.
  const size = usableBox
    ? Math.min(Math.max(box.maxX - box.minX, box.maxY - box.minY), MAX_ARROW_SOURCE_SIZE)
    : FALLBACK_GLYPH_SIZE;
  const arrow = unit ? arrowTriangles(centre, unit, size) : [];
  const cross = rect.length === 0 && arrow.length === 0 ? crossTriangles(centre, FALLBACK_GLYPH_SIZE) : [];

  // The pick target follows what was drawn. A footprint is its own target — as
  // its actual ring, not its bounding box, so a door in a diagonal wall claims
  // only the area it covers. An arrow or a cross gets a square big enough to
  // hit comfortably, which for the arrow means covering its length rather than
  // just its centre.
  const pickRing = usableBox
    ? paddedPickRing(outer!.map(flip), unit)
    : cornersOf(
        squareAround(centre, arrow.length > 0 ? size * ARROW_HALF_LENGTH * 2 : FALLBACK_GLYPH_SIZE),
      );

  return { rect, arrow, cross, pick: extentOfFlipped(pickRing), pickRing };
}
