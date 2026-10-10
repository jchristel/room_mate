import { describe, expect, it } from "vitest";

import {
  addMembers,
  bodyAfterDeleteLink,
  bodyAfterDeleteZone,
  bodyAfterSave,
  connectionsUrl,
  editOf,
  editOfLink,
  idFor,
  newEdit,
  parseCost,
  removeMember,
  roomsInOtherZones,
  bodyAfterAddLink,
  clearHub,
  connectionRows,
  pickForEdit,
  removeCut,
  setTool,
  routeTableRows,
  levelsText,
  bodyAfterDeleteRoute,
  bodyAfterRecolourRoute,
  bodyAfterSaveRoute,
  nextRouteColour,
  ROUTE_COLOURS,
  whyRouteNotSavable,
  toggleMember,
  whyNotSavable,
  type ConnectionsDoc,
} from "./connections.js";

const doc: ConnectionsDoc = {
  schema_version: 1,
  taken_at: "2026-10-07T00:00:00Z",
  zones: [
    { id: "east", name: "East bays", kind: "open", rooms: [{ model_id: "m", room_id: "a" }, { model_id: "m", room_id: "b" }] },
    { id: "west", name: "West bays", kind: "open", rooms: [{ model_id: "m", room_id: "c" }, { model_id: "m", room_id: "d" }] },
  ],
  links: [{ id: "lift-1-2", a: { model_id: "m", room_id: "p" }, b: { model_id: "m", room_id: "q" }, cost_ft: 20 }],
  routes: [],
};

describe("roomsInOtherZones", () => {
  it("maps every saved room to the index of its zone", () => {
    expect([...roomsInOtherZones(doc, null)]).toEqual([["a", 0], ["b", 0], ["c", 1], ["d", 1]]);
  });
  it("leaves out the zone being edited, and tolerates no document", () => {
    expect([...roomsInOtherZones(doc, "east").keys()]).toEqual(["c", "d"]);
    expect(roomsInOtherZones(null, null).size).toBe(0);
  });
});

describe("an open zone's working set", () => {
  it("toggles a room in and out, so a click is its own undo", () => {
    let e = toggleMember(newEdit(), "a");
    expect(e.members.map((m) => m.room_id)).toEqual(["a"]);
    e = toggleMember(e, "b");
    e = toggleMember(e, "a");
    expect(e.members.map((m) => m.room_id)).toEqual(["b"]);
  });

  it("adds many at once and skips the ones already in", () => {
    const e = addMembers(toggleMember(newEdit(), "a"), ["a", "b", "c", "b"]);
    expect(e.members.map((m) => m.room_id)).toEqual(["a", "b", "c"]);
    expect(addMembers(e, ["a"])).toBe(e);
  });

  it("keeps the model of a room loaded from a saved zone, and leaves a picked one unresolved", () => {
    const loaded = editOf(doc.zones[0]!);
    expect(loaded.members[0]).toEqual({ room_id: "a", model_id: "m" });
    expect(toggleMember(loaded, "z").members.at(-1)).toEqual({ room_id: "z", model_id: null });
  });

  it("clears an earlier error on any change", () => {
    const failed = { ...newEdit(), error: "boom" };
    expect(toggleMember(failed, "a").error).toBeNull();
    expect(removeMember(failed, "a").error).toBeNull();
  });
});

describe("a vertical link's working set", () => {
  it("holds two rooms, and a third pick replaces the second", () => {
    let e = newEdit("link");
    e = toggleMember(toggleMember(e, "low"), "up");
    expect(e.members.map((m) => m.room_id)).toEqual(["low", "up"]);
    e = toggleMember(e, "other");
    expect(e.members.map((m) => m.room_id)).toEqual(["low", "other"]);
  });

  it("takes nothing in bulk: a link is two rooms by hand", () => {
    const e = newEdit("link");
    expect(addMembers(e, ["a", "b", "c"])).toBe(e);
  });

  it("loads a saved link with its two rooms and its cost", () => {
    const e = editOfLink(doc.links[0]!);
    expect(e.kind).toBe("link");
    expect(e.members.map((m) => m.room_id)).toEqual(["p", "q"]);
    expect(e.costFt).toBe("20");
  });
});

