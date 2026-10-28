import { describe, expect, it } from "vitest";

import { alignEdge, dominantAngle, insertNode, moveNode, removeNode, rotatePoint, snapNode } from "./boxEdit.js";
import type { Pt } from "./geometry.js";

const rect: Pt[] = [
  [0, 0],
  [6000, 0],
  [6000, 4000],
  [0, 4000],
];

describe("snapNode", () => {
  it("makes the two lines a node ends horizontal or vertical when the pointer is near", () => {
    // Node 1 has neighbours (0,0) and (6000,4000): near y=0 and near x=6000.
    expect(snapNode(rect, 1, [6030, 40], 100)).toEqual([6000, 0]);
  });
  it("leaves an axis alone when no neighbour is within the tolerance", () => {
    expect(snapNode(rect, 1, [5000, 40], 100)).toEqual([5000, 0]);
    expect(snapNode(rect, 1, [5000, 900], 100)).toEqual([5000, 900]);
  });
  it("takes the nearer of two neighbours", () => {
    const ring: Pt[] = [[0, 0], [1000, 500], [1040, 2000], [0, 2000]];
    expect(snapNode(ring, 1, [1020, 1000], 100)[0]).toBe(1040);
  });
});

describe("nodes", () => {
  it("inserts at the point of the line nearest the click, after the edge's start", () => {
    const next = insertNode(rect, 1, [6200, 1500]);
    expect(next).toHaveLength(5);
    expect(next[2]).toEqual([6000, 1500]);
    expect(next[3]).toEqual([6000, 4000]);
  });
  it("inserts on the closing edge at the end of the list", () => {
    const next = insertNode(rect, 3, [-50, 1000]);
    expect(next[4]).toEqual([0, 1000]);
  });
  it("never goes below four nodes", () => {
    expect(removeNode(rect, 2)).toBeNull();
    const five = insertNode(rect, 0, [3000, 0]);
    expect(removeNode(five, 1)).toEqual(rect);
  });
  it("moves one node and leaves the rest", () => {
    expect(moveNode(rect, 2, [6500, 4200])[2]).toEqual([6500, 4200]);
    expect(moveNode(rect, 2, [6500, 4200])[1]).toEqual([6000, 0]);
  });
});

describe("alignEdge", () => {
  it("lays a side along a diagonal wall by dropping its two nodes onto the wall's line", () => {
    // The wall runs at 45 degrees through the origin.
    const out = alignEdge(rect, 1, [0, 0], [1000, 1000]);
    for (const k of [1, 2]) expect(Math.abs(out[k]![0] - out[k]![1])).toBeLessThan(1e-9);
    expect(out[0]).toEqual([0, 0]);
    expect(out[3]).toEqual([0, 4000]);
  });
});

describe("dominantAngle", () => {
  it("reads a turned rectangle as its turn, in a quarter turn", () => {
    const turned = rect.map((p) => rotatePoint(p, 31));
    expect(dominantAngle(turned)).toBeCloseTo(31, 6);
    expect(dominantAngle(rect.map((p) => rotatePoint(p, 100)))).toBeCloseTo(10, 6);
    expect(dominantAngle(rect)).toBe(0);
  });
});
