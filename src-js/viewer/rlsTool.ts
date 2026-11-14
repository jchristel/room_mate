// The RLS tool's rules, with no store and no DOM (the blueprint tool's shape).
//
// A room layout sheet is made from a LIBRARY SHEET and the rooms it is made for, so the tool is a sheet
// choice and a set of rooms: a click on a room adds it, a second click takes it off. Making it queues a
// `create_rls` task per room for the Revit session (the Tasks button). **Nothing here places
// anything**: the server works out where the elevation markers and views go from the walls, and the
// views take their room's phase in Revit, so there is no phase to enter.

import type { TaskIndex } from "./taskStatus.js";

export interface RlsToolState {
  /** The library sheet to copy the layout of, or null before one is chosen. */
  sheetId: string | null;
  /** The rooms to make a sheet for, in the order they were picked. */
  roomIds: string[];
  /** Replace a sheet that already exists (off, it is skipped with a warning). */
  overwrite: boolean;
  /** `manual`: choose one sheet and click rooms. `auto`: the server's solver matches each room to a sheet. `set`: the
   *  rooms picked make ONE set of adjacent rooms, laid out by a person in the composer (`rlsSet.ts`). */
  mode: RlsMode;
  /** Auto: rooms that already have a task but are put back in the solver set by a person. */
  included: string[];
  /** Auto: the room last clicked (the one the sheet override acts on) and when, to tell a double click. */
  focus: string | null;
  lastPickAt: number;
  /** Auto: a person's sheet for a room: a library sheet id, or `NO_SHEET` for none. Its own category. */
  overrides: Record<string, string>;
  /** Auto: the solver's answer by room id, as the server gave it. */
  solved: Record<string, Solved>;
  /** Auto: which categories "Make" queues. */
  bands: Record<Category, boolean>;
}

export type RlsMode = "manual" | "auto" | "set";
export type Band = "green" | "yellow" | "red";
export type Category = Band | "override";

/** The solver's closest sheets for a room (`POST .../sheets/solve`). */
export interface Solved {
  room_id: string;
  room_name?: string;
  choices: { sheet_id: string; sheet_number: string; sheet_name: string; room_name: string; fit: number }[];
  reason?: string;
}

/** An override that leaves the room without a sheet. */
export const NO_SHEET = "none";
/** Fit at or above this is green, at or above `YELLOW_FROM` yellow, else red. */
export const GREEN_FROM = 80;
export const YELLOW_FROM = 60;
/** Two picks of one room this close (ms) are a double click. */
export const DOUBLE_CLICK_MS = 450;

export const newRlsTool = (): RlsToolState => ({
  sheetId: null,
  roomIds: [],
  overwrite: false,
  mode: "manual",
  included: [],
  focus: null,
  lastPickAt: 0,
  overrides: {},
  solved: {},
  bands: { green: true, yellow: false, red: false, override: true },
});

export const bandOf = (fit: number): Band => (fit >= GREEN_FROM ? "green" : fit >= YELLOW_FROM ? "yellow" : "red");

/** A room task in one of these states means the room is dealt with (or being dealt with): it leaves the solver set. */
const HAS_TASK = new Set(["queued", "in_progress", "completed", "completed_with_warnings"]);
export const hasTask = (state: string | null): boolean => state !== null && HAS_TASK.has(state);

/** The room ids whose newest live task means they are dealt with. The id alone, as the solver is asked by id. */
export function doneRoomIds(index: TaskIndex | null): Set<string> {
  const out = new Set<string>();
  if (!index) return out;
  for (const [id, list] of index) if (list[0] && hasTask(list[0].state)) out.add(id);
  return out;
}

/** The rooms the solver is asked about: those without a task, plus the ones a person put back. */
export function solverSet(roomIds: readonly string[], done: ReadonlySet<string>, s: RlsToolState): string[] {
  return roomIds.filter((id) => !done.has(id) || s.included.includes(id));
}

