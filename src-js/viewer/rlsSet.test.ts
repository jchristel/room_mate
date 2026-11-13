import { describe, expect, it } from "vitest";

import {
  addSheet,
  autoLayout,
  carryLayout,
  layoutFindings,
  leaveOut,
  MARGIN_MM,
  place,
  portsOf,
  putBack,
  removeSheet,
  saveBlocked,
  setTaskBody,
  sheetOf,
  sizeOfPaper,
  unplace,
  unplaced,
  type Port,
  type SetPreview,
} from "./rlsSet.js";

const elev = (room: string, i: number) => ({ room_id: room, marker_index: i, scale: 25, paper_mm: [120, 108] as [number, number], height_mm: 2700 });
const preview: SetPreview = {
  valid: true,
  rooms: [
    { room_id: "wc", model_id: "m", name: "WC", number: "00.03", area_m2: 4, height_mm: 2400, height_from: "ceiling" },
    { room_id: "office", model_id: "m", name: "OFFICE", number: "00.02", area_m2: 12, height_mm: 2400, height_from: "ceiling" },
  ],
  lead_room: "office",
  suggestions: [],
  views: {
    plan: { scale: 50, paper_mm: [99, 144] },
    elevations: [0, 2, 4, 6].flatMap((i) => [elev("wc", i), elev("office", i)]),
    schedules: [
      { room_id: "wc", paper_mm: [133, 65], estimate: true },
      { room_id: "office", paper_mm: [133, 65], estimate: true },
    ],
    three_d: { paper_mm: [150, 150], estimate: true },
  },
  findings: [],
};

const box = (id: string, w: number, h: number): Port => ({ id, kind: "elevation", label: id, w, h, estimate: false });

