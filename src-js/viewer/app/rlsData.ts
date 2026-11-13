// What the RLS tool asks of the server: the library's sheets, one sheet in full (for its preview), the one write it
// makes (queue a `create_rls` task per room), and the preview of a set of rooms the composer lays out.

import type { SetPreview, SetTaskBody } from "../rlsSet.js";
import type { Made, Solved } from "../rlsTool.js";
import type { SheetRecord, SheetSummary } from "../../library/types.js";
import { fetchJson } from "./api.js";

/** The library's own types, so the viewer cannot drift from the page that owns them. */
export type LibrarySheet = SheetSummary;

/** The parts of a library sheet the preview draws. */
export type LibrarySheetRecord = Pick<SheetRecord, "size_mm" | "sheet">;

const base = (project: string) => `/projects/${encodeURIComponent(project)}`;

export const listLibrarySheets = (project: string, signal?: AbortSignal) => fetchJson<LibrarySheet[]>(`${base(project)}/sheets`, signal);
export const getLibrarySheet = (project: string, id: string, signal?: AbortSignal) =>
  fetchJson<LibrarySheetRecord>(`${base(project)}/sheets/${encodeURIComponent(id)}`, signal);

/** Queue one task per room. Returns the server's answer for each room, or the text of a failure.
 *  `withTags`: work out which items are tagged where (a sheet bound to a room blueprint knows its items'
 *  tags; it reads the room's FF&E, so it is asked for only then). */
export async function makeRlsTasks(
  project: string,
  sheetId: string,
  roomIds: readonly string[],
  overwrite: boolean,
  withTags: boolean,
): Promise<Made[] | string> {
  try {
    const res = await fetch(`${base(project)}/tasks`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ kind: "create_rls", sheet_id: sheetId, room_ids: roomIds, overwrite_rls: overwrite, with_tags: withTags }),
      cache: "no-store",
    });
    return res.ok ? ((await res.json()) as Made[]) : (await res.text()).trim() || `HTTP ${res.status}`;
  } catch (err) {
    return `Could not reach the server: ${String(err)}`;
  }
}

/** The solver's closest sheets for many rooms at once. Offers only; nothing is queued. */
export async function solveRooms(project: string, roomIds: readonly string[], signal?: AbortSignal): Promise<Solved[] | string> {
  try {
    const res = await fetch(`${base(project)}/sheets/solve`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ room_ids: roomIds }),
      cache: "no-store",
      signal: signal ?? null,
    });
    return res.ok ? ((await res.json()) as Solved[]) : (await res.text()).trim() || `HTTP ${res.status}`;
  } catch (err) {
    return signal?.aborted ? "" : `Could not reach the server: ${String(err)}`;
  }
}

/** The server's preview of a set of adjacent rooms: whether they make one, and the views it gets with their sizes on
 *  paper. Stores nothing. `sheetId` null: the best-matching library sheet. */
export async function previewSet(project: string, roomIds: readonly string[], sheetId: string | null, signal?: AbortSignal): Promise<SetPreview | string> {
  try {
    const res = await fetch(`${base(project)}/tasks/rls-set-preview`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(sheetId ? { room_ids: roomIds, sheet_id: sheetId } : { room_ids: roomIds }),
      cache: "no-store",
      signal: signal ?? null,
    });
    return res.ok ? ((await res.json()) as SetPreview) : (await res.text()).trim() || `HTTP ${res.status}`;
  } catch (err) {
    return signal?.aborted ? "" : `Could not reach the server: ${String(err)}`;
  }
}

/** Queue the one task of a set as laid out. The answer is one row: the task, or why it was refused. */
export async function makeRlsSetTask(project: string, body: SetTaskBody): Promise<Made[] | string> {
  try {
    const res = await fetch(`${base(project)}/tasks`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
      cache: "no-store",
    });
    return res.ok ? ((await res.json()) as Made[]) : (await res.text()).trim() || `HTTP ${res.status}`;
  } catch (err) {
    return `Could not reach the server: ${String(err)}`;
  }
}
