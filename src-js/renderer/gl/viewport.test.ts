import { describe, expect, it } from "vitest";
import { fitViewToAspect, labelTransform } from "./viewport.js";
import type { Rect } from "../types.js";

/** World units per screen pixel, on each axis. Equal on both axes is the whole
 *  property under test — unequal is exactly what "stretched" means. */
function scales(view: Rect, w: number, h: number) {
  const eff = fitViewToAspect(view, w, h);
  return { sx: w / eff.w, sy: h / eff.h, eff };
}

const square: Rect = { x: 0, y: 0, w: 100, h: 100 };

describe("fitViewToAspect", () => {
  it("scales both axes identically on a wide canvas", () => {
    // The case that shipped broken: a 1280x240 zone stretched every room more
    // than 5x horizontally.
    const { sx, sy } = scales(square, 1280, 240);
    expect(sx).toBeCloseTo(sy, 9);
  });

  it("scales both axes identically on a tall canvas", () => {
    const { sx, sy } = scales(square, 400, 900);
    expect(sx).toBeCloseTo(sy, 9);
  });

  it("changes nothing when the view already matches the canvas", () => {
    expect(fitViewToAspect(square, 800, 800)).toEqual(square);
  });

  it("fits INSIDE rather than cropping -- `meet`, not `slice`", () => {
    // Every requested unit must still be visible; the effective rect can only
    // grow. `slice` would shrink one axis and cut the plan off.
    const eff = fitViewToAspect(square, 1280, 240);
    expect(eff.w).toBeGreaterThanOrEqual(square.w);
    expect(eff.h).toBeGreaterThanOrEqual(square.h);
  });

  it("keeps the view's centre centred -- xMidYMid", () => {
    const eff = fitViewToAspect(square, 1280, 240);
    expect(eff.x + eff.w / 2).toBeCloseTo(square.x + square.w / 2, 9);
    expect(eff.y + eff.h / 2).toBeCloseTo(square.y + square.h / 2, 9);
  });

  it("widens the unconstrained axis and leaves the constrained one alone", () => {
    // 1280x240 over a square view: height is the binding axis, so height is
    // untouched and width gains all the slack.
    const eff = fitViewToAspect(square, 1280, 240);
    expect(eff.h).toBeCloseTo(100, 9);
    expect(eff.w).toBeCloseTo(100 * (1280 / 240), 9);
  });

  it("preserves a non-square view's own proportions", () => {
    const tall: Rect = { x: 10, y: -50, w: 40, h: 120 };
    const { sx, sy } = scales(tall, 1000, 300);
    expect(sx).toBeCloseTo(sy, 9);
  });

  it("returns the view untouched for a canvas with no area", () => {
    // A zone that has not been laid out yet. Dividing by zero here would put
    // NaN into the projection uniform and blank the plan.
    expect(fitViewToAspect(square, 0, 0)).toEqual(square);
    expect(fitViewToAspect(square, 800, 0)).toEqual(square);
  });

  it("returns the view untouched for a degenerate view", () => {
    const degenerate: Rect = { x: 0, y: 0, w: 0, h: 0 };
    expect(fitViewToAspect(degenerate, 800, 600)).toEqual(degenerate);
  });
});

describe("labelTransform", () => {
  /** Where the label container puts a world point, in CSS pixels. */
  function place(view: Rect, cssW: number, wx: number, wy: number) {
    const t = labelTransform(view, cssW);
    return { x: t.x + wx * t.scale, y: t.y + wy * t.scale };
  }

  /** Where the PROJECTION puts the same point: `worldToNdc` mapped onto the
   *  canvas. The two must agree — a label that disagrees with the geometry is
   *  the whole bug this pins. */
  function projected(view: Rect, cssW: number, cssH: number, wx: number, wy: number) {
    return { x: ((wx - view.x) / view.w) * cssW, y: ((wy - view.y) / view.h) * cssH };
  }

  const view: Rect = { x: 10, y: -50, w: 200, h: 100 };
  // A canvas whose aspect matches the (already corrected) view, as it always
  // does by the time this is called.
  const cssW = 800;
  const cssH = 400;

  it("agrees with the projection about where a world point lands", () => {
    for (const [wx, wy] of [
      [10, -50],
      [110, 0],
      [210, 50],
    ] as const) {
      const p = place(view, cssW, wx, wy);
      const q = projected(view, cssW, cssH, wx, wy);
      expect(p.x).toBeCloseTo(q.x, 9);
      expect(p.y).toBeCloseTo(q.y, 9);
    }
  });

  it("puts the view's top-left corner on the canvas's", () => {
    const p = place(view, cssW, view.x, view.y);
    expect(p.x).toBeCloseTo(0, 9);
    expect(p.y).toBeCloseTo(0, 9);
  });

  it("centres the view's centre", () => {
    // THE REGRESSION. This held at DPR 1 and failed at every other DPR, because
    // the transform divided by the resolution a second time -- so a label sat
    // at 1/DPR of its correct distance from the corner. Nothing here can go
    // wrong that way now: there is no DPR to divide by.
    const p = place(view, cssW, view.x + view.w / 2, view.y + view.h / 2);
    expect(p.x).toBeCloseTo(cssW / 2, 9);
    expect(p.y).toBeCloseTo(cssH / 2, 9);
  });

  it("scales in CSS pixels per world unit", () => {
    expect(labelTransform(view, cssW).scale).toBeCloseTo(4, 9);
  });

  it("survives a degenerate view rather than emitting NaN", () => {
    // A zone that has not been laid out yet. NaN in a container transform
    // silently drops every label, which reads as "labels stopped working".
    const t = labelTransform({ x: 0, y: 0, w: 0, h: 0 }, 800);
    expect(Number.isFinite(t.scale)).toBe(true);
    expect(Number.isFinite(t.x)).toBe(true);
    expect(Number.isFinite(t.y)).toBe(true);
  });
});
