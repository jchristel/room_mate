// Asking the server for a zone's reach, shared by the bar's Measure button and the
// saved-reaches panel's "show".
//
// Lives outside the bar so a saved reach can be put into a zone and measured without
// the bar being the thing that knows how. One request per zone at a time: starting
// another, or any edit, aborts the one in flight, and an answer that lands after is
// patched into the zone's CURRENT state, not the one the request was made from.

import { reachRequest, type ReachBody, type ReachState } from "../reach.js";
import type { Scope } from "../scope.js";
import { getState, patchZoneReach } from "./store.js";

const inFlight = new Map<string, AbortController>();

/** The zone's reach state now, or null when the tool is off. */
export const currentReach = (zoneId: string): ReachState | null =>
  getState().zones.find((z) => z.id === zoneId)?.reach ?? null;

/** Drop the request in flight for a zone, if any. */
export function abortReach(zoneId: string): void {
  inFlight.get(zoneId)?.abort();
  inFlight.delete(zoneId);
}

/** Measure the zone's reach as it is now and put the answer in the zone. */
export async function measureReach(zoneId: string, scope: Scope): Promise<void> {
  const now = currentReach(zoneId);
  const req = now ? reachRequest(scope, now) : null;
  if (!now || !req) return;
  abortReach(zoneId);
  const ac = new AbortController();
  inFlight.set(zoneId, ac);
  patchZoneReach(zoneId, { ...now, result: { state: "loading" } });
  let next: ReachState["result"];
  try {
    const res = await fetch(req.url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: req.body,
      cache: "no-store",
      signal: ac.signal,
    });
    if (res.status === 204) next = { state: "error", message: "No rooms have been pushed to this project yet." };
    else if (!res.ok) next = { state: "error", message: (await res.text()).trim() || `${req.url} -> ${res.status}` };
    else next = { state: "done", body: (await res.json()) as ReachBody };
  } catch (err) {
    if (ac.signal.aborted) return;
    next = { state: "error", message: `Could not read ${req.url}: ${String(err)}` };
  }
  const after = currentReach(zoneId);
  if (after && !ac.signal.aborted) patchZoneReach(zoneId, { ...after, result: next });
  if (inFlight.get(zoneId) === ac) inFlight.delete(zoneId);
}
