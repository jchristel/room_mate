// The endpoints the tasks page calls. Writes never throw on an HTTP error: the server's own text is
// shown to the person, verbatim.

import { apiGet, apiSend } from "./common.js";
import type { AssignmentRow, Made, ProjectRow, RoomRow, SheetRow, Task, TaskState, TaskSummary, UpdateMode } from "./types.js";

export type Sent<T> = { ok: true; value: T } | { ok: false; error: string };

const base = (project: string) => `/projects/${encodeURIComponent(project)}`;

async function send<T>(method: string, url: string, body: unknown): Promise<Sent<T>> {
  const r = await apiSend(method, url, body);
  if (!r.ok) return { ok: false, error: r.text || `HTTP ${r.status}` };
  try {
    return { ok: true, value: (r.text ? JSON.parse(r.text) : null) as T };
  } catch {
    return { ok: false, error: "the server's answer was not JSON" };
  }
}

const whole = (r: Sent<Task>): Sent<Task> =>
  r.ok ? { ok: true, value: { ...r.value, items: r.value.items ?? [], sheets: r.value.sheets ?? [], tag_rules: r.value.tag_rules ?? [], findings: r.value.findings ?? [], attempts: r.value.attempts ?? [] } } : r;

export const listProjects = () => apiGet<ProjectRow[]>("/projects");
export const listTasks = (project: string) => apiGet<TaskSummary[]>(`${base(project)}/tasks`);
/** The server leaves an empty list out of a task, so read it back with the lists there. */
export const getTask = async (project: string, id: string): Promise<Task> => {
  const t = await apiGet<Partial<Task> & Omit<Task, "items" | "sheets" | "tag_rules" | "findings" | "attempts">>(`${base(project)}/tasks/${encodeURIComponent(id)}`);
  return { ...t, items: t.items ?? [], sheets: t.sheets ?? [], tag_rules: t.tag_rules ?? [], findings: t.findings ?? [], attempts: t.attempts ?? [] };
};
export const listAssignments = (project: string) => apiGet<AssignmentRow[]>(`${base(project)}/assignments`);
export const listSheets = (project: string) => apiGet<SheetRow[]>(`${base(project)}/sheets`);
export const listRooms = (project: string) =>
  apiGet<{ rooms: RoomRow[]; levels: { id: string; name: string }[] }>(`/rooms?project=${encodeURIComponent(project)}`);

export const dismiss = (project: string, id: string) =>
  send<Task>("POST", `${base(project)}/tasks/${encodeURIComponent(id)}/dismiss`, null).then(whole);
export const resend = (project: string, id: string) =>
  send<Task>("POST", `${base(project)}/tasks/${encodeURIComponent(id)}/resend`, null).then(whole);

export interface PopulateRequest {
  assignment_ids: string[];
  phase: string;
  overwrite_layout: boolean;
  create_rls: boolean;
  overwrite_rls: boolean;
}

export const makePopulate = (project: string, r: PopulateRequest) =>
  send<Made[]>("POST", `${base(project)}/tasks`, { kind: "populate_room", ...r });

export const makeUpdate = (project: string, assignmentIds: string[], phase: string, mode: UpdateMode, dryRun: boolean) =>
  send<Made[]>("POST", `${base(project)}/tasks`, {
    kind: "update_room",
    assignment_ids: assignmentIds,
    phase,
    mode,
    dry_run: dryRun,
  });

export type { TaskState };
