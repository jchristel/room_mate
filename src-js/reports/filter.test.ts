import { describe as suite, expect, it } from "vitest";

import {
  addTo,
  complete,
  condition,
  describe,
  fromWire,
  group,
  keepBlanksToo,
  replaceNode,
  setMode,
  toWire,
} from "./filter.js";
import type { Condition, Group } from "./filter.js";

const cond = (over: Partial<Condition>): Condition => ({ ...condition("element", "Type"), ...over });

suite("complete", () => {
  it("leaves a condition with no value out rather than failing the filter", () => {
    expect(complete(cond({ value: "" }))).toBe(false);
    expect(complete(cond({ value: "SGL" }))).toBe(true);
  });

  // The two operators that ask about absence take no value, because a blank
  // could never match one.
  it("counts is-blank and has-a-value as complete on their own", () => {
    expect(complete(cond({ op: "blank", value: "" }))).toBe(true);
    expect(complete(cond({ op: "has_value", value: "" }))).toBe(true);
  });

  it("needs both ends of a between", () => {
    expect(complete(cond({ type: "number", op: "between", value: "1", value2: "" }))).toBe(false);
    expect(complete(cond({ type: "number", op: "between", value: "1", value2: "9" }))).toBe(true);
  });
});

suite("toWire", () => {
  it("drops incomplete conditions rather than sending half of one", () => {
    const root = group("all", [cond({ value: "SGL" }), cond({ value: "" })]);
    expect((toWire(root) as { items: unknown[] }).items).toHaveLength(1);
  });

  it("sends no value for the operators that take none", () => {
    expect(toWire(cond({ op: "blank" }))).toMatchObject({ op: "blank", value: undefined });
  });

  it("carries the side, so the server knows which half of the join to read", () => {
    expect(toWire(cond({ side: "join", field: "overlap_area" }))).toMatchObject({ side: "join", field: "overlap_area" });
  });
});

suite("describe", () => {
  // The line exists so a reader can check the logic they built; a nested form
  // alone does not let them.
  it("reads a room condition plainly", () => {
    expect(describe(group("all", [cond({ side: "room", field: "Level", value: "LEVEL 00" })]), "ceiling")).toBe(
      'Room.Level is "LEVEL 00"',
    );
  });

  // **The set reading.** "is not X" on the associated side asks for a room with
  // NONE, which is not "one that is not X" — and the sentence has to say so, or
  // the reader checks the wrong thing.
  it("reads an associated condition set-wise, negation included", () => {
    const has = describe(group("all", [cond({ field: "Type", value: "EXT" })]), "door");
    expect(has).toBe('the room has a door whose Type is "EXT"');

    const hasNot = describe(group("all", [cond({ field: "Type", op: "ne", value: "EXT" })]), "door");
    expect(hasNot).toBe('the room has no door whose Type is "EXT"');
  });

  it("brackets a nested group and names its connector", () => {
    const root = group("all", [
      cond({ side: "room", field: "Level", value: "LEVEL 00" }),
      group("any", [cond({ field: "Type", value: "SGL" }), cond({ field: "Type", value: "EXT" })]),
    ]);
    expect(describe(root, "door")).toBe(
      'Room.Level is "LEVEL 00" AND (the room has a door whose Type is "SGL" OR the room has a door whose Type is "EXT")',
    );
  });

  it("says so when nothing is filtered", () => {
    expect(describe(group("all", []), "door")).toBe("every row");
  });

  it("marks a case-sensitive condition, since the default is not", () => {
    expect(describe(group("all", [cond({ field: "Type", value: "sgl", caseSensitive: true })]), "door")).toContain(
      "(match case)",
    );
  });
});

suite("tree edits", () => {
  it("adds into the group asked for, not the root", () => {
    const inner = group("any", []);
    const root = group("all", [inner]);
    const next = addTo(root, inner.id, cond({ value: "SGL" }));
    expect((next.items[0] as Group).items).toHaveLength(1);
    expect(next.items).toHaveLength(1);
  });

  it("removes a node by id, wherever it sits", () => {
    const target = cond({ value: "SGL" });
    const root = group("all", [group("any", [target, cond({ value: "EXT" })])]);
    const next = replaceNode(root, target.id, null);
    expect((next.items[0] as Group).items).toHaveLength(1);
  });

  it("changes one group's mode and leaves its siblings alone", () => {
    const inner = group("any", []);
    const root = group("all", [inner]);
    const next = setMode(root, inner.id, "all");
    expect(next.mode).toBe("all");
    expect((next.items[0] as Group).mode).toBe("all");
  });

  // "Keep blanks too" is the answer to the surprise that a room with no
  // Department fails "Department is not Living".
  it("wraps a condition in any(it, is blank)", () => {
    const target = cond({ side: "room", field: "Department", op: "ne", value: "Living" });
    const root = group("all", [target]);
    const next = keepBlanksToo(root, target);
    const wrapped = next.items[0] as Group;
    expect(wrapped.kind).toBe("group");
    expect(wrapped.mode).toBe("any");
    expect((wrapped.items[1] as Condition).op).toBe("blank");
  });
});

suite("fromWire", () => {
  // The saved shape and the editing shape are not the same object. Loading one
  // as the other crashed the page, which is why this round trip is pinned.
  it("rebuilds the editing model from what was saved", () => {
    const built = group("all", [
      cond({ side: "room", field: "Level", value: "LEVEL 00" }),
      group("any", [cond({ field: "Type", op: "not_contains", value: "EXT", caseSensitive: true })]),
    ]);

    const loaded = fromWire(toWire(built));

    expect(loaded.kind).toBe("group");
    expect(loaded.mode).toBe("all");
    expect(loaded.items).toHaveLength(2);
    const first = loaded.items[0] as Condition;
    expect(first.kind).toBe("condition");
    expect(first.side).toBe("room");
    expect(first.value).toBe("LEVEL 00");
    const nested = loaded.items[1] as Group;
    expect(nested.mode).toBe("any");
    expect((nested.items[0] as Condition).caseSensitive).toBe(true);
    expect(describe(loaded, "door")).toBe(describe(built, "door"));
  });

  // An operator that takes no value saves none, so the load has to supply one
  // rather than leaving it undefined — the exact crash this fixed.
  it("gives a value-less operator an empty value rather than undefined", () => {
    const loaded = fromWire(toWire(group("all", [cond({ op: "blank" })])));
    expect((loaded.items[0] as Condition).value).toBe("");
    expect(complete(loaded)).toBe(true);
  });

  // A numeric operator is the only evidence of a numeric field a saved
  // document carries.
  it("infers a number field from a numeric operator", () => {
    const loaded = fromWire({ mode: "all", items: [{ side: "join", field: "overlap_area", op: "ge", value: "10" }] });
    expect((loaded.items[0] as Condition).type).toBe("number");
  });

  it("turns anything unreadable into an empty filter rather than crashing", () => {
    expect(fromWire(null).items).toHaveLength(0);
    expect(fromWire("nonsense").items).toHaveLength(0);
    expect(complete(fromWire({ mode: "all", items: [{ nope: true }] }))).toBe(false);
  });
});
