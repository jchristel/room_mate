// Moving a blueprint between projects on one server, and to and from a file.
//
// **An import never overwrites** (plan decision 10). A blueprint arriving under a name the
// project already has goes in as a PENDING revision of that blueprint, for a person to accept or
// reject; one with a new name is simply created, because there is no active blueprint for it to
// overwrite. Same-server only for now: this reads another project's blueprint through the same
// endpoints and writes it through the same ones, so no new server surface was needed.

import type { Sent } from "./api.js";
import * as api from "./api.js";
import type { Blueprint, BlueprintDoor, BlueprintItem, BlueprintSummary, Bundle, Outline, SourceRoom } from "./types.js";

export const FILE_FORMAT = "roommate-blueprint";

/** One revision of one blueprint, as a file. */
export interface ExportFile {
  format: typeof FILE_FORMAT;
  version: 1;
  name: string;
  source: SourceRoom | null;
  /** The revision the file holds, as it stood. */
  revision: {
    number: number;
    note: string | null;
    outline: Outline;
    items: BlueprintItem[];
    doors?: BlueprintDoor[];
  };
  exported_from: string;
  /** The sheets and tag rules bound to the blueprint (plan decision 27). A file without one is the room alone. */
  bundle?: Bundle;
}

export function toFile(blueprint: Blueprint, revisionNumber: number, project: string, bundle?: Bundle): ExportFile | null {
  const r = blueprint.revisions.find((x) => x.number === revisionNumber);
  if (!r) return null;
  return {
    format: FILE_FORMAT,
    version: 1,
    name: blueprint.name,
    source: blueprint.source ?? null,
    revision: { number: r.number, note: r.note ?? null, outline: r.outline, items: r.items, doors: r.doors ?? [] },
    exported_from: project,
    ...(bundle ? { bundle } : {}),
  };
}

/** Parse a file's text, or say why it is not a blueprint export. Checks the shape the server
 *  would otherwise refuse with a less helpful message. */
export function parseFile(text: string): { ok: true; file: ExportFile } | { ok: false; error: string } {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return { ok: false, error: "That file is not JSON." };
  }
  const f = raw as Partial<ExportFile> | null;
  if (!f || f.format !== FILE_FORMAT) return { ok: false, error: "That file is not a RoomMate blueprint export." };
  if (f.version !== 1) return { ok: false, error: `That export is version ${String(f.version)}; this page reads version 1.` };
  const r = f.revision;
  if (!f.name || !r || !Array.isArray(r.items) || !r.outline || !Array.isArray(r.outline.coarse)) {
    return { ok: false, error: "That export is missing its name, outline or items." };
  }
  return { ok: true, file: f as ExportFile };
}

/** The server refuses an import whose sheets another blueprint holds (a sheet belongs to one). */
export const isSheetConflict = (error: string): boolean => error.includes("already bound to another blueprint");

/** The same file, told to take the sheets from the blueprint that holds them. */
export const takingTheSheets = (file: ExportFile): ExportFile =>
  file.bundle ? { ...file, bundle: { ...file.bundle, take_over_sheets: true } } : file;

export type Imported = { kind: "created"; blueprint: Blueprint } | { kind: "pending"; blueprint: Blueprint; into: string };

/** Put an exported revision into `project`. See the module header for the rule. */
export async function importInto(
  project: string,
  existing: readonly BlueprintSummary[],
  file: ExportFile,
  origin: string,
): Promise<Sent<Imported>> {
  const { outline, items, doors } = file.revision;
  const content = { outline, items, doors: doors ?? [] };
  const same = existing.find((b) => b.name.trim().toLowerCase() === file.name.trim().toLowerCase());
  if (same) {
    const r = await api.pushPending(project, same.id, origin, content, file.revision.note ?? undefined, file.bundle);
    return r.ok ? { ok: true, value: { kind: "pending", blueprint: r.value, into: same.name } } : r;
  }
  const r = await api.createBlueprint(project, file.name, file.source, content, file.revision.note ?? undefined, file.bundle);
  return r.ok ? { ok: true, value: { kind: "created", blueprint: r.value } } : r;
}

/** A file name that is safe on every platform, from a blueprint's name. */
export const fileNameFor = (name: string, revision: number): string =>
  `${name.replace(/[^A-Za-z0-9._-]+/g, "_").replace(/^_+|_+$/g, "") || "blueprint"}.r${revision}.blueprint.json`;
