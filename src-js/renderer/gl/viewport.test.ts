import { describe, expect, it } from "vitest";
import { fitViewToAspect } from "./viewport.js";
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
