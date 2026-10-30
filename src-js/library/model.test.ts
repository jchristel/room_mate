import { describe, expect, it } from "vitest";

import { degrees, filterRules, ruleLabel, sheetNeedsAttention, sheetReasons, sizeText, suggestionText, summaryText } from "./model.js";
import type { RuleSummary, Suggestion } from "./types.js";

const rule = (code: string, tag: [string, string], vp = "floor plan"): RuleSummary => ({
  id: "tr-" + code + tag.join("") + vp,
  key: { tagged_category: "Furniture", code_parameter: "Assembly Code", type_code: code, tag_category: "Furniture Tags", tag_family: tag[0], tag_type: tag[1], viewport_type: vp },
  tag_kind: "independent",
  placement: { head: { x: 1, y: 2 }, rotation: 0, orientation: "Horizontal", has_leader: false, leader_end_condition: "Free" },
  sample_count: 1,
  updated_at: "2026-10-21T00:00:00Z",
});

describe("tag rules", () => {
  const rules = [rule("BED-01", ["FTag", "Std"]), rule("CHAIR-02", ["FTag", "Std"]), rule("BED-01", ["FTag", "Std"], "elevation")];

  it("are labelled by the type code, or the category for a room", () => {
    expect(ruleLabel(rules[0]!)).toBe("BED-01 tagged by FTag / Std");
    const room = { ...rules[0]!, key: { ...rules[0]!.key, tagged_category: "Rooms", code_parameter: "", type_code: "" } };
    expect(ruleLabel(room)).toBe("Rooms tagged by FTag / Std");
  });

  it("are narrowed by every word, in any field, ignoring case", () => {
    expect(filterRules(rules, "bed").length).toBe(2);
    expect(filterRules(rules, "BED elevation").length).toBe(1);
    expect(filterRules(rules, "").length).toBe(3);
    expect(filterRules(rules, "sofa").length).toBe(0);
  });

  it("list floor plan before elevation for the same code", () => {
    const got = filterRules(rules, "bed");
    expect(got.map((r) => r.key.viewport_type)).toEqual(["elevation", "floor plan"]);
  });

  it("show rotation in degrees", () => {
    expect(degrees(Math.PI / 2)).toBe(90);
    expect(degrees(0)).toBe(0);
  });
});

describe("sheets", () => {
  it("need attention only when a push waits", () => {
    expect(sheetNeedsAttention({ pending: 0 })).toBe(false);
    expect(sheetReasons({ pending: 2 })).toEqual(["2 renumbered pushes to accept or reject"]);
  });

  it("state their measured paper size", () => {
    expect(sizeText({ width_mm: 841.2, height_mm: 593.9 })).toBe("841 x 594 mm");
    expect(sizeText(undefined)).toBe("not measured");
  });

  it("summarise their room", () => {
    expect(summaryText({ x_mm: 4023.5, y_mm: 3940, area_m2: 15.86, items: 1 })).toBe("4.0 x 3.9 m, 15.9 m2, 1 item");
    expect(summaryText(undefined)).toBe("no room figures");
  });

  it("say how far a suggestion is in each figure", () => {
    const s = { parts: { area: Math.log(1.5), proportion: 0, items: Math.log(2) } } as Suggestion;
    expect(suggestionText(s)).toBe("area 50% off, proportions 0% off, items 100% off");
  });
});
