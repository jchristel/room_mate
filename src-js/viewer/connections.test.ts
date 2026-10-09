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
