import { describe, expect, it } from "vitest";
import { REACH_ALPHA, formatReachMetres, reachCells, reachColour, reachTags } from "./reach.js";
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

describe("reachTags", () => {
  it("writes the walk in metres on each covered cell, at its centre in the flipped frame", () => {
    const { tags, font, stride } = reachTags(layer([0, 12345, -1, -2, 100000, 5000]), 100);
    expect(stride).toBe(1);
    expect(tags.map((t) => t.text)).toEqual(["0.0", "12.3", "100", "5.0"]);
    // Cell (1, 0): origin (10, 20), spacing 2: its centre is (13, 21), and y is negated.
    expect(tags[1]).toMatchObject({ x: 13, y: -21 });
    // Five glyphs of 0.6 aspect fit a cell at 0.3 of its width.
    expect(font).toBeCloseTo(0.6);
  });

  it("thins to a budget by whole rows and columns and grows the font to match", () => {
    const wide = { origin: { x: 0, y: 0 }, spacing_ft: 1, cols: 10, rows: 10, distance_mm: Array.from({ length: 100 }, (_, i) => i * 10), limit_mm: 1000 };
    const { tags, font, stride } = reachTags(wide, 25);
    expect(stride).toBe(2);
    expect(tags).toHaveLength(25);
    expect(font).toBeCloseTo(0.6);
    expect(reachTags(wide, 100).tags).toHaveLength(100);
  });

  it("formats a walk to a decimal, whole above a hundred metres", () => {
    expect(formatReachMetres(1234)).toBe("1.2");
    expect(formatReachMetres(99950)).toBe("100.0");
    expect(formatReachMetres(100400)).toBe("100");
  });
});
