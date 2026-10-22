import { describe, expect, it } from "vitest";

import { chooseBlueprint, describeTool, ghostCorners, newBlueprintTool, pickRoom, resolveUrl, stepRotation, type BlueprintToolState } from "./blueprintTool.js";

const ready = (over: Partial<BlueprintToolState> = {}): BlueprintToolState => ({ ...newBlueprintTool(), blueprintId: "bp 1", roomId: "r9", ...over });

describe("the blueprint tool", () => {
  it("asks nothing until it has a blueprint and a room", () => {
    expect(resolveUrl("p", newBlueprintTool())).toBeNull();
    expect(resolveUrl("p", { ...newBlueprintTool(), blueprintId: "b" })).toBeNull();
    expect(resolveUrl("p", ready())).toBe("/projects/p/blueprints/bp%201/resolve?room_id=r9");
  });

  it("carries a revision and a rotation when they are chosen", () => {
    const url = resolveUrl("House A", ready({ revision: 2, shift: 3 }));
    expect(url).toBe("/projects/House%20A/blueprints/bp%201/resolve?room_id=r9&revision=2&shift=3");
  });

  it("moves the ghost to another room at the best fit, and leaves the same room alone", () => {
    const here = ready({ shift: 2, result: { state: "loading" } });
    expect(pickRoom(here, "r9")).toBe(here);
    const there = pickRoom(here, "r10");
    expect(there.roomId).toBe("r10");
    expect(there.shift).toBeNull();
    expect(there.result.state).toBe("idle");
  });

  it("forgets the rotation when the blueprint changes but keeps the room", () => {
    const s = chooseBlueprint(ready({ shift: 2, revision: 3 }), "other");
    expect(s).toMatchObject({ blueprintId: "other", roomId: "r9", shift: null, revision: null });
  });

  it("steps through the rotations that fit and wraps round", () => {
    const cands = [{ shift: 0 }, { shift: 2 }];
    const one = stepRotation(ready(), cands, 0, 1)!;
    expect(one.shift).toBe(2);
    expect(stepRotation(ready(), cands, 2, 1)!.shift).toBe(0);
    expect(stepRotation(ready(), cands, 0, -1)!.shift).toBe(2);
  });

  it("has nothing to step through with one rotation or none", () => {
    expect(stepRotation(ready(), [{ shift: 0 }], 0, 1)).toBeNull();
    expect(stepRotation(ready(), [], 0, 1)).toBeNull();
  });

  it("draws an item's footprint from its centre, facing and size", () => {
    // 1000 along +X, 400 across: a box from x 4500..5500, y 1800..2200.
    const c = ghostCorners({ centre: { x: 5000, y: 2000 }, facing: { x: 1, y: 0 }, size_mm: [1000, 400] });
    const xs = c.map((p) => Math.round(p.x)).sort((a, b) => a - b);
    const ys = c.map((p) => Math.round(p.y)).sort((a, b) => a - b);
    expect([xs[0], xs[3], ys[0], ys[3]]).toEqual([4500, 5500, 1800, 2200]);
  });

  it("turns the footprint with the facing", () => {
    const c = ghostCorners({ centre: { x: 0, y: 0 }, facing: { x: 0, y: 1 }, size_mm: [1000, 400] });
    const xs = c.map((p) => Math.round(p.x));
    const ys = c.map((p) => Math.round(p.y));
    expect(Math.max(...xs)).toBe(200);
    expect(Math.max(...ys)).toBe(500);
  });

  it("says where it is in the sequence of a person's choices", () => {
    expect(describeTool(newBlueprintTool(), null)).toBe("choose a blueprint");
    expect(describeTool({ ...newBlueprintTool(), blueprintId: "b" }, null)).toBe("click a room to lay it in");
    const done = ready({
      result: { state: "done", body: { revision: 1, room_name: "BED 02", fit: { kind: "plain", shift: 0 }, placements: [], findings: [{ kind: "outside" }], candidates: [] } },
    });
    expect(describeTool(done, "BED 02 01.02")).toBe("BED 02 01.02: 0 items, fits by shape, 1 to look at");
  });
});
