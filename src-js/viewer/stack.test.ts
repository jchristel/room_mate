import { describe, expect, it } from "vitest";

import type { SavedLink } from "./connections.js";
import { buildStack, describeSide, percent, stackUrl, type StackSide } from "./stack.js";

const m = "m";
const link = (id: string, a: string, b: string): SavedLink => ({
  id,
  a: { model_id: m, room_id: a },
  b: { model_id: m, room_id: b },
});

const levels = [
  { id: "L1", name: "LEVEL 1", elevation: 0 },
  { id: "C1", name: "C 1.5", elevation: 1500 },
  { id: "L2", name: "LEVEL 2", elevation: 3000 },
  { id: "L3", name: "LEVEL 3", elevation: 6000 },
];
const rooms = new Map([
  ["a", { name: "LIFT 1 A", level_id: "L1" }],
  ["b", { name: "LIFT 1 B", level_id: "L2" }],
  ["c", { name: "LIFT 1 C", level_id: "L3" }],
  ["x", { name: "STORE", level_id: "C1" }],
  ["lone", { name: "STAIR", level_id: "L1" }],
]);

describe("buildStack", () => {
  const links = [link("ab", "a", "b"), link("bc", "b", "c")];

  it("lists only the levels that hold a room of the stack, highest first", () => {
    const s = buildStack("b", links, rooms, levels)!;
    expect(s.rows.map((r) => r.levelName)).toEqual(["LEVEL 3", "LEVEL 2", "LEVEL 1"]);
    expect(s.rows.flatMap((r) => r.rooms.map((x) => x.id))).toEqual(["c", "b", "a"]);
  });

  it("never lists the other levels, so a car park's half-level stays out of a hospital stack", () => {
    const s = buildStack("a", links, rooms, levels)!;
    expect(s.rows.map((r) => r.levelId)).not.toContain("C1");
  });

  it("is the same stack from any room in it, with the ends named", () => {
    const fromTop = buildStack("c", links, rooms, levels)!;
    const fromBottom = buildStack("a", links, rooms, levels)!;
    expect(fromTop.rows).toEqual(fromBottom.rows);
    expect([fromTop.top?.id, fromTop.bottom?.id]).toEqual(["c", "a"]);
  });

  it("puts the links between the two rows they join", () => {
    const s = buildStack("a", links, rooms, levels)!;
    expect(s.joins.map((j) => j.links.map((l) => l.id))).toEqual([["bc"], ["ab"]]);
  });

  it("is a stack of one row for a room with no link, and none for no room", () => {
    const s = buildStack("lone", links, rooms, levels)!;
    expect(s.rows).toHaveLength(1);
    expect(s.joins).toEqual([]);
    expect([s.top?.id, s.bottom?.id]).toEqual(["lone", "lone"]);
    expect(buildStack(null, links, rooms, levels)).toBeNull();
    expect(buildStack("ghost", links, rooms, levels)).toBeNull();
  });

  it("leaves out a linked room the page no longer holds", () => {
    const s = buildStack("a", [link("ab", "a", "b"), link("bz", "b", "gone")], rooms, levels)!;
    expect(s.rows.flatMap((r) => r.rooms.map((x) => x.id)).sort()).toEqual(["a", "b"]);
  });

  it("keeps two rooms on one level in one row", () => {
    const s = buildStack("a", [link("ab", "a", "b"), link("ab2", "lone", "b")], rooms, levels)!;
    const bottom = s.rows[s.rows.length - 1]!;
    expect(bottom.rooms.map((r) => r.id).sort()).toEqual(["a", "lone"]);
  });
});

describe("stackUrl and wording", () => {
  it("names the room and the milestone", () => {
    expect(stackUrl("RHH", "12", null)).toBe("/projects/RHH/stack?room=12");
    expect(stackUrl("RHH", "12", "IFC")).toBe("/projects/RHH/stack?room=12&milestone=IFC");
    expect(stackUrl(null, "12", null)).toBeNull();
  });

  it("reads an overlap as a percentage that never exceeds 100", () => {
    expect(percent(0.734)).toBe("73%");
    expect(percent(1.0000001)).toBe("100%");
  });

  it("says whether a suggestion stands alone", () => {
    const side = (confidence: "clear" | "ambiguous", n: number): StackSide => ({
      level_id: "L2",
      level_name: "LEVEL 2",
      confidence,
      candidates: Array.from({ length: n }, (_, i) => ({
        model_id: m,
        room_id: String(i),
        name: String(i),
        overlap: 1,
        fraction_of_room: 1,
        same_stem: false,
      })),
    });
    expect(describeSide(side("clear", 1))).toMatch(/one room fits/);
    expect(describeSide(side("ambiguous", 2))).toMatch(/2 rooms fit.*choose/);
    expect(describeSide(null)).toMatch(/by hand/);
  });
});