describe("whyNotSavable", () => {
  it("wants a name and two rooms for a zone, and says which is missing", () => {
    expect(whyNotSavable(newEdit())).toMatch(/name/);
    const named = { ...newEdit(), name: "Bays" };
    expect(whyNotSavable(named)).toMatch(/at least 2/);
    expect(whyNotSavable(addMembers(named, ["a", "b"]))).toBeNull();
  });

  it("wants exactly two rooms and a sensible cost for a link, and no name", () => {
    const link = newEdit("link");
    expect(whyNotSavable(link)).toMatch(/two rooms/);
    const two = toggleMember(toggleMember(link, "a"), "b");
    expect(whyNotSavable(two)).toBeNull();
    expect(whyNotSavable({ ...two, costFt: "abc" })).toMatch(/cost/);
    expect(whyNotSavable({ ...two, costFt: "0" })).toMatch(/cost/);
    expect(whyNotSavable({ ...two, costFt: "35" })).toBeNull();
  });

  it("parses a cost: blank is the default, a positive number is itself, anything else is bad", () => {
    expect(parseCost("")).toBeNull();
    expect(parseCost(" 12.5 ")).toBe(12.5);
    expect(parseCost("-1")).toBe("bad");
    expect(parseCost("x")).toBe("bad");
  });
});

describe("saving", () => {
  it("appends a new zone with an id made from its name, and carries the links over", () => {
    const edit = { ...addMembers(newEdit(), ["x", "y"]), name: "North Wing" };
    const body = bodyAfterSave(doc, edit);
    expect(body.zones.map((z) => z.id)).toEqual(["east", "west", "north-wing"]);
    expect(body.links.map((l) => l.id)).toEqual(["lift-1-2"]);
    expect(body.zones[2]!.rooms).toEqual([{ room_id: "x", model_id: null }, { room_id: "y", model_id: null }]);
  });

  it("appends a new link with an id from its rooms, and carries the zones over", () => {
    const edit = { ...toggleMember(toggleMember(newEdit("link"), "e"), "f"), costFt: "30" };
    const body = bodyAfterSave(doc, edit);
    expect(body.zones.map((z) => z.id)).toEqual(["east", "west"]);
    expect(body.links.map((l) => l.id)).toEqual(["lift-1-2", "link-e-f"]);
    expect(body.links[1]).toMatchObject({ cost_ft: 30 });
    expect(body.links[1]!.a).toEqual({ room_id: "e", model_id: null });
  });

  it("replaces an existing zone or link in place and leaves the others as they were", () => {
    const zone = bodyAfterSave(doc, { ...editOf(doc.zones[0]!), name: "  East bays v2 " });
    expect(zone.zones.map((z) => z.id)).toEqual(["east", "west"]);
    expect(zone.zones[0]!.name).toBe("East bays v2");
    expect(zone.links).toHaveLength(1);

    const link = bodyAfterSave(doc, { ...editOfLink(doc.links[0]!), costFt: "" });
    expect(link.links).toHaveLength(1);
    expect(link.links[0]!.cost_ft).toBeNull();
    expect(link.zones).toHaveLength(2);
  });

  it("keeps two records with one name apart", () => {
    expect(idFor("East", new Set(["east"]))).toBe("east-2");
    expect(idFor("East", new Set(["east", "east-2"]))).toBe("east-3");
    expect(idFor("!!!", new Set())).toBe("item");
  });

  it("deletes one zone or one link without touching the rest", () => {
    expect(bodyAfterDeleteZone(doc, "east").zones.map((z) => z.id)).toEqual(["west"]);
    expect(bodyAfterDeleteZone(doc, "east").links).toHaveLength(1);
    expect(bodyAfterDeleteLink(doc, "lift-1-2").links).toEqual([]);
    expect(bodyAfterDeleteLink(doc, "lift-1-2").zones).toHaveLength(2);
  });
});

describe("connectionsUrl", () => {
  it("needs a project", () => {
    expect(connectionsUrl({ projectId: "p 1", building: null, milestone: null })).toBe("/projects/p%201/connections");
    expect(connectionsUrl({ projectId: null, building: null, milestone: null })).toBeNull();
  });
});

