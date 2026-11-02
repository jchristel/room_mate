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
