// Room search: which fields can be searched, and which rooms match.
//
// A local substring scan over rooms already in memory — no request, no server
// index. That is the same "a presentation reshuffle of data the browser holds"
// rule the CSV export and the area tabulation follow, and it is what lets the
// result apply to what is ALREADY drawn rather than re-rendering a level per
// keystroke.

import { sourceDisplayName } from "./properties.js";
import type { Room } from "../renderer/types.js";

const FIELD_LABELS: Record<string, string> = { $name: "Name", $id: "Id", $classification: "Classification" };

/** How a field reads in the picker. A `$<source>` pseudo-field routes through
 *  the source's display name, so a newly discovered source needs no entry. */
export function searchFieldLabel(key: string, sources: readonly string[]): string {
  if (key.startsWith("$") && sources.includes(key.slice(1))) return sourceDisplayName(key.slice(1));
  return FIELD_LABELS[key] ?? key;
}

/**
 * Every field the search can look at: the intrinsic keys, one `$<source>`
 * pseudo-field per discovered reference source, and the union of raw property
 * keys across the payload.
 *
 * A pseudo-field per SOURCE rather than per joined field: a reader searching
 * dRofus means "anything dRofus knows about this room", and a checklist of
 * forty joined field names would be a worse question to have to answer.
 */
export function availableSearchFields(rooms: readonly Room[], sources: readonly string[]): string[] {
  const props = new Set<string>();
  for (const r of rooms) for (const k of Object.keys(r.properties ?? {})) props.add(k);
  return ["$name", "$id", "$classification", ...sources.map((n) => `$${n}`), ...[...props].sort()];
}

/** Whether `q` — already lowercased — is a substring of any ENABLED field. */
export function roomMatches(room: Room, q: string, fields: ReadonlySet<string>, sources: readonly string[]): boolean {
  const hit = (v: unknown) => v != null && String(v).toLowerCase().includes(q);
  if (fields.has("$name") && hit(room.name)) return true;
  if (fields.has("$id") && hit(room.id)) return true;
  if (fields.has("$classification") && room.classification?.some((t) => hit(t.name) || hit(t.code))) return true;
  for (const name of sources) {
    if (!fields.has(`$${name}`)) continue;
    const record = (room as unknown as Record<string, { fields?: Record<string, string> }>)[name];
    if (record?.fields && Object.values(record.fields).some(hit)) return true;
  }
  const props = room.properties ?? {};
  for (const k of Object.keys(props)) if (fields.has(k) && hit(props[k]?.value)) return true;
  return false;
}

/**
 * The match set over the WHOLE payload, or `null` for no query.
 *
 * Level-independent on purpose: a level switch then repaints with the same
 * set, and every zone shares it — zones differ only in which level's matches
 * are on screen.
 */
export function computeMatches(
  rooms: readonly Room[],
  query: string,
  fields: ReadonlySet<string>,
  sources: readonly string[],
): Set<string> | null {
  const q = query.trim().toLowerCase();
  if (!q) return null;
  const ids = new Set<string>();
  for (const r of rooms) if (roomMatches(r, q, fields, sources)) ids.add(r.id);
  return ids;
}
