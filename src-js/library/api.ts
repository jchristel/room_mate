// The endpoints the library page calls. Writes never throw on an HTTP error: the server's own text
// is shown to the person, verbatim.

import { send, projectBase as base, type Sent } from "../ui/api.js";

export type { Sent };
import { apiGet } from "../ui/common.js";
import type { PushOutcome } from "./exchange.js";
import type { ExportedRule, ImportOutcome } from "./rulesExchange.js";
import type { ProjectRow, RoomRow, RuleSummary, SheetRecord, SheetSummary, Suggestions, TagRule } from "./types.js";

const enc = encodeURIComponent;

export const listProjects = () => apiGet<ProjectRow[]>("/projects");
export const listRooms = (project: string) => apiGet<{ rooms: RoomRow[] }>(`/rooms?project=${enc(project)}`).then((r) => r.rooms);

export const listSheets = (project: string) => apiGet<SheetSummary[]>(`${base(project)}/sheets`);

/** The server leaves an empty list out of a record, so read it back with the lists there. */
/** The project's room blueprints, for naming the one a sheet is bound to. */
export const listBlueprintNames = (project: string) =>
  apiGet<{ id: string; name: string }[]>(`${base(project)}/blueprints`).then((rows) => new Map(rows.map((r) => [r.id, r.name])));

export const getSheet = async (project: string, id: string): Promise<SheetRecord> => {
  const r = await apiGet<Partial<SheetRecord> & Omit<SheetRecord, "pending" | "rejected">>(`${base(project)}/sheets/${enc(id)}`);
  return { ...r, pending: r.pending ?? [], rejected: r.rejected ?? [] };
};

export const decidePush = (project: string, id: string, pendingId: number, taken_at: string, accept: boolean) =>
  send<SheetRecord>("POST", `${base(project)}/sheets/${enc(id)}/pending/${pendingId}/${accept ? "accept" : "reject"}`, { base: taken_at });
/** The extractor's own push, which an import reuses: identity, renumbering and the pending decision are the server's. */
export const pushSheets = (project: string, sheets: readonly unknown[]) => send<PushOutcome[]>("POST", `${base(project)}/sheets`, { sheets });
export const deleteSheet = (project: string, id: string, taken_at: string) =>
  send<null>("DELETE", `${base(project)}/sheets/${enc(id)}?base=${enc(taken_at)}`, null);
export const suggest = (project: string, roomId: string) => apiGet<Suggestions>(`${base(project)}/sheets/suggest?room_id=${enc(roomId)}`);

export const listRules = (project: string) => apiGet<RuleSummary[]>(`${base(project)}/tagrules`);
export const getRule = (project: string, id: string) => apiGet<TagRule>(`${base(project)}/tagrules/${enc(id)}`);
/** The library's own rule import (the extractor's route takes tags read from Revit, never stored rules). */
export const importRules = (project: string, rules: readonly ExportedRule[]) => send<ImportOutcome>("POST", `${base(project)}/tagrules/import`, { rules });
export const deleteRule = (project: string, id: string) => send<null>("DELETE", `${base(project)}/tagrules/${enc(id)}`, null);

