// The 2-second tick: refresh the scope lists, then read the rooms.
//
// **`setInterval`, not a chain of timeouts**, and a re-entry guard, exactly as
// the old page has it: the interval fires on the clock while a tick is three
// awaited requests, so without the guard a slow server queues ticks that then
// all land together.
//
// The rooms read keeps TWO cursors, and they answer different questions. The
// ETag is the server's; it decides whether a body is sent at all (304). The
// `revision` is the payload's; it decides whether what arrived is DIFFERENT
// from what is on screen, which a 200 alone does not say. Collapsing them
// would either re-render on every tick or miss a push — the argument is
// written out once on `EntityPoll`, which keeps the same pair per layer.
//
// The ETag is NOT cleared on a scope change: it was issued for the old
// project/building/milestone, so it cannot match the new one and the server
// answers 200. That makes "a stale 304 hides a scope switch" structurally
// impossible rather than something a reset has to remember.

import {
  buildingsUrl,
  fetchJson,
  milestonesUrl,
  projectsUrl,
  type BuildingsResponse,
  type MilestonesResponse,
  type ProjectRow,
  type RoomsPayload,
} from "./api.js";
import { persistSelection, seedProjectId, urlParam } from "./common.js";
import { getState, setRoomsLoading, setState } from "./store.js";
import { loadAppearance } from "./appearance.js";
import { pollLayers } from "./layers.js";
import { keepBuilding, keepMilestone, resolveProject, roomsUrl, type Scope } from "../scope.js";
import { elementUrl, visibleStoreys } from "../elementUrls.js";

const TICK_MS = 2000;

let inFlight = false;
let roomsEtag: string | null = null;
let revision: string | null = null;
/** The rooms URL whose answer (a payload or a 204) is on screen, so the
 *  loading bar can tell a scope change from the 2-second revalidation. */
let roomsAccepted: string | null = null;
/** One-shot: the URL/localStorage restore applies on the first resolve only, so
 *  the 2-second refresh can never clobber a later manual change. */
let seeded = false;
/** The project whose appearance is loaded, so the fetch happens on a project
 *  change rather than on every tick. */
let appearanceFor: string | null = null;
/** The element reads' last URL scope, so a storey switch re-reads and a quiet
 *  tick does not. */
let lastStoreyKey: string | null = null;

/** Force the next tick to treat whatever arrives as new. Called when the scope
 *  changes, so the page repaints at once rather than after the next push. */
export function invalidate(): void {
  revision = null;
}

/** Apply a scope change from a picker: store it, mirror it into the URL, and
 *  make the next tick render. The building and milestone lists are reloaded by
 *  the tick itself, which is where their "does the server still offer this"
 *  checks live. */
export async function setScope(next: Scope): Promise<void> {
  setState({ scope: next });
  persistSelection(next.projectId, { building: next.building, milestone: next.milestone });
  invalidate();
  await tick();
}

async function refreshScope(projects: readonly ProjectRow[]): Promise<void> {
  const projectId = resolveProject(projects, getState().scope.projectId, seedProjectId(), seeded);

  if (!projectId) {
    setState({ projects, scope: { ...getState().scope, projectId }, buildings: [], milestones: [] });
    return;
  }

  // Both lists, every tick: a project gains a building the moment a model
  // carrying one is pushed, and the picker that does not know it cannot be
  // asked for it. The responses are small and the server answers them from the
  // index; the old page guarded these with signature strings only to avoid
  // rebuilding <option> nodes under the user's cursor, which React does not
  // have to care about.
  const [buildings, milestones] = await Promise.all([
    fetchJson<BuildingsResponse>(buildingsUrl(projectId)).catch(() => null),
    fetchJson<MilestonesResponse>(milestonesUrl(projectId)).catch(() => null),
  ]);
  const buildingRows = buildings?.buildings ?? getState().buildings;
  const milestoneRows = milestones?.milestones ?? getState().milestones;

  // **Read the scope again, AFTER the awaits.** A picker change lands while a
  // tick is in flight — `setScope` writes the new scope and the tick is already
  // past its own read — so a scope captured at the top and written back here
  // silently reverts the user's pick. Measured on RHH: selecting a building put
  // it in the URL and then the next tick dropped it, so the select snapped back
  // to "All buildings" and the rooms never re-scoped.
  const live = getState().scope;
  let next: Scope = { ...live, projectId };

  if (!seeded) {
    // Once, after the lists exist: a deep link may name a building or a
    // milestone, and it is only honoured if the freshly-loaded list still
    // offers it. URL-only, never localStorage — they are per project.
    seeded = true;
    next = {
      projectId,
      building: keepBuilding(buildingRows, urlParam("building") ?? next.building),
      milestone: keepMilestone(milestoneRows, urlParam("milestone") ?? next.milestone),
    };
    // Mirror the RESOLVED scope back, so a deep link carrying a stale building
    // does not leave the URL claiming a scope the page is not showing, and so
    // the project reaches the other pages' localStorage seed.
    persistSelection(next.projectId, { building: next.building, milestone: next.milestone });
    invalidate();
  } else {
    // A selection the server stopped offering falls away rather than silently
    // scoping every read to something that no longer exists.
    const building = keepBuilding(buildingRows, next.building);
    const milestone = keepMilestone(milestoneRows, next.milestone);
    if (building !== next.building || milestone !== next.milestone) invalidate();
    next = { projectId, building, milestone };
  }

  setState({ projects, scope: next, buildings: buildingRows, milestones: milestoneRows });

  // Once per project change, not per tick: appearance is a property of the
  // project, and the settings read is the viewer's only use of that API.
  if (next.projectId !== appearanceFor) {
    appearanceFor = next.projectId;
    await loadAppearance(next.projectId);
  }
}

