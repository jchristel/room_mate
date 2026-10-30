// The four helpers this page takes from static/common.js, typed.
//
// **Reused, not ported.** common.js is a classic script the viewer loads; this
// page loads it the same way (index.html), so the project-selection rule and
// the API error convention keep exactly one definition.
//
// Duplicated from src-js/settings/common.ts rather than hoisted to a shared
// module, per the house rule on shared helpers: two pages, two small copies,
// no third module for either to depend on.

export interface SendResult {
  ok: boolean;
  status: number;
  text: string;
}

interface CommonJs {
  apiGet(url: string): Promise<unknown>;
  apiSend(method: string, url: string, body: unknown): Promise<SendResult>;
  seedProjectId(): string | null;
  persistSelection(projectId: string | null): void;
}

const common = globalThis as unknown as CommonJs;

/** GET JSON with no-store; throws the server's own error text on a non-2xx. */
export const apiGet = <T>(url: string): Promise<T> => common.apiGet(url) as Promise<T>;

/** Send JSON; never throws on an HTTP error, so a 422's text can be shown verbatim. */
export const apiSend = (method: string, url: string, body: unknown): Promise<SendResult> =>
  common.apiSend(method, url, body);

/** URL `?project=`, then localStorage, then null — a candidate only. */
export const seedProjectId = (): string | null => common.seedProjectId();

/** Mirror the pick into the URL (replaceState) and localStorage. */
export const persistSelection = (projectId: string | null): void => common.persistSelection(projectId);

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
  const words = text.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return [...rows];
  return rows.filter((b) => {
    const hay = [b.name, b.source?.room_name ?? "", b.source?.room_id ?? ""].join(" ").toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}
