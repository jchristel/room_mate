import { describe, expect, it } from "vitest";

import { attentionReasons, blueprintNeedsAttention, filterBlueprints } from "./common.js";

describe("blueprints that need attention", () => {
  it("a blueprint with sheets and nothing pending does not", () => {
    expect(blueprintNeedsAttention({ sheets: 2, pending: 0 })).toBe(false);
    expect(attentionReasons({ sheets: 2, pending: 0 })).toEqual([]);
  });

  it("one with no sheets does, and says why", () => {
    expect(blueprintNeedsAttention({ sheets: 0, pending: 0 })).toBe(true);
    expect(attentionReasons({ sheets: 0, pending: 0 })).toEqual(["no room layout sheets"]);
  });

  it("a pending revision does, and both reasons are given when both hold", () => {
    expect(blueprintNeedsAttention({ sheets: 1, pending: 1 })).toBe(true);
    expect(attentionReasons({ sheets: 0, pending: 2 })).toEqual(["no room layout sheets", "2 pending revisions to accept or reject"]);
  });
});

describe("filtering the blueprint list", () => {
  const rows = [
    { name: "WC_00", source: { room_name: "WC 00.03", room_id: "2626836" } },
    { name: "OFFICE_00", source: { room_name: "OFFICE 00.02", room_id: "2626001" } },
    { name: "Imported", source: null },
  ];

  it("keeps every row for an empty filter, in order", () => {
    expect(filterBlueprints(rows, "  ").map((b) => b.name)).toEqual(["WC_00", "OFFICE_00", "Imported"]);
  });

  it("matches the name or the source room, case-insensitively", () => {
    expect(filterBlueprints(rows, "office").map((b) => b.name)).toEqual(["OFFICE_00"]);
    expect(filterBlueprints(rows, "00.03").map((b) => b.name)).toEqual(["WC_00"]);
  });

  it("needs every word, and copes with a blueprint that has no source room", () => {
    expect(filterBlueprints(rows, "wc 2626836").map((b) => b.name)).toEqual(["WC_00"]);
    expect(filterBlueprints(rows, "wc office")).toEqual([]);
    expect(filterBlueprints(rows, "imp").map((b) => b.name)).toEqual(["Imported"]);
  });
});
