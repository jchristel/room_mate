import { describe, expect, it } from "vitest";

import { areaKey, type AreaGroup } from "../areas.js";
import type { ClassificationTier } from "../../renderer/types.js";
import { buildView, colourKey, colourKeys, tierNames, type AdjacencyNode, type AdjacencyPayload } from "./view.js";

const tier = (t: string, name: string, code: string | null = null, undef = false): ClassificationTier => ({
  tier: t,
  name,
  code,
  undefined: undef,
});

const node = (id: string, cls: ClassificationTier[], x = 0, y = 0, level = "L1"): AdjacencyNode => ({
  room_id: id,
  name: id,
  level_id: level,
  classification: cls,
  centroid: { x, y },
});

const A = tier("Building", "A");
const MED = tier("Dept", "Medical");
const SURG = tier("Dept", "Surgery");

// r1, r2 in Medical; r3 in Surgery; r4 classified only to the building.
const payload: AdjacencyPayload = {
  nodes: [
    node("r1", [A, MED], 0, 0),
    node("r2", [A, MED], 2, 0),
    node("r3", [A, SURG], 4, 0),
    node("r4", [A], 6, 0),
  ],
  edges: [
    { a: "r1", b: "r2", shared_length: 5 },
    { a: "r1", b: "r3", shared_length: 2 },
    { a: "r2", b: "r3", shared_length: 3 },
    { a: "r3", b: "r4", shared_length: 7 },
  ],
};

describe("buildView, rooms", () => {
  it("is the payload verbatim, with neighbours both ways", () => {
    const view = buildView(payload, null);
    expect([...view.nodesById.keys()]).toEqual(["r1", "r2", "r3", "r4"]);
    expect(view.neighbours.get("r3")).toEqual([
      { id: "r1", weight: 2 },
      { id: "r2", weight: 3 },
      { id: "r4", weight: 7 },
    ]);
  });

  it("is empty for no payload", () => {
    const view = buildView(null, null);
    expect(view.nodesById.size).toBe(0);
    expect(view.edges).toEqual([]);
  });
});

describe("buildView, areas", () => {
  it("names each group by the viewer's areaKey, at every tier", () => {
    // The regression this module exists to prevent: graph.js built these ids
    // with its own pathKey, which joined with "/" where areas.ts joins with ">",
    // so every group below tier 0 matched no footprint.
    const group = (path: ClassificationTier[]): AreaGroup => ({ level_id: "L1", path, area: 0, polygons: [] });
    expect([...buildView(payload, 0).nodesById.keys()]).toEqual([areaKey(group([A]))]);
    expect([...buildView(payload, 1).nodesById.keys()].sort()).toEqual(
      [areaKey(group([A, MED])), areaKey(group([A, SURG]))].sort(),
    );
  });

  it("sums the walls between groups and drops the walls inside one", () => {
    const view = buildView(payload, 1);
    // r1-r2 is inside Medical; r1-r3 and r2-r3 are Medical-Surgery, 2 + 3.
    expect(view.edges).toHaveLength(1);
    expect(view.edges[0]!.weight).toBe(5);
  });

  it("drops a room classified shallower than the tier, and its edges", () => {
    const view = buildView(payload, 1);
    const members = [...view.nodesById.values()].reduce((n, g) => n + g.rooms, 0);
    expect(members).toBe(3);
    // r3-r4 went with r4.
    expect(view.edges.every((e) => e.weight !== 7)).toBe(true);
  });

  it("counts members and seeds from their mean centroid", () => {
    const med = [...buildView(payload, 1).nodesById.values()].find((g) => g.name === "Medical")!;
    expect(med.rooms).toBe(2);
    expect(med.centroid).toEqual({ x: 1, y: 0 });
    // Aggregating must not move the payload's own centroids.
    expect(payload.nodes[0]!.centroid).toEqual({ x: 0, y: 0 });
  });

  it("keeps the same department on two levels as two groups", () => {
    const twoLevels: AdjacencyPayload = {
      nodes: [node("a", [A, MED], 0, 0, "L1"), node("b", [A, MED], 0, 0, "L2")],
      edges: [],
    };
    expect(buildView(twoLevels, 1).nodesById.size).toBe(2);
  });
});

describe("colour keys", () => {
  it("prefers the code, falls back to the name, and is null for an undefined tier", () => {
    expect(colourKey([tier("Dept", "Medical", "MED")], 0)).toBe("MED");
    expect(colourKey([tier("Dept", "Medical")], 0)).toBe("Medical");
    expect(colourKey([tier("Dept", "x", null, true)], 0)).toBeNull();
    expect(colourKey([A], 1)).toBeNull();
  });

  it("are sorted and taken from the rooms, so an index is stable across granularities", () => {
    expect(colourKeys(payload, 1)).toEqual(["Medical", "Surgery"]);
    expect(colourKeys(null, 0)).toEqual([]);
  });
});

describe("tierNames", () => {
  it("offers every tier any room carries", () => {
    expect(tierNames(payload)).toEqual(["Building", "Dept"]);
    expect(tierNames(null)).toEqual([]);
  });
});
