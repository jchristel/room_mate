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
//
// **A pick answers a LIST, smallest first.** Everything under the pointer is
// returned, not just one element, because the viewer filters that list per zone
// and shows what survives.
// Within a layer the smallest ring leads: a large item drawn over a small one
// used to catch every click inside it (paint order, last drawn wins), which
// left the small one unreachable. The smaller thing is the more specific one a
// reader can have aimed at -- the rule the layers already follow against each
// other, doors before the room they sit in.

import Flatbush from "flatbush";
import { roomBBox } from "../geometry.js";
import type { Pick } from "../seam.js";
import type { Ceiling, Door, Extent, Floor, Item, Loop, Point2D, Room, Space, WindowOpening } from "../types.js";

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

/** A ring's enclosed area, whatever its winding. The shoelace formula; a flip
 *  of y changes the sign only, so either space gives the same answer. */
function ringArea(points: readonly { x: number; y: number }[]): number {
  let twice = 0;
  for (let i = 0, j = points.length - 1; i < points.length; j = i++)
    twice += points[j]!.x * points[i]!.y - points[i]!.x * points[j]!.y;
  return Math.abs(twice) / 2;
}

/**
 * Tree hits, smallest area first; on equal area the LATER entry first, since it
 * was painted on top. `hit` is the exact test a bbox candidate must also pass.
 *
 * Areas are computed once, when the index is built, so a pick sorts numbers
 * and never re-measures a ring.
 */
function smallestFirst(
  tree: Flatbush | null,
  areas: readonly number[],
  x: number,
  y: number,
  hit: (i: number) => boolean,
): number[] {
  if (!tree) return [];
  return tree
    .search(x, y, x, y)
    .filter(hit)
    .sort((a, b) => areas[a]! - areas[b]! || b - a);
}

export class RoomIndex {
  readonly #rooms: readonly Room[];
  readonly #areas: readonly number[];
  readonly #tree: Flatbush | null;

