// Editing a revision's items, as pure functions over the item list.
//
// **An edit is a new revision, never a change to an old one** (plan decision 9; the server
// refuses nothing here but only ever appends). So the page holds a DRAFT: the items of the
// revision being viewed, edited freely, and "save" sends them as the next revision. Nothing
// in this file mutates its input.
//
// **A dimension is a captured distance plus a MODE** that says how a room of another size reads
// it. Switching the mode never moves the item in its own room (the position is `along_mm`
// either way); it changes what happens when the room changes. The number a person types is
// in the unit the mode implies: mm from the start, mm from the END, or a percentage.

import { depthAt, edges, pointOn, type Pt } from "./geometry.js";
import type { AlongMode, BlueprintItem, OffMode, Placement } from "./types.js";

const withPlacement = (item: BlueprintItem, patch: Partial<Placement>): BlueprintItem => ({
  ...item,
  placement: { ...item.placement, ...patch },
});

/** The along figure as the person reads it, in the unit its mode implies. */
export function alongValue(p: Placement): number {
  switch (p.along_mode) {
    case "fixed_from_start":
      return p.along_mm;
    case "fixed_from_end":
      return p.edge_len_mm - p.along_mm;
    case "percent":
      return p.edge_len_mm === 0 ? 0 : (p.along_mm / p.edge_len_mm) * 100;
  }
}

/** The off-the-wall figure as the person reads it: mm, or a percentage of the room's depth. */
export function offValue(p: Placement): number {
  return p.off_mode === "fixed" ? p.off_mm : p.depth_mm === 0 ? 0 : (p.off_mm / p.depth_mm) * 100;
}

export function alongUnit(mode: AlongMode): string {
  return mode === "percent" ? "%" : "mm";
}

export function offUnit(mode: OffMode): string {
  return mode === "percent" ? "%" : "mm";
}

/** Move the item along its edge to the typed figure. The room's depth in front of it is
 *  re-measured, because on an angled or notched room it changes along the wall. */
export function setAlongValue(item: BlueprintItem, ring: readonly Pt[], value: number): BlueprintItem {
  const p = item.placement;
  const along =
    p.along_mode === "fixed_from_start" ? value : p.along_mode === "fixed_from_end" ? p.edge_len_mm - value : (value / 100) * p.edge_len_mm;
  const e = edges(ring)[p.edge];
  const depth = e ? depthAt(ring, p.edge, pointOn(e, along, 0)) : p.depth_mm;
  // A percentage of depth keeps its fraction when the depth changes under it, so the distance follows.
  const off = p.off_mode === "percent" && p.depth_mm > 0 ? (p.off_mm / p.depth_mm) * depth : p.off_mm;
  return withPlacement(item, { along_mm: along, depth_mm: depth, off_mm: off });
}

export function setOffValue(item: BlueprintItem, value: number): BlueprintItem {
  const p = item.placement;
  return withPlacement(item, { off_mm: p.off_mode === "fixed" ? value : (value / 100) * p.depth_mm });
}

export const setAlongMode = (item: BlueprintItem, mode: AlongMode): BlueprintItem => withPlacement(item, { along_mode: mode });
export const setOffMode = (item: BlueprintItem, mode: OffMode): BlueprintItem => withPlacement(item, { off_mode: mode });
export const setFacing = (item: BlueprintItem, degrees: number): BlueprintItem => withPlacement(item, { facing_deg: degrees });

/** Put the item on another edge, keeping how far along and how far off it is, within the new edge. */
export function moveToEdge(item: BlueprintItem, ring: readonly Pt[], edge: number): BlueprintItem {
  const e = edges(ring)[edge];
  if (!e) return item;
  const along = Math.min(Math.max(item.placement.along_mm, 0), e.len);
  const depth = depthAt(ring, edge, pointOn(e, along, 0));
  return withPlacement(item, { edge, along_mm: along, edge_len_mm: e.len, depth_mm: depth, corner_anchored: false });
}

/** Mark items excluded, or put them back. Excluded items are kept, never deleted. */
export function setExcluded(items: readonly BlueprintItem[], ids: ReadonlySet<string>, excluded: boolean): BlueprintItem[] {
  return items.map((i) => (ids.has(i.id) ? { ...i, excluded } : i));
}

/** Take the placement of each named item from another revision's copy of it. An item the source
 *  does not hold, or whose edge the outline does not have, is left as it is. Returns the new list
 *  and how many items changed. */
export function importPlacements(
  items: readonly BlueprintItem[],
  source: readonly BlueprintItem[],
  ids: ReadonlySet<string>,
  edgeCount: number,
): { items: BlueprintItem[]; changed: number } {
  const from = new Map(source.map((i) => [i.id, i]));
  let changed = 0;
  const next = items.map((i) => {
    const s = from.get(i.id);
    if (!ids.has(i.id) || !s || s.placement.edge >= edgeCount) return i;
    if (JSON.stringify(s.placement) === JSON.stringify(i.placement)) return i;
    changed += 1;
    return { ...i, placement: { ...s.placement } };
  });
  return { items: next, changed };
}

export const sameItems = (a: readonly BlueprintItem[], b: readonly BlueprintItem[]): boolean => JSON.stringify(a) === JSON.stringify(b);

/** Items that follow a given item, directly. */
export const followersOf = (items: readonly BlueprintItem[], id: string): BlueprintItem[] =>
  items.filter((i) => i.follows?.item === id);
