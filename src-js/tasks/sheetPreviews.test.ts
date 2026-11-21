import { describe, expect, it } from "vitest";

import { previewOf } from "../library/preview.js";
import { boxExtent, fitToRoom, sheetStatus, paperOf, taskSheetPreviews } from "./sheetPreviews.js";
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

describe("fitToRoom", () => {
  const sheet = {
    view_ports: [
      {
        vp_type: "floor plan",
        bounding_box: { min_x: 100, min_y: 100, max_x: 200, max_y: 160 },
        centre_point: { x: 150, y: 130 },
        view: { view_scale: 50, bounding_box: { min_x: 0, min_y: 0, max_x: 5000, max_y: 3000 } },
      },
      {
        vp_type: "elevation",
        view_id: 7,
        bounding_box: { min_x: 10, min_y: 10, max_x: 70, max_y: 40 },
        view: { view_scale: 50, line_start: { x: 0, y: 0 }, line_end: { x: 3000, y: 0 }, bounding_box: { min_x: 0, min_y: 450, max_x: 3000, max_y: 2850 } },
      },
      { vp_type: "schedule", bounding_box: { min_x: 1, min_y: 1, max_x: 2, max_y: 2 } },
    ],
  };
  const rls = {
    plan_crop: { min: [0, 0] as [number, number], max: [7500, 3000] as [number, number] },
    markers: [{ views: [{ view_id: 7, marker_index: 0, line_start: [0, 0] as [number, number], line_end: [4500, 0] as [number, number] }] }],
  };

  it("resizes the plan and the elevation about the template's centres and leaves the rest", () => {
    const [plan, elev, sched] = fitToRoom(sheet, rls).view_ports!;
    // 2500 mm more crop at 1:50 is 50 mm more paper, the height is unchanged
    expect(plan!.bounding_box).toEqual({ min_x: 75, max_x: 225, min_y: 100, max_y: 160 });
    expect(plan!.view!.bounding_box!.max_x).toBe(7500);
    // 1500 mm more line is 30 mm more paper
    expect(elev!.bounding_box).toEqual({ min_x: -5, max_x: 85, min_y: 10, max_y: 40 });
    // the crop is as wide as the new line, about its own middle, and as tall as before
    expect(elev!.view!.bounding_box).toEqual({ min_x: -750, max_x: 3750, min_y: 450, max_y: 2850 });
    expect(sched).toBe(sheet.view_ports[2]);
  });

  it("does not touch the template, and is the template when the task has no plan", () => {
    fitToRoom(sheet, rls);
    expect(sheet.view_ports[0]!.bounding_box.max_x).toBe(200);
    expect(fitToRoom(sheet, undefined)).toBe(sheet);
  });
});

describe("3D viewport", () => {
  it("measures a box as the camera sees it", () => {
    // looking along +y with z up: right is +x, so width is the box's x and height its z
    expect(boxExtent([0, 1, 0], [0, 0, 1], [4000, 3000, 2500])).toEqual([4000, 2500]);
    // looking along the diagonal and down: both plan sides and the height all show
    const [w, h] = boxExtent([1, 1, -0.5], [0.25, 0.25, 1], [1000, 1000, 1000]);
    expect(w).toBeCloseTo(1414.2, 0);
    expect(h).toBeGreaterThan(1000);
  });

  it("sizes the box from the task's footprint, the template's height and margin, at the plan's scale", () => {
    const sheet = {
      view_ports: [
        { vp_type: "floor plan", view: { view_scale: 50 } },
        {
          vp_type: "three d",
          bounding_box: { min_x: 0, min_y: 0, max_x: 86, max_y: 56 },
          view: {
            view_scale: 0,
            // 4000 x 2500 on the sheet at 1:50 is 80 x 50, plus a 6 and 6 margin
            bounding_box: { min_x: 0, min_y: 0, max_x: 4000, max_y: 2500 },
            forward_direction: { x: 0, y: 1, z: 0 },
            up_direction: { x: 0, y: 0, z: 1 },
            section_box_min: { x: 0, y: 0, z: 0 },
            section_box_max: { x: 4000, y: 3000, z: 2500 },
          },
        },
      ],
    };
    const rls = {
      plan_crop: { min: [0, 0] as [number, number], max: [6000, 3000] as [number, number] },
      three_ds: [{ index: 0, eye: [0, -3000] as [number, number], looks_at: [3000, 1500] as [number, number] }],
    };
    const b = fitToRoom(sheet, rls).view_ports![1]!.bounding_box!;
    // 6000 mm across at 1:50 is 120 mm, plus the margin; the height is the template's, and it stays where it was
    expect(b.max_x - b.min_x).toBeCloseTo(126, 6);
    expect(b.max_y - b.min_y).toBeCloseTo(56, 6);
    expect((b.min_x + b.max_x) / 2).toBeCloseTo(43, 6);
  });
});

describe("sheet status", () => {
  const sheetOf = (max_x: number) => ({
    revit_sheet_id: "1",
    document_title: "D",
    sheet_number: "S",
    size_mm: { width_mm: 420, height_mm: 297 },
    sheet: { view_ports: [{ vp_type: "floor plan", bounding_box: { min_x: 10, min_y: 10, max_x, max_y: 100 } }] },
  });

  it("counts the viewports past the paper the task asks for", () => {
    expect(sheetStatus(base({ sheets: [sheetOf(400)] }))).toEqual({ state: "ok", clipped: 0 });
    expect(sheetStatus(base({ sheets: [sheetOf(430)] }))).toEqual({ state: "clipping", clipped: 1 });
    // a smaller paper asked for clips what fitted the template's
    expect(sheetStatus(base({ rls: { sheet_size: "a3" }, sheets: [{ ...sheetOf(430), size_mm: { width_mm: 841, height_mm: 594 } }] })).state).toBe("clipping");
  });

  it("is unknown for a task with no sheets", () => {
    expect(sheetStatus(base({}))).toEqual({ state: "unknown", clipped: 0 });
  });
});
