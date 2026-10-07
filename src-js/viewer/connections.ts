// The connections editor's rules, with no store and no DOM.
//
// Two things are authored, and they are different records on purpose:
//
//   - an OPEN ZONE: a set of rooms the user declares to be one open space; the
//     server treats every wall two members share as open;
//   - a VERTICAL LINK: exactly two rooms on different levels, joined by hand, one
//     floor-to-floor hop per link (a lift or stair is a run of them, never a jump
//     from level 1 to 8). It is two rooms and not a list because a list could not
//     tell the lift rooms from the lobbies around them.
//
// What lives here is the part that is a RULE rather than plumbing: what a click
// does to the working set, what a save may contain, and the body that carries it.
// Pure and out of the components for the reason `route.ts` is.

import type { Scope } from "./scope.js";

/** A room as the plan names it. The model is `null` for a room the plan picked
 *  (it does not know models) and set for one loaded from a saved record; the
 *  server resolves the former and refuses an id two models share. */
export interface Member {
  room_id: string;
  model_id: string | null;
}

export interface SavedZone {
  id: string;
  name: string;
  kind: "open";
  rooms: { model_id: string; room_id: string }[];
  note?: string | null;
}

export interface SavedLink {
  id: string;
  a: { model_id: string; room_id: string };
  b: { model_id: string; room_id: string };
  /** Walking-equivalent feet for this hop; absent means the server's default. */
  cost_ft?: number | null;
  note?: string | null;
}

export interface ConnectionsDoc {
  schema_version: number;
  /** The version a save must name; empty when nothing has been authored. */
  taken_at: string;
  zones: SavedZone[];
  links: SavedLink[];
}

/** What the server assumes when a link states no cost. */
export const DEFAULT_LEVEL_COST_FT = 40;

/** What the editor is drawing. */
export type EditKind = "open" | "link";

/** One record being drawn: a new one (`id` null) or an existing one being changed. */
export interface ZoneEdit {
  id: string | null;
  kind: EditKind;
  /** An open zone's name. A link has none: it is named by its two rooms. */
  name: string;
  /** A link's cost as typed, so a half-typed number is not lost; blank means the
   *  default. */
  costFt: string;
  members: Member[];
  /** Why the last save did not happen, in the server's words. */
  error: string | null;
  saving: boolean;
}

export const MIN_MEMBERS = 2;
/** A link joins exactly two rooms. */
export const LINK_MEMBERS = 2;

export function newEdit(kind: EditKind = "open"): ZoneEdit {
  return { id: null, kind, name: "", costFt: "", members: [], error: null, saving: false };
}

/** Load a saved zone for editing. */
export function editOf(zone: SavedZone): ZoneEdit {
  return {
    id: zone.id,
    kind: "open",
    name: zone.name,
    costFt: "",
    members: zone.rooms.map((r) => ({ room_id: r.room_id, model_id: r.model_id })),
    error: null,
    saving: false,
  };
}

/** Load a saved link for editing. */
export function editOfLink(link: SavedLink): ZoneEdit {
  return {
    id: link.id,
    kind: "link",
    name: "",
    costFt: link.cost_ft == null ? "" : String(link.cost_ft),
    members: [link.a, link.b].map((r) => ({ room_id: r.room_id, model_id: r.model_id })),
    error: null,
    saving: false,
  };
}

/** What a click on a room does to the working set: in, if it is out, and out if
 *  it is in. A toggle rather than "add", because the only way to take a room
 *  back out without a control per room is to click it again.
 *
 *  A link holds two. A third pick replaces the SECOND, so after choosing the
 *  lower room and moving up a level, picking again swaps the upper room without a
 *  trip back to remove one. */
export function toggleMember(edit: ZoneEdit, roomId: string): ZoneEdit {
  const has = edit.members.some((m) => m.room_id === roomId);
  if (has) return { ...edit, members: edit.members.filter((m) => m.room_id !== roomId), error: null };
  const added: Member = { room_id: roomId, model_id: null };
  const members =
    edit.kind === "link" && edit.members.length >= LINK_MEMBERS ? [edit.members[0]!, added] : [...edit.members, added];
  return { ...edit, members, error: null };
}

