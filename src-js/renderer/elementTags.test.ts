import { describe, expect, it } from "vitest";
import { placeElementTags } from "./elementTags.js";
import { MARKER_SIZE } from "./gl/itemGlyph.js";
import type { Ceiling, Item, Space } from "./types.js";

const square = (x: number, y: number, s: number) => ({
  points: [
    { x, y },
    { x: x + s, y },
    { x: x + s, y: y + s },
    { x, y: y + s },
  ],
});
const req = (lines: string[]) => ({ text: () => lines });

describe("placeElementTags", () => {
  it("fits a space's tag inside its outline", () => {
    const space: Space = { id: "s", loops: [square(0, 0, 10)] };
    const [tag] = placeElementTags("spaces", [space], req(["AHU-01"]), 5);
    expect(tag).toBeDefined();
    // Y is flipped: the room spans y in [-10, 0].
    expect(tag!.x).toBeGreaterThan(0);
    expect(tag!.x).toBeLessThan(10);
    expect(tag!.y).toBeLessThan(0);
    expect(tag!.y).toBeGreaterThan(-10);
    expect(tag!.fontSize).toBeLessThanOrEqual(5);
  });

  it("shrinks the font for a small element, so the tag scales with the object", () => {
    const small: Space = { id: "a", loops: [square(0, 0, 1)] };
    const big: Space = { id: "b", loops: [square(0, 0, 20)] };
    const [a] = placeElementTags("spaces", [small], req(["WORDS"]), 5);
    const [b] = placeElementTags("spaces", [big], req(["WORDS"]), 5);
    expect(a!.fontSize).toBeLessThan(b!.fontSize);
  });

  it("puts a ceiling's tag in its largest piece", () => {
    const ceiling: Ceiling = {
      id: "c",
      polygons: [{ loops: [square(0, 0, 1)] }, { loops: [square(100, 0, 10)] }],
    };
    const [tag] = placeElementTags("ceilings", [ceiling], req(["T1"]), 5);
    expect(tag!.x).toBeGreaterThan(100);
  });

  it("gives an item with no footprint a marker-sized tag", () => {
    const item: Item = { id: "i", insertion_point: { x: 50, y: 20 } };
    const [tag] = placeElementTags("ffe", [item], req(["Chair"]), 5);
    expect(tag!.x).toBeCloseTo(50, 1);
    expect(tag!.fontSize).toBeLessThan(MARKER_SIZE);
  });

  it("draws nothing for an element with no text or no outline", () => {
    const space: Space = { id: "s", loops: [square(0, 0, 10)] };
    const noOutline: Space = { id: "n" };
    expect(placeElementTags("spaces", [space], req([]), 5)).toEqual([]);
    expect(placeElementTags("spaces", [noOutline], req(["x"]), 5)).toEqual([]);
  });
});
