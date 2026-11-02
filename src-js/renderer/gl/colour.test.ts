import { describe, expect, it } from "vitest";
import { overrideOr, parseColour, withAlpha } from "./colour.js";

describe("parseColour", () => {
  it("parses 6-digit hex, which is what tokens.css authors", () => {
    expect(parseColour("#b4541f")).toEqual([180 / 255, 84 / 255, 31 / 255, 1]);
  });

  it("parses 3-digit hex", () => {
    expect(parseColour("#fff")).toEqual([1, 1, 1, 1]);
  });

  it("is case-insensitive and tolerates surrounding space", () => {
    // `getPropertyValue` returns the value as authored, whitespace included.
    expect(parseColour("  #B4541F  ")).toEqual(parseColour("#b4541f"));
  });

  it("parses rgb() and rgba(), which is what getComputedStyle returns", () => {
    // Custom properties come back as authored (hex); resolved properties come
    // back as rgb(). Both reach this function, so both are supported.
    expect(parseColour("rgb(255, 0, 128)")).toEqual([1, 0, 128 / 255, 1]);
    expect(parseColour("rgba(0, 255, 0, 0.5)")).toEqual([0, 1, 0, 0.5]);
  });

  it("falls back to opaque black rather than throwing on something unparseable", () => {
    // A colour this cannot read is a theming problem, not a reason to take the
    // renderer down -- a visibly wrong colour is diagnosable, a blank plan is not.
    expect(parseColour("oklch(0.7 0.1 200)")).toEqual([0, 0, 0, 1]);
  });
});

describe("withAlpha", () => {
  it("multiplies rather than replaces, so dim composes with a plan fill", () => {
    // `.dim` is opacity 0.15 layered ON TOP of whatever fill applies, which is
    // exactly why this multiplies.
    expect(withAlpha([1, 0.5, 0.25, 0.8], 0.5)).toEqual([1, 0.5, 0.25, 0.4]);
  });
});

describe("overrideOr", () => {
  const THEME = [0.1, 0.2, 0.3, 1] as const;

  it("falls back to the theme when nothing is set, which is every project by default", () => {
    // Absent is the ordinary state, not a missing value: a plan with every
    // colour pinned reads correctly in one theme and badly in the other.
    expect(overrideOr(undefined, THEME)).toBe(THEME);
    expect(overrideOr(null, THEME)).toBe(THEME);
    expect(overrideOr("", THEME)).toBe(THEME);
    expect(overrideOr("   ", THEME)).toBe(THEME);
  });

  it("uses the override when there is one", () => {
    expect(overrideOr("#0066ff", THEME)).toEqual([0, 102 / 255, 1, 1]);
    expect(overrideOr("  #fff  ", THEME)).toEqual([1, 1, 1, 1]);
    expect(overrideOr("rgb(255, 0, 0)", THEME)).toEqual([1, 0, 0, 1]);
  });

  it("falls back rather than blacking a layer out on an unusable value", () => {
    // THE reason this is not a bare `parseColour`. That answers opaque BLACK
    // for anything it cannot read, which is right for a computed style -- the
    // browser only hands back colours -- and wrong here: these strings come
    // from a project's TOML, which a person may edit by hand, and a typo would
    // black out a whole layer rather than being ignored.
    expect(overrideOr("not a colour", THEME)).toBe(THEME);
    expect(overrideOr("#12345", THEME)).toBe(THEME);
    expect(overrideOr("oklch(0.7 0.1 200)", THEME)).toBe(THEME);
  });
});
