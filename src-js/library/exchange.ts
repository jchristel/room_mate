// The library's sheets as a file, and a file back into the library through the extractor's own push (`POST /sheets`):
// identity, renumbering and the pending decision are the server's, so an import needs no endpoint of its own.
//
// **A file is room-agnostic.** A sheet is self-contained: its room outline, figures and labels ride the record. The two
// REFERENCES it may also carry, `room` (a model and room id) and `blueprint` (an id), mean something only in the project
// that holds them and the server never checks either on a push, so an export leaves both out. An import is then a bare
// push, which never unbinds a sheet the library already has bound to a blueprint.

import { safeName } from "../ui/file.js";
import type { SheetRecord } from "./types.js";

export const FILE_FORMAT = "roommate-sheets";

/** A record without its history (pending and rejected pushes are not library content) and without its room or blueprint links. */
export type ExportedSheet = Pick<SheetRecord, "revit_sheet_id" | "document_title" | "sheet_number" | "sheet_name" | "sheet"> &
  Partial<Pick<SheetRecord, "phase" | "room_summary" | "size_mm" | "room_outline" | "rooms">>;

export interface SheetsFile {
  format: typeof FILE_FORMAT;
  version: 1;
  exported_from: string;
  exported_at: string;
  sheets: ExportedSheet[];
}

export function toFile(records: readonly SheetRecord[], project: string, now: Date = new Date()): SheetsFile {
  return {
    format: FILE_FORMAT,
    version: 1,
    exported_from: project,
    exported_at: now.toISOString(),
    sheets: records.map((r) => ({
      revit_sheet_id: r.revit_sheet_id,
      document_title: r.document_title,
      sheet_number: r.sheet_number,
      sheet_name: r.sheet_name,
      ...(r.phase ? { phase: r.phase } : {}),
      ...(r.room_summary ? { room_summary: r.room_summary } : {}),
      ...(r.size_mm ? { size_mm: r.size_mm } : {}),
      ...(r.room_outline?.length ? { room_outline: r.room_outline } : {}),
      ...(r.rooms?.length ? { rooms: r.rooms } : {}),
      sheet: r.sheet,
    })),
  };
}

export const fileNameFor = (project: string, count: number, now: Date = new Date()): string =>
  `${safeName(project, "library")}.${count}-sheets.${now.toISOString().slice(0, 10)}.json`;

const isText = (v: unknown): v is string => typeof v === "string" && v !== "";

/** Parse a file's text, or say why it is not a sheets export (the server's refusal would be less helpful). */
export function parseFile(text: string): { ok: true; file: SheetsFile } | { ok: false; error: string } {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return { ok: false, error: "That file is not JSON." };
  }
  const f = raw as Partial<SheetsFile> | null;
  if (!f || typeof f !== "object" || f.format !== FILE_FORMAT) return { ok: false, error: "That file is not a sheets export from the library." };
  if (f.version !== 1) return { ok: false, error: `That file is version ${String(f.version)}; this page reads version 1.` };
  if (!Array.isArray(f.sheets) || f.sheets.length === 0) return { ok: false, error: "That file holds no sheets." };
  for (const [i, s] of f.sheets.entries()) {
    const at = `Sheet ${i + 1}${isText(s?.sheet_number) ? ` (${s.sheet_number})` : ""}`;
    if (!isText(s?.revit_sheet_id) || !isText(s?.document_title) || !isText(s?.sheet_number)) return { ok: false, error: `${at} has no sheet id, model title or number.` };
    if (!s.sheet || typeof s.sheet !== "object") return { ok: false, error: `${at} has no sheet body.` };
  }
  return { ok: true, file: f as SheetsFile };
}

export interface PushOutcome {
  id: string;
  sheet_number: string;
  outcome: "created" | "updated" | "pending";
  pending_id?: number;
}

/** "3 new, 1 updated, 2 waiting for a decision", for the notice after an import. */
export function outcomeText(outcomes: readonly PushOutcome[]): string {
  const n = (kind: PushOutcome["outcome"]) => outcomes.filter((o) => o.outcome === kind).length;
  const parts = [`${n("created")} new`, `${n("updated")} updated`];
  if (n("pending") > 0) parts.push(`${n("pending")} waiting for a decision (the number changed)`);
  return parts.join(", ");
}
