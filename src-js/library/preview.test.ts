import { describe, expect, it } from "vitest";

import { kindOf, previewOf } from "./preview.js";

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
});
