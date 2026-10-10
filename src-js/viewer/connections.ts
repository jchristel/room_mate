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
  /** Pairs of members whose shared wall stays CLOSED. */
  disconnects?: { a: { model_id: string; room_id: string }; b: { model_id: string; room_id: string } }[];
  /** When set, members connect only through this room. */
  hub?: { model_id: string; room_id: string } | null;
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

/** A route somebody saved: the request, never the path (the server derives that). */
export interface SavedRoute {
  id: string;
  name: string;
  from: { model_id: string; room_id: string };
  to: { model_id: string; room_id: string };
  from_at?: { x: number; y: number } | null;
  to_at?: { x: number; y: number } | null;
  method?: string | null;
  /** The width in mm of the object this route was saved for; absent checks nothing. */
  width_mm?: number | null;
  /** Its height in mm; absent checks nothing. */
  height_mm?: number | null;
  /** `#rrggbb`. Part of the record, so everyone sees the route in one colour. */
  colour: string;
  note?: string | null;
}

export interface ConnectionsDoc {
  schema_version: number;
  /** The version a save must name; empty when nothing has been authored. */
  taken_at: string;
  zones: SavedZone[];
  links: SavedLink[];
  routes: SavedRoute[];
}

/** What the server assumes when a link states no cost. */
export const DEFAULT_LEVEL_COST_FT = 40;

/** What the editor is drawing. `stack` draws nothing of its own: it shows the rooms
 *  joined by vertical links as a column, and each link it adds is saved at once. */
export type EditKind = "open" | "link" | "stack";

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
  /** Walls between two members that stay closed: an open area opens EVERY wall two
   *  members share, which is wrong for bays along a corridor (they touch each other
   *  as well as the corridor). Room ids; the models come from `members`. */
  cuts: Cut[];
  /** The member the others connect only through, or `null`. */
  hub: string | null;
  /** What a room pick does in an open area: add or remove a member, close the wall
   *  between two members, or name the hub. */
  tool: EditTool;
  /** The first room of a cut being drawn, waiting for the second. */
  cutFrom: string | null;
  /** Why the last save did not happen, in the server's words. */
  error: string | null;
  saving: boolean;
}

export interface Cut {
  a: string;
  b: string;
}

export type EditTool = "pick" | "cut" | "hub";

export const MIN_MEMBERS = 2;
/** A link joins exactly two rooms. */
export const LINK_MEMBERS = 2;

export function newEdit(kind: EditKind = "open"): ZoneEdit {
  return { id: null, kind, name: "", costFt: "", members: [], cuts: [], hub: null, tool: "pick", cutFrom: null, error: null, saving: false };
}

