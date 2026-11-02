import { describe, expect, it } from "vitest";

import { buildColumns, buildCsv, columnGroups, computeRows, csvEscape, visibleColumns } from "./grid.js";
import type { Room } from "../renderer/types.js";

const room = (id: string, name: string, level: string, props: Record<string, string> = {}, extra = {}): Room =>
  ({
    id,
    name,
    level_id: level,
    properties: Object.fromEntries(Object.entries(props).map(([k, v]) => [k, { value: v, storage_type: "String" }])),
    ...extra,
  }) as Room;

const payload = {
  levels: [{ id: "L1", name: "LEVEL 01", elevation: 0 }],
  rooms: [
    room("2", "Ward", "L1", { Area: "9" }, { drofus: { fields: { Code: "W2" } } }),
    room("10", "Office", "L1", { Area: "10" }),
  ],
};

describe("buildColumns", () => {
  it("puts identity first, then property keys sorted", () => {
    const cols = buildColumns(payload, null, []);
    expect(cols.map((c) => c.key)).toEqual(["$level", "$name", "$id", "p:Area"]);
  });

  it("resolves the level NAME, which is why it is not just another property", () => {
    const cols = buildColumns(payload, null, []);
    expect(cols[0]!.get(payload.rooms[0]!)).toBe("LEVEL 01");
  });

  /** A column that matched no room in scope must still exist: "not checked"
   *  is a fact about the column, and hiding it is what the coverage report
   *  goes out of its way to avoid. */
  it("takes reference columns from the response labels, including ones nothing matched", () => {
    const withLabels = {
      ...payload,
      reference_labels: { P1: { drofus: { all_labels: ["Code", "Never Matched"], reconciliation: { Code: {} } } } },
    };
    const cols = buildColumns(withLabels, "P1", ["drofus"]);
    expect(cols.filter((c) => c.source === "drofus").map((c) => c.label)).toEqual(["Code", "Never Matched"]);
    expect(cols.find((c) => c.label === "Never Matched")?.unmapped).toBe(true);
    expect(cols.find((c) => c.label === "Code")?.unmapped).toBe(false);
  });

  it("falls back to the rooms' own fields for a source with no vocabulary here", () => {
    const cols = buildColumns(payload, "P1", ["drofus"]);
    expect(cols.filter((c) => c.source === "drofus").map((c) => c.label)).toEqual(["Code"]);
    expect(cols.at(-1)!.get(payload.rooms[0]!)).toBe("W2");
    // An unmatched room has no key for the source: an empty cell, not an error.
    expect(cols.at(-1)!.get(payload.rooms[1]!)).toBe("");
  });
});

describe("visibleColumns", () => {
  const cols = buildColumns(payload, "P1", ["drofus"]);

  it("follows the model and per-source toggles", () => {
    expect(visibleColumns(cols, true, new Set()).every((c) => c.source === "model")).toBe(true);
    expect(visibleColumns(cols, false, new Set(["drofus"])).map((c) => c.source)).toEqual(["drofus"]);
    expect(visibleColumns(cols, false, new Set())).toEqual([]);
  });
});

describe("computeRows", () => {
  const cols = buildColumns(payload, null, []);
  const byArea = cols.find((c) => c.key === "p:Area")!;

  /** "10" must not sort before "9": the column is text on the wire and a
   *  string compare is what a reader reads as broken. */
  it("sorts numerically when both sides parse", () => {
    const asc = computeRows(payload.rooms, cols, new Map(), { key: byArea.key, asc: true });
    expect(asc.map((r) => r.id)).toEqual(["2", "10"]);
    const desc = computeRows(payload.rooms, cols, new Map(), { key: byArea.key, asc: false });
    expect(desc.map((r) => r.id)).toEqual(["10", "2"]);
  });

  it("falls back to a string compare when they do not", () => {
    const byName = cols.find((c) => c.key === "$name")!;
    const rows = computeRows(payload.rooms, cols, new Map(), { key: byName.key, asc: true });
    expect(rows.map((r) => r.name)).toEqual(["Office", "Ward"]);
  });

  it("filters case-insensitively, and every active filter must match", () => {
    const byName = cols.find((c) => c.key === "$name")!;
    expect(computeRows(payload.rooms, cols, new Map([[byName.key, "ward"]]), { key: null, asc: true })).toHaveLength(1);
    expect(
      computeRows(payload.rooms, cols, new Map([[byName.key, "ward"], [byArea.key, "10"]]), { key: null, asc: true }),
    ).toHaveLength(0);
  });

  it("does not reorder the payload's own array", () => {
    computeRows(payload.rooms, cols, new Map(), { key: "$name", asc: true });
    expect(payload.rooms.map((r) => r.id)).toEqual(["2", "10"]);
  });
});

describe("columnGroups", () => {
  it("spans consecutive columns of one source", () => {
    const cols = buildColumns(payload, "P1", ["drofus"]);
    expect(columnGroups(cols)).toEqual([
      { source: "model", label: "Model", span: 4 },
      { source: "drofus", label: "dRofus", span: 1 },
    ]);
  });
});

describe("csv", () => {
  it("quotes only what needs it", () => {
    expect(csvEscape("plain")).toBe("plain");
    expect(csvEscape('say "hi", then')).toBe('"say ""hi"", then"');
  });

  it("qualifies reference columns so two sources cannot collide", () => {
    const cols = buildColumns(payload, "P1", ["drofus"]);
    const csv = buildCsv(payload.rooms, cols);
    expect(csv.split("\r\n")[0]).toBe("Level,Name,Id,Area,drofus.Code");
  });

  /** EVERY filtered row, not the windowed slice: the window is a rendering
   *  detail, and exporting it would give a few dozen rows of a few thousand. */
  it("writes a line per row it is given", () => {
    const cols = buildColumns(payload, null, []);
    expect(buildCsv(payload.rooms, cols).split("\r\n")).toHaveLength(3);
  });

  it("is empty with no columns", () => {
    expect(buildCsv(payload.rooms, [])).toBe("");
  });
});
