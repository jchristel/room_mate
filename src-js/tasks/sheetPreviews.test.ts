import { describe, expect, it } from "vitest";

import { previewOf } from "../library/preview.js";
import { paperOf, taskSheetPreviews } from "./sheetPreviews.js";
import type { Task } from "./types.js";

const base = (over: Partial<Task>): Task =>
  ({
    id: "t",
    kind: "create_rls",
    dry_run: false,
    state: "queued",
    model_id: "m",
    room: { room_id: "1", name: "R" },
    phase: "P",
    overwrite_layout: false,
    create_rls: true,
    overwrite_rls: false,
    created_at: "",
    updated_at: "",
    items: [],
    sheets: [],
    tag_rules: [],
    findings: [],
    attempts: [],
    ...over,
  }) as Task;

describe("task sheet previews", () => {
  it("draws a create-RLS task's template on the paper the task asks for", () => {
    const task = base({
      rls: { sheet_size: "a3" },
      sheets: [
        {
          revit_sheet_id: "1",
          document_title: "D",
          sheet_number: "50H",
          sheet_name: "STORE",
          size_mm: { width_mm: 841, height_mm: 594 },
          sheet: { view_ports: [{ vp_type: "floor plan", bounding_box: { min_x: 10, min_y: 10, max_x: 110, max_y: 90 } }] },
        },
      ],
    });
    const only = taskSheetPreviews(task)[0]!;
    expect(only.title).toBe("50H STORE on A3");
    expect(only.record.size_mm).toEqual({ width_mm: 420, height_mm: 297 });
    expect(previewOf(only.record)?.boxes).toHaveLength(1);
  });

  it("keeps the template's own size when the task names none", () => {
    const sheet = { revit_sheet_id: "1", document_title: "D", sheet_number: "S", size_mm: { width_mm: 594, height_mm: 420 }, sheet: {} };
    expect(taskSheetPreviews(base({ sheets: [sheet] }))[0]!.record.size_mm).toEqual({ width_mm: 594, height_mm: 420 });
  });

  it("draws every sheet of a set from its own layout, y up from the bottom left", () => {
    const task = base({
      kind: "create_rls_set",
      sheets: [{ revit_sheet_id: "1", document_title: "D", sheet_number: "TEMPLATE", sheet: {} }],
      rls_set: {
        sheet_size: "a1",
        paper_mm: [841, 594],
        lead_room: "1",
        rooms: [],
        plan: { scale: 50 },
        markers: [{ room_id: "1", views: [{ marker_index: 2, scale: 25 }] }],
        sheets: [
          { views: [{ kind: "plan", centre_mm: [100, 500], size_mm: [100, 80] }] },
          { views: [{ kind: "elevation", room_id: "1", marker_index: 2, centre_mm: [60, 40], size_mm: [80, 40] }] },
        ],
      },
    });
    const drawn = taskSheetPreviews(task);
    expect(drawn.map((d) => d.title)).toEqual(["Sheet 1 of 2 (A1)", "Sheet 2 of 2 (A1)"]);
    const plan = previewOf(drawn[0]!.record)!.boxes[0];
    // SVG y runs down: a box 460..540 mm up the paper starts 594 - 540 = 54 mm from the top
    expect(plan).toMatchObject({ x: 50, y: 54, w: 100, h: 80, kind: "plan", label: "Plan 1:50" });
    expect(previewOf(drawn[1]!.record)!.boxes[0]!.label).toBe("Elevation 1:25 · #2");
  });

  it("knows the A sizes and nothing else", () => {
    expect(paperOf("A2")).toEqual({ width_mm: 594, height_mm: 420 });
    expect(paperOf("b1")).toBeUndefined();
  });
});
