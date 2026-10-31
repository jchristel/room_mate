// What the RLS tool asks of the server: the library's sheets, one sheet in full (for its preview), and the
// one write it makes (queue a `create_rls` task per room).

import type { Made } from "../rlsTool.js";
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
