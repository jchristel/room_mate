import { describe, expect, it } from "vitest";

import { MAX_ZOOM, WHOLE, kept, viewBoxOf, zoomedAbout } from "./useZoomPan.js";

const paper = { width: 420, height: 297 };

describe("zoom and pan", () => {
  it("show the whole sheet and its margin at 1", () => {
    const b = viewBoxOf(paper, WHOLE);
    expect([b.x, b.y, b.w, b.h]).toEqual([-5, -5, 430, 307]);
  });

  it("keep the paper point under the pointer where it was", () => {
    const p = { x: 100, y: 50 };
    const z = zoomedAbout(paper, WHOLE, p, 2);
    const [before, after] = [viewBoxOf(paper, WHOLE), viewBoxOf(paper, z)];
    // the point's share of the view box, left to right and top to bottom, is the same before and after
    expect((p.x - before.x) / before.w).toBeCloseTo((p.x - after.x) / after.w, 9);
    expect((p.y - before.y) / before.h).toBeCloseTo((p.y - after.y) / after.h, 9);
    expect(z.k).toBe(2);
  });

  it("stop at the limits and go back to the whole sheet at 1", () => {
    expect(zoomedAbout(paper, { k: MAX_ZOOM, c: { x: 1, y: 1 } }, { x: 0, y: 0 }, 2).k).toBe(MAX_ZOOM);
    expect(zoomedAbout(paper, { k: 2, c: { x: 1, y: 1 } }, { x: 0, y: 0 }, 0.1)).toEqual(WHOLE);
  });

  it("keep the centre on the paper", () => {
    expect(kept(paper, { x: -50, y: 900 })).toEqual({ x: 0, y: 297 });
  });
});
