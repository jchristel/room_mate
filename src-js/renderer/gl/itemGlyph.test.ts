// What the FF&E glyph must draw, and — more often — what it must not.
//
// The cases here are chosen from the real House A export rather than from the
// space of possible inputs: 644 items, none with a footprint, 600 with a facing
// and 44 without. So the "no footprint" path is not an edge case being defended
// against, it is the only path anyone has yet exercised, and it is tested first.

import { describe, expect, it } from "vitest";

import { buildItemGlyph, MARKER_SIZE, MIN_FOOTPRINT_EXTENT } from "./itemGlyph.js";
import type { Item, Point2D } from "../types.js";

function item(over: Partial<Item> = {}): Item {
  return { id: "f1", insertion_point: { x: 10, y: 20 }, ...over };
}

/** Every (x, y) pair in a flat triangle list. */
function points(flat: readonly number[]): Point2D[] {
  const out: Point2D[] = [];
  for (let i = 0; i < flat.length; i += 2) out.push({ x: flat[i]!, y: flat[i + 1]! });
  return out;
}

function extent(flat: readonly number[]) {
  const p = points(flat);
  return {
    minX: Math.min(...p.map((q) => q.x)),
    maxX: Math.max(...p.map((q) => q.x)),
    minY: Math.min(...p.map((q) => q.y)),
    maxY: Math.max(...p.map((q) => q.y)),
  };
}

const box = (w: number, h: number) => [
  { points: [{ x: 0, y: 0 }, { x: w, y: 0 }, { x: w, y: h }, { x: 0, y: h }] },
];

