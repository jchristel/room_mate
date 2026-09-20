import { describe, expect, it } from "vitest";

import {
  areaKey,
  bandRows,
  buildAreasCsv,
  footprintPathD,
  groupsForOverlay,
  netAreaIndex,
  pathKey,
  roomNetArea,
  tierLabel,
  tierNames,
  type AreasData,
} from "./areas.js";
import type { ClassificationTier, Room } from "../renderer/types.js";

const tier = (t: string, name: string, undef = false): ClassificationTier => ({ tier: t, name, undefined: undef });

const square = (size: number) => ({
  points: [
    { x: 0, y: 0 },
    { x: size, y: 0 },
    { x: size, y: size },
    { x: 0, y: size },
  ],
});

const room = (id: string, level: string, cls: ClassificationTier[], size = 10): Room =>
  ({ id, level_id: level, classification: cls, loops: [square(size)] }) as Room;

const data: AreasData = {
  levels: [{ id: "L1", name: "LEVEL 01" }],
  groups: [
    {
      level_id: "L1",
      path: [tier("Building", "A")],
      area: 120,
      counted_upward: true,
      polygons: [{ exterior: [[0, 0], [10, 0], [10, 10], [0, 10]] }],
    },
    {
      level_id: "L1",
      path: [tier("Building", "A"), tier("Dept", "Medical")],
      area: 60,
      counted_upward: false,
      polygons: [{ exterior: [[0, 0], [5, 0], [5, 5], [0, 5]] }],
    },
  ],
};

describe("tier identity", () => {
  it("keys a path by its prefix, so a group and its rooms meet on one string", () => {
    const path = [tier("Building", "A"), tier("Dept", "Medical")];
    expect(pathKey(path, 0)).toBe("|A|");
    expect(pathKey(path, 1)).toBe("|A|>|Medical|");
  });

  /** An undefined tier is a REAL group — the areas service treats the
   *  undefined bucket as one — so it is named, never blank. */
  it("names an undefined tier", () => {
    expect(tierLabel(tier("Dept", "x", true))).toBe("(undefined)");
    expect(tierLabel(tier("Dept", "Medical"))).toBe("Medical");
    expect(tierLabel(undefined)).toBe("");
  });

  it("learns the tier names in depth order from the groups", () => {
    expect(tierNames(data)).toEqual(["Building", "Dept"]);
  });

  it("identifies a group by level and full path", () => {
    expect(areaKey(data.groups[1]!)).toBe("L1||A|>|Medical|");
  });
});

describe("roomNetArea", () => {
  it("subtracts holes from the outer loop", () => {
    const r = { id: "a", loops: [square(10), square(4)] } as Room;
    expect(roomNetArea(r)).toBe(100 - 16);
  });

  it("never goes negative, whatever the loops say", () => {
    const r = { id: "a", loops: [square(2), square(10)] } as Room;
    expect(roomNetArea(r)).toBe(0);
  });

  it("is zero with no geometry", () => {
    expect(roomNetArea({ id: "a" })).toBe(0);
  });
});

describe("netAreaIndex", () => {
  /** Every DEPTH, so a group at any tier looks its number up directly rather
   *  than re-summing per row. */
  it("sums a room into each prefix of its classification", () => {
    const rooms = [room("1", "L1", [tier("Building", "A"), tier("Dept", "Medical")])];
    const net = netAreaIndex(rooms);
    expect(net.get("L1|0||A|")).toBe(100);
    expect(net.get("L1|1||A|>|Medical|")).toBe(100);
  });

  it("ignores a room with no classification rather than bucketing it", () => {
    expect(netAreaIndex([room("1", "L1", [])]).size).toBe(0);
  });
});

describe("bandRows", () => {
  const rooms = [room("1", "L1", [tier("Building", "A")], 10)];

  it("reports footprint, net and their difference per group", () => {
    const rows = bandRows(data, rooms, 0).get("L1")!;
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({ levelName: "LEVEL 01", label: "A", footprint: 120, net: 100, delta: 20 });
  });

  it("carries the not-counted-up flag rather than dropping the group", () => {
    const rows = bandRows(data, rooms, 1).get("L1")!;
    expect(rows[0]!.countedUp).toBe(false);
  });
});

describe("groupsForOverlay", () => {
  it("takes this level at this tier only", () => {
    expect(groupsForOverlay(data, "L1", 0)).toHaveLength(1);
    expect(groupsForOverlay(data, "L2", 0)).toHaveLength(0);
    expect(groupsForOverlay(null, "L1", 0)).toHaveLength(0);
  });
});

describe("footprintPathD", () => {
  /** Y is flipped like every other drawn geometry, and a void is a second
   *  closed subpath so even-odd fill cuts it out. */
  it("flips Y and closes every ring", () => {
    const d = footprintPathD({ exterior: [[0, 0], [2, 0], [2, 2]], holes: [[[0, 0], [1, 0], [1, 1]]] });
    expect(d).toBe("M0,0L2,0L2,-2ZM0,0L1,0L1,-1Z");
  });

  it("ignores a degenerate hole", () => {
    const d = footprintPathD({ exterior: [[0, 0], [2, 0], [2, 2]], holes: [[[0, 0], [1, 1]]] });
    expect(d).toBe("M0,0L2,0L2,-2Z");
  });
});

describe("buildAreasCsv", () => {
  const rooms = [room("1", "L1", [tier("Building", "A")], 10)];

  it("writes every level and tier, not the view on screen", () => {
    const lines = buildAreasCsv(data, rooms).split("\r\n");
    expect(lines[0]).toBe("level,Building,Dept,footprint,net,delta,counted_up");
    expect(lines).toHaveLength(3);
    expect(lines[1]).toBe("LEVEL 01,A,,120,100,20,yes");
    expect(lines[2]).toBe("LEVEL 01,A,Medical,60,0,60,no");
  });

  it("is empty when there is nothing to export", () => {
    expect(buildAreasCsv(null, rooms)).toBe("");
  });
});
