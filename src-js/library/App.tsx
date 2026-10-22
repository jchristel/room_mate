// The library page, served at /library/: what Revit has taught the server about layout sheets.
//
// Two tabs. **Sheets** is the room layout sheet library (plan track B): which sheets exist, which
// room each was made for, renumbered pushes waiting for a decision, and a ranking of sheets against
// a room. **Tag rules** is where a tag goes (decision 24), learned from the tags on those sheets.
// Both are authored data kept by identity, not snapshots, so this page reads and edits them in
// place; making a task from a sheet is on the tasks page.

import { useCallback, useEffect, useState } from "react";

import * as api from "./api.js";
import { persistSelection, seedProjectId } from "./common.js";
import { RulesTab } from "./RulesTab.js";
import { SheetsTab } from "./SheetsTab.js";
import type { ProjectRow, RoomRow } from "./types.js";

type Tab = "sheets" | "rules";

const message = (e: unknown): string => String((e as Error).message ?? e);
const tabFromHash = (): Tab => (window.location.hash === "#rules" ? "rules" : "sheets");

export function App() {
  const [projects, setProjects] = useState<ProjectRow[]>([]);
  const [projectId, setProjectId] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>(tabFromHash);
  const [rooms, setRooms] = useState<RoomRow[]>([]);
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

  // The rooms name a sheet's room; a project with none pushed just has no names.
  useEffect(() => {
    setRooms([]);
    if (!projectId) return;
    api.listRooms(projectId).then(setRooms).catch(() => setRooms([]));
  }, [projectId]);

  const choose = (t: Tab) => {
    setTab(t);
    setError(null);
    setNotice(null);
    window.history.replaceState(null, "", t === "rules" ? "#rules" : "#");
  };

  const onError = useCallback((t: string | null) => setError(t), []);
  const onNotice = useCallback((t: string | null) => setNotice(t), []);

  return (
    <>
      <header>
        <h1>Library</h1>
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
        <div className="tabs" role="tablist">
          <button type="button" role="tab" aria-selected={tab === "sheets"} className={tab === "sheets" ? "on" : ""} onClick={() => choose("sheets")}>
            Sheets
          </button>
          <button type="button" role="tab" aria-selected={tab === "rules"} className={tab === "rules" ? "on" : ""} onClick={() => choose("rules")}>
            Tag rules
          </button>
        </div>
        <div className="links">
          <a href="/">viewer</a>
          <a href="/blueprints/">blueprints</a>
          <a href="/tasks/">tasks</a>
          <a href="/reports/">reports</a>
          <a href="/settings/">settings</a>
        </div>
      </header>
      <div className="body">
        {error ? <p className="banner error">{error}</p> : null}
        {notice && !error ? <p className="banner">{notice}</p> : null}
        {projectId ? (
          tab === "sheets" ? (
            <SheetsTab project={projectId} rooms={rooms} onError={onError} onNotice={onNotice} />
          ) : (
            <RulesTab project={projectId} onError={onError} onNotice={onNotice} />
          )
        ) : (
          <p className="hint empty">No project.</p>
        )}
      </div>
    </>
  );
}
