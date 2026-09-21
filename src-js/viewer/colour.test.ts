import { describe, expect, it } from "vitest";

import {
  buildColourContext,
  classKey,
  colourForRoom,
  lighten,
  MATCH_COLOUR,
  MISMATCH_COLOUR,
  NO_DATA_COLOUR,
  FUTURE_COLOUR,
  parseDate,
  roomValue,
  sampleScheme,
  type ColourPlan,
} from "./colour.js";
import type { Room } from "../renderer/types.js";

/** The real stops, from `palette.ts`. RdBu has five, so t = 0.5 lands exactly
 *  on its middle stop and proves nothing about interpolation; the t = 0.125
 *  case sits half-way inside the first segment, and that is the one that pins
 *  the maths. */
const RDBU_FIRST = "#ca0020";
const RDBU_LAST = "#0571b0";
const SET2_0 = "#66c2a5";
const SET2_1 = "#fc8d62";

const room = (id: string, props: Record<string, string>): Room => ({
  id,
  properties: Object.fromEntries(Object.entries(props).map(([k, v]) => [k, { value: v, storage_type: "String" }])),
});

describe("roomValue", () => {
  it("reads a room property", () => {
    expect(roomValue(room("a", { Area: "12" }), "Area", [])).toBe("12");
  });

  /** Absent and blank collapse: a plan cannot act on an empty string any more
   *  than on a missing key, and the server's own lookup says the same. */
  it("treats blank as absent", () => {
    expect(roomValue(room("a", { Area: "" }), "Area", [])).toBeNull();
    expect(roomValue(room("a", {}), "Area", [])).toBeNull();
  });

  it("reads a joined reference field only when the prefix names a real source", () => {
    const r = { id: "a", schedule: { fields: { FireRating: "60" } } } as unknown as Room;
    expect(roomValue(r, "schedule.FireRating", ["schedule"])).toBe("60");
    // Not a known source: the whole name is a property, which is what keeps a
    // Revit property with a dot in it resolving as itself.
    expect(roomValue(r, "schedule.FireRating", [])).toBeNull();
  });
});

describe("sampleScheme and lighten", () => {
  it("interpolates between stops", () => {
    expect(sampleScheme("RdBu", 0)).toBe(RDBU_FIRST);
    expect(sampleScheme("RdBu", 1)).toBe(RDBU_LAST);
    expect(sampleScheme("RdBu", 0.5)).toBe("#f7f7f7");
    // Half-way from #ca0020 to #f4a582, channel by channel and rounded.
    expect(sampleScheme("RdBu", 0.125)).toBe("#df5351");
  });

  it("clamps out-of-range t rather than extrapolating a colour", () => {
    expect(sampleScheme("RdBu", -5)).toBe(RDBU_FIRST);
    expect(sampleScheme("RdBu", 5)).toBe(RDBU_LAST);
  });

  it("falls back to a default scheme rather than throwing", () => {
    expect(sampleScheme("no-such-scheme", 0)).toBe(RDBU_FIRST);
  });

  it("lightens toward white", () => {
    expect(lighten("#000000", 0)).toBe("#000000");
    expect(lighten("#000000", 1)).toBe("#ffffff");
  });
});

describe("classKey", () => {
  const r = {
    id: "a",
    classification: [
      { tier: "Department", code: "MED", name: "Medical" },
      { tier: "Sub", undefined: true, name: "x" },
    ],
  } as unknown as Room;

  it("prefers the code, then the name", () => {
    expect(classKey(r, "Department")).toBe("MED");
  });

  /** An explicitly undefined tier is a reported state, and a room in it is not
   *  in a group with the others that share the word. */
  it("is null for an undefined tier or an absent one", () => {
    expect(classKey(r, "Sub")).toBeNull();
    expect(classKey(r, "Missing")).toBeNull();
  });
});

describe("parseDate", () => {
  it("parses ISO without a format", () => {
    expect(parseDate("2026-09-20", null)).toBe(Date.UTC(2026, 8, 20));
  });

  it("parses an strftime pattern", () => {
    expect(parseDate("20/09/2026", "%d/%m/%Y")).toBe(Date.UTC(2026, 8, 20));
  });

  /** An unsupported specifier refuses rather than guessing: a date guessed
   *  wrong colours a room confidently and wrongly. */
  it("refuses an unsupported specifier", () => {
    expect(parseDate("whatever", "%Q")).toBeNull();
  });

  it("shifts an offset to UTC", () => {
    expect(parseDate("2026-09-20 10:00 +10:00", "%Y-%m-%d %H:%M %z")).toBe(Date.UTC(2026, 8, 20, 0, 0));
  });
});