/** Load a saved zone for editing. */
export function editOf(zone: SavedZone): ZoneEdit {
  return {
    id: zone.id,
    kind: "open",
    name: zone.name,
    costFt: "",
    members: zone.rooms.map((r) => ({ room_id: r.room_id, model_id: r.model_id })),
    cuts: (zone.disconnects ?? []).map((d) => ({ a: d.a.room_id, b: d.b.room_id })),
    hub: zone.hub?.room_id ?? null,
    tool: "pick",
    cutFrom: null,
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
    cuts: [],
    hub: null,
    tool: "pick",
    cutFrom: null,
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
  // A stack is about ONE room: a pick replaces it, and picking it again clears it.
  if (edit.kind === "stack") {
    const same = edit.members.length === 1 && edit.members[0]!.room_id === roomId;
    return { ...edit, members: same ? [] : [{ room_id: roomId, model_id: null }], error: null };
  }
  const has = edit.members.some((m) => m.room_id === roomId);
  if (has) return withoutMember(edit, roomId);
  const added: Member = { room_id: roomId, model_id: null };
  const members =
    edit.kind === "link" && edit.members.length >= LINK_MEMBERS ? [edit.members[0]!, added] : [...edit.members, added];
  return { ...edit, members, error: null };
}

/** Add many rooms at once (every search match), skipping those already in. This
 *  is the bulk gesture for an open zone: 48 rooms are one search and one click.
 *  A link takes two rooms by hand, so there is nothing to add in bulk. */
export function addMembers(edit: ZoneEdit, roomIds: Iterable<string>): ZoneEdit {
  if (edit.kind !== "open") return edit;
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
  return withoutMember(edit, roomId);
}

/** `edit` without one member, and without the cuts and hub that named it: the server
 *  refuses a cut or a hub that is not one of the zone's own rooms, and a cut with
 *  nothing to cut is not something to keep. */
function withoutMember(edit: ZoneEdit, roomId: string): ZoneEdit {
  return {
    ...edit,
    members: edit.members.filter((m) => m.room_id !== roomId),
    cuts: edit.cuts.filter((c) => c.a !== roomId && c.b !== roomId),
    hub: edit.hub === roomId ? null : edit.hub,
    cutFrom: edit.cutFrom === roomId ? null : edit.cutFrom,
    error: null,
  };
}

const sameCut = (c: Cut, a: string, b: string) => (c.a === a && c.b === b) || (c.a === b && c.b === a);

/** What a room pick does to an open area, by the tool in use.
 *
 *  - `pick`: toggle membership, as always.
 *  - `cut`: the first member picked is held, the second closes the wall between them
 *    (or opens it again if it was closed), and the tool stays on for the next cut.
 *    Picking the held room again lets go of it. A room that is not a member is
 *    ignored: a cut is between two of the zone's own rooms.
 *  - `hub`: the member picked becomes the hub (picked again, it stops being one),
 *    and the tool goes back to `pick`, because naming a hub is one act. */
export function pickForEdit(edit: ZoneEdit, roomId: string): ZoneEdit {
  if (edit.kind !== "open" || edit.tool === "pick") return toggleMember(edit, roomId);
  if (!edit.members.some((m) => m.room_id === roomId)) {
    return { ...edit, error: "Pick a room that is already in this open area." };
  }
  if (edit.tool === "hub") {
    return { ...edit, hub: edit.hub === roomId ? null : roomId, tool: "pick", cutFrom: null, error: null };
  }
  if (edit.cutFrom === null) return { ...edit, cutFrom: roomId, error: null };
  if (edit.cutFrom === roomId) return { ...edit, cutFrom: null, error: null };
  const from = edit.cutFrom;
  const cuts = edit.cuts.some((c) => sameCut(c, from, roomId))
    ? edit.cuts.filter((c) => !sameCut(c, from, roomId))
    : [...edit.cuts, { a: from, b: roomId }];
  return { ...edit, cuts, cutFrom: null, error: null };
}

/** Change the tool; any half-drawn cut is dropped, because it belonged to the old one. */
export function setTool(edit: ZoneEdit, tool: EditTool): ZoneEdit {
  return { ...edit, tool: edit.tool === tool ? "pick" : tool, cutFrom: null, error: null };
}

/** Take one cut out. */
export function removeCut(edit: ZoneEdit, cut: Cut): ZoneEdit {
  return { ...edit, cuts: edit.cuts.filter((c) => !sameCut(c, cut.a, cut.b)), error: null };
}

/** Stop connecting through a hub. */
export function clearHub(edit: ZoneEdit): ZoneEdit {
  return { ...edit, hub: null, error: null };
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
  if (edit.kind === "stack") return "The stack view saves each link as you add it.";
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
  disconnects: { a: Member; b: Member }[];
  hub: Member | null;
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
  routes: SavedRoute[];
}

/** A saved zone as a body: **cuts and hub included**, so a save of anything else (a
 *  link, a route, another zone) carries them through. Leaving them out would delete
 *  every cut in the project on the next unrelated save. */
function zoneBody(z: SavedZone): ZoneBody {
  return {
    id: z.id,
    name: z.name,
    kind: "open",
    rooms: z.rooms,
    disconnects: z.disconnects ?? [],
    hub: z.hub ?? null,
    note: z.note ?? null,
  };
}

function routeBody(r: SavedRoute): SavedRoute {
  return {
    id: r.id,
    name: r.name,
    from: r.from,
    to: r.to,
    from_at: r.from_at ?? null,
    to_at: r.to_at ?? null,
    method: r.method ?? null,
    width_mm: r.width_mm ?? null,
    height_mm: r.height_mm ?? null,
    colour: r.colour,
    note: r.note ?? null,
  };
}

function linkBody(l: SavedLink): LinkBody {
  return { id: l.id, a: l.a, b: l.b, cost_ft: l.cost_ft ?? null, note: l.note ?? null };
}

/** The lists after saving `edit`: it replaces its original in place, or is
 *  appended when new, and the other list is carried over untouched. */
export function bodyAfterSave(doc: ConnectionsDoc, edit: ZoneEdit): DocBody {
  const zones = doc.zones.map(zoneBody);
  const links = doc.links.map(linkBody);
  const routes = doc.routes.map(routeBody);
  // The stack view saves each link itself (`bodyAfterAddLink`); it has no working
  // set to save, so a save from it changes nothing.
  if (edit.kind === "stack") return { zones, links, routes };
  if (edit.kind === "link") {
    const [a, b] = edit.members as [Member, Member];
    const cost = parseCost(edit.costFt);
    const id = edit.id ?? idFor(`link ${a.room_id} ${b.room_id}`, new Set(links.map((l) => l.id)));
    const next: LinkBody = { id, a, b, cost_ft: cost === "bad" ? null : cost };
    return { zones, routes, links: edit.id === null ? [...links, next] : links.map((l) => (l.id === id ? next : l)) };
  }
  const id = edit.id ?? idFor(edit.name, new Set(zones.map((z) => z.id)));
  // A cut names rooms by id; the model of each is its member's.
  const member = (roomId: string): Member => edit.members.find((m) => m.room_id === roomId) ?? { room_id: roomId, model_id: null };
  const next: ZoneBody = {
    id,
    name: edit.name.trim(),
    kind: "open",
    rooms: edit.members,
    disconnects: edit.cuts.map((c) => ({ a: member(c.a), b: member(c.b) })),
    hub: edit.hub === null ? null : member(edit.hub),
  };
  return { links, routes, zones: edit.id === null ? [...zones, next] : zones.map((z) => (z.id === id ? next : z)) };
}

/** One line of the connections table: a saved open zone or vertical link. */
export interface ConnectionRow {
  kind: "open" | "link";
  id: string;
  /** A zone's name; a link has none, so it is named by its two rooms. */
  name: string;
  /** What the second column says. */
  type: "Open area" | "Vertical";
  /** The levels the record lies on: an open area's rooms' levels, or the two
   *  levels a link joins. Empty when the page holds none of its rooms. */
  level: string;
}

/** A level as the table needs it. */
export interface LevelOfRoom {
  name: string;
  elevation: number;
}

/** The levels a set of rooms lies on, lowest first and each once, as text. More
 *  than three is said as a count, because an open area that spans that many is a
 *  question of its own and a long list would only widen the column. */
export function levelsText(levels: readonly (LevelOfRoom | null)[], joiner = ", "): string {
  const seen = new Map<string, LevelOfRoom>();
  for (const l of levels) if (l && !seen.has(l.name)) seen.set(l.name, l);
  const sorted = [...seen.values()].sort((a, b) => a.elevation - b.elevation);
  if (sorted.length > 3) return `${sorted.length} levels`;
  return sorted.map((l) => l.name).join(joiner);
}

/** Every saved zone and link as table rows: open areas first, then vertical links,
 *  each group by name. `roomName` turns a room id into the name a reader knows and
 *  `levelOf` into its level; an id the page does not hold stays as the id and has
 *  no level, so a stale link is still listed and can be deleted. */
export function connectionRows(
  doc: ConnectionsDoc | null,
  roomName: (id: string) => string,
  levelOf: (id: string) => LevelOfRoom | null = () => null,
): ConnectionRow[] {
  const byName = (a: ConnectionRow, b: ConnectionRow) => a.name.localeCompare(b.name, undefined, { numeric: true });
  const zones: ConnectionRow[] = (doc?.zones ?? []).map((z) => ({
    kind: "open",
    id: z.id,
    name: z.name,
    type: "Open area",
    level: levelsText(z.rooms.map((r) => levelOf(r.room_id))),
  }));
  const links: ConnectionRow[] = (doc?.links ?? []).map((l) => ({
    kind: "link",
    id: l.id,
    name: `${roomName(l.a.room_id)} ↔ ${roomName(l.b.room_id)}`,
    type: "Vertical",
    level: levelsText([levelOf(l.a.room_id), levelOf(l.b.room_id)], " ↔ "),
  }));
  return [...zones.sort(byName), ...links.sort(byName)];
}

/** One line of the saved-routes table. */
export interface RouteTableRow {
  id: string;
  name: string;
  colour: string;
  /** Where it runs: the start room's level to the end room's, or the one level
   *  when they are the same. Empty when the page holds neither room. */
  levels: string;
}

/** Every saved route as table rows, by name. The levels are the two ends' and not
 *  every level the path crosses: a path is derived and a route that is not shown
 *  has none to read, while its two rooms are always in the record. */
export function routeTableRows(
  doc: ConnectionsDoc | null,
  levelOf: (roomId: string) => LevelOfRoom | null,
): RouteTableRow[] {
  return (doc?.routes ?? [])
    .map((r) => {
      const [from, to] = [levelOf(r.from.room_id), levelOf(r.to.room_id)];
      const levels = from && to && from.name !== to.name ? `${from.name} → ${to.name}` : (from ?? to)?.name ?? "";
      return { id: r.id, name: r.name, colour: r.colour, levels };
    })
    .sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true }));
}

