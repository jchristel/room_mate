import { describe, expect, it } from "vitest";

import {
  alongValue,
  followersOf,
  importPlacements,
  moveToEdge,
  offValue,
  sameItems,
  setAlongMode,
  setAlongValue,
  setExcluded,
  setFacing,
  setOffMode,
  setOffValue,
} from "./draft.js";
import type { Pt } from "./geometry.js";
import type { BlueprintItem } from "./types.js";

const ring: Pt[] = [
  [0, 0],
  [6000, 0],
  [6000, 4000],
  [0, 4000],
];

const make = (id: string, over: Partial<BlueprintItem["placement"]> = {}): BlueprintItem => ({
  id,
  type_name: "Bed",
  category: "Furniture",
  class: "free",
  excluded: false,
  size_mm: [1000, 1600],
  properties: [],
  type_properties: [],
  placement: {
    edge: 0,
    along_mm: 1500,
    along_mode: "fixed_from_start",
    off_mm: 500,
    off_mode: "fixed",
    edge_len_mm: 6000,
    depth_mm: 4000,
    facing_deg: 0,
    ...over,
  },
});

describe("the figure a person reads", () => {
  it("is in the unit its mode implies", () => {
    expect(alongValue(make("a").placement)).toBe(1500);
    expect(alongValue(make("a", { along_mode: "fixed_from_end" }).placement)).toBe(4500);
    expect(alongValue(make("a", { along_mode: "percent" }).placement)).toBeCloseTo(25);
    expect(offValue(make("a", { off_mode: "percent", off_mm: 1000 }).placement)).toBeCloseTo(25);
  });
});

describe("editing a dimension", () => {
  it("moves the item when the figure changes, in whatever unit the mode uses", () => {
    expect(setAlongValue(make("a"), ring, 2000).placement.along_mm).toBe(2000);
    expect(setAlongValue(make("a", { along_mode: "fixed_from_end" }), ring, 1000).placement.along_mm).toBe(5000);
    expect(setAlongValue(make("a", { along_mode: "percent" }), ring, 50).placement.along_mm).toBe(3000);
    expect(setOffValue(make("a"), 700).placement.off_mm).toBe(700);
    expect(setOffValue(make("a", { off_mode: "percent" }), 25).placement.off_mm).toBe(1000);
  });

  it("changing the MODE moves nothing: only what happens when the room changes", () => {
    const a = make("a");
    const b = setOffMode(setAlongMode(a, "percent"), "percent");
    expect(b.placement.along_mm).toBe(a.placement.along_mm);
    expect(b.placement.off_mm).toBe(a.placement.off_mm);
    expect(b.placement.along_mode).toBe("percent");
  });

  it("never mutates the item it is given", () => {
    const a = make("a");
    const before = JSON.stringify(a);
    setAlongValue(a, ring, 99);
    setFacing(a, 90);
    expect(JSON.stringify(a)).toBe(before);
  });
});

describe("moveToEdge", () => {
  it("re-measures the edge's length and the depth in front of it", () => {
    const moved = moveToEdge(make("a", { along_mm: 5000 }), ring, 1);
    expect(moved.placement.edge).toBe(1);
    expect(moved.placement.edge_len_mm).toBe(4000);
    expect(moved.placement.along_mm).toBe(4000); // clamped to the shorter edge
    expect(Math.abs(moved.placement.depth_mm - 6000)).toBeLessThan(1);
  });

  it("leaves the item alone for an edge that is not there", () => {
    const a = make("a");
    expect(moveToEdge(a, ring, 9)).toBe(a);
  });
});

describe("excluding", () => {
  it("marks items excluded and back, and keeps them", () => {
    const items = [make("a"), make("b")];
    const out = setExcluded(items, new Set(["a"]), true);
    expect(out.map((i) => i.excluded)).toEqual([true, false]);
    expect(out).toHaveLength(2);
    expect(setExcluded(out, new Set(["a"]), false)[0]?.excluded).toBe(false);
  });
});

describe("importing placements from another revision", () => {
  it("copies the placement of the chosen items that the source holds", () => {
    const now = [make("a", { along_mm: 100 }), make("b", { along_mm: 100 }), make("c")];
    const then = [make("a", { along_mm: 900 }), make("c")];
    const r = importPlacements(now, then, new Set(["a", "b", "c"]), 4);
    expect(r.items[0]?.placement.along_mm).toBe(900);
    expect(r.items[1]?.placement.along_mm).toBe(100); // not in the source
    expect(r.changed).toBe(1); // c was already identical
  });

  it("leaves items that were not chosen, and edges the outline lacks", () => {
    const now = [make("a"), make("b")];
    const then = [make("a", { along_mm: 900 }), make("b", { edge: 7, along_mm: 900 })];
    const r = importPlacements(now, then, new Set(["a", "b"]), 4);
    expect(r.items[0]?.placement.along_mm).toBe(900);
    expect(r.items[1]?.placement.along_mm).toBe(1500);
    expect(importPlacements(now, then, new Set(), 4).changed).toBe(0);
  });
});

describe("small helpers", () => {
  it("finds followers and compares lists", () => {
    const a = make("a");
    const b = { ...make("b"), follows: { item: "a", gap_mm: 0, inside: false } };
    expect(followersOf([a, b], "a").map((i) => i.id)).toEqual(["b"]);
    expect(sameItems([a], [make("a")])).toBe(true);
    expect(sameItems([a], [b])).toBe(false);
  });
});
