// What the "FF&E tags" checkbox writes on an item: the project's own `[tags] ffe` setting, the one the main
// viewer's FF&E tags use, resolved against the blueprint's item. The blueprint carries the item's instance and
// type properties, so every field the setting can name resolves here except `$name`, which a blueprint item does
// not hold (it says nothing, as it does in the viewer for an element with no name).
//
// Duplicated from src-js/viewer/elementTags.ts rather than imported from it, as the other small helpers here are:
// this page and the viewer are separate bundles.

import type { Property } from "./types.js";

/** What the server defaults to for a project whose settings could not be read. */
export const DEFAULT_TAG_FIELDS: readonly string[] = ["$type_name"];

export interface Taggable {
  id: string;
  type_name: string;
  properties?: readonly Property[];
  type_properties?: readonly Property[];
}

/** The `[tags] ffe` fields out of the answer to `GET /api/settings/resolve/{project}`. */
export function tagFieldsOf(resolved: unknown): readonly string[] {
  const fields = (resolved as { settings?: { tags?: { ffe?: { fields?: unknown } } } } | null)?.settings?.tags?.ffe?.fields;
  const out = Array.isArray(fields) ? fields.filter((f): f is string => typeof f === "string" && f.trim() !== "") : [];
  return out.length > 0 ? out : DEFAULT_TAG_FIELDS;
}

const blank = (v: string | undefined): boolean => (v ?? "").trim() === "" || (v ?? "").trim().toLowerCase() === "none";

const valueOf = (bag: readonly Property[] | undefined, name: string): string | undefined => bag?.find((p) => p.name === name)?.value;

/** The lines an item's tag says, one per field that has something. A property is read from the instance first,
 *  then from the type, and a blank instance value does not hide a real type value. */
export function itemTagLines(fields: readonly string[], item: Taggable): string[] {
  const lines: string[] = [];
  for (const field of fields) {
    let value: string | undefined;
    if (field === "$id") value = item.id;
    else if (field === "$type_name") value = item.type_name;
    else if (field === "$name") value = undefined;
    else {
      const first = valueOf(item.properties, field);
      value = !blank(first) ? first : valueOf(item.type_properties, field);
    }
    if (!blank(value)) lines.push((value as string).trim());
  }
  return lines;
}
