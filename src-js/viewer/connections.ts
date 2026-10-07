// The open-zone editor's rules, with no store and no DOM.
//
// An open zone is a set of rooms the user declares to be one open space; the
// server turns it into edges across the walls its members share (see
// `src/connections.rs`). What lives here is the part that is a RULE rather than
// plumbing: what a click does to the working set, what a save may contain, and
// the body that carries it. Pure and out of the components for the reason
// `route.ts` is.

import type { Scope } from "./scope.js";

/** A room as the plan names it. The model is `null` for a room the plan picked
 *  (it does not know models) and set for one loaded from a saved zone; the
 *  server resolves the former and refuses an id two models share. */
export interface Member {
  room_id: string;
  model_id: string | null;
}

/** A saved zone, as `GET /projects/{id}/connections` answers it. */
export type ZoneKind = "open" | "vertical";

export interface SavedZone {
  id: string;
  name: string;
  kind: ZoneKind;
  rooms: { model_id: string; room_id: string }[];
  /** Walking-equivalent feet per storey change, for a vertical zone. */
  level_cost_ft?: number | null;
  note?: string | null;
}

/** What the server assumes when a vertical zone states no cost. */
export const DEFAULT_LEVEL_COST_FT = 40;

export interface ConnectionsDoc {
  schema_version: number;
  /** The version a save must name; empty when nothing has been authored. */
  taken_at: string;
  zones: SavedZone[];
}

/** One zone being drawn: a new one (`id` null) or an existing one being changed. */
export interface ZoneEdit {
  id: string | null;
  name: string;
  kind: ZoneKind;
  /** The level cost as typed, so a half-typed number is not lost; blank means
   *  the default. Only a vertical zone carries one. */
  levelCost: string;
  members: Member[];
  /** Why the last save did not happen, in the server's words. */
  error: string | null;
  saving: boolean;
}

export const MIN_MEMBERS = 2;

export function newEdit(): ZoneEdit {
  return { id: null, name: "", kind: "open", levelCost: "", members: [], error: null, saving: false };
}

/** Load a saved zone for editing. */
export function editOf(zone: SavedZone): ZoneEdit {
  return {
    id: zone.id,
    name: zone.name,
    kind: zone.kind,
    levelCost: zone.level_cost_ft == null ? "" : String(zone.level_cost_ft),
    members: zone.rooms.map((r) => ({ room_id: r.room_id, model_id: r.model_id })),
    error: null,
    saving: false,
  };
}

/** What a click on a room does to the working set: in, if it is out, and out if
 *  it is in. A toggle rather than "add", because the only way to take a room
 *  back out without a control per room is to click it again. */
export function toggleMember(edit: ZoneEdit, roomId: string): ZoneEdit {
  const has = edit.members.some((m) => m.room_id === roomId);
  const members = has
    ? edit.members.filter((m) => m.room_id !== roomId)
    : [...edit.members, { room_id: roomId, model_id: null }];
  return { ...edit, members, error: null };
}

/** Add many rooms at once (every search match), skipping those already in.
 *  This is the bulk gesture: 48 rooms are one search and one click, not 48. */
export function addMembers(edit: ZoneEdit, roomIds: Iterable<string>): ZoneEdit {
  const have = new Set(edit.members.map((m) => m.room_id));
  const added: Member[] = [];
  for (const id of roomIds) {
    if (!have.has(id)) {
      have.add(id);
      added.push({ room_id: id, model_id: null });
    }
  }
  return added.length ? { ...edit, members: [...edit.members, ...added], error: null } : edit;
}

export function removeMember(edit: ZoneEdit, roomId: string): ZoneEdit {
  return { ...edit, members: edit.members.filter((m) => m.room_id !== roomId), error: null };
}

/** Why a zone cannot be saved yet, or `null` when it can. Said before the
 *  request, in the words the panel shows, so a button is never a guess. */
export function whyNotSavable(edit: ZoneEdit): string | null {
  if (edit.name.trim() === "") return "Give the zone a name.";
  if (edit.members.length < MIN_MEMBERS) return `Pick at least ${MIN_MEMBERS} rooms: one room alone connects nothing.`;
  if (edit.kind === "vertical" && parseLevelCost(edit.levelCost) === "bad") return "The level cost must be a number above 0.";
  return null;
}

/** A new zone's id from its name: a short, file-safe slug plus a suffix that
 *  keeps two zones with one name apart. */
export function idFor(name: string, taken: ReadonlySet<string>): string {
  const slug =
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 32) || "zone";
  let id = slug;
  for (let n = 2; taken.has(id); n += 1) id = `${slug}-${n}`;
  return id;
}

/** The zones list a save sends: the document's zones with this edit applied
 *  (replacing its original, or appended when new). */
export interface ZoneBody {
  id: string;
  name: string;
  kind: ZoneKind;
  rooms: Member[];
  level_cost_ft?: number | null;
  note?: string | null;
}

function bodyOf(z: SavedZone): ZoneBody {
  return { id: z.id, name: z.name, kind: z.kind, rooms: z.rooms, level_cost_ft: z.level_cost_ft ?? null, note: z.note ?? null };
}

export function zonesAfterSave(doc: ConnectionsDoc, edit: ZoneEdit): ZoneBody[] {
  const taken = new Set(doc.zones.map((z) => z.id));
  const id = edit.id ?? idFor(edit.name, taken);
  const cost = edit.kind === "vertical" ? parseLevelCost(edit.levelCost) : null;
  const edited: ZoneBody = {
    id,
    name: edit.name.trim(),
    kind: edit.kind,
    rooms: edit.members,
    level_cost_ft: cost === "bad" ? null : cost,
  };
  const kept = doc.zones.map((z) => (z.id === id ? edited : bodyOf(z)));
  return edit.id === null ? [...kept, edited] : kept;
}

/** The zones list after deleting one. */
export function zonesAfterDelete(doc: ConnectionsDoc, id: string): ZoneBody[] {
  return doc.zones.filter((z) => z.id !== id).map(bodyOf);
}

export function connectionsUrl(scope: Scope): string | null {
  return scope.projectId ? `/projects/${encodeURIComponent(scope.projectId)}/connections` : null;
}

/** The cost a vertical zone sends: `null` for blank (the server's default), the
 *  number when it is one above zero, and `"bad"` when it is neither, so the
 *  panel can say so instead of sending something the server will refuse. */
export function parseLevelCost(typed: string): number | null | "bad" {
  if (typed.trim() === "") return null;
  const n = Number(typed);
  return Number.isFinite(n) && n > 0 ? n : "bad";
}
