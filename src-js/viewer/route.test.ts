import { describe, expect, it } from "vitest";

import {
  connectivityUrl,
  describeClearance,
  describeResult,
  parseWidth,
  markerPoint,
  newRoute,
  pickEndpoint,
  polylinePoints,
  routeRows,
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

describe("a point in the room", () => {
  const at = { x: 3.5, y: -2 };

  it("rides with the pick that set it, and the room's centre is no point at all", () => {
    let r = pickEndpoint(ready(), "a", at);
    expect(r.startAt).toEqual(at);
    r = pickEndpoint(r, "b");
    expect(r.endAt).toBeNull();
    r = pickEndpoint(r, "c", { x: 1, y: 1 });
    expect([r.end, r.endAt]).toEqual(["c", { x: 1, y: 1 }]);
  });

  it("is forgotten with the endpoint it belonged to", () => {
    const placed = pickEndpoint(pickEndpoint(ready(), "a", at), "b", { x: 9, y: 9 });
    const cleared = pickEndpoint(placed, "a");
    expect([cleared.start, cleared.startAt]).toEqual([null, null]);
    expect(cleared.endAt).toEqual({ x: 9, y: 9 });
    const second = pickEndpoint(placed, "b");
    expect([second.end, second.endAt]).toEqual([null, null]);
  });

  it("goes in the url only with a route, and only the coordinates that were set", () => {
    const scope = { projectId: "p", building: null, milestone: null };
    expect(connectivityUrl(scope, "a", "b", null, { from: at, to: null })).toBe(
      "/projects/p/connectivity?from=a&to=b&from_x=3.5&from_y=-2",
    );
    expect(connectivityUrl(scope, "a", "b", null, { from: at, to: { x: 1, y: 2 } })).toContain("to_x=1&to_y=2");
    expect(connectivityUrl(scope, "a", null, null, { from: at, to: null })).toBe("/projects/p/connectivity");
  });
});

describe("routeRows", () => {
  const ref = (id: string) => ({ model_id: "m", room_id: id });
  const levels: Record<string, string> = { a: "L1", b: "L1", c: "L2" };
  const full: RoutePath = {
    found: true,
    reason: null,
    distance_ft: 90,
    rooms: [ref("a"), ref("b"), ref("c")],
    steps: [
      { kind: "door", door_id: "d1", from: ref("a"), to: ref("b"), point: { x: 0, y: 0 }, length: 20 },
      { kind: "vertical", link_id: "lift", from: ref("b"), to: ref("c"), point: { x: 0, y: 0 }, length: 40 },
    ],
    segments: [],
  };

  it("lists the start, then each hop with its kind and length", () => {
    const rows = routeRows(full, (id) => levels[id]);
    expect(rows.map((r) => [r.roomId, r.how, r.lengthFt])).toEqual([
      ["a", "start", 0],
      ["b", "door", 20],
      ["c", "vertical", 40],
    ]);
    expect(rows[1]!.doorId).toBe("d1");
  });

  it("marks where the level changes, and only there", () => {
    expect(routeRows(full, (id) => levels[id]).map((r) => r.levelChanged)).toEqual([false, false, true]);
  });

  it("is empty for no route, and does not need step ends on an older answer", () => {
    expect(routeRows(null, () => null)).toEqual([]);
    expect(routeRows({ ...full, found: false }, () => null)).toEqual([]);
    const bare = { ...full, steps: full.steps.map(({ from: _f, to: _t, ...rest }) => rest) };
    expect(routeRows(bare, (id) => levels[id]).map((r) => r.roomId)).toEqual(["a", "b", "c"]);
  });
});

describe("connectivityUrl with saved rooms", () => {
  it("names each room's model so two models sharing an id cannot be confused", () => {
    const url = connectivityUrl({ projectId: "p", building: null, milestone: null } as never, "1", "2", null, {}, { from: "m1", to: "m2" });
    expect(url).toContain("from_model=m1");
    expect(url).toContain("to_model=m2");
  });
});

describe("the width of the object", () => {
  it("reads what was typed: blank and 0 check nothing, a number is a width, anything else is refused", () => {
    expect(parseWidth("")).toBeNull();
    expect(parseWidth("  ")).toBeNull();
    expect(parseWidth("0")).toBeNull();
    expect(parseWidth("1200")).toBe(1200);
    expect(parseWidth("900.5")).toBe(900.5);
    expect(parseWidth("-5")).toBe("bad");
    expect(parseWidth("wide")).toBe("bad");
    expect(parseWidth("1e9")).toBe("bad");
  });

  it("asks the server for a width only when there is one, and only with a route", () => {
    const scope = { projectId: "p", building: null, milestone: null };
    expect(connectivityUrl(scope, "1", "2", null, {}, {}, 1200)).toContain("width_mm=1200");
    expect(connectivityUrl(scope, "1", "2", null, {}, {}, null)).not.toContain("width_mm");
    expect(connectivityUrl(scope, "1", "2", null, {}, {}, 0)).not.toContain("width_mm");
    expect(connectivityUrl(scope, null, null, null, {}, {}, 1200)).not.toContain("width_mm");
  });

  it("says the tightest hop, a wider route if there is one, and what was not checked", () => {
    const door = { width_mm: 912.4, kind: "door" as const, door_id: "d9" };
    expect(describeClearance({ frame_allowance_mm: 150, narrowest: door, unchecked_hops: 0 })).toBe(
      "narrowest door d9, about 912 mm",
    );
    expect(
      describeClearance({
        frame_allowance_mm: 150,
        narrowest: door,
        widest_possible: { width_mm: 1500, kind: "door", door_id: "d2" },
        unchecked_hops: 2,
      }),
    ).toBe("narrowest door d9, about 912 mm · up to about 1500 mm fits by another route · 2 hops not checked");
    expect(describeClearance({ frame_allowance_mm: 150, unchecked_hops: 1 })).toBe("1 hop not checked");
    expect(describeClearance(undefined)).toBe("");
  });

  it("does not call the limit of a blocked trip 'another route': there is none", () => {
    const limit = { width_mm: 905, kind: "door" as const, door_id: "d1" };
    expect(describeClearance({ frame_allowance_mm: 150, widest_possible: limit, unchecked_hops: 0 })).toBe("");
  });

  it("does not offer a 'wider route' that is the one already taken", () => {
    const door = { width_mm: 900, kind: "door" as const, door_id: "d1" };
    expect(describeClearance({ frame_allowance_mm: 150, narrowest: door, widest_possible: door, unchecked_hops: 0 })).toBe(
      "narrowest door d1, about 900 mm",
    );
  });
});
