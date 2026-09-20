// What decides that an element layer is FETCHED.
//
// Pinned because getting it wrong costs money in one direction and shows a
// false state in the other: an overlay polled while nobody asked for it is a
// 9 MB read on RHH for every viewer, and a layer the room panel lists but
// nobody fetches reads "not loaded yet" forever. Two askers, one answer — the
// rest of the store is plumbing a test would only restate.

import { beforeEach, describe, expect, it } from "vitest";

import { getState, layerWanted, resetState, setRoomContents, setZoneLayer } from "./store.js";

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