describe("buildItemGlyph", () => {
  it("draws a marker for an item with no footprint, which is every item today", () => {
    const glyph = buildItemGlyph(item())!;
    expect(glyph).not.toBeNull();
    expect(glyph.rect).toHaveLength(0);
    expect(glyph.marker.length).toBeGreaterThan(0);
  });

  it("is null only when there is nowhere to draw it", () => {
    // No footprint and no insertion point. Drawing at the origin would claim a
    // position the item does not have.
    expect(buildItemGlyph({ id: "f1" })).toBeNull();
    expect(buildItemGlyph({ id: "f1", loops: [] })).toBeNull();
  });

  it("adds a tick only when the item states a facing", () => {
    expect(buildItemGlyph(item())!.tick).toHaveLength(0);
    expect(buildItemGlyph(item({ facing: { x: 1, y: 0 } }))!.tick.length).toBeGreaterThan(0);
  });

  it("treats a zero-length facing as no facing rather than as a direction", () => {
    // A NaN reaching a vertex buffer silently discards the draw, which looks
    // like the item is missing rather than like the data is odd.
    const glyph = buildItemGlyph(item({ facing: { x: 0, y: 0 } }))!;
    expect(glyph.tick).toHaveLength(0);
    expect(glyph.marker.every(Number.isFinite)).toBe(true);
  });

  it("normalises a facing that is not already a unit vector", () => {
    const unit = buildItemGlyph(item({ facing: { x: 1, y: 0 } }))!;
    const long = buildItemGlyph(item({ facing: { x: 7, y: 0 } }))!;
    expect(extent(long.tick)).toEqual(extent(unit.tick));
  });

  it("flips the facing, so the tick lands on the side the data meant", () => {
    // Payload is Y-up and everything downstream is Y-down. Missing the flip is
    // invisible on one axis and wrong on the other, which is exactly the bug
    // that is easy to ship.
    const glyph = buildItemGlyph(item({ insertion_point: { x: 0, y: 0 }, facing: { x: 0, y: 1 } }))!;
    expect(extent(glyph.tick).minY).toBeLessThan(0);
  });

  it("draws the footprint instead of the marker once one arrives", () => {
    // The upgrade path for duHast's U1: the same code, no change here.
    const glyph = buildItemGlyph(item({ loops: box(3, 2), insertion_point: null }))!;
    expect(glyph.rect.length).toBeGreaterThan(0);
    expect(glyph.marker).toHaveLength(0);
  });

  it("ignores a footprint too small to read and falls back to the marker", () => {
    const tiny = MIN_FOOTPRINT_EXTENT / 2;
    const glyph = buildItemGlyph(item({ loops: box(tiny, tiny) }))!;
    expect(glyph.rect).toHaveLength(0);
    expect(glyph.marker.length).toBeGreaterThan(0);
  });

  it("centres the marker on the insertion point", () => {
    const glyph = buildItemGlyph(item({ insertion_point: { x: 10, y: 20 } }))!;
    const e = extent(glyph.marker);
    expect((e.minX + e.maxX) / 2).toBeCloseTo(10, 6);
    // Flipped: y = 20 becomes -20.
    expect((e.minY + e.maxY) / 2).toBeCloseTo(-20, 6);
  });

  it("keeps the marker the same size regardless of facing", () => {
    // The marker says "an object is here", and that claim does not get bigger
    // when the item happens to be rotated.
    //
    // Measured as the furthest vertex from the centre, NOT as the axis-aligned
    // extent. A rotated square's bounding box is genuinely wider, so an extent
    // comparison would be testing the bounding box rather than the marker --
    // and it does not come out at sqrt(2) either, because the stroke width adds
    // a constant that does not rotate with the square.
    const radius = (facing: Point2D) => {
      const glyph = buildItemGlyph(item({ insertion_point: { x: 0, y: 0 }, facing }))!;
      return Math.max(...points(glyph.marker).map((p) => Math.hypot(p.x, p.y)));
    };
    const straight = radius({ x: 1, y: 0 });
    expect(radius({ x: 1, y: 1 })).toBeCloseTo(straight, 6);
    expect(radius({ x: 0.3, y: -0.9 })).toBeCloseTo(straight, 6);
  });

  it("gives every item a click target at least MIN_PICK_SIZE across", () => {
    // A marker is about a foot; a real footprint may be far thinner in one
    // axis, and an item nobody can click is an item nobody can inspect.
    for (const subject of [item(), item({ loops: box(4, 0.2) })]) {
      const glyph = buildItemGlyph(subject)!;
      expect(glyph.pick.maxX - glyph.pick.minX).toBeGreaterThanOrEqual(1);
      expect(glyph.pick.maxY - glyph.pick.minY).toBeGreaterThanOrEqual(1);
    }
  });

  it("puts the pick ring where the marker is, not where the tick points", () => {
    // The tick is an annotation reaching away from the object. Making it
    // clickable would let an item claim floor it does not occupy, and clicking
    // there would select the item instead of the room.
    const glyph = buildItemGlyph(item({ insertion_point: { x: 0, y: 0 }, facing: { x: 1, y: 0 } }))!;
    expect(glyph.pick.maxX).toBeLessThan(extent(glyph.tick).maxX);
  });

  it("emits whole triangles and finite coordinates for every case", () => {
    // A stray vertex or a NaN does not error — it silently corrupts the draw,
    // which is the failure mode 15 passing glyph tests missed once before.
    const cases = [
      item(),
      item({ facing: { x: 0.6, y: -0.8 } }),
      item({ loops: box(3, 2) }),
      item({ loops: box(3, 2), facing: { x: 0, y: 1 } }),
    ];
    for (const subject of cases) {
      const glyph = buildItemGlyph(subject)!;
      for (const list of [glyph.rect, glyph.marker, glyph.tick]) {
        expect(list.length % 6).toBe(0);
        expect(list.every(Number.isFinite)).toBe(true);
      }
    }
  });

  it("exports a marker smaller than the openings' fallback glyph", () => {
    // Not a style preference: an opening's fallback is rare, this one is drawn
    // hundreds of times on a level, and at 2 ft a furnished floor is a field of
    // overlapping squares.
    expect(MARKER_SIZE).toBeLessThan(2.0);
  });
});
