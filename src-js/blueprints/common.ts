// What the blueprints page decides about a blueprint that is not drawing: which need a person's attention and the
// filter over the list. The shared helpers (the server calls, the filter rule) live in `src-js/ui/`.

import { matchesAll } from "../ui/match.js";

/** A blueprint a person should look at: pushed revisions waiting for a decision, or no sheet bound to
 *  it in the library (a task cannot create sheets from it). */
export const blueprintNeedsAttention = (b: { sheets: number; pending: number; in_use?: boolean }): boolean =>
  b.in_use !== false && (b.sheets === 0 || b.pending > 0);

/** Why, in a sentence. Empty when it does not. */
export function attentionReasons(b: { sheets: number; pending: number; in_use?: boolean }): string[] {
  const out: string[] = [];
  if (b.in_use === false) return out;
  if (b.sheets === 0) out.push("no room layout sheet bound to it");
  if (b.pending > 0) out.push(`${b.pending} pending revision${b.pending === 1 ? "" : "s"} to accept or reject`);
  return out;
}

/** Blueprints whose name or source room contains every word of `text` (case-insensitive). Order is kept:
 *  the list is the server's, and a filter that reorders it would move rows under the pointer. */
export function filterBlueprints<T extends { name: string; source: { room_name: string; room_id: string } | null }>(rows: readonly T[], text: string): T[] {
  return rows.filter((b) => matchesAll([b.name, b.source?.room_name ?? "", b.source?.room_id ?? ""].join(" "), text));
}