/** The lists after adding one vertical link between two rooms, at the default cost.
 *  What the stack view's "Link" button saves: one hop, confirmed by a person. */
export function bodyAfterAddLink(doc: ConnectionsDoc, a: Member, b: Member): DocBody {
  const links = doc.links.map(linkBody);
  const id = idFor(`link ${a.room_id} ${b.room_id}`, new Set(links.map((l) => l.id)));
  return {
    zones: doc.zones.map(zoneBody),
    links: [...links, { id, a, b, cost_ft: null }],
    routes: doc.routes.map(routeBody),
  };
}

/** The lists after deleting one zone. */
export function bodyAfterDeleteZone(doc: ConnectionsDoc, id: string): DocBody {
  return {
    zones: doc.zones.filter((z) => z.id !== id).map(zoneBody),
    links: doc.links.map(linkBody),
    routes: doc.routes.map(routeBody),
  };
}

/** The lists after deleting one link. */
export function bodyAfterDeleteLink(doc: ConnectionsDoc, id: string): DocBody {
  return {
    zones: doc.zones.map(zoneBody),
    links: doc.links.filter((l) => l.id !== id).map(linkBody),
    routes: doc.routes.map(routeBody),
  };
}

/** Colours a saved route may be given, picked to be told apart on the plan's paper
 *  and ink in both themes. A route takes the least-used one when saved, and the
 *  reader can replace it with a colour of their own. */
