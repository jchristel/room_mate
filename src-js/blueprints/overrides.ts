// A room's changes to single blueprint items, kept with its assignment (plan decision 6's
// "per-item overrides"; the rules are on `ItemOverride` in src/assignments.rs).
//
// **The server applies them.** This file only edits the list: the drawing is the resolver's answer
// to a request that carries the list, so a preview and a saved result cannot disagree. One override
// per item, whole millimetres and degrees, none that change nothing.
//
// **A nudge is written in the item's own frame** (`along_mm`, `off_mm`: along the wall it stands against, and into the
// room), so it follows the room when the room is turned. `dx_mm`/`dy_mm` (the plan frame) are what was stored before; they
// are kept and shown, never written by the buttons.
//
// **`base` is the recipe the nudge was made against.** The server stamps it and never overwrites one sent, so this file
// keeps it on an override it did not edit, and DROPS it on one a person has just edited (the edit is against the recipe
// they are looking at now, so the server stamps it afresh).

import type { ItemOverride } from "./types.js";

/** The largest nudge the server keeps; the buttons never go further than this. */
export const MAX_NUDGE_MM = 20_000;

export const isNoop = (o: ItemOverride): boolean =>
  !o.exclude && !o.dx_mm && !o.dy_mm && !o.along_mm && !o.off_mm && !o.rotate_deg;

/** What the list says about one item, or nothing. */
export const overrideFor = (list: readonly ItemOverride[], id: string): ItemOverride | undefined => list.find((o) => o.item_id === id);

const clamp = (n: number, lim: number): number => Math.max(-lim, Math.min(lim, Math.round(n)));

type Patch = { dx_mm?: number; dy_mm?: number; along_mm?: number; off_mm?: number; rotate_deg?: number; exclude?: boolean };

/** Change one item's override by `patch` (deltas for the nudge and turn, a value for `exclude`). An
 *  override that ends up changing nothing is removed. */
export function adjust(list: readonly ItemOverride[], id: string, patch: Patch): ItemOverride[] {
  const now = overrideFor(list, id) ?? { item_id: id };
  const next: ItemOverride = {
    item_id: id,
    exclude: patch.exclude ?? now.exclude ?? false,
    dx_mm: clamp((now.dx_mm ?? 0) + (patch.dx_mm ?? 0), MAX_NUDGE_MM),
    dy_mm: clamp((now.dy_mm ?? 0) + (patch.dy_mm ?? 0), MAX_NUDGE_MM),
    along_mm: clamp((now.along_mm ?? 0) + (patch.along_mm ?? 0), MAX_NUDGE_MM),
    off_mm: clamp((now.off_mm ?? 0) + (patch.off_mm ?? 0), MAX_NUDGE_MM),
    rotate_deg: clamp((now.rotate_deg ?? 0) + (patch.rotate_deg ?? 0), 360),
  };
  // A leave-out does not depend on where the item is, so its base is kept; a nudge or turn is made against what is
  // on screen now.
  const keepsBase = patch.exclude !== undefined && Object.keys(patch).length === 1;
  if (keepsBase && now.base) next.base = now.base;
  const rest = list.filter((o) => o.item_id !== id);
  return isNoop(next) ? rest : [...rest, tidy(next)];
}

/** An item back as the blueprint has it. */
export const reset = (list: readonly ItemOverride[], id: string): ItemOverride[] => list.filter((o) => o.item_id !== id);

/** Keep a held nudge on the item's new recipe: forget what it was made against, and the server stamps the recipe it
 *  now sits on. The person has looked at the item where it is and said the nudge still fits. */
export const keep = (list: readonly ItemOverride[], id: string): ItemOverride[] =>
  list.map((o) => {
    if (o.item_id !== id) return o;
    const { base: _base, ...rest } = o;
    return rest;
  });

/** Drop the zero fields, as the server writes them. */
function tidy(o: ItemOverride): ItemOverride {
  const out: ItemOverride = { item_id: o.item_id };
  if (o.exclude) out.exclude = true;
  if (o.dx_mm) out.dx_mm = o.dx_mm;
  if (o.dy_mm) out.dy_mm = o.dy_mm;
  if (o.along_mm) out.along_mm = o.along_mm;
  if (o.off_mm) out.off_mm = o.off_mm;
  if (o.rotate_deg) out.rotate_deg = o.rotate_deg;
  if (o.base) out.base = o.base;
  return out;
}

/** Whether two lists mean the same thing, whatever their order. */
export function sameOverrides(a: readonly ItemOverride[], b: readonly ItemOverride[]): boolean {
  const key = (l: readonly ItemOverride[]) =>
    JSON.stringify(
      l
        .map(tidy)
        .sort((x, y) => x.item_id.localeCompare(y.item_id)),
    );
  return key(a) === key(b);
}

/** The query value the resolve read takes; absent when there is nothing to preview. */
export const encode = (list: readonly ItemOverride[]): string | undefined => (list.length === 0 ? undefined : JSON.stringify(list.map(tidy)));

/** A short description: "moved 150 along, 40 into the room, turned 90 degrees" or "left out". */
export function describe(o: ItemOverride): string {
  if (o.exclude) return "left out of this room";
  const parts: string[] = [];
  if (o.along_mm || o.off_mm) parts.push(`moved ${o.along_mm ?? 0} along its wall, ${o.off_mm ?? 0} into the room`);
  if (o.dx_mm || o.dy_mm) parts.push(`moved ${o.dx_mm ?? 0}, ${o.dy_mm ?? 0} mm on the plan`);
  if (o.rotate_deg) parts.push(`turned ${o.rotate_deg} degrees`);
  return parts.join(", ");
}
