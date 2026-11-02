import { describe, expect, it } from "vitest";

import { DEFAULT_TAG_FIELDS, itemTagLines, tagFieldsOf } from "./tagText.js";

const item = {
  id: "2626831",
  type_name: "ELGP-208a",
  properties: [
    { name: "Mark", value: "GPO-12" },
    { name: "Comments", value: "" },
  ],
  type_properties: [
    { name: "Comments", value: "double outlet" },
    { name: "Mark", value: "TYPE-MARK" },
    { name: "Model", value: "None" },
  ],
};

describe("the fields a tag says", () => {
  it("come from the project's [tags] ffe setting, else the default", () => {
    expect(tagFieldsOf({ settings: { tags: { ffe: { fields: ["Mark", "$type_name"] } } } })).toEqual(["Mark", "$type_name"]);
    expect(tagFieldsOf({ settings: {} })).toEqual(DEFAULT_TAG_FIELDS);
    expect(tagFieldsOf({ settings: { tags: { ffe: { fields: [] } } } })).toEqual(DEFAULT_TAG_FIELDS);
    expect(tagFieldsOf(null)).toEqual(DEFAULT_TAG_FIELDS);
  });
});

describe("an item's tag lines", () => {
  it("reads $id, $type_name and a property, one line per field", () => {
    expect(itemTagLines(["$id", "$type_name", "Mark"], item)).toEqual(["2626831", "ELGP-208a", "GPO-12"]);
  });

  it("falls back to the type when the instance value is blank, and skips 'None'", () => {
    expect(itemTagLines(["Comments", "Model"], item)).toEqual(["double outlet"]);
  });

  it("says nothing for $name (a blueprint item has none) or a property it does not hold", () => {
    expect(itemTagLines(["$name", "Absent"], item)).toEqual([]);
  });
});
