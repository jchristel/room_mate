import { describe, expect, it } from "vitest";

import type { PaperBox } from "./types.js";
import { cameraArrows, kindOf, previewOf } from "./preview.js";

const port = (vp_type: string, [x0, y0, x1, y1]: number[], scale = 50) => ({
  vp_type,
  bounding_box: { min_x: x0!, min_y: y0!, max_x: x1!, max_y: y1! },
  view: { view_scale: scale },
});

describe("sheet preview", () => {
  it("names the viewport types duHast exports, including 3D", () => {
    expect(kindOf("floor plan")).toBe("plan");
    expect(kindOf("three d")).toBe("three_d");
    expect(kindOf("Elevation")).toBe("elevation");
    expect(kindOf("section")).toBe("other");
    expect(kindOf(undefined)).toBe("other");
  });

  it("draws the measured sheet with y flipped so the sheet's bottom is the page's bottom", () => {
    const p = previewOf({ size_mm: { width_mm: 841, height_mm: 594 }, sheet: { view_ports: [port("floor plan", [10, 20, 110, 220])] } })!;
    expect(p.measured).toBe(true);
    expect([p.width, p.height]).toEqual([841, 594]);
    expect(p.boxes[0]).toMatchObject({ x: 10, y: 594 - 220, w: 100, h: 200, kind: "plan", label: "Plan 1:50" });
  });

  it("falls back to the viewports' extent when the size was not measured", () => {
    const p = previewOf({ sheet: { view_ports: [port("schedule", [0, 0, 300, 100], 0)] } })!;
    expect(p.measured).toBe(false);
    expect([p.width, p.height]).toEqual([320, 120]);
    expect(p.boxes[0]!.label).toBe("Schedule");
  });

  it("has nothing to draw without a size or an outline", () => {
    expect(previewOf({ sheet: { view_ports: [{ vp_type: "floor plan" }] } })).toBeNull();
  });

  it("counts the items of a schedule and names the index an elevation has on its marker", () => {
    const sched = { ...port("schedule", [0, 0, 100, 50]), view: { view_scale: 0, total_number_of_rows: 12 } };
    const elev = { ...port("elevation", [0, 60, 100, 160]), view: { view_scale: 50, marker_index: 2 } };
    const p = previewOf({ sheet: { view_ports: [sched, elev] } })!;
    expect(p.boxes[0]!.label).toBe("Schedule · 12 items");
    expect(p.boxes[1]!.label).toBe("Elevation 1:50 · #2");
  });

  it("draws each elevation's line on the plan, scaled from the plan's crop to its viewport", () => {
    const plan = {
      ...port("floor plan", [0, 0, 100, 100]),
      centre_point: { x: 50, y: 50 },
      view: { view_scale: 50, bounding_box: { min_x: 0, min_y: 0, max_x: 5000, max_y: 5000 } },
    };
    const elev = {
      ...port("elevation", [200, 0, 300, 100]),
      view: { view_scale: 50, marker_index: 1, line_start: { x: 0, y: 0 }, line_end: { x: 5000, y: 0 } },
    };
    const p = previewOf({ size_mm: { width_mm: 400, height_mm: 300 }, sheet: { view_ports: [plan, elev] } })!;
    // the crop's centre (2500, 2500) is the viewport's centre (50, 50); 2500 mm is 50 mm on paper
    expect(p.lines).toEqual([{ x1: 0, y1: 300 - 0, x2: 100, y2: 300 - 0, label: "1" }]);
  });

  it("draws no lines when the plan has no crop or no elevation has a line", () => {
    const plan = { ...port("floor plan", [0, 0, 100, 100]), view: { view_scale: 50 } };
    expect(previewOf({ size_mm: { width_mm: 400, height_mm: 300 }, sheet: { view_ports: [plan] } })!.lines).toEqual([]);
  });

  it("draws each 3D view's camera on the plan: an arrow along its direction ending at its section box's middle", () => {
    const plan = {
      ...port("floor plan", [0, 0, 100, 100]),
      centre_point: { x: 50, y: 50 },
      view: { view_scale: 50, bounding_box: { min_x: 0, min_y: 0, max_x: 5000, max_y: 5000 } },
    };
    const three = {
      ...port("three d", [200, 0, 300, 100], 0),
      view: {
        view_scale: 0,
        forward_direction: { x: 0, y: 203.2, z: -101.6 },
        section_box_min: { x: 0, y: 0, z: -245 },
        section_box_max: { x: 5000, y: 2000, z: 3000 },
      },
    };
    const [a] = cameraArrows([plan, three]);
    // the box's middle (2500, 1000) is (50, 20) on paper; half its diagonal (2693 mm) back is 53.9 mm south
    expect(a!.head[0]).toEqual({ x: 50, y: 20 });
    expect(a!.x1).toBeCloseTo(50);
    expect(a!.y1).toBeCloseTo(20 - Math.hypot(5000, 2000) / 2 / 50);
    expect(a!.label).toBe("3D");
    const p = previewOf({ size_mm: { width_mm: 400, height_mm: 300 }, sheet: { view_ports: [plan, three] } })!;
    expect(p.arrows[0]!.head[0]).toEqual({ x: 50, y: 280 });
  });

  it("draws no camera for a 3D view looking straight down or without a section box", () => {
    const plan = { ...port("floor plan", [0, 0, 100, 100]), centre_point: { x: 50, y: 50 }, view: { view_scale: 50, bounding_box: { min_x: 0, min_y: 0, max_x: 5000, max_y: 5000 } } };
    const down = { ...port("three d", [0, 0, 1, 1]), view: { forward_direction: { x: 0, y: 0, z: -1 }, section_box_min: { x: 0, y: 0, z: 0 }, section_box_max: { x: 1, y: 1, z: 1 } } };
    const bare = { ...port("three d", [0, 0, 1, 1]), view: { forward_direction: { x: 1, y: 0, z: 0 } } };
    expect(cameraArrows([plan, down, bare])).toEqual([]);
  });
});

