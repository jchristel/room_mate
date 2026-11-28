// The approve flow's rules: which rooms of an impact read a person may tick, and what to say about each. The impact itself
// is the server's (`service::blueprint_impact`); this file only chooses and words.

import type { AssignmentRow, RoomImpact, RoomStatus } from "./types.js";

/** A room can be approved when there is something to do, or nothing and the person wants it re-pinned. A held nudge, a
 *  fit that does not hold, a room that has gone and an error are all a person's to look at first. */
export const canTick = (status: RoomStatus): boolean => status === "changes" || status === "no_change";

/** Whether the room carries its own changes (nudges, leave-outs, additions) the person may want to look at before
 *  letting an update through. */
export const isOverridden = (a: AssignmentRow | undefined): boolean => (a?.overrides?.length ?? 0) + (a?.additions?.length ?? 0) > 0;

/** The rooms to tick for "everything with changes", or only those with no changes of their own ("all not overridden"). */
export function tickable(rooms: readonly RoomImpact[], byId: ReadonlyMap<string, AssignmentRow>, notOverridden: boolean): Set<string> {
  return new Set(
    rooms
      .filter((r) => r.status === "changes" && !(notOverridden && isOverridden(byId.get(r.assignment_id))))
      .map((r) => r.assignment_id),
  );
}

export const STATUS_TEXT: Record<RoomStatus, string> = {
  error: "could not be read",
  room_gone: "room is not in the model any more",
  cannot_judge: "the revision does not fit this room",
  needs_decision: "a nudge is held: decide in the assign view first",
  changes: "has changes",
  no_change: "already as the revision has it",
};

/** "2 to add, 1 to move, 3 to look at", or "nothing to do". */
export function planText(counts: RoomImpact["plan_counts"]): string {
  const parts: string[] = [];
  if (counts.add) parts.push(`${counts.add} to add`);
  if (counts.move) parts.push(`${counts.move} to move`);
  if (counts.report) parts.push(`${counts.report} to look at`);
  return parts.length === 0 ? "nothing to do" : parts.join(", ");
}