describe("saved routes", () => {
  const draft = { name: "Bed to lift", colour: "#1c7ed6", from: "a", to: "b", fromAt: { x: 1, y: 2 }, toAt: null, method: null };
  const withRoute = bodyAfterSaveRoute(doc, draft);
  const saved: ConnectionsDoc = { ...doc, routes: withRoute.routes };

  it("saves the request with bare rooms for the server to resolve, and never a path", () => {
    expect(withRoute.routes).toHaveLength(1);
    const r = withRoute.routes[0]!;
    expect(r).toMatchObject({ id: "bed-to-lift", from: { model_id: "", room_id: "a" }, colour: "#1c7ed6", from_at: { x: 1, y: 2 } });
    expect(Object.keys(r)).not.toContain("path");
  });

  it("carries zones and links through untouched, and keeps an id unique", () => {
    expect(withRoute.zones.map((z) => z.id)).toEqual(["east", "west"]);
    expect(withRoute.links).toHaveLength(1);
    expect(bodyAfterSaveRoute(saved, draft).routes.map((r) => r.id)).toEqual(["bed-to-lift", "bed-to-lift-2"]);
  });

  it("keeps routes through every other save, so a zone edit cannot drop them", () => {
    expect(bodyAfterDeleteZone(saved, "east").routes).toHaveLength(1);
    expect(bodyAfterDeleteLink(saved, "lift-1-2").routes).toHaveLength(1);
    expect(bodyAfterSave(saved, newEdit("open")).routes).toHaveLength(1);
  });

  it("recolours and deletes one route", () => {
    expect(bodyAfterRecolourRoute(saved, "bed-to-lift", "#000000").routes[0]!.colour).toBe("#000000");
    expect(bodyAfterDeleteRoute(saved, "bed-to-lift").routes).toEqual([]);
  });

  it("gives each new route the least-used palette colour", () => {
    expect(nextRouteColour(null)).toBe(ROUTE_COLOURS[0]);
    const used = { ...doc, routes: [{ ...withRoute.routes[0]!, colour: ROUTE_COLOURS[0] }] };
    expect(nextRouteColour(used)).toBe(ROUTE_COLOURS[1]);
  });

  it("says what is missing before a save", () => {
    expect(whyRouteNotSavable({ name: " ", from: "a", to: "b" })).toMatch(/name/);
    expect(whyRouteNotSavable({ name: "x", from: "a", to: "" })).toMatch(/start and an end/);
    expect(whyRouteNotSavable({ name: "x", from: "a", to: "b" })).toBeNull();
  });
});

describe("the stack editor kind", () => {
  it("holds one room: a pick replaces it, and picking it again clears it", () => {
    const e1 = toggleMember(newEdit("stack"), "a");
    const e2 = toggleMember(e1, "b");
    expect(e2.members.map((m) => m.room_id)).toEqual(["b"]);
    expect(toggleMember(e2, "b").members).toEqual([]);
  });

  it("takes nothing in bulk and saves nothing of its own", () => {
    expect(addMembers(newEdit("stack"), ["a", "b"]).members).toEqual([]);
    expect(whyNotSavable(newEdit("stack"))).toMatch(/each link/);
    const body = bodyAfterSave(doc, toggleMember(newEdit("stack"), "a"));
    expect(body.zones).toHaveLength(2);
    expect(body.links).toHaveLength(1);
  });

  it("adds one link at the default cost, and keeps zones, links and routes", () => {
    const body = bodyAfterAddLink(doc, { room_id: "x", model_id: "m" }, { room_id: "y", model_id: "m" });
    expect(body.links).toHaveLength(2);
    expect(body.links[1]).toMatchObject({ a: { room_id: "x" }, b: { room_id: "y" }, cost_ft: null });
    expect(body.zones).toHaveLength(2);
    expect(bodyAfterAddLink(doc, { room_id: "x", model_id: "m" }, { room_id: "y", model_id: "m" }).links.map((l) => l.id))
      .toEqual(["lift-1-2", "link-x-y"]);
  });
});

describe("connectionRows", () => {
  const name = (id: string) => ({ p: "LIFT P", q: "LIFT Q" })[id] ?? id;

  it("lists open areas first and then vertical links, each with its type", () => {
    expect(connectionRows(doc, name).map((r) => [r.kind, r.name, r.type])).toEqual([
      ["open", "East bays", "Open area"],
      ["open", "West bays", "Open area"],
      ["link", "LIFT P ↔ LIFT Q", "Vertical"],
    ]);
  });

  it("names a link by its two rooms, and keeps the id of one the page no longer holds", () => {
    const stale = { ...doc, links: [{ id: "x", a: { model_id: "m", room_id: "gone" }, b: { model_id: "m", room_id: "q" } }] };
    expect(connectionRows(stale, name).find((r) => r.kind === "link")!.name).toBe("gone ↔ LIFT Q");
  });

  it("sorts names the way a reader counts, and handles no document", () => {
    const many = { ...doc, zones: ["Bay 10", "Bay 2", "Bay 1"].map((n) => ({ id: n, name: n, kind: "open" as const, rooms: [] })) };
    expect(connectionRows(many, name).filter((r) => r.kind === "open").map((r) => r.name)).toEqual(["Bay 1", "Bay 2", "Bay 10"]);
    expect(connectionRows(null, name)).toEqual([]);
  });
});

