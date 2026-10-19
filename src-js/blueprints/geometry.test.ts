import { describe, expect, it } from "vitest";

import { boundsOf, corners, depthAt, edges, frameOf, pointInPolygon, pointOn, type Pt } from "./geometry.js";
import type { BlueprintItem } from "./types.js";

const rect = (w: number, h: number): Pt[] => [
  [0, 0],
  [w, 0],
  [w, h],
  [0, h],
];

const item = (over: Partial<BlueprintItem["placement"]> = {}): BlueprintItem => ({
  id: "i",
  type_name: "Bed",
  category: "Furniture",
  class: "free",
  excluded: false,
  size_mm: [1000, 1600],
  properties: [],
  type_properties: [],
  placement: {
    edge: 0,
    along_mm: 3000,
    along_mode: "percent",
    off_mm: 600,
    off_mode: "fixed",
    edge_len_mm: 6000,
    depth_mm: 4000,
    facing_deg: 0,
    ...over,
  },
});

describe("edges", () => {
  it("runs counter-clockwise with the normal pointing into the room", () => {
    const es = edges(rect(6000, 4000));
    expect(es.map((e) => e.len)).toEqual([6000, 4000, 6000, 4000]);
    expect(es[0]?.n).toEqual([-0, 1]);
    es.forEach((e) => expect(pointInPolygon(pointOn(e, e.len / 2, 100), rect(6000, 4000))).toBe(true));
  });
});

describe("depthAt", () => {
  it("is the distance to the wall in front", () => {
    const ring = rect(6000, 4000);
    const es = edges(ring);
    // The ray starts half a millimetre inside the wall, as on the server.
    expect(Math.abs(depthAt(ring, 0, pointOn(es[0] as never, 1000, 0)) - 4000)).toBeLessThan(1);
    expect(Math.abs(depthAt(ring, 1, pointOn(es[1] as never, 1000, 0)) - 6000)).toBeLessThan(1);
  });
});

describe("frameOf", () => {
  it("puts an item where its recipe says, facing along the edge normal turned by the facing", () => {
    const f = frameOf(item(), rect(6000, 4000));
    expect(f?.centre[0]).toBeCloseTo(3000);
    expect(f?.centre[1]).toBeCloseTo(600);
    expect(f?.angle).toBeCloseTo(Math.PI / 2);
    const turned = frameOf(item({ facing_deg: -90 }), rect(6000, 4000));
    expect(turned?.angle).toBeCloseTo(0);
  });

  it("is null for an edge the outline does not have", () => {
    expect(frameOf(item({ edge: 9 }), rect(6000, 4000))).toBeNull();
  });
});

describe("corners", () => {
  it("lays the footprint out along the facing, then across it", () => {
    const f = frameOf(item(), rect(6000, 4000));
    const box = corners(f as never, [1000, 1600]);
    const b = boundsOf(box);
    // Facing +y: 1000 deep up the plan, 1600 wide across it.
    expect(b.maxX - b.minX).toBeCloseTo(1600);
    expect(b.maxY - b.minY).toBeCloseTo(1000);
  });
});
