import { describe, expect, it } from "vitest";
import { add, edit, encode, isAddition, nextId, remove, sameAdditions } from "./additions.js";

describe("room additions", () => {
  it("numbers them add-1, add-2 and never reuses a live id", () => {
    let l = add([], "7");
    l = add(l, "7");
    expect(l.map((a) => a.id)).toEqual(["add-1", "add-2"]);
    l = remove(l, "add-1");
    expect(nextId(l)).toBe("add-1");
    expect(isAddition("add-2")).toBe(true);
    expect(isAddition("2618110")).toBe(false);
  });

  it("edits one place in whole numbers and drops a zero facing", () => {
    let l = add([], "7");
    l = edit(l, "add-1", { along_mm: 120.6, facing_deg: 90 });
    expect(l[0]).toMatchObject({ along_mm: 121, off_mm: 400, facing_deg: 90 });
    l = edit(l, "add-1", { facing_deg: 0, edge: 2 });
    expect(l[0]?.facing_deg).toBeUndefined();
    expect(l[0]?.edge).toBe(2);
  });

  it("keeps the numbers in the range the server accepts", () => {
    const l = edit(add([], "7"), "add-1", { along_mm: 1e9, off_mm: Number.NaN });
    expect(l[0]?.along_mm).toBe(100_000);
    expect(l[0]?.off_mm).toBe(0);
  });

  it("compares lists whatever their order, a missing facing being none", () => {
    const a = [{ id: "add-1", clone_of: "7", edge: 0, along_mm: 1, off_mm: 2 }, { id: "add-2", clone_of: "7", edge: 1, along_mm: 1, off_mm: 2 }];
    expect(sameAdditions(a, [a[1]!, { ...a[0]!, facing_deg: 0 }])).toBe(true);
    expect(sameAdditions(a, [a[0]!])).toBe(false);
  });

  it("sends nothing to preview when there are none", () => {
    expect(encode([])).toBeUndefined();
    expect(encode(add([], "7"))).toContain("add-1");
  });
});
