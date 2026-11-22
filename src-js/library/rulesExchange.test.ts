import { describe, expect, it } from "vitest";

import { exportedRule, fileNameFor, outcomeText, parseFile, sum, toFile } from "./rulesExchange.js";
import type { Sample, TagRule } from "./types.js";

const sample = (tag: number, boundary = false): Sample => ({
  source: { sheet: { document_title: "M", revit_sheet_id: "1", sheet_number: "A" }, tag_id: tag, element_id: tag },
  view_scale: 50,
  head: { x: 1, y: 2 },
  rotation: 0,
  orientation: "Horizontal",
  has_leader: false,
  leader_end_condition: "",
  ...(boundary ? { boundary: { of: "wall" as const, edge: 1, along: 0.5, off: 4 } } : {}),
  from_centre: true,
});

const rule = (over: Partial<TagRule["key"]> = {}, samples: Sample[] = [sample(1)]): TagRule =>
  ({
    id: "tr-1",
    key: { tagged_category: "Furniture", code_parameter: "Code", type_code: "B1", tag_category: "Tags", tag_family: "F", tag_type: "T", viewport_type: "plan", blueprint: "", instance: "", ...over },
    tag_kind: "independent",
    placement: { head: { x: 1, y: 2 }, rotation: 0, orientation: "Horizontal", has_leader: false, leader_end_condition: "" },
    sample_count: samples.length,
    updated_at: "t",
    samples,
  }) as TagRule;

const NOW = new Date("2026-11-22T10:00:00Z");

describe("a tag rules file", () => {
  it("holds what a rule is and none of what is worked out from it", () => {
    const f = toFile([rule()], "House A", NOW);
    expect(f).toMatchObject({ format: "roommate-tag-rules", version: 1, exported_from: "House A" });
    expect(Object.keys(f.rules[0] ?? {}).sort()).toEqual(["key", "samples", "tag_kind"]);
  });

  it("files a blueprint's rule as the generic one it would become, without the boundary its room gave it", () => {
    const e = exportedRule(rule({ blueprint: "bp-1", instance: "42" }, [sample(1, true)]));
    expect(e.key).toMatchObject({ blueprint: "", instance: "", type_code: "B1" });
    expect(e.samples[0]).not.toHaveProperty("boundary");
    expect(e.samples[0]?.from_centre).toBe(true);
  });

  it("keeps a generic rule's boundary", () => {
    expect(exportedRule(rule({}, [sample(1, true)])).samples[0]).toHaveProperty("boundary");
  });

  it("round-trips through its own text", () => {
    const f = toFile([rule(), rule({ type_code: "B2" })], "House A", NOW);
    expect(parseFile(JSON.stringify(f))).toEqual({ ok: true, file: f });
  });

  it("names itself after the project, the count and the day", () => {
    expect(fileNameFor("House A", 4, NOW)).toBe("House_A.4-tag-rules.2026-11-22.json");
  });
});

describe("reading a file", () => {
  const ok = JSON.stringify(toFile([rule()], "House A", NOW));
  it("says why a file is not accepted", () => {
    expect(parseFile("{nope")).toMatchObject({ ok: false, error: "That file is not JSON." });
    expect(parseFile(JSON.stringify({ format: "roommate-sheets" }))).toMatchObject({ ok: false });
    expect(parseFile(ok.replace('"version":1', '"version":3'))).toMatchObject({ ok: false });
    expect(parseFile(JSON.stringify({ ...JSON.parse(ok), rules: [] }))).toMatchObject({ ok: false, error: "That file holds no rules." });
  });
  it("names the rule that lacks what the library keys it by", () => {
    const f = JSON.parse(ok);
    f.rules[0].key.tag_category = "";
    expect(parseFile(JSON.stringify(f))).toMatchObject({ ok: false, error: "Rule 1 has no tagged or tag category." });
    f.rules[0].key.tag_category = "Tags";
    f.rules[0].samples = null;
    expect(parseFile(JSON.stringify(f))).toMatchObject({ ok: false, error: "Rule 1 has no samples." });
  });
});

describe("the result", () => {
  const o = { rules_created: 2, rules_updated: 1, samples: 9, empty: 0, unused: 0 };
  it("counts rules and samples", () => {
    expect(outcomeText(o)).toBe("2 new, 1 updated (9 samples)");
  });
  it("says what the person should know", () => {
    expect(outcomeText({ ...o, empty: 1, unused: 3 })).toContain("1 had no samples");
    expect(outcomeText({ ...o, unused: 3 })).toContain("3 read a type-code parameter other than this project's");
  });
  it("adds the batches up", () => {
    expect(sum([o, o])).toEqual({ rules_created: 4, rules_updated: 2, samples: 18, empty: 0, unused: 0 });
    expect(sum([])).toEqual({ rules_created: 0, rules_updated: 0, samples: 0, empty: 0, unused: 0 });
  });
});
