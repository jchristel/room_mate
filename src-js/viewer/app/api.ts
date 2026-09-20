// The reads this page makes, and the shapes they answer with.
//
// A hand-written subset of each response, like `src-js/renderer/types.ts`: the
// page touches a handful of fields and the generated settings types do not
// cover the data API. The rule for widening it is the settings page's — when a
// component needs MOST of a response, generate it.

/** GET JSON, no-store, throwing the URL and status on a non-2xx.
 *
 *  The viewer's own variant rather than `common.js`'s `apiGet`: this page shows
 *  a connection state in its own words ("connection lost") and never the
 *  server's error text, which belongs to the settings API's save path. */
export async function fetchJson<T>(url: string): Promise<T> {
  const res = await fetch(url, { cache: "no-store" });
  if (!res.ok) throw new Error(`${url} -> ${res.status}`);
  return (await res.json()) as T;
}

export interface ProjectRow {
  id: string;
  name: string;
}

export interface BuildingRow {
  key: string;
  name?: string | null;
  code?: string | null;
  unclassified?: boolean;
  ambiguous?: boolean;
}

export interface BuildingsResponse {
  tier_configured?: boolean;
  buildings: BuildingRow[];
}

export interface MilestoneRow {
  name: string;
  date?: string | null;
}

export interface MilestonesResponse {
  milestones: MilestoneRow[];
}

/** The `/rooms` payload, as far as this slice reads it. Rooms and levels are
 *  the renderer's types and arrive with the zones slice (B3). */
export interface RoomsPayload {
  /** The server's one-value content revision — what the poll compares instead
   *  of re-stringifying the payload every two seconds. */
  revision?: string;
  rooms?: unknown[];
  levels?: unknown[];
  taken_at?: string;
  schema_version?: number;
}

export const projectsUrl = "/projects";
export const buildingsUrl = (projectId: string): string =>
  `/projects/${encodeURIComponent(projectId)}/buildings`;
export const milestonesUrl = (projectId: string): string =>
  `/projects/${encodeURIComponent(projectId)}/milestones`;