async function pollRooms(): Promise<void> {
  const url = roomsUrl(getState().scope);
  // Cleared however the read ends -- a thrown one included, or the bar would
  // run on under "connection lost".
  const fresh = url !== roomsAccepted;
  if (fresh) setRoomsLoading(true);
  try {
    await readRooms(url);
  } finally {
    if (fresh) setRoomsLoading(false);
  }
}

async function readRooms(url: string): Promise<void> {
  const headers: Record<string, string> = roomsEtag ? { "If-None-Match": roomsEtag } : {};
  const res = await fetch(url, { cache: "no-store", headers });

  if (res.status === 204) {
    // The scope holds no rooms snapshot at all — an ordinary state for a
    // project nobody has pushed to yet, not a failure.
    roomsEtag = null;
    roomsAccepted = url;
    setState({ status: "waiting for data", payload: null });
    return;
  }
  if (res.status !== 304 && !res.ok) throw new Error(String(res.status));

  if (res.status === 304) {
    roomsAccepted = url;
    setState({ status: "ready" });
    return;
  }

  const payload = (await res.json()) as RoomsPayload;
  // After the parse, never before: a tag stored ahead of a body that failed to
  // arrive turns every later tick into a 304 for data that never landed.
  roomsEtag = res.headers.get("ETag");
  roomsAccepted = url;
  const incoming = payload.revision ?? JSON.stringify(payload);
  if (incoming === revision) {
    setState({ status: "ready" });
    return;
  }
  revision = incoming;
  setState({ status: "ready", payload, updatedAt: new Date() });
}

export async function tick(): Promise<void> {
  if (inFlight) return;
  inFlight = true;
  try {
    // A failed project list is not a failed tick: the rooms read below reports
    // the connection, in the words the reader sees.
    const projects = await fetchJson<ProjectRow[]>(projectsUrl).catch(() => null);
    if (projects) await refreshScope(projects);
    // Every layer is asked on every tick and only re-renders when IT changed.
    // Before the rooms read, so a tick that changed both paints once with both
    // rather than painting rooms and then repainting to add the elements.
    await pollLayers();
    await pollRooms();
  } catch {
    setState({ status: "connection lost" });
  } finally {
    inFlight = false;
  }
}

/**
 * Re-read the element layers because the storeys on screen moved — a level
 * switch, a zone added or removed, or the first rooms payload choosing each
 * zone's level.
 *
 * Keyed on the URL the reads would use: that string IS the scope plus the
 * storeys, so an unchanged key means nothing to ask for. Before the first rooms
 * payload the key is null and nothing is asked, which is what stops a load
 * fetching every storey once.
 */
export async function onStoreysChanged(): Promise<void> {
  const key = elementUrl("doors", getState().scope, storeysKey());
  if (key === null || key === lastStoreyKey) return;
  lastStoreyKey = key;
  await pollLayers();
}

function storeysKey() {
  const { payload, zones } = getState();
  return visibleStoreys(
    payload?.levels ?? null,
    zones.map((z) => z.levelId),
  );
}

/** Start the loop. Returns the stop function, for React's effect cleanup —
 *  which matters under StrictMode, where an effect runs twice in development
 *  and a loop without cleanup would double the request rate. */
export function startPolling(): () => void {
  void tick();
  const id = setInterval(() => void tick(), TICK_MS);
  return () => clearInterval(id);
}