describe("crop rectangles", () => {
  // the crop is 4000 x 2000 mm at 1:50: 80 x 40 on paper, in a box 100 x 60 centred at (60, 50)
  const port = (view: object) => ({
    vp_type: "floor plan",
    bounding_box: { min_x: 10, min_y: 20, max_x: 110, max_y: 80 },
    view: { view_scale: 50, bounding_box: { min_x: 0, min_y: 0, max_x: 4000, max_y: 2000 }, ...view },
  });
  const sheet = (view: object) => ({ size_mm: { width_mm: 200, height_mm: 100 }, sheet: { view_ports: [port(view)] } });

  it("centres the crop in the box at its own size and pushes the annotation crop out by each side's offset", () => {
    const box = previewOf(sheet({ annotation_crop_active: true, annotation_crop_offsets: { left: 5, right: 10, top: 4, bottom: 6 } }))!.boxes[0]!;
    // y up: crop x 20..100, y 30..70; flipped on a 100 mm sheet the top is 30 and the height 40
    expect(box.crop).toEqual({ x: 20, y: 30, w: 80, h: 40 });
    // 5 left, 10 right, 4 above, 6 below: x 15..110, y up 24..74, flipped top 26, height 50
    expect(box.annotation).toEqual({ x: 15, y: 26, w: 95, h: 50 });
  });

  it("draws none when the annotation crop is off or unread, or the crop's size is unknown", () => {
    const offsets = { left: 5, right: 5, top: 5, bottom: 5 };
    expect(previewOf(sheet({ annotation_crop_active: false, annotation_crop_offsets: offsets }))!.boxes[0]!.crop).toBeUndefined();
    expect(previewOf(sheet({}))!.boxes[0]!.annotation).toBeUndefined();
    expect(previewOf(sheet({ annotation_crop_active: true, annotation_crop_offsets: offsets, view_scale: undefined }))!.boxes[0]!.crop).toBeUndefined();
  });
});

describe("clipping", () => {
  const sheet = (box: PaperBox, size?: { width_mm: number; height_mm: number }) => ({
    ...(size && { size_mm: size }),
    sheet: { view_ports: [{ vp_type: "floor plan", bounding_box: box }] },
  });
  const paper = { width_mm: 200, height_mm: 100 };

  it("flags a box past any edge of a measured sheet, with a little tolerance", () => {
    const flag = (b: PaperBox) => previewOf(sheet(b, paper))!.boxes[0]!.clipped;
    expect(flag({ min_x: 10, min_y: 10, max_x: 190, max_y: 90 })).toBeUndefined();
    expect(flag({ min_x: -0.4, min_y: 0, max_x: 200.4, max_y: 100.4 })).toBeUndefined();
    expect(flag({ min_x: -1, min_y: 10, max_x: 190, max_y: 90 })).toBe(true);
    expect(flag({ min_x: 10, min_y: 10, max_x: 201, max_y: 90 })).toBe(true);
    expect(flag({ min_x: 10, min_y: 10, max_x: 190, max_y: 101 })).toBe(true);
    expect(flag({ min_x: 10, min_y: -2, max_x: 190, max_y: 90 })).toBe(true);
  });

  it("says nothing when the paper was not measured: the outline is drawn round the viewports", () => {
    expect(previewOf(sheet({ min_x: -5, min_y: 0, max_x: 50, max_y: 50 }))!.boxes[0]!.clipped).toBeUndefined();
  });
});

describe("box ports", () => {
  it("says which viewport each box draws, counting those with no outline", () => {
    const box = { min_x: 0, min_y: 0, max_x: 10, max_y: 10 };
    const preview = previewOf({
      size_mm: { width_mm: 100, height_mm: 100 },
      sheet: { view_ports: [{ vp_type: "floor plan" }, { vp_type: "elevation", bounding_box: box }, { vp_type: "schedule", bounding_box: box }] },
    })!;
    expect(preview.boxes.map((b) => b.port)).toEqual([1, 2]);
  });
});

