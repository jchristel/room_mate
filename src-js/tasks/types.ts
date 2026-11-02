// The shapes the tasks page reads, as the server writes them (`src/tasks.rs`, `tasks_api.rs`).

import type { RuleKey } from "../library/types.js";

export type TaskKind = "populate_room" | "create_rls" | "update_room";

/** What an update changes: the items (add, remove, change, move) or only where they are. */
export type UpdateMode = "items" | "locations";

export type TaskState =
  | "queued"
  | "in_progress"
  | "completed"
  | "completed_with_warnings"
  | "failed"
  | "aborted"
  | "dismissed";

export interface TaskRoom {
  room_id: string;
  number?: string;
  name: string;
}

export interface BlueprintRef {
  id: string;
  name: string;
  revision: number;
}

export interface TaskSummary {
  id: string;
  kind: TaskKind;
  state: TaskState;
  model_id: string;
  room: TaskRoom;
  phase: string;
  overwrite_layout: boolean;
  create_rls: boolean;
  overwrite_rls: boolean;
  update_mode?: UpdateMode;
  /** An update that reports its difference and writes nothing. */
  dry_run: boolean;
  blueprint?: BlueprintRef;
  items: number;
  sheets: number;
  tag_rules: number;
  findings: number;
  created_at: string;
  updated_at: string;
  attempts: number;
  last_messages?: string[];
}

export interface TaskItem {
  item_id: string;
  family_name?: string;
  type_name: string;
  category: string;
  placement_type: "free" | "wall_device";
  host?: string;
  x_mm: number;
  y_mm: number;
  facing_x: number;
  facing_y: number;
  size_mm: [number, number];
}

export interface TaskSheet {
  revit_sheet_id: string;
  document_title: string;
  sheet_number: string;
  sheet_name?: string;
  sheet: { view_ports?: unknown[] } & Record<string, unknown>;
}

/** The resolver's findings and the task's own warnings: a `kind` and, for most, a `message`. */
export interface Finding {
  kind: string;
  message?: string;
  [key: string]: unknown;
}

export interface Attempt {
  started_at: string;
  finished_at?: string;
  status?: TaskState;
  messages?: string[];
}

export interface Offset {
  x: number;
  y: number;
}

/** One tag rule a task carries: keyed by the item's type code, offsets in paper millimetres in the element's frame. */
export interface TagRule {
  id: string;
  key: {
    tagged_category: string;
    code_parameter: string;
    type_code: string;
    tag_category: string;
    tag_family: string;
    tag_type: string;
    viewport_type: string;
  };
  tag_kind: string;
  placement: { head: Offset; rotation: number; has_leader: boolean };
  sample_count: number;
}

export interface Task extends Omit<TaskSummary, "items" | "sheets" | "tag_rules" | "findings" | "attempts" | "last_messages"> {
  library_sheet_id?: string;
  items: TaskItem[];
  sheets: TaskSheet[];
  tag_rules: TagRule[];
  findings: Finding[];
  attempts: Attempt[];
}

export interface ProjectRow {
  id: string;
  name: string;
}

export interface RoomRow {
  id: string;
  name: string;
  level_id: string;
}

export interface AssignmentRow {
  id: string;
  blueprint_id: string;
  blueprint_name: string | null;
  revision: number;
  room: { model_id: string; room_id: string };
  room_name: string;
  blueprint_missing: boolean;
}

export interface SheetRow {
  id: string;
  sheet_number: string;
  sheet_name: string;
  document_title: string;
  room?: { model_id: string; room_id: string };
}

/** What happened to one room of a make request: the task made, or why not. */
export interface Made {
  subject: string;
  room_name: string;
  task?: TaskSummary;
  refused?: string;
}
