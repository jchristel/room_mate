import { describe, expect, it } from "vitest";

import { batches } from "./batch.js";

describe("batches", () => {
  const items = Array.from({ length: 7 }, (_, i) => ({ n: i }));
  it("closes a group at the count", () => {
    expect(batches(items, 3).map((g) => g.length)).toEqual([3, 3, 1]);
  });
  it("closes a group at the size, and sends an oversized sheet alone", () => {
    const big = { text: "x".repeat(600) };
    expect(batches([big, big, { n: 1 }], 50, 500).map((g) => g.length)).toEqual([1, 1, 1]);
    expect(batches([{ n: 1 }, { n: 2 }, big], 50, 500).map((g) => g.length)).toEqual([2, 1]);
  });
  it("sends nothing for nothing", () => {
    expect(batches([])).toEqual([]);
  });
});
