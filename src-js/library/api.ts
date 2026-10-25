// The endpoints the library page calls. Writes never throw on an HTTP error: the server's own text
// is shown to the person, verbatim.

import { apiGet, apiSend } from "./common.js";
import type { ProjectRow, RoomRow, RuleSummary, SheetRecord, SheetSummary, Suggestions, TagRule } from "./types.js";

export type Sent<T> = { ok: true; value: T } | { ok: false; error: string };

const base = (project: string) => `/projects/${encodeURIComponent(project)}`;
const enc = encodeURIComponent;

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
export const listRooms = (project: string) => apiGet<{ rooms: RoomRow[] }>(`/rooms?project=${enc(project)}`).then((r) => r.rooms);

export const listSheets = (project: string) => apiGet<SheetSummary[]>(`${base(project)}/sheets`);

/** The server leaves an empty list out of a record, so read it back with the lists there. */
export const getSheet = async (project: string, id: string): Promise<SheetRecord> => {
  const r = await apiGet<Partial<SheetRecord> & Omit<SheetRecord, "pending" | "rejected">>(`${base(project)}/sheets/${enc(id)}`);
  return { ...r, pending: r.pending ?? [], rejected: r.rejected ?? [] };
};

/** Set the sheet's room by id; the server finds the room's model. */
export const setSheetRoom = (project: string, id: string, taken_at: string, roomId: string) =>
  send<SheetRecord>("PUT", `${base(project)}/sheets/${enc(id)}`, { base: taken_at, room_id: roomId });
export const clearSheetRoom = (project: string, id: string, taken_at: string) =>
  send<SheetRecord>("PUT", `${base(project)}/sheets/${enc(id)}`, { base: taken_at, clear_room: true });
export const decidePush = (project: string, id: string, pendingId: number, taken_at: string, accept: boolean) =>
  send<SheetRecord>("POST", `${base(project)}/sheets/${enc(id)}/pending/${pendingId}/${accept ? "accept" : "reject"}`, { base: taken_at });
export const deleteSheet = (project: string, id: string, taken_at: string) =>
  send<null>("DELETE", `${base(project)}/sheets/${enc(id)}?base=${enc(taken_at)}`, null);
export const suggest = (project: string, roomId: string) => apiGet<Suggestions>(`${base(project)}/sheets/suggest?room_id=${enc(roomId)}`);

export const listRules = (project: string) => apiGet<RuleSummary[]>(`${base(project)}/tagrules`);
export const getRule = (project: string, id: string) => apiGet<TagRule>(`${base(project)}/tagrules/${enc(id)}`);
export const deleteRule = (project: string, id: string) => send<null>("DELETE", `${base(project)}/tagrules/${enc(id)}`, null);
