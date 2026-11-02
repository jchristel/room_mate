import { describe, expect, it } from "vitest";

import { viewCentredOn } from "./panTo.js";

const view = { x: 0, y: 0, w: 100, h: 100 };

describe("viewCentredOn", () => {
  it("does not move for a room wholly inside the view", () => {
    expect(viewCentredOn(view, { x: 10, y: 10, w: 20, h: 20 })).toBeNull();
  });

  it("does not move for a room exactly filling the view", () => {
    expect(viewCentredOn(view, { x: 0, y: 0, w: 100, h: 100 })).toBeNull();
  });

  it("centres a room outside the view, keeping the scale", () => {
    expect(viewCentredOn(view, { x: 200, y: 400, w: 20, h: 20 })).toEqual({ x: 160, y: 360, w: 100, h: 100 });
  });

  it("moves for a room only PARTLY inside, which is the common case", () => {
    // Crossing the right edge: visible, and not readable.
    expect(viewCentredOn(view, { x: 90, y: 40, w: 20, h: 20 })).toEqual({ x: 50, y: 0, w: 100, h: 100 });
  });

  it("does not move a room LARGER than the view once it is centred", () => {
    // It can never be wholly inside, so the containment test alone would
    // re-commit a view on every click. Centred is centred.
    const big = { x: -50, y: -50, w: 200, h: 200 };
    const first = viewCentredOn(view, big);
    expect(first).toBeNull();
    // ...and from somewhere else it still centres, once.
    const moved = viewCentredOn({ x: 500, y: 500, w: 100, h: 100 }, big);
    expect(moved).toEqual({ x: 0, y: 0, w: 100, h: 100 });
    expect(viewCentredOn(moved!, big)).toBeNull();
  });
});