describe("the level column", () => {
  const level = (id: string) =>
    ({ a: { name: "LEVEL 1", elevation: 0 }, b: { name: "LEVEL 1", elevation: 0 }, c: { name: "LEVEL 2", elevation: 3000 }, p: { name: "LEVEL 1", elevation: 0 }, q: { name: "LEVEL 2", elevation: 3000 } })[id as "a"] ?? null;

  it("names the one level an open area lies on, and each level once", () => {
    const rows = connectionRows(doc, (id) => id, level);
    expect(rows.find((r) => r.id === "east")!.level).toBe("LEVEL 1");
  });

  it("lists the levels lowest first when an open area spans two", () => {
    const spans = { ...doc, zones: [{ id: "s", name: "S", kind: "open" as const, rooms: [{ model_id: "m", room_id: "c" }, { model_id: "m", room_id: "a" }] }] };
    expect(connectionRows(spans, (id) => id, level)[0]!.level).toBe("LEVEL 1, LEVEL 2");
  });

  it("shows the two levels a link joins, and nothing for rooms the page does not hold", () => {
    expect(connectionRows(doc, (id) => id, level).find((r) => r.kind === "link")!.level).toBe("LEVEL 1 ↔ LEVEL 2");
    expect(connectionRows(doc, (id) => id).every((r) => r.level === "")).toBe(true);
  });

  it("says a count rather than a long list", () => {
    const l = (n: number) => ({ name: "L" + n, elevation: n });
    expect(levelsText([l(1), l(2), l(3)])).toBe("L1, L2, L3");
    expect(levelsText([l(1), l(2), l(3), l(4)])).toBe("4 levels");
  });
});

describe("routeTableRows", () => {
  const route = (id: string, name: string, from: string, to: string) => ({
    id,
    name,
    from: { model_id: "m", room_id: from },
    to: { model_id: "m", room_id: to },
    colour: "#d9480f",
  });
  const docWith = { ...doc, routes: [route("r2", "Route 10", "a", "c"), route("r1", "Route 2", "a", "b")] };
  const level = (id: string) =>
    ({ a: { name: "LEVEL 1", elevation: 0 }, b: { name: "LEVEL 1", elevation: 0 }, c: { name: "LEVEL 3", elevation: 6000 } })[id as "a"] ?? null;

  it("names where each route runs: one level, or from the start's to the end's", () => {
    const rows = routeTableRows(docWith, level);
    expect(rows.map((r) => [r.name, r.levels])).toEqual([
      ["Route 2", "LEVEL 1"],
      ["Route 10", "LEVEL 1 → LEVEL 3"],
    ]);
    expect(rows[0]!.colour).toBe("#d9480f");
  });

  it("keeps the order a route runs in, even downhill", () => {
    const down = { ...doc, routes: [route("r", "Down", "c", "a")] };
    expect(routeTableRows(down, level)[0]!.levels).toBe("LEVEL 3 → LEVEL 1");
  });

  it("is empty for no routes and says nothing for rooms the page does not hold", () => {
    expect(routeTableRows(null, level)).toEqual([]);
    expect(routeTableRows(docWith, () => null).every((r) => r.levels === "")).toBe(true);
  });
});

