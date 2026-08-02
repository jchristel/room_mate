import { describe, expect, it } from "vitest";
import {
  bounds,
  centroid,
  fittedBounds,
  flip,
  loopBox,
  pointsAttr,
  roomBBox,
} from "./geometry.js";
import type { Loop, Room } from "./types.js";

const square: Loop = {
  points: [{ x: 0, y: 0 }, { x: 10, y: 0 }, { x: 10, y: 6 }, { x: 0, y: 6 }],
};

const room = (loops: Loop[]): Room => ({ id: "r", loops });

/**
 * Normalize `-0` to `0` before comparing.
 *
 * `flip()` turns y=0 into -0, so extents and centroids legitimately carry it.
 * That is harmless where it matters — `String(-0)` is `"0"`, so a points
 * attribute and an exported .svg are unaffected — but `toEqual` distinguishes
 * the two and would fail on a difference no output can express. Normalizing
 * here says that explicitly, rather than sprinkling `-0` literals through the
 * expectations and leaving the next reader to wonder which are load-bearing.
 */
function z<T extends object>(o: T): T {
  return Object.fromEntries(
    Object.entries(o).map(([k, v]) => [k, v === 0 ? 0 : v]),
  ) as T;
}

describe("flip", () => {
  it("negates Y and leaves X alone", () => {
    // The single place the coordinate space changes. Payload data is Y-up;
    // everything downstream of here is Y-down.
    expect(flip({ x: 3, y: 4 })).toEqual({ x: 3, y: -4 });
  });

  it("maps -0 to 0 without changing the string form", () => {
    // `-0` serializes as "0" in a points attribute, so this must not become a
    // silent diff in an exported .svg.
    expect(String(flip({ x: 1, y: 0 }).y)).toBe("0");
  });
});

describe("bounds", () => {
  it("returns the flipped extent across every loop of every room", () => {
    expect(z(bounds([room([square])])!)).toEqual({ minX: 0, minY: -6, maxX: 10, maxY: 0 });
  });

  it("returns null when nothing is drawable", () => {
    // Not an error: a door family with no 3D geometry legitimately arrives with
    // empty loops, and a level of them is empty, not broken.
    expect(bounds([])).toBeNull();
    expect(bounds([room([])])).toBeNull();
  });

  it("ignores a room with no loops rather than throwing", () => {
    expect(z(bounds([{ id: "x" }, room([square])])!)).toEqual({
      minX: 0,
      minY: -6,
      maxX: 10,
      maxY: 0,
    });
  });
});

describe("loopBox", () => {
  it("returns width and height in the flipped space", () => {
    expect(loopBox(square)).toEqual({ w: 10, h: 6 });
  });
});

describe("roomBBox", () => {
  it("uses ONLY the outer ring, ignoring holes", () => {
    // A hole is inside the outer ring by definition, so including it could only
    // ever produce the same box or a wrong one -- but the cull reads this box,
    // and a wrong one hides a room that is on screen.
    const hole: Loop = { points: [{ x: 2, y: 2 }, { x: 4, y: 2 }, { x: 4, y: 4 }] };
    expect(z(roomBBox(room([square, hole])))).toEqual({ minX: 0, minY: -6, maxX: 10, maxY: 0 });
  });
});

describe("pointsAttr", () => {
  it("serializes flipped points as `x,y` separated by spaces", () => {
    expect(pointsAttr(square)).toBe("0,0 10,0 10,-6 0,-6");
  });
});

describe("centroid", () => {
  it("returns the area centroid of a convex ring", () => {
    expect(z(centroid(square))).toEqual({ x: 5, y: -3 });
  });

  it("falls back to the vertex average for a zero-area ring", () => {
    // Load-bearing, not defensive: the area formula divides by zero here, and a
    // NaN centroid renders no label at all -- which reads as a missing label
    // rather than as bad geometry.
    const degenerate: Loop = {
      points: [{ x: 0, y: 0 }, { x: 4, y: 0 }, { x: 8, y: 0 }],
    };
    expect(z(centroid(degenerate))).toEqual({ x: 4, y: 0 });
  });

  it("sits inside the notch-free part of a concave ring", () => {
    const l: Loop = {
      points: [
        { x: 0, y: 0 }, { x: 24, y: 0 }, { x: 24, y: 8 },
        { x: 10, y: 8 }, { x: 10, y: 20 }, { x: 0, y: 20 },
      ],
    };
    const c = centroid(l);
    expect(c.x).toBeGreaterThan(0);
    expect(c.x).toBeLessThan(24);
  });
});

describe("fittedBounds", () => {
  it("pads by 4% of the larger extent", () => {
    // 10 wide, 6 tall -> pad 0.4 on every side.
    expect(fittedBounds([room([square])])).toEqual({
      x: -0.4,
      y: -6.4,
      w: 10.8,
      h: 6.8,
    });
  });

  it("gives a degenerate level a non-zero frame", () => {
    // A single point has zero extent, so a proportional pad would be 0 and the
    // viewBox would have zero width -- which renders nothing at all.
    const point: Loop = { points: [{ x: 5, y: 5 }] };
    const f = fittedBounds([room([point])]);
    expect(f).not.toBeNull();
    expect(f!.w).toBeGreaterThan(0);
    expect(f!.h).toBeGreaterThan(0);
  });

  it("returns null when nothing is drawable", () => {
    expect(fittedBounds([])).toBeNull();
  });
});
