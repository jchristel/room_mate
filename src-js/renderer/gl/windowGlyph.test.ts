import { describe, expect, it } from "vitest";

import { buildWindowGlyph, FALLBACK_GLYPH_SIZE, MIN_FOOTPRINT_EXTENT } from "./windowGlyph.js";
import type { Point2D, WindowOpening } from "../types.js";

/** A window in an east-west wall, 3 ft wide and a wall thick, facing north
 *  (Y-up +y). Y-up in, so every expectation below is in flipped space. */
function wallWindow(extra: Partial<WindowOpening> = {}): WindowOpening {
  return {
    id: "w1",
    loops: [{ points: [
      { x: 0, y: 0 },
      { x: 3, y: 0 },
      { x: 3, y: 0.5 },
      { x: 0, y: 0.5 },
    ] }],
    insertion_point: { x: 1.5, y: 0.25 },
    through_wall_normal: { x: 0, y: 1 },
    ...extra,
  };
}

/** Every vertex of a flat triangle run, as points. */
function points(run: number[]): Point2D[] {
  const out: Point2D[] = [];
  for (let i = 0; i < run.length; i += 2) out.push({ x: run[i]!, y: run[i + 1]! });
  return out;
}

/** How far a point sits along `n` from `c` — the through-wall coordinate. */
function along(p: Point2D, c: Point2D, n: Point2D): number {
  return (p.x - c.x) * n.x + (p.y - c.y) * n.y;
}

