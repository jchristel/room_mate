// Which levels a payload has, which rooms are on one, and how a zone keeps a
// level selected across a new payload.
//
// Pure, and out of the components for the reason `scope.ts` is: the rules are
// small, they are shared by the level picker, the meta line and the storey
// scope of every element read, and two of them are only wrong in ways nobody
// sees — a level list that silently loses the level a zone is showing, or a
// room whose `level_id` is absent quietly vanishing from every level.

import type { Level, Room } from "../renderer/types.js";

/** The id a room with no `level_id` is filed under. A single bucket rather
 *  than dropping the room: a room the payload cannot place is still a room,
 *  and a plan that silently omits it is the worse answer. */
export const UNKNOWN_LEVEL = "unknown";

/** A payload, as far as the level rules read it. */
export interface LevelSource {
  levels?: readonly Level[] | undefined;
  rooms?: readonly Room[] | undefined;
}

/**
 * The payload's levels, lowest elevation first.
 *
 * A payload that declares none is not an error: the rooms are grouped by
 * their own `level_id`, each becoming a level named after its id at elevation
 * 0. That keeps the picker and the storey scope working against an older
 * snapshot, which is the only place this case comes from.
 */
export function levelsForPayload(payload: LevelSource): Level[] {
  const declared = payload.levels;
  if (declared?.length) return declared.slice().sort((a, b) => a.elevation - b.elevation);

  const seen = new Map<string, Level>();
  for (const room of payload.rooms ?? []) {
    const id = room.level_id || UNKNOWN_LEVEL;
    if (!seen.has(id)) seen.set(id, { id, name: id, elevation: 0 });
  }
  return [...seen.values()];
}

/** The rooms on one level. */
export function roomsOnLevel(payload: LevelSource, levelId: string | null): Room[] {
  if (!levelId) return [];
  return (payload.rooms ?? []).filter((r) => (r.level_id || UNKNOWN_LEVEL) === levelId);
}

/** The picker's order: HIGHEST elevation first, which is how a stack of floors
 *  is read on paper. The opposite of `levelsForPayload`'s order, deliberately —
 *  that one is the data's order and this one is the reader's. */
export function pickerOrder(levels: readonly Level[]): Level[] {
  return levels.slice().sort((a, b) => b.elevation - a.elevation);
}

/** A level option's text: the name and how many rooms are on it. */
export function levelLabel(payload: LevelSource, level: Level): string {
  const n = roomsOnLevel(payload, level.id).length;
  return `${level.name} (${n} rm)`;
}

/**
 * The level a zone should show, given a new payload.
 *
 * Keeps the zone's current level while the payload still has it — a push must
 * not move the reader to another floor — and otherwise takes the first, which
 * is the lowest. `null` when the payload has no levels at all, which is the
 * "no rooms in this scope" state rather than a missing selection.
 */
export function resolveLevel(levels: readonly Level[], current: string | null): string | null {
  if (current && levels.some((l) => l.id === current)) return current;
  return levels.length ? levels[0]!.id : null;
}
