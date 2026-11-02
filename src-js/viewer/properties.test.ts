import { describe, expect, it } from "vitest";

import {
  applyFilters,
  detectReferenceSources,
  isEmptyPropValue,
  keepChosen,
  propertyRows,
  referenceRows,
  sourceDisplayName,
  tieredValue,
} from "./properties.js";
import type { Room } from "../renderer/types.js";

describe("isEmptyPropValue", () => {
  /** THE detail that makes hide-empty worth having: Revit writes the literal
   *  string "None" for an unset parameter, so a test for "" alone hides
   *  nothing. */
  it("counts Revit's literal None as empty", () => {
    expect(isEmptyPropValue("None")).toBe(true);
    expect(isEmptyPropValue("none")).toBe(true);
    expect(isEmptyPropValue("  ")).toBe(true);
    expect(isEmptyPropValue(null)).toBe(true);
  });

  it("keeps a real value, including one that merely contains None", () => {
    expect(isEmptyPropValue("0")).toBe(false);
    expect(isEmptyPropValue("None-standard")).toBe(false);
  });
});

describe("applyFilters", () => {
  const rows = [
    ["Area", "12"],
    ["Base Finish", "None"],
    ["Ceiling", ""],
  ] as const;

  it("matches the name, not the value", () => {
    expect(applyFilters(rows, { filter: "fin", hideEmpty: false }).map(([k]) => k)).toEqual(["Base Finish"]);
    expect(applyFilters(rows, { filter: "12", hideEmpty: false })).toEqual([]);
  });

  it("hides empty values when asked", () => {
    expect(applyFilters(rows, { filter: "", hideEmpty: true }).map(([k]) => k)).toEqual(["Area"]);
  });

  /** Classification opts out: a room whose tiers are all resolved-but-undefined
   *  would otherwise lose the whole section, and "unclassified" is a fact. */
  it("can exempt a section from hide-empty", () => {
    const tiers = [["Department", "(undefined)"], ["Sub", ""]] as const;
    expect(applyFilters(tiers, { filter: "", hideEmpty: true }, { hideEmpty: false })).toHaveLength(2);
  });
});

describe("propertyRows", () => {
  it("sorts by name and reads the value out of the record", () => {
    const props = { B: { value: "2", storage_type: "String" }, A: { value: "1", storage_type: "String" } };
    expect(propertyRows(props)).toEqual([
      ["A", "1"],
      ["B", "2"],
    ]);
  });

  it("is empty for a room with no properties", () => {
    expect(propertyRows(undefined)).toEqual([]);
  });
});

describe("detectReferenceSources", () => {
  const room = (extra: Record<string, unknown> = {}): Room => ({ id: "a", ...extra }) as Room;

  it("finds a source by the shape of its record", () => {
    expect(detectReferenceSources({ rooms: [room({ drofus: { fields: { Name: "x" } } })] })).toEqual(["drofus"]);
  });

  it("ignores the room's own fields and anything not shaped like a record", () => {
    const r = room({ name: "Ward", loops: [], schedule: "not a record", classification: [] });
    expect(detectReferenceSources({ rooms: [r] })).toEqual([]);
  });

  /** A source whose rows ALL failed to match carries no record on any room.
   *  It is still discovered, so the panel says "not joined" rather than
   *  silently omitting it. */
  it("finds a source that matched nothing, from the labels", () => {
    expect(detectReferenceSources({ reference_labels: { room: { schedule: {} } }, rooms: [room()] })).toEqual([
      "schedule",
    ]);
  });

  it("is empty for no payload", () => {
    expect(detectReferenceSources(null)).toEqual([]);
  });
});

describe("sourceDisplayName", () => {
  it("capitalises, except where the product spells itself", () => {
    expect(sourceDisplayName("drofus")).toBe("dRofus");
    expect(sourceDisplayName("schedule")).toBe("Schedule");
  });
});

describe("referenceRows", () => {
  it("is null when the room matched no record, which is not the same as empty", () => {
    expect(referenceRows({ id: "a" }, "drofus")).toBeNull();
    expect(referenceRows({ id: "a", drofus: { fields: {} } } as unknown as Room, "drofus")).toEqual([]);
  });

  it("sorts the fields", () => {
    const r = { id: "a", drofus: { fields: { B: "2", A: "1" } } } as unknown as Room;
    expect(referenceRows(r, "drofus")).toEqual([
      ["A", "1"],
      ["B", "2"],
    ]);
  });
});

describe("keepChosen", () => {
  const rows = [
    ["Area", "12"],
    ["Name", "BED"],
    ["Workset", "1"],
  ] as const;

  it("shows everything when nothing was turned off", () => {
    expect(keepChosen(rows, undefined)).toEqual(rows);
    expect(keepChosen(rows, new Set())).toEqual(rows);
  });

  it("drops the names that were turned off", () => {
    expect(keepChosen(rows, new Set(["Workset"]))).toEqual([
      ["Area", "12"],
      ["Name", "BED"],
    ]);
  });

  /** The reason it records what is HIDDEN rather than what is chosen: the next
   *  element of the same kind may carry a property the last one did not, and a
   *  snapshot of chosen names would leave it silently absent. */
  it("leaves a newly arrived property on", () => {
    const later = [...rows, ["Fire Rating", "60"]] as const;
    expect(keepChosen(later, new Set(["Workset"])).map(([k]) => k)).toEqual(["Area", "Name", "Fire Rating"]);
  });

  it("never hands back the caller's array", () => {
    expect(keepChosen(rows, undefined)).not.toBe(rows);
  });
});

describe("tieredValue", () => {
  it("takes the instance when it holds something", () => {
    expect(tieredValue("Width", { Width: { value: "900" } }, { Width: { value: "820" } })).toBe("900");
  });

  /** THE rule this function exists for, and the measurement behind it: a blank
   *  instance parameter must not shadow a real type value. `Door Leaf
   *  Thickness` is blank on 22 of 26 sample doors while the type says 40.0. */
  it("falls through a BLANK instance to the type", () => {
    expect(tieredValue("Door Leaf Thickness", { "Door Leaf Thickness": { value: "" } }, { "Door Leaf Thickness": { value: "40.0" } })).toBe("40.0");
  });

  /** And through Revit's literal "None", which is what an unset parameter
   *  actually arrives as -- the same detail hide-empty turns on. */
  it("treats Revit's None as absent in either tier", () => {
    expect(tieredValue("Finish", { Finish: { value: "None" } }, { Finish: { value: "PAINT" } })).toBe("PAINT");
    expect(tieredValue("Finish", { Finish: { value: "None" } }, { Finish: { value: "None" } })).toBeNull();
  });

  it("is null when neither tier carries it, so a caller can say so in its own words", () => {
    expect(tieredValue("Nope", { A: { value: "1" } }, { B: { value: "2" } })).toBeNull();
    expect(tieredValue("Nope", undefined, undefined)).toBeNull();
  });
});
