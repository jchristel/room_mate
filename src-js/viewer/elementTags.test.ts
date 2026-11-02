import { describe, expect, it } from "vitest";
import { tagLines } from "./elementTags.js";

describe("tagLines", () => {
  const el = { id: "7", name: "Plant", type_name: "Chiller", properties: { Mark: { value: "M-1" }, Blank: { value: "" } } };
  it("reads intrinsics and properties in field order", () => {
    expect(tagLines(["$name", "Mark", "$id"], el, undefined)).toEqual(["Plant", "M-1", "7"]);
  });
  it("falls through to the type tier when the instance is blank", () => {
    expect(tagLines(["Blank"], el, { Blank: { value: "from type" } })).toEqual(["from type"]);
  });
  it("skips a field nothing carries, and Revit's literal None", () => {
    expect(tagLines(["Missing", "$type_name"], el, undefined)).toEqual(["Chiller"]);
    expect(tagLines(["X"], { id: "1", properties: { X: { value: "None" } } }, undefined)).toEqual([]);
  });
});
