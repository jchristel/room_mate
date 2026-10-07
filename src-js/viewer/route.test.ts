import { describe, expect, it } from "vitest";

import {
  connectivityUrl,
  describeResult,
  markerPoint,
  newRoute,
  pickEndpoint,
  polylinePoints,
  segmentsOnLevel,
  type RoutePath,
  type RouteState,
} from "./route.js";

const ready = (unreachable: string[] = []): RouteState => ({ ...newRoute(), unreachable: new Set(unreachable) });

const path = (found: boolean): RoutePath => ({
  found,
  reason: found ? null : "no route through doors",
  distance_ft: 41.6,
  rooms: [],
  steps: [
    { kind: "door", door_id: "d1", point: { x: 1, y: 2 } },
    { kind: "door", door_id: "d2", point: { x: 3, y: 4 } },
  ],
  segments: [
    { level_id: "L1", points: [{ x: 0, y: 0 }, { x: 5, y: 5 }] },
    { level_id: "L2", points: [{ x: 9, y: 9 }] },
  ],
});

describe("pickEndpoint", () => {
  it("fills start, then end, then replaces the end", () => {
    let r = pickEndpoint(ready(), "a");
    expect(r.start).toBe("a");
    r = pickEndpoint(r, "b");
    expect([r.start, r.end]).toEqual(["a", "b"]);
    r = pickEndpoint(r, "c");
    expect([r.start, r.end]).toEqual(["a", "c"]);
  });

  it("refuses a room no door reaches and changes nothing else", () => {
    const before = pickEndpoint(ready(["bay"]), "a");
    const after = pickEndpoint(before, "bay");
    expect(after.start).toBe("a");
    expect(after.end).toBeNull();
    expect(after.notice).toEqual({ kind: "no-door", roomId: "bay" });
  });

  it("refuses an unreachable room as the END too", () => {
    const r = pickEndpoint(pickEndpoint(ready(["bay"]), "a"), "bay");
    expect(r.end).toBeNull();
  });

  it("ignores a click until the door connections are known", () => {
    const r = pickEndpoint(newRoute(), "a");
    expect(r.start).toBeNull();
    expect(r.notice).toEqual({ kind: "loading" });
  });

  it("clears a placed endpoint when it is clicked again, and the result with it", () => {
    let r = pickEndpoint(pickEndpoint(ready(), "a"), "b");
    r = { ...r, result: { state: "done", path: path(true) } };
    const cleared = pickEndpoint(r, "b");
    expect(cleared.end).toBeNull();
    expect(cleared.result).toEqual({ state: "idle" });
    expect(pickEndpoint(r, "a").start).toBeNull();
  });

  it("a good pick clears an earlier notice", () => {
    const refused = pickEndpoint(ready(["bay"]), "bay");
    expect(pickEndpoint(refused, "a").notice).toBeNull();
  });

  it("does not mutate its input", () => {
    const r = ready();
    pickEndpoint(r, "a");
    expect(r.start).toBeNull();
  });
});

describe("connectivityUrl", () => {
  const scope = { projectId: "p 1", building: null, milestone: null };

  it("is the bare summary until both ends are placed", () => {
    expect(connectivityUrl(scope, "a", null)).toBe("/projects/p%201/connectivity");
    expect(connectivityUrl(scope, null, null)).toBe("/projects/p%201/connectivity");
  });

  it("carries the scope and both ends", () => {
    const url = connectivityUrl({ ...scope, building: "B1", milestone: "Freeze" }, "a", "b");
    expect(url).toBe("/projects/p%201/connectivity?building=B1&milestone=Freeze&from=a&to=b");
  });

  it("carries the method only with a route, and omits it for the default", () => {
    expect(connectivityUrl(scope, "a", "b", "centroid")).toBe("/projects/p%201/connectivity?from=a&to=b&method=centroid");
    expect(connectivityUrl(scope, "a", "b", null)).toBe("/projects/p%201/connectivity?from=a&to=b");
    expect(connectivityUrl(scope, null, null, "centroid")).toBe("/projects/p%201/connectivity");
  });

  it("has no URL before a project is chosen", () => {
    expect(connectivityUrl({ projectId: null, building: null, milestone: null }, null, null)).toBeNull();
  });
});

describe("drawing", () => {
  it("gives a zone only the runs on its level", () => {
    expect(segmentsOnLevel(path(true), "L1")).toHaveLength(1);
    expect(segmentsOnLevel(path(true), "L2")[0]!.points).toHaveLength(1);
    expect(segmentsOnLevel(path(true), "L9")).toEqual([]);
    expect(segmentsOnLevel(path(false), "L1")).toEqual([]);
    expect(segmentsOnLevel(null, "L1")).toEqual([]);
    expect(segmentsOnLevel(path(true), null)).toEqual([]);
  });

  it("flips Y like every other drawn shape", () => {
    expect(polylinePoints([{ x: 1, y: 2 }, { x: 3, y: -4 }])).toBe("1,-2 3,4");
  });

  it("marks a room at the mean of its outer ring, or nowhere without one", () => {
    const loops = [{ points: [{ x: 0, y: 0 }, { x: 4, y: 0 }, { x: 4, y: 2 }, { x: 0, y: 2 }] }];
    expect(markerPoint({ loops })).toEqual({ x: 2, y: 1 });
    expect(markerPoint({ loops: [] })).toBeNull();
  });
});

describe("describeResult", () => {
  it("says what happened in one line", () => {
    expect(describeResult({ state: "idle" })).toBe("");
    expect(describeResult({ state: "loading" })).toBe("Finding route…");
    expect(describeResult({ state: "error", message: "nope" })).toBe("nope");
    expect(describeResult({ state: "done", path: path(true) })).toBe("2 doors, about 42 ft");
    const mixed = path(true);
    mixed.steps[1] = { kind: "zone", zone_id: "z1", point: { x: 3, y: 4 } };
    expect(describeResult({ state: "done", path: mixed })).toBe("1 door, 1 open-zone hop, about 42 ft");
    const stacked = path(true);
    stacked.steps[1] = { kind: "vertical", zone_id: "lift", point: { x: 3, y: 4 } };
    expect(describeResult({ state: "done", path: stacked })).toBe("1 door, 1 level change, about 42 ft");
    expect(describeResult({ state: "done", path: path(false) })).toBe("no route through doors");
  });
});
