// A room's changes to single blueprint items, kept with its assignment (plan decision 6's
// "per-item overrides"; the rules are on `ItemOverride` in src/assignments.rs).
//
// **The server applies them.** This file only edits the list: the drawing is the resolver's answer
// to a request that carries the list, so a preview and a saved result cannot disagree. One override
// per item, whole millimetres and degrees, none that change nothing.

import type { ItemOverride } from "./types.js";

/** The largest nudge the server keeps; the buttons never go further than this. */
export const MAX_NUDGE_MM = 20_000;

export const isNoop = (o: ItemOverride): boolean => !o.exclude && !o.dx_mm && !o.dy_mm && !o.rotate_deg;

/** What the list says about one item, or nothing. */
export const overrideFor = (list: readonly ItemOverride[], id: string): ItemOverride | undefined => list.find((o) => o.item_id === id);

const clamp = (n: number, lim: number): number => Math.max(-lim, Math.min(lim, Math.round(n)));

/** Change one item's override by `patch` (deltas for the nudge and turn, a value for `exclude`). An
 *  override that ends up changing nothing is removed. */
export function adjust(
  list: readonly ItemOverride[],
  id: string,
  patch: { dx_mm?: number; dy_mm?: number; rotate_deg?: number; exclude?: boolean },
): ItemOverride[] {
  const now = overrideFor(list, id) ?? { item_id: id };
  const next: ItemOverride = {
    item_id: id,
    exclude: patch.exclude ?? now.exclude ?? false,
    dx_mm: clamp((now.dx_mm ?? 0) + (patch.dx_mm ?? 0), MAX_NUDGE_MM),
    dy_mm: clamp((now.dy_mm ?? 0) + (patch.dy_mm ?? 0), MAX_NUDGE_MM),
    rotate_deg: clamp((now.rotate_deg ?? 0) + (patch.rotate_deg ?? 0), 360),
  };
  const rest = list.filter((o) => o.item_id !== id);
  return isNoop(next) ? rest : [...rest, tidy(next)];
}

/** An item back as the blueprint has it. */
export const reset = (list: readonly ItemOverride[], id: string): ItemOverride[] => list.filter((o) => o.item_id !== id);

/** Drop the zero fields, as the server writes them. */
function tidy(o: ItemOverride): ItemOverride {
  const out: ItemOverride = { item_id: o.item_id };
  if (o.exclude) out.exclude = true;
  if (o.dx_mm) out.dx_mm = o.dx_mm;
  if (o.dy_mm) out.dy_mm = o.dy_mm;
  if (o.rotate_deg) out.rotate_deg = o.rotate_deg;
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

/** A short description: "moved 150, -50 mm, turned 90 degrees" or "left out". */
export function describe(o: ItemOverride): string {
  if (o.exclude) return "left out of this room";
  const parts: string[] = [];
  if (o.dx_mm || o.dy_mm) parts.push(`moved ${o.dx_mm ?? 0}, ${o.dy_mm ?? 0} mm`);
  if (o.rotate_deg) parts.push(`turned ${o.rotate_deg} degrees`);
  return parts.join(", ");
}