describe("disconnects and the hub", () => {
  const withMembers = (ids: string[]) => ids.reduce((e, id) => toggleMember(e, id), newEdit("open"));
  const bays = withMembers(["b1", "b2", "b3", "cor"]);

  it("closes the wall between two members with two picks in the cut tool, and keeps the tool on", () => {
    const e1 = pickForEdit(setTool(bays, "cut"), "b1");
    expect(e1.cutFrom).toBe("b1");
    const e2 = pickForEdit(e1, "b2");
    expect(e2.cuts).toEqual([{ a: "b1", b: "b2" }]);
    expect(e2.cutFrom).toBeNull();
    expect(e2.tool).toBe("cut");
  });

  it("opens a closed wall again when the same pair is cut twice, either way round", () => {
    const cut = (e: typeof bays, a: string, b: string) => pickForEdit(pickForEdit(e, a), b);
    const closed = cut(setTool(bays, "cut"), "b1", "b2");
    expect(cut(closed, "b2", "b1").cuts).toEqual([]);
  });

  it("lets go of a half-drawn cut when its room is picked again, and ignores a non-member", () => {
    const held = pickForEdit(setTool(bays, "cut"), "b1");
    expect(pickForEdit(held, "b1").cutFrom).toBeNull();
    const refused = pickForEdit(held, "elsewhere");
    expect(refused.cuts).toEqual([]);
    expect(refused.error).toMatch(/already in this open area/);
  });

  it("names a hub with one pick and goes back to picking; picking it again clears it", () => {
    const named = pickForEdit(setTool(bays, "hub"), "cor");
    expect(named.hub).toBe("cor");
    expect(named.tool).toBe("pick");
    expect(clearHub(named).hub).toBeNull();
    expect(pickForEdit(setTool(named, "hub"), "cor").hub).toBeNull();
  });

  it("ignores the tools for a link, which is two rooms by hand", () => {
    const link = pickForEdit({ ...newEdit("link"), tool: "cut" }, "x");
    expect(link.members.map((m) => m.room_id)).toEqual(["x"]);
    expect(link.cuts).toEqual([]);
  });

  it("drops the cuts and the hub of a room that leaves the zone", () => {
    let e = pickForEdit(pickForEdit(setTool(bays, "cut"), "b1"), "b2");
    e = pickForEdit(setTool(e, "hub"), "cor");
    const without = toggleMember(e, "b1");
    expect(without.cuts).toEqual([]);
    expect(toggleMember(without, "cor").hub).toBeNull();
    expect(removeMember(e, "b2").cuts).toEqual([]);
  });

  it("switches a tool off by choosing it again, and drops a half-drawn cut on a change", () => {
    const cutting = setTool(bays, "cut");
    expect(setTool(cutting, "cut").tool).toBe("pick");
    expect(setTool(pickForEdit(cutting, "b1"), "hub").cutFrom).toBeNull();
  });

  it("saves the cuts and the hub with each room's own model", () => {
    const saved = { id: "z", name: "Bays", kind: "open" as const, rooms: [{ model_id: "m1", room_id: "b1" }, { model_id: "m2", room_id: "b2" }] };
    const e = { ...editOf(saved), cuts: [{ a: "b1", b: "b2" }], hub: "b2" };
    const body = bodyAfterSave({ ...doc, zones: [saved] }, e).zones[0]!;
    expect(body.disconnects).toEqual([{ a: { room_id: "b1", model_id: "m1" }, b: { room_id: "b2", model_id: "m2" } }]);
    expect(body.hub).toEqual({ room_id: "b2", model_id: "m2" });
  });

  it("loads a saved zone's cuts and hub for editing", () => {
    const saved = {
      id: "z",
      name: "Bays",
      kind: "open" as const,
      rooms: [{ model_id: "m", room_id: "b1" }, { model_id: "m", room_id: "b2" }],
      disconnects: [{ a: { model_id: "m", room_id: "b1" }, b: { model_id: "m", room_id: "b2" } }],
      hub: { model_id: "m", room_id: "b2" },
    };
    const e = editOf(saved);
    expect(e.cuts).toEqual([{ a: "b1", b: "b2" }]);
    expect(e.hub).toBe("b2");
  });

  it("carries every zone's cuts and hub through a save of something else", () => {
    const cutDoc = {
      ...doc,
      zones: [
        {
          id: "z",
          name: "Bays",
          kind: "open" as const,
          rooms: [{ model_id: "m", room_id: "b1" }, { model_id: "m", room_id: "b2" }],
          disconnects: [{ a: { model_id: "m", room_id: "b1" }, b: { model_id: "m", room_id: "b2" } }],
          hub: { model_id: "m", room_id: "b2" },
        },
      ],
    };
    const keeps = (b: { zones: { disconnects: unknown[]; hub: unknown }[] }) => b.zones[0]!.disconnects.length === 1 && b.zones[0]!.hub !== null;
    expect(keeps(bodyAfterAddLink(cutDoc, { room_id: "x", model_id: "m" }, { room_id: "y", model_id: "m" }))).toBe(true);
    expect(keeps(bodyAfterDeleteLink(cutDoc, "lift-1-2"))).toBe(true);
    expect(keeps(bodyAfterSaveRoute(cutDoc, { name: "R", colour: "#000000", from: "a", to: "b", fromAt: null, toAt: null, method: null }))).toBe(true);
    expect(keeps(bodyAfterDeleteRoute(cutDoc, "none"))).toBe(true);
  });
});
