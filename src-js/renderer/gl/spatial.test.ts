import { describe, expect, it } from "vitest";
import { DoorIndex, RoomIndex } from "./spatial.js";
import { buildDoorGlyph } from "./doorGlyph.js";
import type { Door, Room } from "../types.js";

/** Payload coordinates are Y-UP; the index works in flipped (Y-down) space, so
 *  a room authored at y=0..10 is queried at y=-10..0. Getting this backwards is
 *  the single most likely bug in a pick, so the fixtures state it explicitly. */
const rect = (id: string, x0: number, y0: number, x1: number, y1: number): Room => ({
  id,
  loops: [{ points: [{ x: x0, y: y0 }, { x: x1, y: y0 }, { x: x1, y: y1 }, { x: x0, y: y1 }] }],
});

const withHole: Room = {
  id: "courtyard",
  loops: [
    { points: [{ x: 0, y: 0 }, { x: 30, y: 0 }, { x: 30, y: 30 }, { x: 0, y: 30 }] },
    { points: [{ x: 10, y: 10 }, { x: 20, y: 10 }, { x: 20, y: 20 }, { x: 10, y: 20 }] },
  ],
};

describe("RoomIndex", () => {
  it("indexes only rooms with geometry", () => {
    const i = new RoomIndex([rect("a", 0, 0, 10, 10), { id: "no-geom" }, { id: "empty", loops: [] }]);
    expect(i.size).toBe(1);
  });

  it("survives an empty level", () => {
    // Flatbush cannot be constructed with zero items, so this is a real branch
    // rather than a defensive one -- and an empty level is ordinary (House A's
    // LEVEL 02 has no rooms).
    const i = new RoomIndex([]);
    expect(i.size).toBe(0);
    expect(i.roomAt(0, 0)).toBeNull();
    expect(i.search(-100, -100, 100, 100)).toEqual([]);
  });

  describe("roomAt", () => {
    const index = new RoomIndex([rect("a", 0, 0, 10, 10), rect("b", 20, 0, 30, 10)]);

    it("finds the room containing a point, in FLIPPED space", () => {
      expect(index.roomAt(5, -5)?.id).toBe("a");
      expect(index.roomAt(25, -5)?.id).toBe("b");
    });

    it("returns null in the gap between rooms", () => {
      // A bbox hit is not a hit. This is the case a naive index gets wrong.
      expect(index.roomAt(15, -5)).toBeNull();
    });

    it("returns null outside everything", () => {
      expect(index.roomAt(-50, -50)).toBeNull();
      expect(index.roomAt(5, 5)).toBeNull(); // unflipped Y: above the plan
    });

    it("misses the bounding box of an L-shape where the room is not", () => {
      // The notch is inside the bbox and outside the room, which is most of the
      // difference between a bbox test and a real one.
      const l = new RoomIndex([
        {
          id: "L",
          loops: [{ points: [
            { x: 0, y: 0 }, { x: 20, y: 0 }, { x: 20, y: 5 },
            { x: 5, y: 5 }, { x: 5, y: 20 }, { x: 0, y: 20 },
          ] }],
        },
      ]);
      expect(l.roomAt(2, -2)?.id).toBe("L");   // in the arm
      expect(l.roomAt(15, -15)).toBeNull();    // in the notch
    });

    it("treats a void as a miss", () => {
      // A courtyard is not the room around it -- which is what the eye expects,
      // and what the SVG renderer did by painting the hole over the fill.
      const i = new RoomIndex([withHole]);
      expect(i.roomAt(5, -5)?.id).toBe("courtyard");
      expect(i.roomAt(15, -15)).toBeNull();
    });
  });

  describe("search", () => {
    it("returns bbox overlaps, not exact hits", () => {
      const i = new RoomIndex([rect("a", 0, 0, 10, 10), rect("b", 20, 0, 30, 10)]);
      expect(i.search(-1, -11, 11, 1).map((r) => r.id)).toEqual(["a"]);
      expect(i.search(-1, -11, 31, 1).map((r) => r.id).sort()).toEqual(["a", "b"]);
      expect(i.search(100, 100, 200, 200)).toEqual([]);
    });
  });

  describe("DoorIndex", () => {
    /** A door authored at y=+5 in payload space lives at y=-5 once flipped. */
    const door = (id: string, x0: number, y0: number, x1: number, y1: number): Door => ({
      id,
      loops: [{ points: [{ x: x0, y: y0 }, { x: x1, y: y0 }, { x: x1, y: y1 }, { x: x0, y: y1 }] }],
      through_wall_normal: { x: 0, y: 1 },
    });

    /** Index a door exactly the way the renderer does — through the glyph it is
     *  about to draw — so the test cannot pass with a pick target the renderer
     *  would never build. */
    const indexOf = (doors: Door[]) =>
      new DoorIndex(
        doors.flatMap((d) => {
          const g = buildDoorGlyph(d);
          return g ? [{ door: d, ring: g.pickRing, box: g.pick }] : [];
        }),
      );

    /**
     * THE DOUBLE FLIP, which shipped for exactly one manual test run.
     *
     * The glyph's pick ring is baked in flipped space, but `pointInRing` flips
     * as it reads because rooms hand it raw payload points. Read through the
     * default the ring lands mirrored about y=0, so every door tested as a miss
     * — and because doors sit inside rooms, every click on a door quietly
     * selected the room underneath it. Nothing about that reads as a coordinate
     * bug from the outside, which is why it is pinned here by the sign of y.
     */
    it("hits a door at its flipped position, not its mirror", () => {
      const i = indexOf([door("d1", 0, 4, 3, 6)]);
      expect(i.doorAt(1.5, -5)?.id).toBe("d1");
      expect(i.doorAt(1.5, 5)).toBeNull();
    });

    it("misses outside the footprint", () => {
      const i = indexOf([door("d1", 0, 4, 3, 6)]);
      expect(i.doorAt(50, -5)).toBeNull();
    });

    /** A geometry-less door is clickable through the square its arrow drew —
     *  the door that most needs explaining must not be the one that cannot be
     *  inspected. */
    it("hits a door that has no footprint", () => {
      const i = indexOf([
        { id: "d1", loops: [], insertion_point: { x: 10, y: 20 }, through_wall_normal: { x: 0, y: 1 } },
      ]);
      expect(i.doorAt(10, -20)?.id).toBe("d1");
      expect(i.doorAt(10, 20)).toBeNull();
    });

    it("is empty-safe, because a level with no doors is ordinary", () => {
      const i = new DoorIndex([]);
      expect(i.size).toBe(0);
      expect(i.doorAt(0, 0)).toBeNull();
    });
  });
});
