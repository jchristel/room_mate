import { describe, expect, it } from "vitest";

import { availableSearchFields, computeMatches, roomMatches, searchFieldLabel } from "./search.js";
import type { Room } from "../renderer/types.js";

const room = (id: string, name: string, props: Record<string, string> = {}, extra = {}): Room =>
  ({
    id,
    name,
    properties: Object.fromEntries(Object.entries(props).map(([k, v]) => [k, { value: v, storage_type: "String" }])),
    ...extra,
  }) as Room;

const rooms = [
  room("1", "Ward A", { Department: "Medical" }, { drofus: { fields: { Code: "WARD-1" } } }),
  room("2", "Office", { Department: "Admin" }, { classification: [{ tier: "Dept", name: "Surgery", undefined: false }] }),
];

describe("availableSearchFields", () => {
  it("lists intrinsics, a pseudo-field per source, then property keys sorted", () => {
    expect(availableSearchFields(rooms, ["drofus"])).toEqual([
      "$name",
      "$id",
      "$classification",
      "$drofus",
      "Department",
    ]);
  });
});

describe("searchFieldLabel", () => {
  it("names the intrinsics and routes a source through its display name", () => {
    expect(searchFieldLabel("$name", [])).toBe("Name");
    expect(searchFieldLabel("$drofus", ["drofus"])).toBe("dRofus");
    // A property key is its own label.
    expect(searchFieldLabel("Department", [])).toBe("Department");
  });
});

describe("roomMatches", () => {
  const all = new Set(["$name", "$id", "$classification", "$drofus", "Department"]);

  it("matches a name, an id, a property, a classification and a joined field", () => {
    expect(roomMatches(rooms[0]!, "ward", all, ["drofus"])).toBe(true);
    expect(roomMatches(rooms[0]!, "1", all, ["drofus"])).toBe(true);
    expect(roomMatches(rooms[0]!, "medical", all, ["drofus"])).toBe(true);
    expect(roomMatches(rooms[0]!, "ward-1", all, ["drofus"])).toBe(true);
    expect(roomMatches(rooms[1]!, "surgery", all, ["drofus"])).toBe(true);
  });

  /** The field picker is the whole point: a disabled field must not match,
   *  or "search names only" would be a lie. */
  it("looks only at enabled fields", () => {
    expect(roomMatches(rooms[0]!, "medical", new Set(["$name"]), ["drofus"])).toBe(false);
    expect(roomMatches(rooms[0]!, "ward-1", new Set(["$name", "Department"]), ["drofus"])).toBe(false);
  });

  it("matches nothing with no fields enabled", () => {
    expect(roomMatches(rooms[0]!, "ward", new Set(), ["drofus"])).toBe(false);
  });
});

describe("computeMatches", () => {
  const all = new Set(["$name", "$id", "$classification", "$drofus", "Department"]);

  /** `null`, not an empty set: "no query" and "a query that matched nothing"
   *  are opposite states, and only the second should dim the plan. */
  it("is null for an empty query", () => {
    expect(computeMatches(rooms, "   ", all, [])).toBeNull();
  });

  it("is an empty set for a query that matched nothing", () => {
    expect(computeMatches(rooms, "no-such-room", all, [])?.size).toBe(0);
  });

  it("collects ids over the whole payload, not one level", () => {
    expect([...computeMatches(rooms, "a", all, ["drofus"])!]).toEqual(["1", "2"]);
  });
});
