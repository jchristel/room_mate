// What the blueprint tool asks of the server: the project's blueprints, the resolver's answer for
// one blueprint in one room, and the one write it makes (assigning).
//
// One resolve per zone at a time (the reach tool's rule): asking again, or any change to what is
// asked, aborts the one in flight, and an answer that lands after is patched into the zone's
// CURRENT state rather than the one the request was made from.

import { resolveUrl, type BlueprintToolState, type PlanXY, type ResolveBody } from "../blueprintTool.js";
import { fetchJson } from "./api.js";
import { getState, patchZoneBlueprint } from "./store.js";

export interface BlueprintRow {
  id: string;
  name: string;
  current_revision: number;
  items: number;
  sheets: number;
}

export interface AssignmentLite {
  blueprint_id: string;
  blueprint_name: string | null;
  revision: number;
  room: { model_id: string; room_id: string };
}

const base = (project: string) => `/projects/${encodeURIComponent(project)}`;

export const listBlueprints = (project: string, signal?: AbortSignal) => fetchJson<BlueprintRow[]>(`${base(project)}/blueprints`, signal);
export const listAssignments = (project: string, signal?: AbortSignal) => fetchJson<AssignmentLite[]>(`${base(project)}/assignments`, signal);

/** The blueprint's coarse outline for a revision (the current one when `revision` is null), in mm:
 *  what a box laid over a room is made of. */
export async function loadOutline(project: string, blueprintId: string, revision: number | null, signal?: AbortSignal): Promise<PlanXY[]> {
  const bp = await fetchJson<{ current_revision: number; revisions: { number: number; outline: { coarse: PlanXY[] } }[] }>(
    `${base(project)}/blueprints/${encodeURIComponent(blueprintId)}`,
    signal,
  );
  const wanted = revision ?? bp.current_revision;
  const found = bp.revisions.find((r) => r.number === wanted);
  if (!found) throw new Error(`the blueprint has no revision ${wanted}`);
  return found.outline.coarse;
}

const inFlight = new Map<string, AbortController>();

/** The zone's blueprint tool state now, or null when the tool is off. */
export const currentTool = (zoneId: string): BlueprintToolState | null => getState().zones.find((z) => z.id === zoneId)?.blueprint ?? null;

export function abortResolve(zoneId: string): void {
  inFlight.get(zoneId)?.abort();
  inFlight.delete(zoneId);
}

/** Ask for the zone's blueprint in the zone's room and put the answer in the zone. */
export async function resolveTool(zoneId: string, project: string): Promise<void> {
  const now = currentTool(zoneId);
  const url = now ? resolveUrl(project, now) : null;
  if (!now || !url) return;
  abortResolve(zoneId);
  const ac = new AbortController();
  inFlight.set(zoneId, ac);
  patchZoneBlueprint(zoneId, { ...now, result: { state: "loading" } });
  let next: BlueprintToolState["result"];
  try {
    next = { state: "done", body: await fetchJson<ResolveBody>(url, ac.signal) };
  } catch (err) {
    if (ac.signal.aborted) return;
    next = { state: "error", message: String((err as Error).message ?? err) };
  }
  const after = currentTool(zoneId);
  if (after && !ac.signal.aborted) patchZoneBlueprint(zoneId, { ...after, result: next });
  if (inFlight.get(zoneId) === ac) inFlight.delete(zoneId);
}

/** Assign the tool's blueprint to its room at the rotation it shows. Returns the server's refusal,
 *  or null when it was saved. The room's item overrides are not sent, so the server keeps them. */
export async function assignTool(project: string, s: BlueprintToolState): Promise<string | null> {
  if (!s.blueprintId || !s.roomId) return "choose a blueprint and a room first";
  const body: Record<string, unknown> = {
    blueprint_id: s.blueprintId,
    room_id: s.roomId,
    mapping: s.frame ? { frame: s.frame } : s.shift === null ? {} : { shift: s.shift },
  };
  if (s.revision !== null) body.revision = s.revision;
  try {
    const res = await fetch(`${base(project)}/assignments`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
      cache: "no-store",
    });
    return res.ok ? null : (await res.text()).trim() || `HTTP ${res.status}`;
  } catch (err) {
    return `Could not reach the server: ${String(err)}`;
  }
}
