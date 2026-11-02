// Colour a plan by the last room task each room had.
//
// **Pure, and fed by the tasks list the tasks page reads** (`GET /projects/{id}/tasks`), so the
// plan and the page cannot disagree about what a task's state is. It is a colour mode of its own
// rather than a `ColourPlan`: a plan is a project setting over room properties, and a task is
// not a property of a room.
//
// **A room's colour is its newest task that is not dismissed.** A dismissed task is a person
// saying "ignore this", so it must not hide the one before it. "Newest" is `updated_at`, which a
// session's report moves, so a room that failed and was then resent reads queued again.

import { NO_DATA_COLOUR } from "./colour.js";
import type { Room } from "../renderer/types.js";

/** The name the zone's colour picker stores for this mode. A real plan could in principle be
 *  called this, which is why the picker lists it apart and the lookup tries it first. */
export const TASKS_PLAN = "Room tasks";

export type TaskState =
  | "queued"
  | "in_progress"
  | "completed"
  | "completed_with_warnings"
  | "failed"
  | "aborted"
  | "dismissed";

/** The part of a task summary this needs. */
export interface TaskRow {
  state: TaskState;
  updated_at: string;
  room: { room_id: string; number?: string | null };
}

export const TASK_STATE_COLOUR: Readonly<Record<Exclude<TaskState, "dismissed">, string>> = {
  queued: "#2c7fb8",
  in_progress: "#fee08b",
  completed: "#1a9850",
  completed_with_warnings: "#fdae61",
  failed: "#d73027",
  aborted: "#7b3294",
};

export const TASK_STATE_LABEL: Readonly<Record<keyof typeof TASK_STATE_COLOUR, string>> = {
  queued: "Queued",
  in_progress: "In progress",
  completed: "Completed",
  completed_with_warnings: "With warnings",
  failed: "Failed",
  aborted: "Aborted",
};

export type TaskIndex = ReadonlyMap<string, readonly TaskRow[]>;

/** Live tasks by room id, newest first. */
export function indexTasks(tasks: readonly TaskRow[]): TaskIndex {
  const byRoom = new Map<string, TaskRow[]>();
  for (const t of tasks) {
    if (t.state === "dismissed") continue;
    const list = byRoom.get(t.room.room_id);
    if (list) list.push(t);
    else byRoom.set(t.room.room_id, [t]);
  }
  for (const list of byRoom.values()) list.sort((a, b) => Date.parse(b.updated_at) - Date.parse(a.updated_at));
  return byRoom;
}

/** The room's `Number` property: how a task names its room besides the id. */
function numberOf(room: Room): string | null {
  const v = room.properties?.["Number"]?.value;
  return v != null && v !== "" ? String(v) : null;
}

/** The state a room's last task is in, or null when it has none.
 *
 *  A room id is unique only within a model and a viewer room does not name its model, so a task
 *  that carries a number must agree with the room's number too. A task without one matches on the
 *  id alone: that is the weaker test, and the only one there is. */
export function lastTaskState(room: Room, index: TaskIndex): TaskState | null {
  const list = index.get(room.id);
  if (!list) return null;
  const number = numberOf(room);
  const hit = list.find((t) => !t.room.number || number === null || t.room.number === number);
  return hit ? hit.state : null;
}

export function taskColourForRoom(room: Room, index: TaskIndex): string {
  const state = lastTaskState(room, index);
  return state && state !== "dismissed" ? TASK_STATE_COLOUR[state] : NO_DATA_COLOUR;
}
