// The tag written on an FF&E item, a space, a ceiling or a floor: where it goes and
// how big it is.
//
// **Rooms' rule, applied to the element's own outline.** The text is placed by
// `placeLabel` (the room-label placement) on the element's footprint, so it is sized
// to the object it belongs to and never spills over a neighbour. One function, both
// painters, as for the labels: the GL layer and the SVG export call this and nothing
// else decides where a tag is.
//
// **What the footprint is, per entity.** A space's outer ring and holes. A ceiling or
// floor is a list of pieces (see `Ceiling`); the tag goes in the LARGEST, because a
// tag cannot sit in several places and the biggest piece is where it reads. An item
// uses its footprint when it has a usable one; most have none, and then the tag is
// fitted to the marker the item is drawn as -- small, because the object is.

import { MIN_FOOTPRINT_EXTENT } from "./gl/doorGlyph.js";
import { MARKER_SIZE } from "./gl/itemGlyph.js";
import { placeLabel } from "./labelPlacement.js";
import type { Ceiling, Item, Loop, Room, Space } from "./types.js";

export type TagEntity = "ffe" | "spaces" | "ceilings" | "floors";

/** What a zone asks of the renderer for one entity's tags. `text` is the page's
 *  resolution of the settings' fields for one element; an element it returns nothing
 *  for gets no tag. The colours are the settings' strings, unvalidated. */
export interface TagRequest<E> {
  text: (element: E) => string[];
  primary?: string | null | undefined;
  secondary?: string | null | undefined;
}

export interface TagsRequest {
  ffe?: TagRequest<Item> | undefined;
  spaces?: TagRequest<Space> | undefined;
  ceilings?: TagRequest<Ceiling> | undefined;
  floors?: TagRequest<Ceiling> | undefined;
}

/** One tag, placed: the centre of its text block in the flipped plan space. */
export interface PlacedTag {
  x: number;
  y: number;
  fontSize: number;
  lines: string[];
}

/** The stand-in `Room` a footprint is placed as; remembered per element so the
 *  placement cache in `placeLabel` (keyed by object) survives a repaint. */
const footprints = new WeakMap<object, Room | null>();

function asRoom(loops: Loop[]): Room {
  return { id: "", loops } as Room;
}

function shoelace(loop: Loop): number {
  let a = 0;
  const p = loop.points;
  for (let i = 0, j = p.length - 1; i < p.length; j = i++) a += p[j]!.x * p[i]!.y - p[i]!.x * p[j]!.y;
  return Math.abs(a) / 2;
}

function surfaceFootprint(surface: Ceiling): Room | null {
  let best: Loop[] | null = null;
  let bestArea = 0;
  for (const piece of surface.polygons ?? []) {
    const outer = piece.loops?.[0];
    if (!outer || outer.points.length < 3) continue;
    const area = shoelace(outer);
    if (area > bestArea) {
      bestArea = area;
      best = piece.loops as Loop[];
    }
  }
  return best ? asRoom(best) : null;
}

function itemFootprint(item: Item): Room | null {
  const outer = item.loops?.[0];
  if (outer && outer.points.length >= 3) {
    const xs = outer.points.map((p) => p.x);
    const ys = outer.points.map((p) => p.y);
    if (Math.max(Math.max(...xs) - Math.min(...xs), Math.max(...ys) - Math.min(...ys)) >= MIN_FOOTPRINT_EXTENT) {
      return asRoom([outer]);
    }
  }
  const at = item.insertion_point;
  if (!at) return null;
  const h = MARKER_SIZE / 2;
  const points = [
    { x: at.x - h, y: at.y - h },
    { x: at.x + h, y: at.y - h },
    { x: at.x + h, y: at.y + h },
    { x: at.x - h, y: at.y + h },
  ];
  return asRoom([{ points } as Loop]);
}

function footprintOf(entity: TagEntity, element: Item | Space | Ceiling): Room | null {
  if (footprints.has(element)) return footprints.get(element)!;
  const room =
    entity === "ffe"
      ? itemFootprint(element as Item)
      : entity === "spaces"
        ? (element as Space).loops?.[0] && (element as Space).loops!.length
          ? asRoom((element as Space).loops as Loop[])
          : null
        : surfaceFootprint(element as Ceiling);
  footprints.set(element, room);
  return room;
}

/**
 * Every tag one entity has on a level. `baseFont` is the level's ceiling on the font
 * (the room labels' own), so an element larger than a room does not get a banner.
 */
export function placeElementTags<E extends Item | Space | Ceiling>(
  entity: TagEntity,
  elements: readonly E[],
  request: TagRequest<E>,
  baseFont: number,
): PlacedTag[] {
  const out: PlacedTag[] = [];
  for (const element of elements) {
    const lines = request.text(element).filter((l) => l !== "");
    if (lines.length === 0) continue;
    const footprint = footprintOf(entity, element);
    if (!footprint) continue;
    const longest = lines.reduce((n, l) => Math.max(n, l.length), 1);
    const placed = placeLabel(footprint, longest, lines.length, baseFont);
    if (!placed || !(placed.fontSize > 0)) continue;
    out.push({ x: placed.x, y: placed.y, fontSize: placed.fontSize, lines });
  }
  return out;
}
