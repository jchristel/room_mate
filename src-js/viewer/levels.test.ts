import { describe, expect, it } from "vitest";

import { levelLabel, levelsForPayload, pickerOrder, resolveLevel, roomsOnLevel } from "./levels.js";
import type { Level, Room } from "../renderer/types.js";

const level = (id: string, name: string, elevation: number): Level => ({ id, name, elevation });
const room = (id: string, levelId?: string): Room => ({ id, ...(levelId ? { level_id: levelId } : {}) });

describe("levelsForPayload", () => {
  it("returns declared levels, lowest first", () => {
    const payload = { levels: [level("2", "L2", 8), level("1", "L1", 0)] };
    expect(levelsForPayload(payload).map((l) => l.id)).toEqual(["1", "2"]);
  });

  /** An older snapshot declares no levels. The picker still has to work, so
   *  the rooms' own ids become the levels rather than the page showing none. */
  it("falls back to the rooms' own level ids", () => {
    const payload = { rooms: [room("a", "L1"), room("b", "L1"), room("c", "L2")] };
    expect(levelsForPayload(payload).map((l) => l.id)).toEqual(["L1", "L2"]);
  });

  it("files a room with no level under one bucket rather than dropping it", () => {
    const payload = { rooms: [room("a"), room("b")] };
    expect(levelsForPayload(payload).map((l) => l.id)).toEqual(["unknown"]);
    expect(roomsOnLevel(payload, "unknown")).toHaveLength(2);
  });
});

describe("roomsOnLevel", () => {
  const payload = { levels: [level("1", "L1", 0)], rooms: [room("a", "1"), room("b", "2")] };

  it("filters by level", () => {
    expect(roomsOnLevel(payload, "1").map((r) => r.id)).toEqual(["a"]);
  });

  it("is empty for no level, rather than every room", () => {
    expect(roomsOnLevel(payload, null)).toEqual([]);
  });
});

describe("pickerOrder", () => {
  it("is highest first, the way a stack of floors is read", () => {
    const levels = [level("1", "L1", 0), level("2", "L2", 8), level("0", "B1", -4)];
    expect(pickerOrder(levels).map((l) => l.id)).toEqual(["2", "1", "0"]);
  });

  it("does not reorder its argument", () => {
    const levels = [level("1", "L1", 0), level("2", "L2", 8)];
    pickerOrder(levels);
    expect(levels.map((l) => l.id)).toEqual(["1", "2"]);
  });
});

describe("levelLabel", () => {
  it("names the level and counts its rooms", () => {
    const payload = { levels: [level("1", "LEVEL 01", 0)], rooms: [room("a", "1")] };
    expect(levelLabel(payload, level("1", "LEVEL 01", 0))).toBe("LEVEL 01 (1 rm)");
  });

  /** An empty level is ordinary — House A's LEVEL 02 — and the count is how a
   *  reader tells it apart from one that failed to draw. */
  it("says zero for an empty level", () => {
    expect(levelLabel({ levels: [], rooms: [] }, level("9", "LEVEL 02", 8))).toBe("LEVEL 02 (0 rm)");
  });
});

describe("resolveLevel", () => {
  const levels = [level("1", "L1", 0), level("2", "L2", 8)];

  it("keeps the level the zone is showing across a push", () => {
    expect(resolveLevel(levels, "2")).toBe("2");
  });

  it("takes the lowest when the current level is gone", () => {
    expect(resolveLevel(levels, "gone")).toBe("1");
  });

  it("is null when the payload has no levels", () => {
    expect(resolveLevel([], "1")).toBeNull();
  });
});