  constructor(rooms: readonly Room[]) {
    // Only rooms with geometry go in, and the array is kept so a tree index
    // maps back to a room. Flatbush cannot be built empty.
    const drawable = rooms.filter((r) => r.loops?.[0]);
    this.#rooms = drawable;
    this.#areas = drawable.map((r) => ringArea(r.loops![0]!.points));
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
   * The first of `roomsAt`, so hover, the tooltip and a click all name the
   * same room where two overlap -- which linked models' rooms do.
   */
  roomAt(x: number, y: number): Room | null {
    return this.roomsAt(x, y)[0] ?? null;
  }

  /** Every room containing the point, smallest outer ring first. One model's
   *  rooms do not overlap, but linked models' can -- a room inside another
   *  model's site room -- and the smaller one is what was aimed at. */
  roomsAt(x: number, y: number): Room[] {
    return smallestFirst(this.#tree, this.#areas, x, y, (i) => {
      const loops = this.#rooms[i]!.loops!;
      if (!pointInRing(x, y, loops[0]!.points)) return false;
      for (let h = 1; h < loops.length; h++) if (pointInRing(x, y, loops[h]!.points)) return false;
      return true;
    }).map((i) => this.#rooms[i]!);
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
  readonly #areas: readonly number[];
  readonly #tree: Flatbush | null;

  constructor(doors: readonly PickableDoor[]) {
    this.#doors = doors;
    this.#areas = doors.map((d) => ringArea(d.ring));
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

  /** The door under a point in flipped world space, or `null`: the first of
   *  `doorsAt`. */
  doorAt(x: number, y: number): Door | null {
    return this.doorsAt(x, y)[0] ?? null;
  }

  /** Every door under a point, smallest pick ring first. */
  doorsAt(x: number, y: number): Door[] {
    // `true`: the glyph's pick ring is already in flipped space.
    const hit = (i: number) => pointInRing(x, y, this.#doors[i]!.ring, true);
    return smallestFirst(this.#tree, this.#areas, x, y, hit).map((i) => this.#doors[i]!.door);
  }
}

/**
 * One surface-like element to index: the element, and the rings that ARE its
 * pickable area, as PIECES -- each an outer ring then its holes, in raw payload
 * space (Y-up), exactly as the payload carries them.
 *
 * The caller decides the pieces, because they must be what the layer DRAWS: a
 * ceiling or floor draws every piece and every hole, a space draws only its
 * outer ring. Clicking inside a ring the plan never showed would select an
 * outline the reader cannot see.
 */
export interface PickableSurface<T> {
  element: T;
  pieces: readonly (readonly Loop[])[];
}

/**
 * The pick index for the three outline layers -- spaces, ceilings, floors.
 *
 * A third tree, not the room one reused, because these are not rooms: a
 * ceiling is a LIST of disjoint pieces (RHH's multi-polygon ceilings lose up to
 * 99% of their area if only one is kept), so one element has one box around all
 * of them and a hit is "inside any piece's outer ring and none of that piece's
 * holes". A hole is a light well or a void over an atrium, drawn as its own
 * ring, and a click in it is a click on whatever is below -- the rule a room's
 * void follows.
 */
export class SurfaceIndex<T> {
  readonly #items: readonly PickableSurface<T>[];
  readonly #areas: readonly number[];
  readonly #tree: Flatbush | null;

  constructor(items: readonly PickableSurface<T>[]) {
    // Only elements with at least one real outer ring go in: a ceiling duHast
    // could not measure draws nothing, so it must not be clickable either.
    const drawable = items
      .map((s) => ({ element: s.element, pieces: s.pieces.filter((p) => p[0]?.points.length) }))
      .filter((s) => s.pieces.length);
    this.#items = drawable;
    // NET area, holes out: a ceiling with a large light well is a smaller thing
    // to have aimed at than its outline suggests.
    this.#areas = drawable.map((s) =>
      s.pieces.reduce(
        (sum, piece) => sum + ringArea(piece[0]!.points) - piece.slice(1).reduce((h, l) => h + ringArea(l.points), 0),
        0,
      ),
    );
    if (drawable.length === 0) {
      this.#tree = null;
    } else {
      const tree = new Flatbush(drawable.length);
      for (const s of drawable) {
        let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
        for (const piece of s.pieces)
          for (const raw of piece[0]!.points) {
            // Flipped, like every box in this file.
            if (raw.x < minX) minX = raw.x;
            if (raw.x > maxX) maxX = raw.x;
            if (-raw.y < minY) minY = -raw.y;
            if (-raw.y > maxY) maxY = -raw.y;
          }
        tree.add(minX, minY, maxX, maxY);
      }
      tree.finish();
      this.#tree = tree;
    }
  }

  get size(): number {
    return this.#items.length;
  }

  /** Every element under a point in flipped world space, smallest net area first. */
  at(x: number, y: number): T[] {
    const hit = (i: number) =>
      this.#items[i]!.pieces.some(
        (piece) =>
          pointInRing(x, y, piece[0]!.points) && !piece.slice(1).some((hole) => pointInRing(x, y, hole.points)),
      );
    return smallestFirst(this.#tree, this.#areas, x, y, hit).map((i) => this.#items[i]!.element);
  }
}

/** The indexes a pick consults, one per selectable layer. */
export interface PickLayers {
  doors: DoorIndex;
  windows: DoorIndex;
  /** Items ride a `DoorIndex` -- it keys on a ring and a box and does not care
   *  what the element is -- so what comes back is an `Item` typed as a door. */
  ffe: DoorIndex;
  rooms: RoomIndex;
  spaces: SurfaceIndex<Space>;
  ceilings: SurfaceIndex<Ceiling>;
  floors: SurfaceIndex<Floor>;
}

/**
 * Everything under a point in flipped world space, in pick order.
 *
 * Doors, then windows, then FF&E, then the room. Each element layer is drawn
 * over the room it serves and is far smaller, so a click inside one is a click
 * on it. Doors lead windows only because a fixed order beats an ambiguous one
 * where the two could overlap, which they hardly ever do; FF&E follows both
 * because an item pushed against a door is the less likely target of the two.
 *
 * Spaces, ceilings and floors come AFTER the room, in their paint order top
 * first. They cover the same ground as the room under them, so ahead of it a
 * room could not be clicked at all while one of their layers was on.
 *
 * **What may be picked at all is the CALLER's question.** This returns the
 * whole stack in one order; the viewer filters it per zone, which is what the
 * selection filter is. There was briefly a `PICK_FIRST` set here naming the
 * four kinds a plain click could land on -- it was a stand-in for that filter,
 * and two rules for one question is one too many.
 *
 * Within a layer, smallest first -- see the header. Here rather than in the
 * renderer so the order is tested without WebGL.
 */
export function pickStack(layers: PickLayers, x: number, y: number): Pick[] {
  return [
    ...layers.doors.doorsAt(x, y).map((door): Pick => ({ kind: "door", door })),
    ...layers.windows.doorsAt(x, y).map((d): Pick => ({ kind: "window", window: d as WindowOpening })),
    ...layers.ffe.doorsAt(x, y).map((d): Pick => ({ kind: "item", item: d as unknown as Item })),
    ...layers.rooms.roomsAt(x, y).map((room): Pick => ({ kind: "room", room })),
    ...layers.spaces.at(x, y).map((space): Pick => ({ kind: "space", space })),
    ...layers.ceilings.at(x, y).map((ceiling): Pick => ({ kind: "ceiling", ceiling })),
    ...layers.floors.at(x, y).map((floor): Pick => ({ kind: "floor", floor })),
  ];
}
