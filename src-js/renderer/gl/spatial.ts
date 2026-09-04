// The spatial index — one Flatbush R-tree over room bounding boxes, doing two
// jobs the handover is explicit about not letting Pixi do.
//
// NOT Pixi's culler and NOT its scene-graph hit-testing: both are CPU walks over
// the display list. The POC measured a Flatbush tree doing the same two jobs in
// ~0.1 ms at 1.6 M rooms. Built once per level, because the box set is static
// per snapshot — which is exactly the case Flatbush is faster than RBush for,
// and why it was chosen over it.
//
// Two jobs, one structure:
//   - the viewport query, which for GL culls LABELS rather than geometry (see
//     renderer.ts — fills and lines are three draw calls whatever the room
//     count, so there is nothing to save by hiding them);
//   - the pick, which replaces `roomAtNode`'s DOM scan now that there are no
//     nodes to scan.
//
// A third job is already anticipated: snapping. It is an index query plus a
// cheap overlay redraw, and it will reuse this.

import Flatbush from "flatbush";
import { roomBBox } from "../geometry.js";
import type { Door, Extent, Point2D, Room } from "../types.js";

/**
 * Winding-number-free even-odd test against one ring, in flipped space.
 *
 * `alreadyFlipped` says which space the RING is in — the probe is always
 * flipped. Rooms hand over raw payload rings and this flips them as it reads,
 * which avoids allocating a flipped copy per pick on a hot path; doors hand
 * over the glyph's pick ring, which was flipped when the glyph was baked and
 * must not be flipped a second time.
 *
 * The flag exists because the alternative — flipping in the caller — went wrong
 * exactly once already: an already-flipped door ring read through the default
 * lands mirrored about y=0, so every door tested as a miss and every click on a
 * door quietly selected the room underneath it. Nothing about that looks like a
 * coordinate bug from the outside.
 */
function pointInRing(
  px: number,
  py: number,
  points: readonly { x: number; y: number }[],
  alreadyFlipped = false,
): boolean {
  let inside = false;
  for (let i = 0, j = points.length - 1; i < points.length; j = i++) {
    const a = points[i]!;
    const b = points[j]!;
    const ay = alreadyFlipped ? a.y : -a.y;
    const by = alreadyFlipped ? b.y : -b.y;
    if (ay > py !== by > py && px < ((b.x - a.x) * (py - ay)) / (by - ay) + a.x) inside = !inside;
  }
  return inside;
}

export class RoomIndex {
  readonly #rooms: readonly Room[];
  readonly #tree: Flatbush | null;

  constructor(rooms: readonly Room[]) {
    // Only rooms with geometry go in, and the array is kept so a tree index
    // maps back to a room. Flatbush cannot be built empty.
    const drawable = rooms.filter((r) => r.loops?.[0]);
    this.#rooms = drawable;
    if (drawable.length === 0) {
      this.#tree = null;
    } else {
      const tree = new Flatbush(drawable.length);
      for (const room of drawable) {
        const b = roomBBox(room);
        tree.add(b.minX, b.minY, b.maxX, b.maxY);
      }
      tree.finish();
      this.#tree = tree;
    }
  }

  get size(): number {
    return this.#rooms.length;
  }

  /** Rooms whose bbox overlaps the rectangle. */
  search(minX: number, minY: number, maxX: number, maxY: number): Room[] {
    if (!this.#tree) return [];
    return this.#tree.search(minX, minY, maxX, maxY).map((i) => this.#rooms[i]!);
  }

  /**
   * The room containing a point in flipped world space, or `null`.
   *
   * Bbox candidates first, then an exact ring test — a bbox hit is not a hit,
   * and on an L-shaped room the difference is most of the bounding box. A point
   * inside a VOID counts as a miss, which matches what the eye expects: a
   * courtyard is not the room around it.
   *
   * Last match wins, mirroring SVG's paint order where the last polygon drawn
   * is the one on top.
   */
  roomAt(x: number, y: number): Room | null {
    let found: Room | null = null;
    for (const room of this.search(x, y, x, y)) {
      const loops = room.loops!;
      if (!pointInRing(x, y, loops[0]!.points)) continue;
      let inHole = false;
      for (let i = 1; i < loops.length; i++)
        if (pointInRing(x, y, loops[i]!.points)) {
          inHole = true;
          break;
        }
      if (!inHole) found = room;
    }
    return found;
  }
}

/** A door and the target it drew, ready to be indexed. Built by the renderer
 *  from the glyph it is about to draw, so the thing you can click is by
 *  construction the thing you can see — they cannot drift, because there is
 *  only one computation. */
/** Also used for windows: the index keys on a ring and a box and does not care
 *  what the element is, so a second class would be the same code under a
 *  different name. `door` is the element, whichever entity it came from. */
export interface PickableDoor {
  door: Door;
  /** The pick ring, already flipped (`DoorGlyph.pickRing`). */
  ring: readonly Point2D[];
  /** That ring's bounding box (`DoorGlyph.pick`). */
  box: Extent;
}

/**
 * The door pick index — a second Flatbush tree, deliberately not a shared one.
 *
 * SEPARATE FROM `RoomIndex` because doors and rooms overlap by nature: a door
 * sits in the wall of the room it serves, and on a plan its glyph is drawn over
 * that room. One tree returning "the thing at this point" would have to rank
 * two entity types against each other on every query. Two trees let the caller
 * state the precedence once — doors first, because they are smaller, drawn on
 * top, and a click inside a door glyph is a click on the door.
 *
 * The ring test is the same one rooms use, and for the same reason: a footprint
 * is a rectangle in the WALL's frame, so on a diagonal wall its bounding box is
 * much larger than the door. Answering on the box would let a door swallow
 * clicks meant for the room around it.
 */
export class DoorIndex {
  readonly #doors: readonly PickableDoor[];
  readonly #tree: Flatbush | null;

  constructor(doors: readonly PickableDoor[]) {
    this.#doors = doors;
    if (doors.length === 0) {
      // Flatbush cannot be built empty, and a level with no doors is ordinary
      // — a shell, or a pre-fit-out phase.
      this.#tree = null;
    } else {
      const tree = new Flatbush(doors.length);
      for (const d of doors) tree.add(d.box.minX, d.box.minY, d.box.maxX, d.box.maxY);
      tree.finish();
      this.#tree = tree;
    }
  }

  get size(): number {
    return this.#doors.length;
  }

  /** The door under a point in flipped world space, or `null`. Last match wins,
   *  mirroring paint order exactly as `roomAt` does. */
  doorAt(x: number, y: number): Door | null {
    if (!this.#tree) return null;
    let found: Door | null = null;
    for (const i of this.#tree.search(x, y, x, y)) {
      const d = this.#doors[i]!;
      // `true`: the glyph's pick ring is already in flipped space.
      if (pointInRing(x, y, d.ring, true)) found = d.door;
    }
    return found;
  }
}
