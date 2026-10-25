// What the tasks page knows about a task that is not drawing: the words for a state, what a person
// may do with a task in each state, and how a finding reads. Pure, so it is tested.

import type { Finding, Task, TaskKind, TaskState, TaskSummary, UpdateMode } from "./types.js";

export const STATES: readonly TaskState[] = [
  "queued",
  "in_progress",
  "completed",
  "completed_with_warnings",
  "failed",
  "aborted",
  "dismissed",
];

export const STATE_TEXT: Record<TaskState, string> = {
  queued: "queued",
  in_progress: "in progress",
  completed: "completed",
  completed_with_warnings: "completed with warnings",
  failed: "failed",
  aborted: "aborted",
  dismissed: "dismissed",
};

export const KIND_TEXT: Record<TaskKind, string> = {
  populate_room: "Populate room",
  create_rls: "Create RLS",
  update_room: "Update room",
};

export const UPDATE_MODE_TEXT: Record<UpdateMode, string> = {
  items: "items (add, remove, change, move)",
  locations: "locations only (move)",
};

/** A task a person can still drop: any that is not already dismissed (including one in progress,
 *  which is how a session that will never report is cleared). */
export const canDismiss = (state: TaskState): boolean => state !== "dismissed";

/** A task a person can put back in the queue: any but one already waiting. */
export const canResend = (state: TaskState): boolean => state !== "queued";

/** Needs a person's eye: the session said it did not finish cleanly. */
export const needsAttention = (state: TaskState): boolean =>
  state === "failed" || state === "aborted" || state === "completed_with_warnings";

export function countByState(rows: readonly TaskSummary[]): Record<TaskState, number> {
  const out = Object.fromEntries(STATES.map((s) => [s, 0])) as Record<TaskState, number>;
  for (const r of rows) out[r.state] += 1;
  return out;
}

/** The models the tasks are for, sorted, for the filter. */
export const modelsOf = (rows: readonly TaskSummary[]): string[] => [...new Set(rows.map((r) => r.model_id))].sort();

/** Newest first: what a person wants to see after making or running tasks. */
export const newestFirst = (rows: readonly TaskSummary[]): TaskSummary[] =>
  [...rows].sort((a, b) => b.created_at.localeCompare(a.created_at) || a.id.localeCompare(b.id));

/** A finding as one sentence. The resolver's findings name items by id, which the task can turn
 *  into the item's type; a finding that carries its own message is shown as it is. */
export function findingText(f: Finding, task: Pick<Task, "items">): string {
  const name = (id: string): string => task.items.find((i) => i.item_id === id)?.type_name ?? `item ${id}`;
  switch (f.kind) {
    case "outside":
      return `${name(String(f.item))} is outside the room (it was inside the blueprint's)`;
    case "overlap":
      return `${name(String(f.a))} and ${name(String(f.b))} overlap (they did not in the blueprint)`;
    case "follow_broken":
      return `${name(String(f.item))}: ${String(f.reason)}`;
    case "corner_anchored":
      return `${name(String(f.item))} had no wall beside it when captured; its position is approximate`;
    default:
      return typeof f.message === "string" ? f.message : f.kind;
  }
}

/** "2026-10-20T09:10:22.430142Z" as "2026-10-20 09:10". */
export const shortTime = (iso: string): string => iso.slice(0, 16).replace("T", " ");
