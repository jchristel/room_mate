// The two helpers this page takes from `static/common.js`, typed.
//
// **Reused, not ported** — the settings page's `common.ts` says why at length:
// the project-selection rule has one definition, and four pages load the
// classic script that holds it. A copy here would be the fifth page's own
// answer to "which project was chosen", which is exactly the drift the shared
// file exists to prevent.
//
// Its own file rather than an import from `src-js/settings/`: a page importing
// another page's module couples their builds, and the house rule duplicates a
// small helper per module rather than hoisting it.

interface CommonJs {
  seedProjectId(): string | null;
  persistSelection(projectId: string | null, extraParams?: Record<string, string | null>): void;
  urlParam(name: string): string | null;
}

const common = globalThis as unknown as CommonJs;

/** URL `?project=`, then `localStorage`, then null — a candidate only, never
 *  a decision: `scope.resolveProject` checks it against what the server lists. */
export const seedProjectId = (): string | null => common.seedProjectId();

/** Mirror the scope into the URL (`replaceState`) and the project into
 *  `localStorage`. Building and milestone are URL-only, deliberately: they are
 *  per-project and viewer-specific, so carrying them to the settings page
 *  would scope a page that has no such concept. */
export const persistSelection = (
  projectId: string | null,
  extraParams: Record<string, string | null> = {},
): void => common.persistSelection(projectId, extraParams);

/** One query parameter of the current URL, or null. */
export const urlParam = (name: string): string | null => common.urlParam(name);
