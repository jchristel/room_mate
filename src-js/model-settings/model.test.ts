import { describe, expect, it } from "vitest";

import { missing, normalise, tokensIn, unknownTokens, withToken } from "./model.js";

describe("normalise", () => {
  it("fills every field and keeps keys it does not know", () => {
    const s = normalise({ plan_view_type: "Floor Plan : RLS", future: { x: 1 } });
    expect(s.plan_view_type).toBe("Floor Plan : RLS");
    expect(s.sheet_sizes).toEqual({ a0: "", a1: "", a2: "", a3: "" });
    expect(s.future).toEqual({ x: 1 });
  });
  it("ignores values of the wrong type", () => {
    const s = normalise({ sheet_sizes: 5, room_tag: 7 });
    expect(s.room_tag).toBe("");
    expect(s.sheet_sizes.a1).toBe("");
  });
});

describe("templates", () => {
  it("finds tokens in order", () => {
    expect(tokensIn("RLS-{Number}-{Name}")).toEqual(["Number", "Name"]);
    expect(tokensIn("fixed")).toEqual([]);
  });
  it("calls a token unknown only when Revit offered a list", () => {
    expect(unknownTokens("{Number}{Nmae}", ["Number", "Name"])).toEqual(["Nmae"]);
    expect(unknownTokens("{Anything}", undefined)).toEqual([]);
    expect(unknownTokens("{Anything}", [])).toEqual([]);
  });
  it("adds a token at the end", () => {
    expect(withToken("RLS-", "Number")).toBe("RLS-{Number}");
  });
});

describe("missing", () => {
  it("lists what is not chosen yet", () => {
    const m = missing(normalise({}));
    expect(m).toContain("title block for A0");
    expect(m).toContain("schedule base");
    expect(m).toContain("view name logic");
  });
  it("does not require the tags, which only warn", () => {
    const s = normalise({
      sheet_sizes: { a0: "x", a1: "x", a2: "x", a3: "x" },
      plan_view_type: "p", elevation_view_type: "e", schedule_base: "s",
      sheet_number: "{Number}", sheet_name: "{Name}", view_name: "{Name}",
    });
    expect(missing(s)).toEqual([]);
  });
});
