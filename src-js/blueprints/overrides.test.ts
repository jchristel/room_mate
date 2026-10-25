import { describe, expect, it } from "vitest";

import { adjust, describe as say, encode, overrideFor, reset, sameOverrides } from "./overrides.js";

describe("item overrides", () => {
  it("accumulate nudges and turns on one item", () => {
    let l = adjust([], "7", { dx_mm: 10 });
    l = adjust(l, "7", { dx_mm: 40, dy_mm: -5 });
    l = adjust(l, "7", { rotate_deg: 90 });
    expect(l).toEqual([{ item_id: "7", dx_mm: 50, dy_mm: -5, rotate_deg: 90 }]);
  });

  it("drop an override that has been nudged back to nothing", () => {
    const l = adjust(adjust([], "7", { dx_mm: 10 }), "7", { dx_mm: -10 });
    expect(l).toEqual([]);
  });

  it("keep one entry per item", () => {
    const l = adjust(adjust([], "1", { dx_mm: 1 }), "2", { dy_mm: 2 });
    expect(l.map((o) => o.item_id).sort()).toEqual(["1", "2"]);
    expect(adjust(l, "1", { dy_mm: 1 }).filter((o) => o.item_id === "1").length).toBe(1);
  });

  it("leave an item out, and put it back", () => {
    const out = adjust([], "7", { exclude: true });
    expect(overrideFor(out, "7")?.exclude).toBe(true);
    expect(adjust(out, "7", { exclude: false })).toEqual([]);
    expect(reset(out, "7")).toEqual([]);
  });

  it("never go past what the server keeps", () => {
    expect(adjust([], "7", { dx_mm: 99_999 })[0]?.dx_mm).toBe(20_000);
    expect(adjust([], "7", { rotate_deg: 900 })[0]?.rotate_deg).toBe(360);
  });

  it("compare by meaning, not order", () => {
    const a = [{ item_id: "1", dx_mm: 5 }, { item_id: "2", exclude: true }];
    const b = [{ item_id: "2", exclude: true }, { item_id: "1", dx_mm: 5, rotate_deg: 0 }];
    expect(sameOverrides(a, b)).toBe(true);
    expect(sameOverrides(a, [])).toBe(false);
  });

  it("encode nothing as absent and something as JSON without the zero fields", () => {
    expect(encode([])).toBeUndefined();
    expect(encode([{ item_id: "1", dx_mm: 5, dy_mm: 0 }])).toBe('[{"item_id":"1","dx_mm":5}]');
  });

  it("say what was done", () => {
    expect(say({ item_id: "1", exclude: true })).toBe("left out of this room");
    expect(say({ item_id: "1", dx_mm: 150, dy_mm: -50, rotate_deg: 90 })).toBe("moved 150, -50 mm, turned 90 degrees");
  });
});