describe("buildWindowGlyph", () => {
  it("draws the footprint and the symbol when both are available", () => {
    const g = buildWindowGlyph(wallWindow())!;
    expect(g.rect.length).toBeGreaterThan(0);
    expect(g.symbol.length).toBeGreaterThan(0);
    expect(g.cross).toHaveLength(0);
  });

  /**
   * **The whole point of the symbol.** `through_wall_normal` is Y-up and points
   * from the from-room toward the to-room; flipped space is Y-down. So a window
   * facing +y must put its symbol at NEGATIVE flipped y — and a missing flip
   * lands the entire symbol in the room on the other side of the wall, which is
   * invisible on a north-south wall and wrong on every other.
   */
  it("puts the symbol on the to-room side, in flipped space", () => {
    const g = buildWindowGlyph(wallWindow())!;
    const centre = { x: 1.5, y: -0.25 }; // the footprint centre, flipped
    const flippedNormal = { x: 0, y: -1 }; // +y Y-up is -y Y-down

    const depths = points(g.symbol).map((p) => along(p, centre, flippedNormal));
    expect(Math.min(...depths)).toBeGreaterThan(0);
  });

  /** Mirror of the above: reversing the window's facing reverses the side the
   *  symbol lands on. Asserted separately because a sign error that cancels
   *  itself would pass the first test alone. */
  it("follows the normal when the window faces the other way", () => {
    const g = buildWindowGlyph(wallWindow({ through_wall_normal: { x: 0, y: -1 } }))!;
    const centre = { x: 1.5, y: -0.25 };
    const depths = points(g.symbol).map((p) => along(p, centre, { x: 0, y: 1 }));
    expect(Math.min(...depths)).toBeGreaterThan(0);
  });

  /** The symbol is one-sided by design — nothing on the from-room side at all.
   *  A door's arrow straddles the wall; a window's symbol must not, or it would
   *  put ink in the space outside the building. */
  it("puts nothing on the from-room side", () => {
    const g = buildWindowGlyph(wallWindow())!;
    const centre = { x: 1.5, y: -0.25 };
    const depths = points(g.symbol).map((p) => along(p, centre, { x: 0, y: -1 }));
    expect(Math.min(...depths)).toBeGreaterThan(0);
  });

  /** With no direction there is no side, so no symbol — the footprint alone.
   *  A guessed side is worse than none, because nothing downstream could tell
   *  it from a measured one. */
  it("draws no symbol when the window has no plan direction", () => {
    const g = buildWindowGlyph(wallWindow({ through_wall_normal: null }))!;
    expect(g.rect.length).toBeGreaterThan(0);
    expect(g.symbol).toHaveLength(0);
    expect(g.cross).toHaveLength(0);
  });

  /** A window with a direction but no usable footprint still draws its symbol,
   *  positioned from the insertion point. This is the case a facade model
   *  produces when duHast could not measure the family. */
  it("draws the symbol alone when the footprint is unusable", () => {
    const g = buildWindowGlyph(wallWindow({ loops: [] }))!;
    expect(g.rect).toHaveLength(0);
    expect(g.symbol.length).toBeGreaterThan(0);
  });

  /** A footprint below the threshold is not a footprint. */
  it("treats a sub-threshold box as no footprint", () => {
    const tiny = MIN_FOOTPRINT_EXTENT / 2;
    const g = buildWindowGlyph(wallWindow({
      loops: [{ points: [
        { x: 0, y: 0 }, { x: tiny, y: 0 }, { x: tiny, y: tiny }, { x: 0, y: tiny },
      ] }],
    }))!;
    expect(g.rect).toHaveLength(0);
    expect(g.symbol.length).toBeGreaterThan(0);
  });

  /** Neither footprint nor direction: the marker, and deliberately the same
   *  marker a door in that state gets. */
  it("falls back to a cross when nothing is known", () => {
    const g = buildWindowGlyph(wallWindow({ loops: [], through_wall_normal: null }))!;
    expect(g.rect).toHaveLength(0);
    expect(g.symbol).toHaveLength(0);
    expect(g.cross.length).toBeGreaterThan(0);
  });

  /** Nowhere to draw it at all. */
  it("returns null with neither a footprint nor an insertion point", () => {
    expect(buildWindowGlyph({ id: "w1", loops: [], insertion_point: null })).toBeNull();
  });

  /**
   * **The pick target is the opening, not the symbol.** The symbol sits a foot
   * into the room; if it were clickable, clicking that room near its window
   * would select the window instead.
   */
  it("keeps the click target on the opening", () => {
    const g = buildWindowGlyph(wallWindow())!;
    const centre = { x: 1.5, y: -0.25 };
    const flippedNormal = { x: 0, y: -1 };
    const depths = g.pickRing.map((p) => along(p, centre, flippedNormal));
    // Padded through the wall, so it straddles the opening symmetrically and
    // reaches nowhere near the symbol's far edge.
    expect(Math.min(...depths)).toBeLessThan(0);
    expect(Math.max(...depths)).toBeLessThan(0.6);
  });

  /** A sliver footprint is padded through the wall so it can be hit, and only
   *  through it — the extent along the wall is what a user aims with. */
  it("pads a sliver footprint through the wall only", () => {
    const g = buildWindowGlyph(wallWindow({
      loops: [{ points: [
        { x: 0, y: 0 }, { x: 3, y: 0 }, { x: 3, y: 0.1 }, { x: 0, y: 0.1 },
      ] }],
    }))!;
    const width = g.pick.maxX - g.pick.minX;
    const depth = g.pick.maxY - g.pick.minY;
    expect(width).toBeCloseTo(3, 5);
    expect(depth).toBeGreaterThanOrEqual(1);
  });

  /** The symbol scales with the opening, so a wide window gets a wide symbol —
   *  and is capped, so a shopfront does not put the largest thing on the
   *  drawing over its own wall. */
  it("scales with the opening and clamps at the cap", () => {
    const spanOf = (w: number): number => {
      const g = buildWindowGlyph(wallWindow({
        loops: [{ points: [
          { x: 0, y: 0 }, { x: w, y: 0 }, { x: w, y: 0.5 }, { x: 0, y: 0.5 },
        ] }],
      }))!;
      const xs = points(g.symbol).map((p) => p.x);
      return Math.max(...xs) - Math.min(...xs);
    };
    expect(spanOf(6)).toBeGreaterThan(spanOf(3));
    // Both are past the 4 ft cap, so they must come out identical.
    expect(spanOf(20)).toBeCloseTo(spanOf(8), 5);
  });

  /** A window with no footprint sizes its symbol off the fallback constant, so
   *  it is visible rather than zero-sized. */
  it("sizes a footprint-less symbol off the fallback", () => {
    const g = buildWindowGlyph(wallWindow({ loops: [] }))!;
    const xs = points(g.symbol).map((p) => p.x);
    expect(Math.max(...xs) - Math.min(...xs)).toBeGreaterThan(FALLBACK_GLYPH_SIZE * 0.3);
  });

  /** Every emitted run is whole triangles. A stray coordinate silently
   *  discards the tail of the draw, which is invisible until someone counts. */
  it("emits complete triangles", () => {
    const g = buildWindowGlyph(wallWindow())!;
    for (const run of [g.rect, g.symbol]) {
      expect(run.length % 6).toBe(0);
      expect(run.every(Number.isFinite)).toBe(true);
    }
  });

  /** A diagonal wall is the case a baked orientation exists for: the symbol
   *  must rotate with the window rather than staying axis-aligned. */
  it("bakes the orientation for a diagonal wall", () => {
    const s = Math.SQRT1_2;
    const g = buildWindowGlyph(wallWindow({ through_wall_normal: { x: s, y: s } }))!;
    const centre = { x: 1.5, y: -0.25 };
    const depths = points(g.symbol).map((p) => along(p, centre, { x: s, y: -s }));
    expect(Math.min(...depths)).toBeGreaterThan(0);
  });
});
