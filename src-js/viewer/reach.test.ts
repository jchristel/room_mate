import { describe, expect, it } from "vitest";
import { describeReach, describeSaved, draftOf, layerFor, newReach, stateFromSaved, parseMetres, pickSource, problem, reachRequest, type ReachResult } from "./reach.js";

const scope = { projectId: "RHH", building: null, milestone: null } as never;

const done = (): ReachResult => ({
  state: "done",
  body: {
    limit_mm: 30000,
    method: "exact",
    levels: [{ level_id: "L1", name: "GROUND", origin: { x: 0, y: 0 }, spacing_ft: 3.28, cols: 2, rows: 1, distance_mm: [0, -2] }],
    summary: { cells: 2, covered: 1, fraction: 0.5, area_sqm: 2, covered_sqm: 1, rooms: 3, rooms_full: 1, rooms_partial: 1, rooms_none: 1 },
  },
});

describe("pickSource", () => {
  it("adds a room, removes it on a second click, and forgets a stale answer", () => {
    let s = { ...newReach(), result: done() };
    s = pickSource(s, "a", { x: 1, y: 2 });
    expect(s.sources).toEqual([{ roomId: "a", at: { x: 1, y: 2 } }]);
    expect(s.result.state).toBe("idle");
    s = pickSource(s, "b");
    expect(s.sources.map((x) => x.roomId)).toEqual(["a", "b"]);
    s = pickSource(s, "a");
    expect(s.sources.map((x) => x.roomId)).toEqual(["b"]);
  });
});

describe("the typed numbers", () => {
  it("reads metres as millimetres and refuses what is not a positive length", () => {
    expect(parseMetres("30")).toBe(30000);
    expect(parseMetres("0.5")).toBe(500);
    expect(parseMetres("")).toBeNull();
    expect(parseMetres("-3")).toBe("bad");
    expect(parseMetres("x")).toBe("bad");
  });

  it("says why a request cannot be made, before the server has to", () => {
    expect(problem(newReach())).toMatch(/Click a room/);
    const s = pickSource(newReach(), "a");
    expect(problem(s)).toBeNull();
    expect(problem({ ...s, limit: "" })).toMatch(/Walk/);
    expect(problem({ ...s, spacing: "0.01" })).toMatch(/Grid/);
    expect(problem({ ...s, spacing: "50" })).toMatch(/Grid/);
  });
});

describe("reachRequest", () => {
  it("carries the sources with their points, the limit and spacing in mm, the method and the cuts", () => {
    const s = { ...pickSource(pickSource(newReach(), "a", { x: 1, y: 2 }), "b"), method: "grid" as const, ignoreCuts: true };
    const req = reachRequest(scope, s)!;
    expect(req.url).toBe("/projects/RHH/reach");
    expect(JSON.parse(req.body)).toEqual({
      sources: [{ room_id: "a", x: 1, y: 2 }, { room_id: "b" }],
      limit_mm: 30000,
      spacing_mm: 1000,
      method: "grid",
      ignore_cuts: true,
    });
  });

  it("is null while there is nothing to ask", () => {
    expect(reachRequest(scope, newReach())).toBeNull();
  });
});

describe("layerFor", () => {
  it("is the grid of the zone's own storey at the reader's opacity", () => {
    const layer = layerFor(done(), "L1", 40)!;
    expect(layer.cols).toBe(2);
    expect(layer.alpha).toBeCloseTo(0.4);
    expect(layer.limit_mm).toBe(30000);
  });

  it("is nothing for another storey, no answer, or no storey", () => {
    expect(layerFor(done(), "L2", 55)).toBeNull();
    expect(layerFor({ state: "idle" }, "L1", 55)).toBeNull();
    expect(layerFor(done(), null, 55)).toBeNull();
  });

  it("never lets the map vanish or go solid", () => {
    expect(layerFor(done(), "L1", 0)!.alpha).toBeCloseTo(0.05);
    expect(layerFor(done(), "L1", 500)!.alpha).toBe(1);
  });
});

describe("describeReach", () => {
  it("states coverage and the rooms", () => {
    expect(describeReach(done())).toBe("50% covered (1 of 2 m²) · rooms: 1 full, 1 partial, 1 none");
  });
});

describe("saved reaches", () => {
  const ready = { ...newReach(), limit: "30", spacing: "2", method: "grid" as const, ignoreCuts: true };
  const picked = pickSource(pickSource(ready, "a", { x: 1, y: 2 }), "b");

  it("drafts what is asked in millimetres, and not at all when it cannot be asked", () => {
    expect(draftOf(picked, "Hose")).toEqual({
      name: "Hose",
      sources: [
        { roomId: "a", modelId: undefined, at: { x: 1, y: 2 } },
        { roomId: "b", modelId: undefined, at: null },
      ],
      limitMm: 30000,
      spacingMm: 2000,
      method: "grid",
      ignoreCuts: true,
    });
    expect(draftOf(ready, "none picked")).toBeNull();
    expect(draftOf({ ...picked, limit: "x" }, "bad walk")).toBeNull();
  });

  it("loads a saved reach back into the tool, unmeasured, keeping the reader's opacity", () => {
    const saved = {
      id: "h",
      name: "Hose",
      sources: [{ room: { model_id: "m", room_id: "a" }, at: { x: 1, y: 2 } }, { room: { model_id: "m", room_id: "b" } }],
      limit_mm: 30000,
      spacing_mm: 1500,
      method: "grid",
      ignore_cuts: true,
    };
    const state = stateFromSaved(saved, 40);
    expect(state).toMatchObject({ limit: "30", spacing: "1.5", method: "grid", ignoreCuts: true, opacity: 40, result: { state: "idle" } });
    expect(state.sources).toEqual([
      { roomId: "a", modelId: "m", at: { x: 1, y: 2 } },
      { roomId: "b", modelId: "m", at: null },
    ]);
    // The model rides the request, so two models sharing a room id cannot be confused.
    expect(JSON.parse(reachRequest(scope, state)!.body).sources[0]).toMatchObject({ room_id: "a", model_id: "m", x: 1, y: 2 });
    // A method the page does not know reads as the default.
    expect(stateFromSaved({ ...saved, method: null }, 55).method).toBe("exact");
  });

  it("says what a saved reach asks in one line", () => {
    const base = { id: "h", name: "H", sources: [{ room: { model_id: "m", room_id: "a" } }], limit_mm: 30000, spacing_mm: 1000, ignore_cuts: false };
    expect(describeSaved(base)).toBe("1 room · 30 m · exact");
    expect(describeSaved({ ...base, method: "grid", ignore_cuts: true })).toBe("1 room · 30 m · grid · ignoring cuts");
  });
});
