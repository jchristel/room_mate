// Fetching the dissolved footprints.
//
// **On demand, and once for the page.** The dissolve is real server work, so it
// is fetched when a zone first switches its overlay on rather than with the
// rooms — and one dataset serves every zone's overlay AND the band's figures,
// because it is scope-derived and a second copy could only disagree with
// itself.

import { fetchJson } from "./api.js";
import { getState, setAreas } from "./store.js";
import type { AreasData } from "../areas.js";

/** The scope the held dataset answers, so a project or building change refetches
 *  and a second zone switching on does not. */
let loadedFor: string | null = null;

function areasUrl(): string | null {
  const { scope } = getState();
  if (!scope.projectId) return null;
  const params = new URLSearchParams();
  if (scope.building) params.set("building", scope.building);
  if (scope.milestone) params.set("milestone", scope.milestone);
  const qs = params.toString();
  return `/projects/${encodeURIComponent(scope.projectId)}/areas${qs ? `?${qs}` : ""}`;
}

export async function loadAreas(): Promise<void> {
  const url = areasUrl();
  if (!url) {
    setAreas(null);
    loadedFor = null;
    return;
  }
  if (url === loadedFor) return;
  loadedFor = url;
  try {
    setAreas(await fetchJson<AreasData>(url));
  } catch {
    // A failed dissolve leaves the overlay empty rather than throwing: the
    // plan itself is still worth reading, and the next toggle retries.
    loadedFor = null;
    setAreas(null);
  }
}
