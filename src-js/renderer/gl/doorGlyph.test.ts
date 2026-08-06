import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  buildDoorGlyph,
  FALLBACK_GLYPH_SIZE,
  MIN_FOOTPRINT_EXTENT,
  MAX_ARROW_SOURCE_SIZE,
  MIN_PICK_DEPTH,
} from "./doorGlyph.js";
import type { Door } from "../types.js";

/** A 3ft x 0.5ft footprint centred on the origin, Y-up as the payload has it. */
function boxDoor(over: Partial<Door> = {}): Door {
  return {
    id: "d1",
    loops: [
      {
        points: [
          { x: -1.5, y: -0.25 },
          { x: 1.5, y: -0.25 },
          { x: 1.5, y: 0.25 },
          { x: -1.5, y: 0.25 },
        ],
      },
    ],
    ...over,
  };
}

/** Every vertex of a triangle list, as points. */
function points(tris: readonly number[]): { x: number; y: number }[] {
  const out: { x: number; y: number }[] = [];
  for (let i = 0; i < tris.length; i += 2) out.push({ x: tris[i]!, y: tris[i + 1]! });
  return out;
}

/** The vertex furthest along `dir` — the arrow's head apex when `dir` is the
 *  direction it points. */
function tipAlong(tris: readonly number[], dir: { x: number; y: number }): { x: number; y: number } {
  let best = { x: 0, y: 0 };
  let bestDot = -Infinity;
  for (const p of points(tris)) {
    const d = p.x * dir.x + p.y * dir.y;
    if (d > bestDot) {
      bestDot = d;
      best = p;
    }
  }
  return best;
}

