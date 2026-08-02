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
import type { Room } from "../types.js";

/** Winding-number-free even-odd test against one ring, in flipped space. */
function pointInRing(px: number, py: number, points: readonly { x: number; y: number }[]): boolean {
  let inside = false;
  for (let i = 0, j = points.length - 1; i < points.length; j = i++) {
    const a = points[i]!;
    const b = points[j]!;
    // Y is flipped on read, consistently for both the ring and the probe.
    const ay = -a.y;
    const by = -b.y;
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
