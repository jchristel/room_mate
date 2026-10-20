// Making tasks. Two kinds, each with its own input (plan decision 26): **populate** takes
// assignments (a blueprint already placed in a room, its mapping and revision pinned) and
// **create RLS** takes a library sheet and the rooms to make sheets for. The server makes each
// room on its own, so the answer is a row per room: the task, or why it could not be made.

import { useMemo, useState } from "react";

import * as api from "./api.js";
import type { AssignmentRow, Made, RoomRow, SheetRow } from "./types.js";

export interface MakeProps {
  project: string;
  assignments: readonly AssignmentRow[];
  sheets: readonly SheetRow[];
  rooms: readonly RoomRow[];
  onMade(made: Made[]): void;
  onClose(): void;
}

type Kind = "populate_room" | "create_rls";

export function MakeTasks({ project, assignments, sheets, rooms, onMade, onClose }: MakeProps) {
  const [kind, setKind] = useState<Kind>("populate_room");
  const [phase, setPhase] = useState("");
  const [filter, setFilter] = useState("");
  const [picked, setPicked] = useState<ReadonlySet<string>>(new Set());
  const [overwriteLayout, setOverwriteLayout] = useState(false);
  const [createRls, setCreateRls] = useState(false);
  const [overwriteRls, setOverwriteRls] = useState(false);
  const [sheetId, setSheetId] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [made, setMade] = useState<Made[] | null>(null);

  const f = filter.trim().toLowerCase();
  const subjects = useMemo(
    () =>
      kind === "populate_room"
        ? assignments
            .filter((a) => !a.blueprint_missing && (!f || a.room_name.toLowerCase().includes(f) || (a.blueprint_name ?? "").toLowerCase().includes(f)))
            .map((a) => ({ id: a.id, label: a.room_name, sub: `${a.blueprint_name ?? "?"} r${a.revision}` }))
        : rooms.filter((r) => !f || r.name.toLowerCase().includes(f)).slice(0, 200).map((r) => ({ id: r.id, label: r.name, sub: "" })),
    [kind, assignments, rooms, f],
  );

  const toggle = (id: string) =>
    setPicked((p) => {
      const n = new Set(p);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  const switchKind = (k: Kind) => {
    setKind(k);
    setPicked(new Set());
    setMade(null);
    setError(null);
  };

  const ready = phase.trim() !== "" && picked.size > 0 && (kind === "populate_room" || sheetId !== "");

  const submit = async () => {
    setBusy(true);
    setError(null);
    const ids = [...picked];
    const r =
      kind === "populate_room"
        ? await api.makePopulate(project, { assignment_ids: ids, phase: phase.trim(), overwrite_layout: overwriteLayout, create_rls: createRls, overwrite_rls: overwriteRls })
        : await api.makeRls(project, sheetId, ids, phase.trim(), overwriteRls);
    setBusy(false);
    if (!r.ok) return setError(r.error);
    setMade(r.value);
    // Ones made are done with; ones refused stay ticked so the reason can be dealt with.
    setPicked(new Set(r.value.filter((m) => m.refused).map((m) => m.subject)));
    onMade(r.value);
  };

  return (
    <div className="make">
      <div className="toolbar">
        <strong>Make tasks</strong>
        <span className="spacer" />
        <button type="button" onClick={onClose}>
          Close
        </button>
      </div>
      <div className="actions">
        <button type="button" className={kind === "populate_room" ? "on" : ""} onClick={() => switchKind("populate_room")}>
          Populate rooms from a blueprint
        </button>
        <button type="button" className={kind === "create_rls" ? "on" : ""} onClick={() => switchKind("create_rls")}>
          Create RLS from a library sheet
        </button>
      </div>

      <label className="stack">
        Phase to work in (the name Revit knows it by)
        <input type="text" value={phase} onChange={(e) => setPhase(e.target.value)} placeholder="New Construction" />
      </label>

      {kind === "populate_room" ? (
        <>
          <label>
            <input type="checkbox" checked={overwriteLayout} onChange={(e) => setOverwriteLayout(e.target.checked)} /> Overwrite the room's existing layout (deletes its content in the phase; off, the task aborts if the room has any)
          </label>
          <label>
            <input type="checkbox" checked={createRls} onChange={(e) => setCreateRls(e.target.checked)} /> Create the room layout sheets from the blueprint
          </label>
          {createRls ? (
            <label>
              <input type="checkbox" checked={overwriteRls} onChange={(e) => setOverwriteRls(e.target.checked)} /> Overwrite sheets that already exist (off, they are skipped with a warning)
            </label>
          ) : null}
        </>
      ) : (
        <>
          <label className="stack">
            Library sheet
            <select value={sheetId} onChange={(e) => setSheetId(e.target.value)}>
              <option value="">Choose a sheet…</option>
              {sheets.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.sheet_number} {s.sheet_name} ({s.document_title})
                </option>
              ))}
            </select>
          </label>
          <label>
            <input type="checkbox" checked={overwriteRls} onChange={(e) => setOverwriteRls(e.target.checked)} /> Overwrite a sheet that already exists (off, it is skipped with a warning)
          </label>
        </>
      )}

      <h3>{kind === "populate_room" ? "Assignments" : "Rooms"}</h3>
      <input type="search" placeholder="Filter by name" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Filter" />
      {subjects.length === 0 ? <p className="hint">{kind === "populate_room" ? "No assignments. Assign a blueprint to rooms on the blueprints page first." : "No rooms."}</p> : null}
      <ul className="picks">
        {subjects.map((s) => (
          <li key={s.id}>
            <label>
              <input type="checkbox" checked={picked.has(s.id)} onChange={() => toggle(s.id)} /> {s.label} <span>{s.sub}</span>
            </label>
          </li>
        ))}
      </ul>
      <div className="actions">
        <button type="button" disabled={busy || !ready} onClick={() => void submit()}>
          {busy ? "Working…" : `Make ${picked.size} task${picked.size === 1 ? "" : "s"}`}
        </button>
        <button type="button" disabled={picked.size === 0} onClick={() => setPicked(new Set())}>
          Clear
        </button>
      </div>
      {error ? <p className="banner error">{error}</p> : null}
      {made ? (
        <ul className="findings">
          {made.map((m) => (
            <li key={m.subject} className={m.refused ? "refused" : "made"}>
              {m.refused ? "✗" : "✓"} <b>{m.room_name || m.subject}</b>: {m.refused ?? "queued"}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
