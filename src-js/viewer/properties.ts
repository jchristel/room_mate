// Property rows: which of them are worth showing, and which joined sources a
// payload carries.
//
// **Hide-empty is on by default, and the reason is one detail**: Revit emits
// the literal string "None" for an unset parameter, so an emptiness test that
// only checks for "" hides nothing at all. A room carries 45 properties on
// House A and 19 of them hold no real value on ANY room; without the "None"
// case the default would be pointless and the panel unreadable.

import type { Room } from "../renderer/types.js";

/** A property value with nothing in it: blank, or Revit's literal "None". */
export function isEmptyPropValue(v: string | null | undefined): boolean {
  const s = (v ?? "").trim();
  return s === "" || s.toLowerCase() === "none";
}

export interface FilterState {
  /** Substring match on the property NAME. */
  filter: string;
  hideEmpty: boolean;
}

/**
 * Apply the panel's filters to `[key, value]` rows.
 *
 * `hideEmpty: false` exempts a section. Classification uses it: hide-empty
 * exists for the 45 model properties, and applying it to a two-row tier path
 * deleted the whole section for every room whose tiers are resolved-but-
 * `undefined`. "This room is unclassified" is a fact worth showing — the areas
 * service treats the undefined bucket as a real group — not an empty value.
 */
export function applyFilters(
  entries: readonly (readonly [string, string])[],
  state: FilterState,
  opts: { hideEmpty?: boolean } = {},
): (readonly [string, string])[] {
  const q = state.filter.trim().toLowerCase();
  const hideEmpty = (opts.hideEmpty ?? true) && state.hideEmpty;
  return entries.filter(([k, v]) => (!q || k.toLowerCase().includes(q)) && (!hideEmpty || !isEmptyPropValue(v)));
}

/** A property map as sorted `[key, value]` rows. */
export function propertyRows(properties: Room["properties"] | undefined): (readonly [string, string])[] {
  return Object.entries(properties ?? {})
    .map(([k, v]) => [k, v?.value ?? ""] as const)
    .sort((a, b) => a[0].localeCompare(b[0]));
}

/** `RoomResponse`'s own fields. Everything else on a room object is either a
 *  reference-source sub-object or a plain response field — `room` and
 *  `reference` are both flattened onto one JSON object by the server. */
const ROOM_FIXED_KEYS = new Set(["id", "name", "level_id", "loops", "properties", "classification", "label"]);

/**
 * Every joined reference source the payload carries, sorted.
 *
 * Data-driven rather than configured: any room key shaped like
 * `{ fields: {...} }` is a reference record, plus every source named in
 * `reference_labels` — so a source whose rows all failed to match is still
 * discovered, and the panel can say "not joined" rather than omitting it.
 */
export function detectReferenceSources(payload: {
  reference_labels?: Record<string, Record<string, unknown>>;
  rooms?: readonly Room[];
} | null): string[] {
  const names = new Set<string>();
  for (const perSource of Object.values(payload?.reference_labels ?? {})) {
    for (const name of Object.keys(perSource)) names.add(name);
  }
  for (const room of payload?.rooms ?? []) {
    for (const [key, value] of Object.entries(room as unknown as Record<string, unknown>)) {
      if (ROOM_FIXED_KEYS.has(key)) continue;
      if (value && typeof value === "object" && !Array.isArray(value) && "fields" in value) {
        if (typeof (value as { fields: unknown }).fields === "object") names.add(key);
      }
    }
  }
  return [...names].sort();
}

const DISPLAY_NAMES: Record<string, string> = { drofus: "dRofus" };

/** How a source is named on screen. Capitalised, except where the product has
 *  a spelling of its own. */
export function sourceDisplayName(name: string): string {
  return DISPLAY_NAMES[name] ?? name.charAt(0).toUpperCase() + name.slice(1);
}

/** One room's fields from a joined source, sorted, or `null` when the room
 *  matched no record — which is a real and common state, and a different
 *  statement from "matched but blank". */
export function referenceRows(room: Room, source: string): (readonly [string, string])[] | null {
  const record = (room as unknown as Record<string, { fields?: Record<string, string> }>)[source];
  if (!record) return null;
  return Object.entries(record.fields ?? {})
    .map(([k, v]) => [k, v ?? ""] as const)
    .sort((a, b) => a[0].localeCompare(b[0]));
}
