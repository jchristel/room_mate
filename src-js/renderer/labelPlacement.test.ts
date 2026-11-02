import { describe, expect, it } from "vitest";

import { flip } from "./geometry.js";
import { GLYPH_ASPECT, placeLabel, poleOfInaccessibility } from "./labelPlacement.js";
import type { Room } from "./types.js";

/** A room from Y-up points (payload space). */
const room = (...rings: [number, number][][]): Room => ({
  id: "r",
  loops: rings.map((r) => ({ points: r.map(([x, y]) => ({ x, y })) })),
});

// A T: a wide bar on top (y 8..10) over a narrow stem (x 4..6, y 0..8).
const T = room([
  [0, 10],
  [10, 10],
  [10, 8],
  [6, 8],
  [6, 0],
  [4, 0],
  [4, 8],
  [0, 8],
]);
// An L: a tall arm (x 0..2) and a foot (y 0..2) out to x 10.
const L = room([
  [0, 0],
  [10, 0],
  [10, 2],
  [2, 2],
  [2, 10],
  [0, 10],
]);

/** Whether the block placed lies wholly inside the room (flipped space). */
function inside(r: Room, p: { x: number; y: number; fontSize: number }, longest: number, lines = 1): boolean {
  const w = p.fontSize * GLYPH_ASPECT * longest;
  const h = p.fontSize * 1.2 * (1 + 0.7 * (lines - 1));
  const ring = r.loops![0]!.points.map(flip);
  const hit = (x: number, y: number) => {
    let c = false;
    for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
      const a = ring[i]!, b = ring[j]!;
      if (a.y > y !== b.y > y && x < ((b.x - a.x) * (y - a.y)) / (b.y - a.y) + a.x) c = !c;
    }
    return c;
  };
  return [
    [p.x - w / 2, p.y - h / 2],
    [p.x + w / 2, p.y - h / 2],
    [p.x - w / 2, p.y + h / 2],
    [p.x + w / 2, p.y + h / 2],
  ].every(([x, y]) => hit(x!, y!));
}

describe("placeLabel", () => {
  it("puts a T's label inside the T, where the centroid and the bbox would not keep it", () => {
    const p = placeLabel(T, 8, 1, 3)!;
    expect(inside(T, p, 8)).toBe(true);
  });

  it("puts an L's label inside the L, which a bbox-sized label would spill out of", () => {
    const p = placeLabel(L, 6, 1, 3)!;
    expect(inside(L, p, 6)).toBe(true);
  });

  it("keeps a stacked label inside as well", () => {
    const p = placeLabel(T, 6, 3, 3)!;
    expect(inside(T, p, 6, 3)).toBe(true);
  });

  it("gives a plain rectangle the full ceiling and its middle", () => {
    const box = room([
      [0, 0],
      [20, 0],
      [20, 10],
      [0, 10],
    ]);
    const p = placeLabel(box, 4, 1, 1)!;
    expect(p.fontSize).toBe(1);
    expect(p.x).toBeCloseTo(10, 0);
    expect(p.y).toBeCloseTo(-5, 0);
  });

  it("stays out of a hole", () => {
    // A 20 x 20 room with a 12 x 12 void in the middle: the pole must be in the ring.
    const holed = room(
      [
        [0, 0],
        [20, 0],
        [20, 20],
        [0, 20],
      ],
      [
        [4, 4],
        [16, 4],
        [16, 16],
        [4, 16],
      ],
    );
    const p = placeLabel(holed, 3, 1, 1)!;
    const inHole = p.x > 4 && p.x < 16 && p.y < -4 && p.y > -16;
    expect(inHole).toBe(false);
  });

  it("returns null for a room with no outline", () => {
    expect(placeLabel({ id: "x" }, 3, 1, 1)).toBeNull();
    expect(placeLabel(room([[0, 0], [1, 1]]), 3, 1, 1)).toBeNull();
  });

  it("shrinks the font for a narrow arm rather than letting it spill", () => {
    const wide = placeLabel(T, 3, 1, 5)!;
    const long = placeLabel(T, 30, 1, 5)!;
    expect(long.fontSize).toBeLessThan(wide.fontSize);
  });
});

describe("poleOfInaccessibility", () => {
  it("is the middle of a square, and the thick part of a T", () => {
    const sq = room([
      [0, 0],
      [10, 0],
      [10, 10],
      [0, 10],
    ]);
    const ring = (r: Room) => [r.loops![0]!.points.map(flip)];
    const s = poleOfInaccessibility(ring(sq), 0.01);
    expect(s.d).toBeCloseTo(5, 1);
    const t = poleOfInaccessibility(ring(T), 0.01);
    // The bar is 2 thick and the stem 2 wide: the pole sits where both are widest.
    expect(t.d).toBeGreaterThan(0.9);
  });
});