/** The room's category in auto mode, or null when it has no verdict (not solved, or the solver found no sheet). */
export function categoryOf(s: RlsToolState, roomId: string): Category | null {
  if (roomId in s.overrides) return "override";
  const best = s.solved[roomId]?.choices[0];
  return best ? bandOf(best.fit) : null;
}

/** The sheet auto mode would make for the room, or null for none. An override wins. */
export function sheetFor(s: RlsToolState, roomId: string): string | null {
  const o = s.overrides[roomId];
  if (o !== undefined) return o === NO_SHEET ? null : o;
  return s.solved[roomId]?.choices[0]?.sheet_id ?? null;
}

/** What "Make" would queue: rooms of the ticked categories that have a sheet, grouped by sheet. */
export function autoBatches(s: RlsToolState, set: readonly string[]): Map<string, string[]> {
  const out = new Map<string, string[]>();
  for (const id of set) {
    const cat = categoryOf(s, id);
    const sheet = sheetFor(s, id);
    if (!cat || !sheet || !s.bands[cat]) continue;
    const list = out.get(sheet);
    if (list) list.push(id);
    else out.set(sheet, [id]);
  }
  return out;
}

/** Counts per category for the bar. `done` rooms are not in `set`. */
export function tally(s: RlsToolState, set: readonly string[]): Record<Category, number> {
  const out: Record<Category, number> = { green: 0, yellow: 0, red: 0, override: 0 };
  for (const id of set) {
    const c = categoryOf(s, id);
    if (c) out[c] += 1;
  }
  return out;
}

/** An auto-mode pick: it focuses the room (the one the override acts on); a second pick of the same room within
 *  `DOUBLE_CLICK_MS` is a double click, which puts a room that has a task back in the solver set (or takes it out again). */
export function pickAuto(s: RlsToolState, roomId: string, now: number, done: ReadonlySet<string>): RlsToolState {
  const twice = s.focus === roomId && now - s.lastPickAt <= DOUBLE_CLICK_MS;
  const next = { ...s, focus: roomId, lastPickAt: now };
  if (!twice || !done.has(roomId)) return next;
  const has = s.included.includes(roomId);
  return { ...next, included: has ? s.included.filter((r) => r !== roomId) : [...s.included, roomId] };
}

/** "Include all": every room that has a task goes back in the solver set. */
export const includeAll = (s: RlsToolState, done: ReadonlySet<string>): RlsToolState => ({ ...s, included: [...done] });

export const setOverride = (s: RlsToolState, roomId: string, sheet: string | null): RlsToolState => {
  const overrides = { ...s.overrides };
  if (sheet === null) delete overrides[roomId];
  else overrides[roomId] = sheet;
  return { ...s, overrides };
};

/** A room click: picked rooms come off, others go on. */
export function pickRoom(s: RlsToolState, roomId: string): RlsToolState {
  const has = s.roomIds.includes(roomId);
  return { ...s, roomIds: has ? s.roomIds.filter((r) => r !== roomId) : [...s.roomIds, roomId] };
}

export const chooseSheet = (s: RlsToolState, sheetId: string | null): RlsToolState => ({ ...s, sheetId });

/** What stops tasks being made, or null when they can be. */
export function whyNot(s: RlsToolState): string | null {
  if (!s.sheetId) return "choose a sheet";
  if (s.roomIds.length === 0) return "click the rooms to make sheets for";
  return null;
}

/** "3 rooms" */
export const roomsText = (n: number): string => (n === 1 ? "1 room" : `${n} rooms`);

/** The result of one room of a make request, as the server answers (`tasks_api`). */
export interface Made {
  subject: string;
  room_name?: string;
  refused?: string;
}

/** One line for the bar after a request: what was queued and what was refused. */
export function madeText(made: readonly Made[]): string {
  const refused = made.filter((m) => m.refused);
  const queued = made.length - refused.length;
  const first = refused[0];
  const why = first ? ` Refused: ${first.room_name || first.subject}: ${first.refused}${refused.length > 1 ? ` (+${refused.length - 1} more)` : ""}` : "";
  return `Queued ${roomsText(queued)}.${why}`;
}
