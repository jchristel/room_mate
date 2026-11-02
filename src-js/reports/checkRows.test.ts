import { describe, expect, it } from "vitest";

import { CHECKS, visibleRows } from "./checkRows.js";
import type { ValidationResponse } from "./types.js";

const check = (id: string) => CHECKS.find((c) => c.id === id)!;
const defaults = (id: string) => Object.fromEntries(check(id).options.map((o) => [o.id, o.on]));

const empty: ValidationResponse = { sources: {}, total_rooms: 0, phases: { disagree: false }, openings: {} };

describe("the reference check", () => {
  const data: ValidationResponse = {
    ...empty,
    total_rooms: 2,
    sources: {
      drofus: {
        link_property: "Number",
        rooms_missing_link_value: ["r1"],
        duplicate_link_values: [{ value: "1042", room_ids: ["r2", "r3"] }],
        rooms_unmatched: [],
        reference_unmatched: ["1099"],
        property_mismatches: [],
        fields_absent_in_revit: [],
        fields_empty_in_revit: [],
        error_rooms: { r1: { number: "G01", name: "ENTRY" }, r2: { number: "G02", name: "KITCHEN", link_value: "1042" } },
      },
    },
  };

  it("names a room by number and name, not by its id", () => {
    const rows = visibleRows(check("reference"), data, defaults("reference"));
    expect(rows[0]?.cells.Room).toBe("G01 ENTRY");
  });

  // One row per (room, finding) pair, so a duplicated key lists both rooms —
  // whoever fixes it has to visit two.
  it("lists every room sharing a duplicated key", () => {
    const rows = visibleRows(check("reference"), data, defaults("reference"));
    const dupes = rows.filter((r) => (r.cells.Finding ?? "").startsWith("duplicate link value"));
    expect(dupes).toHaveLength(2);
  });

  // The reference side is a finding too: a record no room reached is exactly
  // half of what this check exists to surface.
  it("reports a reference row that matched no room", () => {
    const rows = visibleRows(check("reference"), data, defaults("reference"));
    expect(rows.some((r) => r.cells["Link value"] === "1099")).toBe(true);
  });

  // The phase disagreement has no source and fills no room column, and it is
  // still listed here rather than split into a second report nobody opens.
  it("carries the phase disagreement, and drops it when the option is off", () => {
    const phased: ValidationResponse = {
      ...empty,
      phases: { disagree: true, by_model: { "m1": "New Construction" } },
    };
    expect(visibleRows(check("reference"), phased, { phase: true })).toHaveLength(1);
    expect(visibleRows(check("reference"), phased, { phase: false })).toHaveLength(0);
  });
});

describe("the openings check", () => {
  const data: ValidationResponse = {
    ...empty,
    openings: {
      doors: {
        total: 3,
        unresolved_room: [{ model_id: "m1", opening_id: "d7", side: "to", room_id: "r-999" }],
        pending_rooms: [{ model_id: "m2", opening_id: "d8" }],
        without_room_reference: [{ model_id: "m3", opening_id: "d9" }],
      },
    },
  };

  // The distinction the ingest gate could not make: dangling is a finding,
  // pending is a state that resolves when the rooms land.
  it("shows the dangling reference by default and hides the pending one", () => {
    const rows = visibleRows(check("openings"), data, defaults("openings"));
    expect(rows).toHaveLength(1);
    expect(rows[0]?.severity).toBe("finding");
    expect(rows[0]?.cells["Names room"]).toBe("r-999");
  });

  it("admits pending and homeless rows when asked, marked as expected", () => {
    const rows = visibleRows(check("openings"), data, { pending: true, homeless: true });
    expect(rows).toHaveLength(3);
    expect(rows.filter((r) => r.severity === "expected")).toHaveLength(2);
  });
});

describe("the spaces check", () => {
  const data: ValidationResponse = {
    ...empty,
    spaces: {
      models_not_audited: ["arch"],
      models_without_spaces: ["hydraulic"],
      total_spaces: 4,
      room_models: ["arch"],
      total_rooms: 6,
      by_model: [
        {
          model_id: "mech",
          total: 4,
          matched: 3,
          unmatched: [{ model_id: "mech", space_id: "s1", name: "PLANT 01", key: "P01" }],
          ambiguous_keys: [{ key: "G02", count: 2 }],
        },
      ],
      rooms_without_space: [{ model_id: "arch", room_id: "r4", name: "WC", key: "G04" }],
      ambiguous_room_keys: [],
    },
  };

  // Both directions by default: a project expecting 1:1 cares equally about
  // a room with no space, and suppressing it would hide half the answer.
  it("reports both an unmatched space and an unmatched room", () => {
    const rows = visibleRows(check("spaces"), data, defaults("spaces"));
    expect(rows.filter((r) => r.cells.Direction === "space")).toHaveLength(1);
    expect(rows.filter((r) => r.cells.Direction === "room")).toHaveLength(1);
  });

  // An ambiguous key is reported, never resolved: matching it would mean
  // guessing which candidate was meant.
  it("reports an ambiguous key as a warning rather than a match", () => {
    const rows = visibleRows(check("spaces"), data, defaults("spaces"));
    const ambiguous = rows.find((r) => r.cells.Direction === "key");
    expect(ambiguous?.severity).toBe("warning");
    expect(ambiguous?.cells.Finding).toContain("2 spaces share this key");
  });

  // Audited-and-empty is the finding the entity exists to report; never
  // audited is a different fact and is off by default.
  it("separates a model with no spaces from one never audited", () => {
    const onlyFindings = visibleRows(check("spaces"), data, defaults("spaces"));
    expect(onlyFindings.some((r) => r.cells.Model === "hydraulic")).toBe(true);
    expect(onlyFindings.some((r) => r.cells.Model === "arch" && r.cells.Direction === "model")).toBe(false);

    const withAll = visibleRows(check("spaces"), data, { rooms: true, notAudited: true });
    expect(withAll.some((r) => r.cells.Model === "arch" && r.cells.Direction === "model")).toBe(true);
  });

  it("reports nothing when the project has no spaces report at all", () => {
    expect(visibleRows(check("spaces"), empty, defaults("spaces"))).toEqual([]);
  });
});
