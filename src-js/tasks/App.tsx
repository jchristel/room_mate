// The tasks page, served at /tasks/.
//
// Left: the project's tasks, newest first, narrowed by model and by state. Right: the selected task
// (what Revit is told, the findings, and each attempt with what the session said), or the panel that
// makes tasks. A person can dismiss a task or put it back in the queue; making, dismissing and
// resending are the only writes. A Revit session asks for and reports tasks over its own endpoints,
// so the list is read again every few seconds to show what it did.
//
// A task is a RECORD (plan decision 21): it holds what Revit is told as of when it was made. The
// resolver is not asked again here.

import { useCallback, useEffect, useState } from "react";

import * as api from "./api.js";
import { persistSelection, seedProjectId } from "./common.js";
import { MakeTasks } from "./MakeTasks.js";
import { TaskDetail } from "./TaskDetail.js";
import { TaskList } from "./TaskList.js";
import type { AssignmentRow, Made, ProjectRow, Task, TaskState, TaskSummary } from "./types.js";

const message = (e: unknown): string => String((e as Error).message ?? e);
const POLL_MS = 8000;

export function App() {
  const [projects, setProjects] = useState<ProjectRow[]>([]);
  const [projectId, setProjectId] = useState<string | null>(null);
  const [rows, setRows] = useState<TaskSummary[] | null>(null);
  const [selected, setSelected] = useState<Task | null>(null);
  const [model, setModel] = useState("");
  const [states, setStates] = useState<ReadonlySet<TaskState>>(new Set());
  const [making, setMaking] = useState(false);
  const [assignments, setAssignments] = useState<AssignmentRow[]>([]);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .listProjects()
      .then((list) => {
        setProjects(list);
        const first = list[0];
        if (!first) return;
        const seed = seedProjectId();
        const id = seed && list.some((p) => p.id === seed) ? seed : first.id;
        setProjectId(id);
        persistSelection(id);
      })
      .catch((e: unknown) => setError(message(e)));
  }, []);

  const refresh = useCallback(
    (project: string) => api.listTasks(project).then(setRows).catch((e: unknown) => setError(message(e))),
    [],
  );

  // A new project: start clean, read its tasks, and keep reading them.
  useEffect(() => {
    setRows(null);
    setSelected(null);
    setMaking(false);
    setModel("");
    if (!projectId) return;
    void refresh(projectId);
    const timer = window.setInterval(() => void refresh(projectId), POLL_MS);
    return () => window.clearInterval(timer);
  }, [projectId, refresh]);

  // What the make panel offers, read when it opens.
  useEffect(() => {
    if (!making || !projectId) return;
    api.listAssignments(projectId).then(setAssignments).catch((e: unknown) => setError(message(e)));
  }, [making, projectId]);

  // Keep the open task in step with the list: the session may have reported on it.
  const updatedAt = rows?.find((r) => r.id === selected?.id)?.updated_at;
  useEffect(() => {
    if (!projectId || !selected || !updatedAt || updatedAt === selected.updated_at) return;
    api.getTask(projectId, selected.id).then(setSelected).catch((e: unknown) => setError(message(e)));
  }, [projectId, selected, updatedAt]);

  const open = async (id: string) => {
    if (!projectId) return;
    setError(null);
    setNotice(null);
    setMaking(false);
    try {
      setSelected(await api.getTask(projectId, id));
    } catch (e) {
      setError(message(e));
    }
  };

  const act = async (what: "dismiss" | "resend") => {
    if (!projectId || !selected) return;
    if (what === "dismiss" && selected.state === "in_progress" && !window.confirm("A Revit session was handed this task. Dismiss it anyway? A late report from that session will be refused.")) return;
    setBusy(true);
    const r = await (what === "dismiss" ? api.dismiss(projectId, selected.id) : api.resend(projectId, selected.id));
    setBusy(false);
    if (!r.ok) return setError(r.error);
    setError(null);
    setNotice(what === "dismiss" ? "Dismissed." : "Put back in the queue.");
    setSelected(r.value);
    void refresh(projectId);
  };

  const toggleState = (s: TaskState) =>
    setStates((cur) => {
      const n = new Set(cur);
      if (n.has(s)) n.delete(s);
      else n.add(s);
      return n;
    });

  const made = (list: Made[]) => {
    const ok = list.filter((m) => m.task).length;
    setNotice(`Made ${ok} task${ok === 1 ? "" : "s"}${list.length > ok ? `, ${list.length - ok} refused` : ""}.`);
    if (projectId) void refresh(projectId);
  };

  return (
    <>
      <header>
        <h1>Tasks</h1>
        <select
          aria-label="Project"
          value={projectId ?? ""}
          onChange={(e) => {
            setProjectId(e.target.value);
            persistSelection(e.target.value);
          }}
        >
          {projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
        <div className="links">
          <a href="/">viewer</a>
          <a href="/blueprints/">blueprints</a>
          <a href="/library/">library</a>
          <a href="/reports/">reports</a>
          <a href="/settings/">settings</a>
        </div>
      </header>
      <div className="page">
        <TaskList
          rows={rows}
          selectedId={selected?.id ?? null}
          model={model}
          states={states}
          onModel={setModel}
          onToggleState={toggleState}
          onSelect={(id) => void open(id)}
          onMake={() => {
            setMaking(true);
            setSelected(null);
            setNotice(null);
          }}
        />
        <main>
          {error ? <p className="banner error">{error}</p> : null}
          {notice && !error ? <p className="banner">{notice}</p> : null}
          {making && projectId ? (
            <MakeTasks project={projectId} assignments={assignments} onMade={made} onClose={() => setMaking(false)} />
          ) : selected ? (
            <TaskDetail task={selected} busy={busy} onDismiss={() => void act("dismiss")} onResend={() => void act("resend")} />
          ) : (
            <p className="hint empty">Choose a task on the left, or make some.</p>
          )}
        </main>
      </div>
    </>
  );
}
