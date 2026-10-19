// The blueprint endpoints, as the page calls them. Every write names the `taken_at` it read
// (`base`), and a 409 is shown to the person verbatim: two people editing one layout is the
// ordinary case for authored data, and the last writer winning unseen is what would make
// anyone distrust it.

import { apiGet, apiSend } from "./common.js";
import type {
  Blueprint,
  BlueprintSummary,
  CaptureResponse,
  ProjectRow,
  RevisionContent,
  RoomRow,
  SourceRoom,
} from "./types.js";

export type Sent<T> = { ok: true; value: T } | { ok: false; error: string };

const base = (project: string) => `/projects/${encodeURIComponent(project)}`;

async function send<T>(method: string, url: string, body: unknown): Promise<Sent<T>> {
  const r = await apiSend(method, url, body);
  if (!r.ok) return { ok: false, error: r.text || `HTTP ${r.status}` };
  try {
    return { ok: true, value: (r.text ? JSON.parse(r.text) : null) as T };
  } catch {
    return { ok: false, error: "the server's answer was not JSON" };
  }
}

export const listProjects = () => apiGet<ProjectRow[]>("/projects");
export const listBlueprints = (project: string) => apiGet<BlueprintSummary[]>(`${base(project)}/blueprints`);
export const getBlueprint = (project: string, id: string) =>
  apiGet<Blueprint>(`${base(project)}/blueprints/${encodeURIComponent(id)}`);

/** The plan's rooms. A row names no model, so a room is addressed by id alone and the server
 *  looks it up (refusing, naming the models, if more than one has it). */
export const listRooms = (project: string) =>
  apiGet<{ rooms: RoomRow[]; levels: { id: string; name: string }[] }>(`/rooms?project=${encodeURIComponent(project)}`);

export const capture = (project: string, roomId: string) =>
  apiGet<CaptureResponse>(`${base(project)}/blueprints/capture?room_id=${encodeURIComponent(roomId)}`);

export const createBlueprint = (project: string, name: string, source: SourceRoom | null, content: RevisionContent, note?: string) =>
  send<Blueprint>("POST", `${base(project)}/blueprints`, { name, source, note, ...content });

export const addRevision = (project: string, id: string, baseStamp: string, content: RevisionContent, note: string | null) =>
  send<Blueprint>("POST", `${base(project)}/blueprints/${encodeURIComponent(id)}/revisions`, {
    base: baseStamp,
    note,
    make_current: true,
    ...content,
  });

export const updateBlueprint = (
  project: string,
  id: string,
  baseStamp: string,
  patch: { name?: string; current_revision?: number },
) => send<Blueprint>("PUT", `${base(project)}/blueprints/${encodeURIComponent(id)}`, { base: baseStamp, ...patch });

export const deleteBlueprint = (project: string, id: string, baseStamp: string) =>
  send<null>("DELETE", `${base(project)}/blueprints/${encodeURIComponent(id)}?base=${encodeURIComponent(baseStamp)}`, null);

export const pushPending = (project: string, id: string, origin: string, content: RevisionContent, note?: string) =>
  send<Blueprint>("POST", `${base(project)}/blueprints/${encodeURIComponent(id)}/pending`, { origin, note, ...content });

export const decidePending = (project: string, id: string, pendingId: number, baseStamp: string, accept: boolean) =>
  send<Blueprint>(
    "POST",
    `${base(project)}/blueprints/${encodeURIComponent(id)}/pending/${pendingId}/${accept ? "accept" : "reject"}`,
    { base: baseStamp },
  );