/** Add many rooms at once (every search match), skipping those already in. This
 *  is the bulk gesture for an open zone: 48 rooms are one search and one click.
 *  A link takes two rooms by hand, so there is nothing to add in bulk. */
export function addMembers(edit: ZoneEdit, roomIds: Iterable<string>): ZoneEdit {
  if (edit.kind === "link") return edit;
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

/** The cost a link sends: `null` for blank (the server's default), the number
 *  when it is one above zero, and `"bad"` when it is neither, so the panel can
 *  say so instead of sending something the server will refuse. */
export function parseCost(typed: string): number | null | "bad" {
  if (typed.trim() === "") return null;
  const n = Number(typed);
  return Number.isFinite(n) && n > 0 ? n : "bad";
}

/** Why a record cannot be saved yet, or `null` when it can. Said before the
 *  request, in the words the panel shows, so a button is never a guess. */
export function whyNotSavable(edit: ZoneEdit): string | null {
  if (edit.kind === "link") {
    if (edit.members.length < LINK_MEMBERS) {
      return "Pick two rooms on different levels: change the level picker between the picks.";
    }
    if (parseCost(edit.costFt) === "bad") return "The cost must be a number above 0.";
    return null;
  }
  if (edit.name.trim() === "") return "Give the zone a name.";
  if (edit.members.length < MIN_MEMBERS) return `Pick at least ${MIN_MEMBERS} rooms: one room alone connects nothing.`;
  return null;
}

/** A short, file-safe id from a name, with a suffix that keeps two with one name
 *  apart. */
export function idFor(name: string, taken: ReadonlySet<string>): string {
  const slug =
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 32) || "item";
  let id = slug;
  for (let n = 2; taken.has(id); n += 1) id = `${slug}-${n}`;
  return id;
}

export interface ZoneBody {
  id: string;
  name: string;
  kind: "open";
  rooms: Member[];
  note?: string | null;
}

export interface LinkBody {
  id: string;
  a: Member;
  b: Member;
  cost_ft?: number | null;
  note?: string | null;
}

/** The whole document body a save sends: both lists, so a save of one never
 *  drops the other. */
export interface DocBody {
  zones: ZoneBody[];
  links: LinkBody[];
}

function zoneBody(z: SavedZone): ZoneBody {
  return { id: z.id, name: z.name, kind: "open", rooms: z.rooms, note: z.note ?? null };
}

function linkBody(l: SavedLink): LinkBody {
  return { id: l.id, a: l.a, b: l.b, cost_ft: l.cost_ft ?? null, note: l.note ?? null };
}

/** The lists after saving `edit`: it replaces its original in place, or is
 *  appended when new, and the other list is carried over untouched. */
export function bodyAfterSave(doc: ConnectionsDoc, edit: ZoneEdit): DocBody {
  const zones = doc.zones.map(zoneBody);
  const links = doc.links.map(linkBody);
  if (edit.kind === "link") {
    const [a, b] = edit.members as [Member, Member];
    const cost = parseCost(edit.costFt);
    const id = edit.id ?? idFor(`link ${a.room_id} ${b.room_id}`, new Set(links.map((l) => l.id)));
    const next: LinkBody = { id, a, b, cost_ft: cost === "bad" ? null : cost };
    return { zones, links: edit.id === null ? [...links, next] : links.map((l) => (l.id === id ? next : l)) };
  }
  const id = edit.id ?? idFor(edit.name, new Set(zones.map((z) => z.id)));
  const next: ZoneBody = { id, name: edit.name.trim(), kind: "open", rooms: edit.members };
  return { links, zones: edit.id === null ? [...zones, next] : zones.map((z) => (z.id === id ? next : z)) };
}

/** The lists after deleting one zone. */
export function bodyAfterDeleteZone(doc: ConnectionsDoc, id: string): DocBody {
  return { zones: doc.zones.filter((z) => z.id !== id).map(zoneBody), links: doc.links.map(linkBody) };
}

/** The lists after deleting one link. */
export function bodyAfterDeleteLink(doc: ConnectionsDoc, id: string): DocBody {
  return { zones: doc.zones.map(zoneBody), links: doc.links.filter((l) => l.id !== id).map(linkBody) };
}

export function connectionsUrl(scope: Scope): string | null {
  return scope.projectId ? `/projects/${encodeURIComponent(scope.projectId)}/connections` : null;
}