describe("buildDoorGlyph", () => {
  it("draws a rectangle and an arrow when it has both inputs", () => {
    const g = buildDoorGlyph(boxDoor({ through_wall_normal: { x: 0, y: 1 } }))!;
    expect(g.rect.length).toBeGreaterThan(0);
    expect(g.arrow.length).toBeGreaterThan(0);
    expect(g.cross).toHaveLength(0);
  });

  /**
   * THE Y-FLIP, which is the single easiest thing to get wrong in this file.
   *
   * The normal arrives Y-up like every other payload value and the glyph is
   * built in flipped (Y-down) space, so a normal of +Y must produce an arrow
   * whose HEAD is at negative y. Miss the flip and every arrow is mirrored,
   * which is invisible on a door in a north-south wall and wrong everywhere
   * else.
   *
   * Tested through the apex rather than through `min`/`max`, because the arrow
   * runs through the wall in BOTH directions — its extent is symmetric, so the
   * extremes alone cannot tell the tip from the tail. The head's apex is the
   * one vertex sitting on the arrow's own axis at the far end, which is exactly
   * what "which way does it point" means.
   */
  it("flips the normal into Y-down space", () => {
    for (const [payloadY, expectedApexSign] of [
      [1, -1],
      [-1, 1],
    ] as const) {
      const g = buildDoorGlyph(boxDoor({ through_wall_normal: { x: 0, y: payloadY } }))!;
      const apex = tipAlong(g.arrow, { x: 0, y: expectedApexSign });
      expect(Math.sign(apex.y)).toBe(expectedApexSign);
      expect(Math.abs(apex.y)).toBeGreaterThan(1.0);
      // On the axis: the apex is the point at a = 0 in the door's frame. The
      // tail end has no such vertex — shaft corners and chevron arms are all
      // off-axis — which is what makes this a direction test and not a
      // magnitude one.
      expect(Math.abs(apex.x)).toBeLessThan(1e-9);

      const tail = tipAlong(g.arrow, { x: 0, y: -expectedApexSign });
      expect(Math.abs(tail.x)).toBeGreaterThan(1e-6);
    }
  });

  it("points the arrow along the normal, not across it", () => {
    const g = buildDoorGlyph(boxDoor({ through_wall_normal: { x: 1, y: 0 } }))!;
    const xs = points(g.arrow).map((p) => p.x);
    const ys = points(g.arrow).map((p) => p.y);
    // A +X normal: the arrow runs along x and stays narrow in y.
    expect(Math.max(...xs)).toBeGreaterThan(1.0);
    expect(Math.max(...ys) - Math.min(...ys)).toBeLessThan(1.2);
  });

  it("draws the rectangle only when there is no direction", () => {
    const g = buildDoorGlyph(boxDoor())!;
    expect(g.rect.length).toBeGreaterThan(0);
    expect(g.arrow).toHaveLength(0);
    expect(g.cross).toHaveLength(0);
  });

  /**
   * The geometry-less door — 2 of the 26 in the House A sample, the `±1e30`
   * family whose footprint the producer correctly suppresses. It keeps its
   * insertion point, so it draws as an arrow rather than vanishing. That is the
   * case `insertion_point` was added to the contract for.
   */
  it("draws the arrow only when the footprint is missing", () => {
    const g = buildDoorGlyph({
      id: "d1",
      loops: [],
      insertion_point: { x: 10, y: 20 },
      through_wall_normal: { x: 0, y: 1 },
    })!;
    expect(g.rect).toHaveLength(0);
    expect(g.arrow.length).toBeGreaterThan(0);
    expect(g.cross).toHaveLength(0);
    // Placed at the insertion point, in flipped space.
    const xs = points(g.arrow).map((p) => p.x);
    expect(Math.min(...xs)).toBeGreaterThan(9);
    expect(Math.max(...xs)).toBeLessThan(11);
  });

  it("treats a footprint below the threshold as no footprint", () => {
    const tiny = MIN_FOOTPRINT_EXTENT / 10;
    const g = buildDoorGlyph({
      id: "d1",
      loops: [
        {
          points: [
            { x: 0, y: 0 },
            { x: tiny, y: 0 },
            { x: tiny, y: tiny },
          ],
        },
      ],
      through_wall_normal: { x: 1, y: 0 },
    })!;
    expect(g.rect).toHaveLength(0);
    expect(g.arrow.length).toBeGreaterThan(0);
  });

  it("draws a cross only when neither input is usable", () => {
    const g = buildDoorGlyph({ id: "d1", loops: [], insertion_point: { x: 5, y: 5 } })!;
    expect(g.rect).toHaveLength(0);
    expect(g.arrow).toHaveLength(0);
    expect(g.cross.length).toBeGreaterThan(0);
    // Fixed size — a placeholder, not a measurement.
    const xs = points(g.cross).map((p) => p.x);
    expect(Math.max(...xs) - Math.min(...xs)).toBeCloseTo(FALLBACK_GLYPH_SIZE, 5);
  });

  /** No footprint and no point is the one door that cannot be placed. It stays
   *  possible only for a snapshot pushed before `insertion_point` existed. */
  it("returns null when the door cannot be placed at all", () => {
    expect(buildDoorGlyph({ id: "d1", loops: [] })).toBeNull();
  });

  it("never emits a NaN, including for a zero-length normal", () => {
    const g = buildDoorGlyph(boxDoor({ through_wall_normal: { x: 0, y: 0 } }))!;
    // A zero vector is not a direction: no arrow, and nothing poisoned.
    expect(g.arrow).toHaveLength(0);
    for (const v of [...g.rect, ...g.cross, g.pick.minX, g.pick.maxY]) expect(Number.isFinite(v)).toBe(true);
  });

  /** House A's garage panel-lift door is 17.75 ft across, which unclamped
   *  produced a ~19 ft arrow that dominated the plan and read as an annotation
   *  rather than a door marking. Ordinary doors keep their proportional arrow;
   *  only the outliers are reined in. */
  it("clamps the arrow on a very wide opening", () => {
    const wide = buildDoorGlyph({
      id: "d1",
      loops: [{ points: [
        { x: -9, y: -0.25 }, { x: 9, y: -0.25 }, { x: 9, y: 0.25 }, { x: -9, y: 0.25 },
      ] }],
      through_wall_normal: { x: 0, y: 1 },
    })!;
    const ys = points(wide.arrow).map((p) => p.y);
    const reach = Math.max(...ys) - Math.min(...ys);
    expect(reach).toBeLessThanOrEqual(MAX_ARROW_SOURCE_SIZE * 1.1 + 1e-9);
    // The footprint itself is untouched — only the arrow is clamped.
    expect(wide.pick.maxX - wide.pick.minX).toBeCloseTo(18, 5);
  });

  it("normalises a non-unit normal rather than scaling the arrow by it", () => {
    const unit = buildDoorGlyph(boxDoor({ through_wall_normal: { x: 0, y: 1 } }))!;
    const long = buildDoorGlyph(boxDoor({ through_wall_normal: { x: 0, y: 7 } }))!;
    expect(long.arrow).toEqual(unit.arrow);
  });

  /**
   * The real export, not a hand-written shape. Hand-made cases prove the rules
   * hold for inputs chosen to exercise them; this proves the rules cover what
   * an actual Revit model produces — which is where the two geometry-less doors
   * and the curtain-panel door came from in the first place.
   */
  describe("against the House A export", () => {
    const doors = JSON.parse(
      readFileSync(resolve(import.meta.dirname, "..", "fixtures", "house-a.doors.json"), "utf8"),
    ) as Door[];

    it("places every door", () => {
      expect(doors).toHaveLength(26);
      expect(doors.filter((d) => buildDoorGlyph(d) === null)).toHaveLength(0);
    });

    it("draws the cases the export actually contains", () => {
      const tally = { full: 0, rectOnly: 0, arrowOnly: 0, cross: 0 };
      for (const door of doors) {
        const g = buildDoorGlyph(door)!;
        if (g.cross.length) tally.cross++;
        else if (g.rect.length && g.arrow.length) tally.full++;
        else if (g.rect.length) tally.rectOnly++;
        else tally.arrowOnly++;
      }
      // 3475937 and 3479042 are the `±1e30` family: no footprint, but an
      // insertion point and a normal, so they draw as arrows instead of
      // vanishing. Nothing degrades to a cross, and nothing is undrawable.
      expect(tally).toEqual({ full: 24, rectOnly: 0, arrowOnly: 2, cross: 0 });
    });

    it("emits no NaN anywhere in the real set", () => {
      for (const door of doors) {
        const g = buildDoorGlyph(door)!;
        for (const v of [...g.rect, ...g.arrow, ...g.cross]) expect(Number.isFinite(v)).toBe(true);
      }
    });

    /** The curtain panel door: a footprint and a normal, but no
     *  `LocationPoint`. It must draw a full glyph off the footprint alone. */
    it("draws the curtain panel door with no insertion point", () => {
      const door = doors.find((d) => d.id === "3784724")!;
      expect(door.insertion_point ?? null).toBeNull();
      const g = buildDoorGlyph(door)!;
      expect(g.rect.length).toBeGreaterThan(0);
      expect(g.arrow.length).toBeGreaterThan(0);
    });
  });

  describe("pick target", () => {
    it("keeps the footprint's extent ALONG the wall exactly", () => {
      const g = buildDoorGlyph(boxDoor({ through_wall_normal: { x: 0, y: 1 } }))!;
      expect(g.pick.minX).toBeCloseTo(-1.5, 5);
      expect(g.pick.maxX).toBeCloseTo(1.5, 5);
    });

    /**
     * A door footprint is as deep as its wall and no deeper — 4 px on the House
     * A plan at a fitted view, and 1.2 px for the garage panel-lift door. An
     * exact test against that is a target nobody can hit, and the miss resolves
     * to the room underneath, so the door reads as unselectable rather than as
     * fiddly. Padded through the wall only; the dimension the user aims with is
     * left alone.
     */
    it("pads a sliver footprint to a clickable depth", () => {
      const g = buildDoorGlyph(boxDoor({ through_wall_normal: { x: 0, y: 1 } }))!;
      expect(g.pick.maxY - g.pick.minY).toBeCloseTo(MIN_PICK_DEPTH, 5);
      // Padding is symmetric, so the door stays centred in its wall.
      expect((g.pick.minY + g.pick.maxY) / 2).toBeCloseTo(0, 5);
    });

    it("does not shrink a footprint that is already deep enough", () => {
      const deep = buildDoorGlyph({
        id: "d1",
        loops: [{ points: [
          { x: -1.5, y: -2 }, { x: 1.5, y: -2 }, { x: 1.5, y: 2 }, { x: -1.5, y: 2 },
        ] }],
        through_wall_normal: { x: 0, y: 1 },
      })!;
      expect(deep.pick.maxY - deep.pick.minY).toBeCloseTo(4, 5);
    });

    /** In the DOOR's frame, not the world's: a diagonal door grows
     *  perpendicular to itself rather than into a bounding box several times
     *  its size, which would swallow clicks meant for the room around it. */
    it("pads along the door's own axis for a diagonal wall", () => {
      const s = Math.SQRT1_2;
      const g = buildDoorGlyph({
        id: "d1",
        // A 4ft-long sliver at 45 degrees, 0.1ft thick.
        loops: [{ points: [
          { x: -2 * s, y: -2 * s }, { x: 2 * s, y: 2 * s },
          { x: 2 * s + 0.05 * s, y: 2 * s - 0.05 * s }, { x: -2 * s + 0.05 * s, y: -2 * s - 0.05 * s },
        ] }],
        through_wall_normal: { x: s, y: -s },
      })!;
      // A world-axis pad would make both spans ~4 + MIN_PICK_DEPTH. Padding in
      // the door's frame leaves the diagonal length alone, so each axis grows
      // by only the diagonal component of the depth.
      const span = g.pick.maxX - g.pick.minX;
      expect(span).toBeLessThan(4 * s + MIN_PICK_DEPTH);
      expect(span).toBeGreaterThan(4 * s);
    });

    /** The door that most needs explaining must not be the one that cannot be
     *  clicked — so the target follows what was drawn, not what was available. */
    it("is a square around the insertion point when there is no footprint", () => {
      const g = buildDoorGlyph({
        id: "d1",
        loops: [],
        insertion_point: { x: 10, y: -20 },
        through_wall_normal: { x: 0, y: 1 },
      })!;
      expect((g.pick.minX + g.pick.maxX) / 2).toBeCloseTo(10, 5);
      // Flipped: payload y=-20 -> world y=+20.
      expect((g.pick.minY + g.pick.maxY) / 2).toBeCloseTo(20, 5);
      expect(g.pick.maxX - g.pick.minX).toBeGreaterThan(0);
    });

    it("covers the cross for a door that drew one", () => {
      const g = buildDoorGlyph({ id: "d1", loops: [], insertion_point: { x: 0, y: 0 } })!;
      expect(g.pick.maxX - g.pick.minX).toBeCloseTo(FALLBACK_GLYPH_SIZE, 5);
    });
  });
});
