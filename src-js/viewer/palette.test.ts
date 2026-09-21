import { describe, expect, it } from "vitest";

import { hexToRgb, qualitative, rgbToHex, SCHEMES, schemeStops } from "./palette.js";

describe("schemeStops", () => {
  it("falls back to RdBu for an unknown scheme rather than throwing", () => {
    expect(schemeStops("Blues")).toBe(SCHEMES["Blues"]);
    expect(schemeStops("no-such-scheme")).toBe(SCHEMES["RdBu"]);
  });
});

describe("qualitative", () => {
  it("indexes a categorical scheme and wraps", () => {
    const set2 = SCHEMES["Set2"]!;
    expect(qualitative("Set2", 0)).toBe(set2[0]);
    expect(qualitative("Set2", set2.length + 1)).toBe(set2[1]);
  });

  it("falls back to Set2, not RdBu, for an unknown scheme", () => {
    expect(qualitative("no-such-scheme", 0)).toBe(SCHEMES["Set2"]![0]);
  });
});

describe("hex round trip", () => {
  it("parses and formats, clamping and rounding channels", () => {
    expect(hexToRgb("#0571b0")).toEqual([5, 113, 176]);
    expect(rgbToHex(5, 113, 176)).toBe("#0571b0");
    expect(rgbToHex(-4, 127.6, 300)).toBe("#0080ff");
  });
});
