// The RLS tool's rules, with no store and no DOM (the blueprint tool's shape).
//
// A room layout sheet is made from a LIBRARY SHEET and the rooms it is made for, so the tool is a sheet
// choice and a set of rooms: a click on a room adds it, a second click takes it off. Making it queues a
// `create_rls` task per room for the Revit session (the RLS create button). **Nothing here places
// anything**: the server works out where the elevation markers and views go from the walls, and the
// views take their room's phase in Revit, so there is no phase to enter.

export interface RlsToolState {
  /** The library sheet to copy the layout of, or null before one is chosen. */
  sheetId: string | null;
  /** The rooms to make a sheet for, in the order they were picked. */
  roomIds: string[];
  /** Replace a sheet that already exists (off, it is skipped with a warning). */
  overwrite: boolean;
}

export const newRlsTool = (): RlsToolState => ({ sheetId: null, roomIds: [], overwrite: false });

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
