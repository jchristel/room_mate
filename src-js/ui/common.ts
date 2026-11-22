// What every page of the app shares: the four helpers it takes from static/common.js, typed.
//
// **Reused, not ported.** common.js is a classic script each page loads (index.html), so the project-selection rule
// and the API error convention keep exactly one definition. Exposed as typed exports rather than `declare function`
// globals so only the files that import this see them: the renderer bundle shares the tsconfig and has no business
// with any of it.
//
// **One copy, in `src-js/ui/`** (2026-11-22). Each page used to carry its own copy of this and of its API helpers, on
// the rule that two small copies beat a module two pages depend on. At five pages the copies drifted and every new page
// started with another one; what the pages hold in common lives here now, and a page keeps only what is its own.

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
