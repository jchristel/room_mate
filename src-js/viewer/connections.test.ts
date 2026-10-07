import { describe, expect, it } from "vitest";

import {
  addMembers,
  connectionsUrl,
  editOf,
  idFor,
  newEdit,
  removeMember,
  toggleMember,
  whyNotSavable,
  zonesAfterDelete,
  zonesAfterSave,
  type ConnectionsDoc,
} from "./connections.js";

const doc: ConnectionsDoc = {
  schema_version: 1,
  taken_at: "2026-10-07T00:00:00Z",
  zones: [
    { id: "east", name: "East bays", kind: "open", rooms: [{ model_id: "m", room_id: "a" }, { model_id: "m", room_id: "b" }] },
    { id: "west", name: "West bays", kind: "open", rooms: [{ model_id: "m", room_id: "c" }, { model_id: "m", room_id: "d" }] },
  ],
};

describe("the working set", () => {
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

describe("whyNotSavable", () => {
  it("wants a name and two rooms, and says which is missing", () => {
    expect(whyNotSavable(newEdit())).toMatch(/name/);
    const named = { ...newEdit(), name: "Bays" };
    expect(whyNotSavable(named)).toMatch(/at least 2/);
    expect(whyNotSavable(addMembers(named, ["a", "b"]))).toBeNull();
    expect(whyNotSavable({ ...addMembers(named, ["a", "b"]), name: "   " })).toMatch(/name/);
  });
});

describe("saving", () => {
  it("appends a new zone with an id made from its name", () => {
    const edit = { ...addMembers(newEdit(), ["x", "y"]), name: "North Wing" };
    const zones = zonesAfterSave(doc, edit);
    expect(zones.map((z) => z.id)).toEqual(["east", "west", "north-wing"]);
    expect(zones[2]!.rooms).toEqual([{ room_id: "x", model_id: null }, { room_id: "y", model_id: null }]);
  });

  it("replaces an existing zone in place and leaves the others as they were", () => {
    const edit = { ...editOf(doc.zones[0]!), name: "  East bays v2 " };
    const zones = zonesAfterSave(doc, edit);
    expect(zones.map((z) => z.id)).toEqual(["east", "west"]);
    expect(zones[0]!.name).toBe("East bays v2");
    expect(zones[1]!.rooms).toEqual(doc.zones[1]!.rooms);
  });

  it("keeps two zones with one name apart", () => {
    expect(idFor("East", new Set(["east"]))).toBe("east-2");
    expect(idFor("East", new Set(["east", "east-2"]))).toBe("east-3");
    expect(idFor("!!!", new Set())).toBe("zone");
  });

  it("deletes one zone without touching the rest", () => {
    expect(zonesAfterDelete(doc, "east").map((z) => z.id)).toEqual(["west"]);
  });
});

describe("connectionsUrl", () => {
  it("needs a project", () => {
    expect(connectionsUrl({ projectId: "p 1", building: null, milestone: null })).toBe("/projects/p%201/connections");
    expect(connectionsUrl({ projectId: null, building: null, milestone: null })).toBeNull();
  });
});

describe("vertical zones", () => {
  const vertical: ConnectionsDoc = {
    schema_version: 1,
    taken_at: "t",
    zones: [
      { id: "lift", name: "Lift 1", kind: "vertical", level_cost_ft: 20, rooms: [{ model_id: "m", room_id: "a" }, { model_id: "m", room_id: "b" }] },
      doc.zones[0]!,
    ],
  };

  it("loads a saved zone's kind and cost for editing", () => {
    const e = editOf(vertical.zones[0]!);
    expect(e.kind).toBe("vertical");
    expect(e.levelCost).toBe("20");
    expect(editOf(doc.zones[0]!).levelCost).toBe("");
  });

  it("sends a typed cost, blank as the default, and none at all for an open zone", () => {
    const base = { ...addMembers(newEdit(), ["x", "y"]), name: "Stair B", kind: "vertical" as const };
    const sent = (levelCost: string, kind: "open" | "vertical" = "vertical") =>
      zonesAfterSave(doc, { ...base, kind, levelCost }).at(-1)!.level_cost_ft;
    expect(sent("35")).toBe(35);
    expect(sent("")).toBeNull();
    expect(sent("35", "open")).toBeNull();
  });

  it("refuses a cost that is not a number above zero, in words, before the request", () => {
    const base = { ...addMembers(newEdit(), ["x", "y"]), name: "Stair B", kind: "vertical" as const };
    expect(whyNotSavable({ ...base, levelCost: "abc" })).toMatch(/level cost/);
    expect(whyNotSavable({ ...base, levelCost: "0" })).toMatch(/level cost/);
    expect(whyNotSavable({ ...base, levelCost: "-3" })).toMatch(/level cost/);
    expect(whyNotSavable({ ...base, levelCost: "" })).toBeNull();
    expect(whyNotSavable({ ...base, kind: "open", levelCost: "abc" })).toBeNull();
  });

  it("keeps every other zone's kind and cost when one is saved or deleted", () => {
    const edit = { ...editOf(doc.zones[0]!), name: "East v2" };
    const saved = zonesAfterSave(vertical, edit);
    expect(saved.find((z) => z.id === "lift")).toMatchObject({ kind: "vertical", level_cost_ft: 20 });
    expect(zonesAfterDelete(vertical, "east").find((z) => z.id === "lift")).toMatchObject({ kind: "vertical", level_cost_ft: 20 });
  });
});
