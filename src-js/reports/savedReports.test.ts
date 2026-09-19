import { describe, expect, it } from "vitest";

import { idFor, toForm, toSaved, typeIdFor } from "./savedReports.js";
import type { FormState } from "./reportRequest.js";
import type { SavedReport } from "./generated/SavedReport.js";

const form: FormState = {
  entityId: "ceilings",
  byRoom: true,
  columns: ["$type_name"],
  roomColumns: ["Number"],
  measures: ["overlap_area"],
  shape: "per_room",
  includeRoomsWithout: true,
  includeUnattributed: false,
  limit: 500,
};

describe("idFor", () => {
  it("makes a file-safe id from a name", () => {
    expect(idFor("Ceilings by room")).toBe("ceilings-by-room");
    expect(idFor("FF&E — level 00!")).toBe("ff-e-level-00");
  });

  // The server refuses an id that cannot be a file name, so a name that
  // reduces to nothing must not produce one.
  it("falls back rather than producing an empty id", () => {
    expect(idFor("!!!")).toMatch(/^report-\d+$/);
  });
});

describe("round trip", () => {
  // A saved report that loses a field on the way out or back looks like a
  // report somebody built wrong, which is why both directions are pinned.
  it("carries every part of the form through save and load", () => {
    const saved = toSaved("r1", "Ceilings by room", form, "DD issue", { mode: "all", items: [] });
    expect(saved).toMatchObject({
      id: "r1",
      name: "Ceilings by room",
      entity: "ceilings",
      by_room: true,
      columns: ["$type_name"],
      room_columns: ["Number"],
      measures: ["overlap_area"],
      shape: "per_room",
      include_rooms_without: true,
      include_unattributed: false,
      milestone: "DD issue",
      limit: 500,
    });

    expect(toForm(saved)).toEqual(form);
  });

  // An older document, written before a field existed, has to read as the
  // default the server would apply — or loading it and running it disagree.
  it("reads a document missing the newer fields as the server's defaults", () => {
    const old = {
      id: "r2",
      name: "Old one",
      entity: "doors",
      by_room: false,
      columns: ["$id"],
      room_columns: [],
      measures: [],
      shape: null,
      include_rooms_without: false,
      include_unattributed: true,
      building: null,
      milestone: null,
      limit: null,
      filter: null,
    } as unknown as SavedReport;

    expect(toForm(old)).toMatchObject({ shape: "per_match", limit: 500 });
  });
});

describe("typeIdFor", () => {
  it("points at the dropdown entry the saved report belongs to", () => {
    expect(typeIdFor({ entity: "ceilings", by_room: true })).toBe("by_room.ceilings");
    expect(typeIdFor({ entity: "rooms", by_room: false })).toBe("sched.rooms");
  });
});