describe("the set composer", () => {
  it("lists the plan, the 3D view, then each room's views with the lead room first", () => {
    const ids = portsOf(preview).map((p) => p.id);
    expect(ids.slice(0, 3)).toEqual(["plan", "three_d", "elevation:office:0"]);
    expect(ids.indexOf("schedule:office")).toBeLessThan(ids.indexOf("elevation:wc:0"));
    expect(ids).toHaveLength(1 + 1 + 8 + 2);
    expect(portsOf(preview).find((p) => p.id === "elevation:wc:2")?.label).toBe("WC 00.03 N 1:25");
    const named = { ...preview, rooms: preview.rooms.map((r) => ({ ...r, name: `${r.name} ${r.number}` })) };
    expect(portsOf(named).find((p) => p.id === "schedule:office")?.label).toBe("OFFICE 00.02 schedule");
  });

  it("has nothing to place for a set the server refused", () => {
    const { views: _views, ...refused } = preview;
    expect(portsOf({ ...refused, valid: false })).toEqual([]);
  });

  it("lays views out in rows inside the margin, and starts a sheet when one is full", () => {
    const ports = [box("a", 200, 100), box("b", 200, 100), box("c", 200, 100)];
    const a3 = autoLayout(ports, "a3");
    // A3 is 420 wide: two 200 mm views do not fit in one row with the margins and a gap, so each row holds one.
    expect(a3.sheets[0]?.[0]).toEqual({ port: "a", x: MARGIN_MM, y: MARGIN_MM });
    expect(a3.sheets[0]?.[1]).toEqual({ port: "b", x: MARGIN_MM, y: MARGIN_MM + 100 + 10 });
    expect(a3.sheets).toHaveLength(2);
    expect(layoutFindings(a3, ports, "a3")).toEqual([]);
    expect(autoLayout(ports, "a1").sheets).toHaveLength(1);
  });

  it("gives a view too big for the paper a sheet of its own, and says it does not fit", () => {
    const ports = [box("small", 50, 50), box("huge", 900, 100)];
    const l = autoLayout(ports, "a3");
    expect(l.sheets.map((s) => s.map((p) => p.port))).toEqual([["small"], ["huge"]]);
    expect(layoutFindings(l, ports, "a3").map((f) => f.code)).toEqual(["too_big"]);
  });

  it("says a view is too big for the paper, how big, and the smallest paper it fits, wherever it is", () => {
    const plan: Port = { id: "plan", kind: "plan", label: "Plan 1:50", w: 700, h: 480, estimate: false };
    const placed = autoLayout([plan], "a1");
    const [f] = layoutFindings(placed, [plan], "a2");
    expect(f?.code).toBe("too_big");
    expect(f?.serious).toBe(true);
    expect(f?.message).toBe(
      "Plan 1:50 is too big for the sheet: it is 700 x 480 mm, but an A2 sheet holds 564 x 390 mm inside its border. It fits on A1: choose that paper size, or a library sheet with a smaller scale.",
    );
    // Not again as running past the edge, and said while it waits in the list too.
    expect(layoutFindings(placed, [plan], "a2").map((x) => x.code)).toEqual(["too_big"]);
    expect(layoutFindings(unplace(placed, "plan"), [plan], "a2").map((x) => x.code)).toEqual(["too_big", "not_placed", "empty_sheet"]);
    const huge = { ...plan, w: 1500 };
    expect(layoutFindings(autoLayout([huge], "a0"), [huge], "a0")[0]?.message).toContain("No paper size up to A0 holds it");
    // Left out, it is not made, so it is not too big for anything.
    expect(layoutFindings(leaveOut(placed, "plan"), [plan], "a2").map((x) => x.code)).toEqual(["empty_sheet"]);
  });

  it("keeps a view on one sheet only, and moving it puts back a view that was left out", () => {
    const ports = [box("a", 50, 50), box("b", 50, 50)];
    let l = addSheet(autoLayout(ports, "a3"));
    l = place(l, "a", 1, 20.4, 30.6);
    expect(sheetOf(l, "a")).toBe(1);
    expect(l.sheets[0]?.some((p) => p.port === "a")).toBe(false);
    expect(l.sheets[1]).toEqual([{ port: "a", x: 20, y: 31 }]);
    l = leaveOut(l, "b");
    expect(sheetOf(l, "b")).toBe(-1);
    expect(unplaced(l, ports)).toEqual([]);
    expect(unplaced(putBack(l, "b"), ports).map((p) => p.id)).toEqual(["b"]);
    expect(place(l, "b", 0, 20, 20).leftOut).toEqual([]);
  });

  it("removes only an empty sheet, and never the last", () => {
    const ports = [box("a", 50, 50)];
    const l = addSheet(autoLayout(ports, "a3"));
    expect(removeSheet(l, 0)).toBe(l);
    expect(removeSheet(l, 1).sheets).toHaveLength(1);
    expect(removeSheet(autoLayout([], "a3"), 0).sheets).toHaveLength(1);
  });

  it("reports views waiting, overlaps and empty sheets, and a smaller paper size without moving anything", () => {
    const ports = [box("a", 100, 100), box("b", 100, 100), box("c", 10, 10)];
    let l = autoLayout(ports, "a1");
    l = place(l, "b", 0, MARGIN_MM + 50, MARGIN_MM + 50);
    l = addSheet(place(l, "c", 0, 300, 300));
    l = { ...l, sheets: l.sheets.map((s) => s.filter((p) => p.port !== "c")) };
    expect(layoutFindings(l, ports, "a1").map((f) => f.code)).toEqual(["not_placed", "overlap", "empty_sheet"]);
    const big = place(autoLayout([box("a", 100, 100)], "a1"), "a", 0, 400, 300);
    expect(layoutFindings(big, [box("a", 100, 100)], "a3").map((f) => f.code)).toEqual(["off_sheet"]);
  });

  it("keeps a person's layout over a new preview of the same views, and lays out again when the views change", () => {
    const ports = portsOf(preview);
    const moved = place(autoLayout(ports, "a1"), "plan", 0, 300, 200);
    expect(carryLayout(moved, ports, "a1")).toBe(moved);
    expect(carryLayout(moved, ports.slice(1), "a1")).not.toBe(moved);
  });

  it("knows a template's paper by its measured size, either way up", () => {
    expect(sizeOfPaper({ width_mm: 841.2, height_mm: 594 })).toBe("a1");
    expect(sizeOfPaper({ width_mm: 297, height_mm: 420 })).toBe("a3");
    expect(sizeOfPaper({ width_mm: 500, height_mm: 400 })).toBeNull();
    expect(sizeOfPaper(undefined)).toBeNull();
  });

  it("sends where each view is and what is left out, and is held back only by what the server would refuse", () => {
    const ports = [box("a", 50, 50), box("b", 50, 50), box("c", 10, 10)];
    let l = leaveOut(autoLayout(ports, "a3"), "c");
    const body = setTaskBody(["r1", "r2"], "sheet", "a3", true, l);
    expect(body).toEqual({
      kind: "create_rls_set",
      room_ids: ["r1", "r2"],
      sheet_id: "sheet",
      sheet_size: "a3",
      overwrite_rls: true,
      sheets: [{ views: [{ view: "a", x_mm: MARGIN_MM, y_mm: MARGIN_MM }, { view: "b", x_mm: MARGIN_MM + 60, y_mm: MARGIN_MM }] }],
      left_out: ["c"],
    });
    expect(saveBlocked(layoutFindings(l, ports, "a3"))).toBeNull();
    // an overlap and a view past the edge are advice; a view on no sheet and an empty sheet are not
    expect(saveBlocked(layoutFindings(place(l, "b", 0, 30, 30), ports, "a3"))).toBeNull();
    expect(saveBlocked(layoutFindings(putBack(l, "c"), ports, "a3"))).toContain("not on a sheet");
    l = addSheet(l);
    expect(saveBlocked(layoutFindings(l, ports, "a3"))).toContain("Sheet 2 is empty");
  });
});