export const ROUTE_COLOURS = ["#d9480f", "#1c7ed6", "#2f9e44", "#ae3ec9", "#e8a50c", "#0b7285"] as const;

/** The first palette colour no saved route uses, else the one used least. */
export function nextRouteColour(doc: ConnectionsDoc | null): string {
  const used = new Map<string, number>(ROUTE_COLOURS.map((c) => [c, 0]));
  for (const r of doc?.routes ?? []) {
    const c = r.colour.toLowerCase();
    if (used.has(c)) used.set(c, used.get(c)! + 1);
  }
  let best: string = ROUTE_COLOURS[0];
  for (const c of ROUTE_COLOURS) if (used.get(c)! < used.get(best)!) best = c;
  return best;
}

/** What the route tool hands over to be saved. The rooms are bare ids because the
 *  plan does not know models; the server resolves them and refuses an id two models
 *  share. */
export interface RouteDraft {
  name: string;
  colour: string;
  from: string;
  to: string;
  fromAt: { x: number; y: number } | null;
  toAt: { x: number; y: number } | null;
  method: string | null;
  /** The width in mm to check, or `null` for none. */
  widthMm: number | null;
  /** The height in mm to check, or `null` for none. */
  heightMm: number | null;
}

/** Why a draft cannot be saved yet, or `null`. */
export function whyRouteNotSavable(draft: Pick<RouteDraft, "name" | "from" | "to">): string | null {
  if (draft.name.trim() === "") return "Give the route a name.";
  if (!draft.from || !draft.to) return "Place a start and an end first.";
  return null;
}

/** The lists after saving a new route. */
export function bodyAfterSaveRoute(doc: ConnectionsDoc, draft: RouteDraft): DocBody {
  const routes = doc.routes.map(routeBody);
  const id = idFor(draft.name, new Set(routes.map((r) => r.id)));
  // A bare room has no model yet: a blank one is the wire's "not named", which the
  // server resolves.
  const next: SavedRoute = {
    id,
    name: draft.name.trim(),
    from: { model_id: "", room_id: draft.from },
    to: { model_id: "", room_id: draft.to },
    from_at: draft.fromAt,
    to_at: draft.toAt,
    method: draft.method,
    width_mm: draft.widthMm,
    height_mm: draft.heightMm,
    colour: draft.colour,
  };
  return { zones: doc.zones.map(zoneBody), links: doc.links.map(linkBody), routes: [...routes, next] };
}

/** The lists after changing one saved route's colour. */
export function bodyAfterRecolourRoute(doc: ConnectionsDoc, id: string, colour: string): DocBody {
  return {
    zones: doc.zones.map(zoneBody),
    links: doc.links.map(linkBody),
    routes: doc.routes.map((r) => routeBody(r.id === id ? { ...r, colour } : r)),
  };
}

/** The lists after deleting one saved route. */
export function bodyAfterDeleteRoute(doc: ConnectionsDoc, id: string): DocBody {
  return {
    zones: doc.zones.map(zoneBody),
    links: doc.links.map(linkBody),
    routes: doc.routes.filter((r) => r.id !== id).map(routeBody),
  };
}

/** The rooms already in a saved open zone, by room id, with the index of the zone
 *  they are in, so a neighbour can be tinted apart. The zone being edited is left
 *  out: it is drawn from the working set, which may differ from what was saved. */
export function roomsInOtherZones(doc: ConnectionsDoc | null, editingId: string | null): Map<string, number> {
  const out = new Map<string, number>();
  (doc?.zones ?? []).forEach((z, i) => {
    if (z.id === editingId) return;
    for (const r of z.rooms) if (!out.has(r.room_id)) out.set(r.room_id, i);
  });
  return out;
}

export function connectionsUrl(scope: Scope): string | null {
  return scope.projectId ? `/projects/${encodeURIComponent(scope.projectId)}/connections` : null;
}
