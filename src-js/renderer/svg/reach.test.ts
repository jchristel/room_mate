// The reach layer in the exported SVG: banded paths, a legend, and nothing when
// nothing is covered.

import { describe, expect, it } from "vitest";
import type { ReachLayer } from "../seam.js";
import { paintLevel } from "./paint.js";
import { paintReach } from "./reach.js";

const SVG_NS = "http://www.w3.org/2000/svg";
const svg = () => document.createElementNS(SVG_NS, "svg");
const fitted = { x: 0, y: -10, w: 10, h: 10 };

// Row 0 is the plan's lowest row. 2 x 2 cells: a near one, a far one, one in no
// room (-1) and one beyond the limit (-2).
const layer = (distance_mm: number[], alpha?: number): ReachLayer => ({
  origin: { x: 0, y: 0 },
  spacing_ft: 1,
  cols: 2,
  rows: 2,
  distance_mm,
  limit_mm: 10000,
  ...(alpha === undefined ? {} : { alpha }),
});

describe("paintReach", () => {
  it("draws nothing when no cell is covered", () => {
    const s = svg();
    paintReach(s, layer([-1, -2, -2, -1]), fitted);
    expect(s.childNodes.length).toBe(0);
    paintReach(s, undefined, fitted);
    expect(s.childNodes.length).toBe(0);
  });

  it("draws covered cells only, banded by distance, with the opacity on the group", () => {
    const s = svg();
    paintReach(s, layer([0, 10000, -1, -2], 0.4), fitted);
    const group = s.querySelector("g.reach")!;
    expect(group.getAttribute("fill-opacity")).toBe("0.4");
    const paths = [...group.querySelectorAll("path")];
    expect(paths.length).toBe(2);
    expect(paths.every((p) => (p.getAttribute("d") ?? "").split("M").length === 2)).toBe(true);
    expect(new Set(paths.map((p) => p.getAttribute("fill"))).size).toBe(2);
  });

  it("flips y: the plan's row 0 sits just above the file's zero line", () => {
    const s = svg();
    paintReach(s, layer([5000, -1, -1, -1]), fitted);
    expect(s.querySelector("g.reach path")!.getAttribute("d")).toBe("M0 -1h1v1h-1z");
  });

  it("states its scale in a legend", () => {
    const s = svg();
    paintReach(s, layer([5000, -1, -1, -1]), fitted);
    const texts = [...s.querySelectorAll("g.reach-legend text")].map((t) => t.textContent);
    expect(texts).toEqual(["Walk 0 m", "10 m"]);
  });

  it("sits under the labels when painted with a level", () => {
    const s = svg();
    const room = {
      id: "r",
      name: "R",
      level_id: "L",
      loops: [{ points: [{ x: 0, y: 0 }, { x: 2, y: 0 }, { x: 2, y: 2 }, { x: 0, y: 2 }] }],
    } as unknown as Parameters<typeof paintLevel>[1][number];
    paintLevel(s, [room], fitted, { reach: layer([5000, -1, -1, -1]) });
    const order = [...s.children].map((c) => c.getAttribute("class") ?? c.tagName);
    expect(order.indexOf("reach")).toBeGreaterThan(order.indexOf("grid"));
    expect(order.indexOf("reach")).toBeLessThan(order.indexOf("label"));
  });

  it("writes the walk on each cell only when asked, thin ink on a pale halo", () => {
    const s = svg();
    paintReach(s, layer([5000, 12000, -1, -2]), fitted);
    expect(s.querySelector("g.reach-tags")).toBeNull();
    const t = svg();
    paintReach(t, { ...layer([5000, 12000, -1, -2]), tags: true }, fitted);
    const g = t.querySelector("g.reach-tags")!;
    expect([...g.querySelectorAll("text")].map((x) => x.textContent)).toEqual(["5.0", "12.0"]);
    expect(g.getAttribute("paint-order")).toBe("stroke");
    // The tags are over the cells and under the legend.
    const order = [...t.children].map((c) => c.getAttribute("class"));
    expect(order).toEqual(["reach", "reach-tags", "reach-legend"]);
  });
});
