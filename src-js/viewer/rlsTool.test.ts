import { describe, expect, it } from "vitest";

import { chooseSheet, madeText, newRlsTool, pickRoom, roomsText, whyNot } from "./rlsTool.js";

describe("the RLS tool", () => {
  it("adds a room on one click and takes it off on the next", () => {
    const a = pickRoom(newRlsTool(), "r1");
    const b = pickRoom(a, "r2");
    expect(b.roomIds).toEqual(["r1", "r2"]);
    expect(pickRoom(b, "r1").roomIds).toEqual(["r2"]);
  });

  it("needs a sheet and a room, and says which", () => {
    expect(whyNot(newRlsTool())).toBe("choose a sheet");
    expect(whyNot(chooseSheet(newRlsTool(), "s"))).toBe("click the rooms to make sheets for");
    expect(whyNot(pickRoom(chooseSheet(newRlsTool(), "s"), "r"))).toBeNull();
  });

  it("counts rooms", () => {
    expect(roomsText(1)).toBe("1 room");
    expect(roomsText(2)).toBe("2 rooms");
  });

  it("reports what was queued and the first refusal", () => {
    expect(madeText([{ subject: "1" }, { subject: "2", room_name: "BED", refused: "a task is already queued" }, { subject: "3", refused: "x" }])).toBe(
      "Queued 1 room. Refused: BED: a task is already queued (+1 more)",
    );
    expect(madeText([{ subject: "1" }])).toBe("Queued 1 room.");
  });
});

import { autoBatches, bandOf, categoryOf, includeAll, NO_SHEET, pickAuto, setOverride, sheetFor, solverSet, tally, type RlsToolState, type Solved } from "./rlsTool.js";

const solved = (id: string, fit: number, sheet = "s1"): Solved => ({
  room_id: id,
  choices: [{ sheet_id: sheet, sheet_number: "1", sheet_name: "", room_name: "", fit }],
});

describe("RLS auto mode", () => {
  it("bands the fit green from 80, yellow from 60, red below", () => {
    expect([bandOf(100), bandOf(80), bandOf(79.9), bandOf(60), bandOf(59.9)]).toEqual(["green", "green", "yellow", "yellow", "red"]);
  });

  it("asks the solver about rooms without a task, plus the ones put back", () => {
    const s = { ...newRlsTool(), included: ["b"] };
    expect(solverSet(["a", "b", "c"], new Set(["b", "c"]), s)).toEqual(["a", "b"]);
  });

  it("focuses on one pick and toggles a room with a task on a double click", () => {
    const done = new Set(["d"]);
    const one = pickAuto(newRlsTool(), "d", 1000, done);
    expect(one.focus).toBe("d");
    expect(one.included).toEqual([]);
    const two = pickAuto(one, "d", 1200, done);
    expect(two.included).toEqual(["d"]);
    expect(pickAuto(two, "d", 1300, done).included).toEqual([]);
    // A slow second pick, or a room with no task, is only a focus.
    expect(pickAuto(one, "d", 5000, done).included).toEqual([]);
    expect(pickAuto(pickAuto(newRlsTool(), "x", 0, done), "x", 10, done).included).toEqual([]);
  });

  it("includes all rooms that have a task", () => {
    expect(includeAll(newRlsTool(), new Set(["a", "b"])).included).toEqual(["a", "b"]);
  });

  it("makes overrides their own category, and the choice theirs", () => {
    let s: RlsToolState = { ...newRlsTool(), solved: { a: solved("a", 90), b: solved("b", 50, "s2") } };
    s = setOverride(s, "b", "s9");
    expect(categoryOf(s, "a")).toBe("green");
    expect(categoryOf(s, "b")).toBe("override");
    expect(sheetFor(s, "b")).toBe("s9");
    expect(sheetFor(setOverride(s, "b", NO_SHEET), "b")).toBeNull();
    expect(categoryOf(setOverride(s, "b", null), "b")).toBe("red");
    expect(tally(s, ["a", "b", "zz"])).toEqual({ green: 1, yellow: 0, red: 0, override: 1 });
  });

  it("queues only the ticked categories, grouped by sheet", () => {
    let s: RlsToolState = { ...newRlsTool(), solved: { a: solved("a", 95), b: solved("b", 85, "s2"), c: solved("c", 70), d: solved("d", 30) } };
    s = setOverride(s, "d", "s3");
    expect([...autoBatches(s, ["a", "b", "c", "d"])]).toEqual([
      ["s1", ["a"]],
      ["s2", ["b"]],
      ["s3", ["d"]],
    ]);
    s = { ...s, bands: { green: false, yellow: true, red: false, override: false } };
    expect([...autoBatches(s, ["a", "b", "c", "d"])]).toEqual([["s1", ["c"]]]);
    expect(autoBatches(setOverride(s, "c", NO_SHEET), ["c"]).size).toBe(0);
  });
});
