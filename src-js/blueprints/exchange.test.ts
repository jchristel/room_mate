import { describe, expect, it } from "vitest";

import { fileNameFor, isSheetConflict, parseFile, takingTheSheets, toFile } from "./exchange.js";
import type { Blueprint } from "./types.js";

const bp: Blueprint = {
  id: "bp-1",
  name: "1 BED B",
  taken_at: "t",
  current_revision: 2,
  revisions: [
    { number: 1, created_at: "a", outline: { coarse: [], fine: [] }, items: [] },
    {
      number: 2,
      created_at: "b",
      note: "moved",
      outline: { coarse: [{ x: 0, y: 0 }], fine: [] },
      items: [],
    },
  ],
};

describe("export files", () => {
  it("round-trips one revision", () => {
    const f = toFile(bp, 2, "RHH");
    expect(f?.revision.note).toBe("moved");
    const back = parseFile(JSON.stringify(f));
    expect(back.ok && back.file.name).toBe("1 BED B");
  });

  it("carries the room in the file's revision and the sheets and tags in a bundle beside it", () => {
    const f = toFile(bp, 1, "x");
    expect(f && "sheets" in f.revision).toBe(false);
    expect(f && "bundle" in f).toBe(false);
    expect(parseFile(JSON.stringify(f))).toMatchObject({ ok: true });
    const bundle = { sheets: [{ revit_sheet_id: "1", document_title: "D", sheet_number: "RLS-01", sheet: {} }], rules: [] };
    const withBundle = parseFile(JSON.stringify(toFile(bp, 1, "x", bundle)));
    expect(withBundle.ok && withBundle.file.bundle?.sheets.length).toBe(1);
  });

  it("recognises the server's refusal of sheets another blueprint holds, and answers it by taking them", () => {
    const refusal = "1 of the sheets are already bound to another blueprint (RLS-1 (bound to \"A\")); a sheet belongs to one blueprint";
    expect(isSheetConflict(refusal)).toBe(true);
    expect(isSheetConflict("a blueprint holds at most 200 revisions")).toBe(false);
    const bundle = { sheets: [{ revit_sheet_id: "1", document_title: "D", sheet_number: "RLS-1", sheet: {} }] };
    const file = toFile(bp, 1, "x", bundle)!;
    expect(file.bundle?.take_over_sheets).toBeUndefined();
    expect(takingTheSheets(file).bundle?.take_over_sheets).toBe(true);
    const roomOnly = toFile(bp, 1, "x")!;
    expect(takingTheSheets(roomOnly)).toBe(roomOnly);
  });

  it("has nothing to export for a revision the blueprint lacks", () => {
    expect(toFile(bp, 9, "RHH")).toBeNull();
  });

  it("says why a file is not a blueprint export", () => {
    expect(parseFile("not json")).toEqual({ ok: false, error: "That file is not JSON." });
    expect(parseFile("{}")).toMatchObject({ ok: false });
    const wrongVersion = JSON.stringify({ ...toFile(bp, 1, "x"), version: 2 });
    expect(parseFile(wrongVersion)).toMatchObject({ ok: false });
    const noItems = JSON.stringify({ format: "roommate-blueprint", version: 1, name: "n", revision: {} });
    expect(parseFile(noItems)).toMatchObject({ ok: false });
  });

  it("makes a file name that is safe everywhere", () => {
    expect(fileNameFor("1 BED B / IP2022", 3)).toBe("1_BED_B_IP2022.r3.blueprint.json");
    expect(fileNameFor("///", 1)).toBe("blueprint.r1.blueprint.json");
  });
});
