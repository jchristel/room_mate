import { describe, expect, it } from "vitest";
import { REACH_ALPHA, reachCells, reachColour } from "./reach.js";
import type { ReachLayer } from "./seam.js";

const layer = (distance_mm: number[], over: Partial<ReachLayer> = {}): ReachLayer => ({
  origin: { x: 10, y: 20 },
  spacing_ft: 2,
  cols: 3,
  rows: 2,
  distance_mm,
  limit_mm: 1000,
  ...over,
});

describe("reachCells", () => {
  it("draws covered cells only: no room and beyond-the-limit are left bare", () => {
    const cells = reachCells(layer([0, 500, -1, -2, 1000, 250]));
    expect(cells).toHaveLength(4);
  });

  it("places a cell at origin + (col, row) * spacing, with y negated for the renderer", () => {
    // Row 1, col 2 of a 3-wide grid: x 14..16, y 22..24 in the plan, so -22..-24 drawn.
    const [cell] = reachCells(layer([-1, -1, -1, -1, -1, 100]));
    expect(cell!.triangles).toEqual([14, -22, 16, -22, 16, -24, 14, -22, 16, -24, 14, -24]);
  });

  it("is translucent by default, so the plan beneath shows, and takes an override", () => {
    expect(reachCells(layer([0]))[0]!.colour[3]).toBe(REACH_ALPHA);
    expect(reachCells(layer([0], { alpha: 0.3 }))[0]!.colour[3]).toBe(0.3);
    expect(REACH_ALPHA).toBeGreaterThan(0);
    expect(REACH_ALPHA).toBeLessThan(1);
  });

  it("an empty or fully uncovered layer draws nothing", () => {
    expect(reachCells(layer([-1, -2, -1, -2, -1, -2]))).toEqual([]);
  });
});

describe("reachColour", () => {
  it("runs from blue at the source to red at the limit and clamps past it", () => {
    const near = reachColour(0, 1000);
    const far = reachColour(1000, 1000);
    expect(near[2]).toBeGreaterThan(near[0]);
    expect(far[0]).toBeGreaterThan(far[2]);
    expect(reachColour(5000, 1000)).toEqual(far);
  });

  it("is continuous across a stop", () => {
    const a = reachColour(329, 1000);
    const b = reachColour(331, 1000);
    for (let i = 0; i < 3; i++) expect(Math.abs(a[i]! - b[i]!)).toBeLessThan(0.02);
  });
});
