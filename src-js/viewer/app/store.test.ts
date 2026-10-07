// What decides that an element layer is FETCHED.
//
// Pinned because getting it wrong costs money in one direction and shows a
// false state in the other: an overlay polled while nobody asked for it is a
// 9 MB read on RHH for every viewer, and a layer the room panel lists but
// nobody fetches reads "not loaded yet" forever. Two askers, one answer — the
// rest of the store is plumbing a test would only restate.

import { beforeEach, describe, expect, it } from "vitest";

import {
  addZone,
  getState,
  layerWanted,
  patchZoneRoute,
  resetState,
  select,
  setRoomContents,
  setZoneLayer,
  setZoneEditMode,
  setZoneRouteMode,
} from "./store.js";

describe("layerWanted", () => {
  beforeEach(() => resetState());

  it("starts false for the three overlays, which is what keeps them unread", () => {
    expect(layerWanted("spaces")).toBe(false);
    expect(layerWanted("ceilings")).toBe(false);
    expect(layerWanted("floors")).toBe(false);
  });

  it("is true while a zone draws the layer", () => {
    setZoneLayer(getState().zones[0]!.id, { layers: { ceilings: true } });
    expect(layerWanted("ceilings")).toBe(true);
  });

  it("is true while the room panel lists it, with no zone drawing it", () => {
    setRoomContents("floors", true);
    expect(getState().zones.some((z) => z.layers.floors)).toBe(false);
    expect(layerWanted("floors")).toBe(true);
  });

  it("goes back to false when neither asker wants it", () => {
    const zone = getState().zones[0]!.id;
    setZoneLayer(zone, { layers: { ceilings: true } });
    setRoomContents("ceilings", true);
    setZoneLayer(zone, { layers: { ceilings: false } });
    expect(layerWanted("ceilings")).toBe(true);
    setRoomContents("ceilings", false);
    expect(layerWanted("ceilings")).toBe(false);
  });

  it("never lets the chooser reach spaces, which it cannot list", () => {
    // Not a defensive check in the code so much as a typed impossibility the
    // test states out loud: a space carries no room reference, so it is not a
    // `ContentsEntity` and nothing can tick it into a fetch.
    expect(Object.keys(getState().roomContents)).not.toContain("spaces");
    expect(layerWanted("spaces")).toBe(false);
  });
});

describe("a zone's route tool", () => {
  beforeEach(() => resetState());

  const ready = (zone: string, unreachable: string[] = []) => {
    setZoneRouteMode(zone, true);
    patchZoneRoute(zone, { unreachable: new Set(unreachable) });
  };
  const route = (zone: string) => getState().zones.find((z) => z.id === zone)!.route;

  it("takes a plan click as an endpoint instead of selecting, and only in the zone that asked", () => {
    addZone();
    const [a, b] = getState().zones.map((z) => z.id) as [string, string];
    ready(a);

    select("room", "r1", a);
    expect(route(a)!.start).toBe("r1");
    expect(getState().selection).toBeNull();

    // A click in the OTHER zone is an ordinary selection: it did not ask.
    select("room", "r2", b);
    expect(getState().selection?.id).toBe("r2");
    expect(route(b)).toBeNull();
  });

  it("sends a pick with no zone (a grid row, a search chip) to the zone that last took one", () => {
    addZone();
    const [a, b] = getState().zones.map((z) => z.id) as [string, string];
    ready(a);
    ready(b);

    select("room", "r1", a);
    select("room", "r2", null);
    expect(route(a)!.end).toBe("r2");
    select("room", "r3", b);
    select("room", "r4", null);
    expect(route(b)!.end).toBe("r4");
    expect(route(a)!.end).toBe("r2");
  });

  it("selects normally when no zone is routing, and for every kind but rooms", () => {
    const a = getState().zones[0]!.id;
    select("room", "r1", null);
    expect(getState().selection?.id).toBe("r1");
    ready(a);
    select("door", "d1", a);
    expect(getState().selection).toMatchObject({ kind: "door", id: "d1" });
    expect(route(a)!.start).toBeNull();
  });

  it("turning the tool off forgets the route, and a copied zone does not inherit one", () => {
    const a = getState().zones[0]!.id;
    ready(a);
    select("room", "r1", a);
    addZone();
    expect(getState().zones[1]!.route).toBeNull();
    setZoneRouteMode(a, false);
    expect(route(a)).toBeNull();
  });
});

describe("a zone's open-zone editor", () => {
  beforeEach(() => resetState());

  const edit = (zone: string) => getState().zones.find((z) => z.id === zone)!.edit;

  it("takes room picks into its working set, toggling, instead of selecting", () => {
    const a = getState().zones[0]!.id;
    setZoneEditMode(a, true);
    select("room", "r1", a);
    select("room", "r2", a);
    select("room", "r1", a);
    expect(edit(a)!.members.map((m) => m.room_id)).toEqual(["r2"]);
    expect(getState().selection).toBeNull();
  });

  it("is exclusive with the route tool in the same zone, in both directions", () => {
    const a = getState().zones[0]!.id;
    setZoneRouteMode(a, true);
    setZoneEditMode(a, true);
    expect(getState().zones[0]!.route).toBeNull();
    expect(edit(a)).not.toBeNull();
    setZoneRouteMode(a, true);
    expect(edit(a)).toBeNull();
  });

  it("sends a pick with no zone to whichever zone has a tool on, whichever kind", () => {
    addZone();
    const [a, b] = getState().zones.map((z) => z.id) as [string, string];
    setZoneEditMode(b, true);
    select("room", "r1", null);
    expect(edit(b)!.members).toHaveLength(1);
    expect(edit(a)).toBeNull();
  });

  it("still lets a door be selected while editing", () => {
    const a = getState().zones[0]!.id;
    setZoneEditMode(a, true);
    select("door", "d1", a);
    expect(getState().selection).toMatchObject({ kind: "door", id: "d1" });
    expect(edit(a)!.members).toHaveLength(0);
  });
});