describe("propertycompare", () => {
  const rooms = [room("a", { A: "10", B: "4" }), room("b", { A: "2", B: "4" }), room("c", { A: "x", B: "4" })];

  it("bands on a half-open [lo, hi) first match", () => {
    const plan: ColourPlan = {
      name: "p",
      mode: {
        kind: "propertycompare",
        property_a: "A",
        property_b: "B",
        colouring: {
          style: "bands",
          bands: [
            { lo: null, hi: 0, colour: "#low" },
            { lo: 0, hi: 10, colour: "#mid" },
            { lo: 10, hi: null, colour: "#high" },
          ],
        },
      },
    };
    const ctx = buildColourContext(rooms, plan);
    expect(colourForRoom(rooms[0]!, plan, ctx)).toBe("#mid"); // 6
    expect(colourForRoom(rooms[1]!, plan, ctx)).toBe("#low"); // -2
    expect(colourForRoom(rooms[2]!, plan, ctx)).toBe(NO_DATA_COLOUR); // unparseable
  });

  it("matches within a tolerance", () => {
    const plan: ColourPlan = {
      name: "p",
      mode: {
        kind: "propertycompare",
        property_a: "A",
        property_b: "B",
        colouring: { style: "match", tolerance: 3 },
      },
    };
    const ctx = buildColourContext(rooms, plan);
    expect(colourForRoom(rooms[0]!, plan, ctx)).toBe(MISMATCH_COLOUR); // 6
    expect(colourForRoom(rooms[1]!, plan, ctx)).toBe(MATCH_COLOUR); // -2
  });

  /** The diverging ramp is measured against the LEVEL's largest absolute
   *  difference, which is the whole reason the context exists. */
  it("centres a diverging ramp on the level's extent", () => {
    const plan: ColourPlan = {
      name: "p",
      mode: {
        kind: "propertycompare",
        property_a: "A",
        property_b: "B",
        colouring: { style: "diverging", scheme: "RdBu" },
      },
    };
    const ctx = buildColourContext(rooms, plan);
    expect(ctx.maxAbs).toBe(6);
    expect(colourForRoom(rooms[0]!, plan, ctx)).toBe(RDBU_LAST); // +6 -> t=1
    expect(colourForRoom(rooms[1]!, plan, ctx)).toBe("#f5c0a9"); // -2 -> t=1/3, inside the second segment
  });

  it("refuses a ratio by zero rather than colouring an infinity", () => {
    const plan: ColourPlan = {
      name: "p",
      mode: {
        kind: "propertycompare",
        property_a: "A",
        property_b: "B",
        op: "ratio",
        colouring: { style: "match", tolerance: 0 },
      },
    };
    const zeroed = [room("z", { A: "1", B: "0" })];
    expect(colourForRoom(zeroed[0]!, plan, buildColourContext(zeroed, plan))).toBe(NO_DATA_COLOUR);
  });
});

describe("hierarchy", () => {
  const withTiers = (id: string, dept: string, sub?: string): Room =>
    ({
      id,
      classification: [
        { tier: "Department", code: dept },
        ...(sub ? [{ tier: "Sub", code: sub }] : []),
      ],
    }) as unknown as Room;

  const plan: ColourPlan = { name: "h", mode: { kind: "hierarchy", tiers: ["Department", "Sub"], scheme: "Set2" } };

  it("gives each parent a distinct hue, in sorted order so it survives a repaint", () => {
    const rooms = [withTiers("b", "SURG"), withTiers("a", "MED")];
    const ctx = buildColourContext(rooms, plan);
    expect(ctx.parentHue?.get("MED")).toBe(SET2_0);
    expect(ctx.parentHue?.get("SURG")).toBe(SET2_1);
  });

  it("tints children of one parent so they read as siblings", () => {
    const rooms = [withTiers("a", "MED", "WARD"), withTiers("b", "MED", "THEATRE")];
    const ctx = buildColourContext(rooms, plan);
    const first = colourForRoom(rooms[0]!, plan, ctx);
    const second = colourForRoom(rooms[1]!, plan, ctx);
    expect(first).not.toBe(second);
    // Both lighter than or equal to the parent hue, never a different hue.
    expect(colourForRoom(withTiers("c", "MED"), plan, ctx)).toBe(SET2_0);
  });

  it("is the no-data colour for a room with no parent tier", () => {
    const ctx = buildColourContext([withTiers("a", "MED")], plan);
    expect(colourForRoom({ id: "x" }, plan, ctx)).toBe(NO_DATA_COLOUR);
  });
});

describe("daterange", () => {
  const plan: ColourPlan = {
    name: "d",
    mode: { kind: "daterange", property: "Date", near_date: "2026-09-20", scheme: "RdBu" },
  };
  const rooms = [
    room("near", { Date: "2026-09-20" }),
    room("old", { Date: "2026-09-10" }),
    room("future", { Date: "2026-12-01" }),
    room("blank", { Date: "" }),
  ];

  it("ramps the past and flags the future separately", () => {
    const ctx = buildColourContext(rooms, plan);
    expect(colourForRoom(rooms[0]!, plan, ctx)).toBe(RDBU_LAST); // at the near date -> t=1
    expect(colourForRoom(rooms[1]!, plan, ctx)).toBe(RDBU_FIRST); // the oldest -> t=0
    expect(colourForRoom(rooms[2]!, plan, ctx)).toBe(FUTURE_COLOUR);
    expect(colourForRoom(rooms[3]!, plan, ctx)).toBe(NO_DATA_COLOUR);
  });

  /** The future must not stretch the ramp the past is measured against. */
  it("measures the ramp on past dates only", () => {
    expect(buildColourContext(rooms, plan).maxPast).toBe(10 * 24 * 3600 * 1000);
  });
});
