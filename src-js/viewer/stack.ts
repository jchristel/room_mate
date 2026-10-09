// The stack view's rules, with no store and no DOM.
//
// A STACK is the rooms joined by vertical links, drawn as a column of levels. It
// shows **only the levels that hold a room of the stack** (and the one level a
// suggestion points at), never the project's other levels: RHH has two buildings
// with distinct levels, and a hospital lift's column must not list the car park's
// half-levels it passes. That is why the rows are built from the rooms, not from
// the level list.
//
// Nothing here is stored. The links are the authored record (`connections.ts`); a
// stack is a reading of them, and a suggestion is the server's `/stack` answer.

import type { SavedLink } from "./connections.js";

export interface StackRoom {
  id: string;
  name: string;
  levelId: string;
}

export interface StackRow {
  levelId: string;
  levelName: string;
  elevation: number;
  rooms: StackRoom[];
}

/** What joins one row to the one below it. */
export interface StackJoin {
  /** The links between a room of the upper row and a room of the lower one. */
  links: SavedLink[];
}

export interface Stack {
  /** Highest first, the way a building is read. */
  rows: StackRow[];
  /** `joins[i]` sits between `rows[i]` and `rows[i + 1]`. */
  joins: StackJoin[];
  /** The room a suggestion above would attach to, and the one below. */
  top: StackRoom | null;
  bottom: StackRoom | null;
}

export interface LevelInfo {
  id: string;
  name: string;
  elevation: number;
}

/**
 * The stack `anchor` belongs to: every room reachable from it through vertical
 * links, grouped by level.
 *
 * A room with no link is a stack of one row. Rooms the page does not hold (a link
 * naming a room that left the model) are left out, not drawn as blanks: the
 * server reports those on the link.
 */
export function buildStack(
  anchor: string | null,
  links: readonly SavedLink[],
  rooms: ReadonlyMap<string, { name?: string | undefined; level_id?: string | undefined }>,
  levels: readonly LevelInfo[],
): Stack | null {
  if (anchor === null || !rooms.has(anchor)) return null;
  const linked = new Map<string, SavedLink[]>();
  for (const l of links) {
    for (const end of [l.a.room_id, l.b.room_id]) linked.set(end, [...(linked.get(end) ?? []), l]);
  }

  const seen = new Set<string>([anchor]);
  const queue = [anchor];
  while (queue.length > 0) {
    const at = queue.shift()!;
    for (const l of linked.get(at) ?? []) {
      const next = l.a.room_id === at ? l.b.room_id : l.a.room_id;
      if (!seen.has(next) && rooms.has(next)) {
        seen.add(next);
        queue.push(next);
      }
    }
  }

  const info = new Map(levels.map((l) => [l.id, l]));
  const byLevel = new Map<string, StackRoom[]>();
  for (const id of seen) {
    const r = rooms.get(id)!;
    const levelId = r.level_id ?? "";
    byLevel.set(levelId, [...(byLevel.get(levelId) ?? []), { id, name: r.name || id, levelId }]);
  }
  const rows: StackRow[] = [...byLevel.entries()]
    .map(([levelId, rs]) => ({
      levelId,
      levelName: info.get(levelId)?.name ?? levelId,
      elevation: info.get(levelId)?.elevation ?? 0,
      rooms: rs,
    }))
    .sort((a, b) => b.elevation - a.elevation);

  const joins: StackJoin[] = rows.slice(0, -1).map((upper, i) => {
    const lower = rows[i + 1]!;
    const up = new Set(upper.rooms.map((r) => r.id));
    const low = new Set(lower.rooms.map((r) => r.id));
    return {
      links: links.filter(
        (l) =>
          (up.has(l.a.room_id) && low.has(l.b.room_id)) || (up.has(l.b.room_id) && low.has(l.a.room_id)),
      ),
    };
  });

  return {
    rows,
    joins,
    top: rows[0]?.rooms[0] ?? null,
    bottom: rows[rows.length - 1]?.rooms[0] ?? null,
  };
}

/** The `/stack` URL for one room. */
export function stackUrl(projectId: string | null, roomId: string, milestone: string | null): string | null {
  if (!projectId) return null;
  const q = new URLSearchParams({ room: roomId });
  if (milestone) q.set("milestone", milestone);
  return `/projects/${encodeURIComponent(projectId)}/stack?${q.toString()}`;
}

/** The server's answer for one side, as the page reads it. */
export interface StackSide {
  level_id: string;
  level_name: string;
  confidence: "clear" | "ambiguous";
  candidates: {
    model_id: string;
    room_id: string;
    name: string;
    overlap: number;
    fraction_of_room: number;
    same_stem: boolean;
  }[];
}

export interface StackAnswer {
  room: { model_id: string; room_id: string; name: string; level_id: string; level_name: string };
  up: StackSide | null;
  down: StackSide | null;
}

/** An overlap as the percentage a reader weighs. */
export function percent(overlap: number): string {
  return `${Math.round(Math.min(overlap, 1) * 100)}%`;
}

/** One line saying how sure a side's suggestion is. */
export function describeSide(side: StackSide | null): string {
  if (!side) return "nothing sits over or under this room: a hop here is drawn by hand";
  const n = side.candidates.length;
  if (side.confidence === "clear") return `one room fits on ${side.level_name}`;
  return `${n} rooms fit on ${side.level_name}: choose one`;
}
