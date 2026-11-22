import { describe, expect, it } from "vitest";

import { fileNameFor, outcomeText, parseFile, toFile } from "./exchange.js";
import type { SheetRecord } from "./types.js";

const record = (over: Partial<SheetRecord> = {}): SheetRecord =>
  ({
    id: "s1",
    taken_at: "t",
    revit_sheet_id: "100",
    document_title: "Model A",
    sheet_number: "50H- 00.01",
    sheet_name: "RUMPUS",
    room: { model_id: "m", room_id: "7" },
    blueprint: "bp-1",
    room_outline: [[0, 0], [3000, 0], [3000, 2000]],
    rooms: ["00.01 RUMPUS"],
    received_at: "r",
    sheet: { view_ports: [] },
    pending: [{ pending_id: 1 }],
    rejected: [],
    ...over,
  }) as unknown as SheetRecord;

const NOW = new Date("2026-11-22T10:00:00Z");

describe("a sheets file", () => {
  it("carries the sheet and its room data but none of its history or links", () => {
    const f = toFile([record()], "House A", NOW);
    expect(f).toMatchObject({ format: "roommate-sheets", version: 1, exported_from: "House A", exported_at: NOW.toISOString() });
    expect(f.sheets[0]).toMatchObject({ revit_sheet_id: "100", room_outline: [[0, 0], [3000, 0], [3000, 2000]], rooms: ["00.01 RUMPUS"] });
    expect(f.sheets[0]).not.toHaveProperty("pending");
    expect(f.sheets[0]).not.toHaveProperty("id");
    expect(f.sheets[0]).not.toHaveProperty("room");
    expect(f.sheets[0]).not.toHaveProperty("blueprint");
  });

  it("round-trips through its own text", () => {
    const f = toFile([record()], "House A", NOW);
    const parsed = parseFile(JSON.stringify(f));
    expect(parsed).toEqual({ ok: true, file: f });
  });

  it("names itself after the project, the count and the day", () => {
    expect(fileNameFor("House A", 3, NOW)).toBe("House_A.3-sheets.2026-11-22.json");
  });
});

describe("reading a file", () => {
  const ok = JSON.stringify(toFile([record()], "House A", NOW));
  it("says why a file is not accepted", () => {
    expect(parseFile("{nope")).toMatchObject({ ok: false, error: "That file is not JSON." });
    expect(parseFile(JSON.stringify({ format: "roommate-blueprint" }))).toMatchObject({ ok: false });
    expect(parseFile(ok.replace('"version":1', '"version":2'))).toMatchObject({ ok: false });
    expect(parseFile(JSON.stringify({ ...JSON.parse(ok), sheets: [] }))).toMatchObject({ ok: false, error: "That file holds no sheets." });
  });

  it("names the sheet that is missing what the library keys it by", () => {
    const bad = { ...JSON.parse(ok), sheets: [{ ...JSON.parse(ok).sheets[0], document_title: "" }] };
    const r = parseFile(JSON.stringify(bad));
    expect(r).toMatchObject({ ok: false });
    expect(!r.ok && r.error).toContain("50H- 00.01");
  });

  it("refuses a sheet with no body", () => {
    const bad = { ...JSON.parse(ok), sheets: [{ ...JSON.parse(ok).sheets[0], sheet: null }] };
    expect(parseFile(JSON.stringify(bad))).toMatchObject({ ok: false });
  });
});

describe("the result line", () => {
  it("counts what happened and says what waits", () => {
    const o = (outcome: "created" | "updated" | "pending") => ({ id: "x", sheet_number: "1", outcome });
    expect(outcomeText([o("created"), o("created"), o("updated"), o("pending")])).toBe("2 new, 1 updated, 1 waiting for a decision (the number changed)");
    expect(outcomeText([o("updated")])).toBe("0 new, 1 updated");
  });
});
