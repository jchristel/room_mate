// The RLS tool's bar: which library sheet to copy, the rooms picked (click rooms on the plan), and the button that
// queues a `create_rls` task per room for the Revit session. Mounted inside a zone beside the blueprint bar and
// renders nothing while the tool is off. Set mode (several adjacent rooms on one set of sheets) is `RlsSetPanel`.
//
// Nothing is asked about a phase (the views take their room's) or about where things go (the server works that
// out from the walls). The preview under the bar is the sheet's layout, so a person knows what is being copied.

import { useEffect, useState } from "react";

import { SheetPreview } from "../../library/SheetPreview.js";
import type { Room } from "../../renderer/types.js";
import {
  autoBatches,
  categoryOf,
  chooseSheet,
  doneRoomIds,
  includeAll,
  madeText,
  NO_SHEET,
  roomsText,
  setOverride,
  solverSet,
  tally,
  whyNot,
  type Category,
  type Made,
  type RlsMode,
} from "../rlsTool.js";
import { RlsSetPanel } from "./RlsSetPanel.js";
import { getLibrarySheet, listLibrarySheets, makeRlsTasks, solveRooms, type LibrarySheet, type LibrarySheetRecord } from "./rlsData.js";
import { getState, patchZoneRls, setZoneRlsMode, type ZoneRow } from "./store.js";
import { watchRoomTasks } from "./taskData.js";
import { useViewer } from "./useViewer.js";

const CATEGORY_LABEL: Record<Category, string> = { green: "green (80+)", yellow: "yellow (60-80)", red: "red (below 60)", override: "overrides" };

export function RlsPanel({ zone, rooms }: { zone: ZoneRow; rooms: readonly Room[] }) {
  const { scope, payload, toolFocus, roomTasks } = useViewer();
  const tool = zone.rls;
  const zoneId = zone.id;
  const active = tool !== null;
  const project = scope.projectId;
  const sheetId = tool?.sheetId ?? null;
  const [sheets, setSheets] = useState<LibrarySheet[] | null>(null);
  const [record, setRecord] = useState<LibrarySheetRecord | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // The library's sheets, read while the tool is on.
  useEffect(() => {
    if (!active || !project) return;
    const ac = new AbortController();
    setSheets(null);
    listLibrarySheets(project, ac.signal).then(setSheets).catch(() => !ac.signal.aborted && setSheets([]));
    return () => ac.abort();
  }, [active, project]);

  // The chosen sheet in full, for its preview.
  useEffect(() => {
    setRecord(null);
    if (!active || !project || !sheetId) return;
    const ac = new AbortController();
    getLibrarySheet(project, sheetId, ac.signal).then(setRecord).catch(() => undefined);
    return () => ac.abort();
  }, [active, project, sheetId]);

  // Auto mode reads the room tasks (the rooms with one leave the solver set) while it is on.
  const auto = tool?.mode === "auto";
  useEffect(() => (auto ? watchRoomTasks() : undefined), [auto]);

  // The solver set: this storey's rooms without a task, plus the ones put back.
  const done = doneRoomIds(roomTasks);
  const set = tool && auto ? solverSet(rooms.map((r) => r.id), done, tool) : [];
  const setKey = set.join("\u0000");
  useEffect(() => {
    if (!auto || !project || set.length === 0) return;
    const ac = new AbortController();
    void solveRooms(project, set, ac.signal).then((answer) => {
      if (ac.signal.aborted) return;
      if (typeof answer === "string") return answer ? setNotice(answer) : undefined;
      const solved = Object.fromEntries(answer.map((a) => [a.room_id, a]));
      const now = getZoneTool(zoneId);
      if (now) patchZoneRls(zoneId, { ...now, solved });
    });
    return () => ac.abort();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [auto, project, setKey]);

  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && toolFocus === zoneId) setZoneRlsMode(zoneId, false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active, toolFocus, zoneId]);

  if (!tool) return null;
  const mode = (m: RlsMode) => patchZoneRls(zoneId, { ...tool, mode: m });
  const blocked = whyNot(tool);
  const name = (id: string) => payload?.rooms?.find((r) => r.id === id)?.name ?? id;

  const make = async () => {
    if (!project || !tool.sheetId) return;
    setBusy(true);
    setNotice(null);
    const answer = await makeRlsTasks(
      project,
      tool.sheetId,
      tool.roomIds,
      tool.overwrite,
      Boolean(sheets?.find((s) => s.id === tool.sheetId)?.blueprint),
    );
    setBusy(false);
    if (typeof answer === "string") return setNotice(answer);
    setNotice(madeText(answer));
    // Rooms that got a task are done with; refused ones stay picked so the reason can be dealt with.
    patchZoneRls(zoneId, { ...tool, roomIds: answer.filter((m) => m.refused).map((m) => m.subject) });
  };

  const batches = autoBatches(tool, set);
  const queued = [...batches.values()].reduce((n, r) => n + r.length, 0);
  const counts = tally(tool, set);
  const focus = tool.focus;
  const makeAuto = async () => {
    if (!project) return;
    setBusy(true);
    setNotice(null);
    const made: Made[] = [];
    for (const [sheet, ids] of batches) {
      const answer = await makeRlsTasks(project, sheet, ids, tool.overwrite, Boolean(sheets?.find((x) => x.id === sheet)?.blueprint));
      if (typeof answer === "string") {
        setBusy(false);
        return setNotice(answer);
      }
      made.push(...answer);
    }
    setBusy(false);
    setNotice(madeText(made));
  };

  if (tool.mode === "set") return <RlsSetPanel zoneId={zoneId} tool={tool} project={project} sheets={sheets} name={name} />;

  if (auto) {
    const nameOf = (id: string) => rooms.find((r) => r.id === id)?.name ?? id;
    const best = focus ? tool.solved[focus]?.choices[0] : undefined;
    return (
      <div className="routeBar rlsBar">
        <strong>RLS</strong>
        <select className="picker" value={tool.mode} onChange={(e) => mode(e.target.value as RlsMode)} aria-label="RLS mode">
          <option value="manual">RLS manual</option>
          <option value="auto">RLS auto</option>
          <option value="set">RLS set</option>
        </select>
        <span className="routeResult" title="Rooms of this level without a task are matched to the closest library sheet by area, proportions and item count">
          {set.length} rooms matched: {counts.green} green, {counts.yellow} yellow, {counts.red} red, {counts.override} overridden
          {done.size > 0 ? `; ${rooms.filter((r) => done.has(r.id) && !tool.included.includes(r.id)).length} faded (have a task)` : ""}
        </span>
        <button className="chip" title="Put every faded room (one that has a task) back in the solver set. Double click one room to put back just that one." onClick={() => patchZoneRls(zoneId, includeAll(tool, new Set(rooms.filter((r) => done.has(r.id)).map((r) => r.id))))}>
          Include all
        </button>
        {(["green", "yellow", "red", "override"] as const).map((c) => (
          <label key={c} title={`Queue the ${CATEGORY_LABEL[c]} rooms`}>
            <input type="checkbox" checked={tool.bands[c]} onChange={(e) => patchZoneRls(zoneId, { ...tool, bands: { ...tool.bands, [c]: e.target.checked } })} /> {CATEGORY_LABEL[c]}
          </label>
        ))}
        <label title="Replace a sheet that already exists (off, it is skipped with a warning)">
          <input type="checkbox" checked={tool.overwrite} onChange={(e) => patchZoneRls(zoneId, { ...tool, overwrite: e.target.checked })} /> overwrite
        </label>
        {focus ? (
          <span className="rlsRooms">
            <strong>{nameOf(focus)}</strong>
            {best ? ` ${Math.round(best.fit)}% ${best.sheet_number}` : ` ${tool.solved[focus]?.reason ?? ""}`}
            <select
              className="picker"
              value={tool.overrides[focus] ?? ""}
              onChange={(e) => patchZoneRls(zoneId, setOverride(tool, focus, e.target.value || null))}
              aria-label="Sheet for this room"
            >
              <option value="">{categoryOf(tool, focus) === "override" ? "Back to the solver's choice" : "Solver's choice"}</option>
              <option value={NO_SHEET}>No sheet (leave this room out)</option>
              {sheets?.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.sheet_number} {s.sheet_name}
                </option>
              ))}
            </select>
          </span>
        ) : (
          <span className="meta">click a room to override its sheet</span>
        )}
        <button className="chip on" disabled={busy || queued === 0} title={queued === 0 ? "No room in the ticked categories has a sheet" : "Queue a room layout sheet task per room, for the Revit session to make (the Tasks button)"} onClick={() => void makeAuto()}>
          {busy ? "Working…" : `Make ${queued} task${queued === 1 ? "" : "s"}`}
        </button>
        {notice ? <span className="meta">{notice}</span> : null}
        <a className="chip" href={`/tasks/?project=${encodeURIComponent(project ?? "")}`}>
          Tasks page
        </a>
        <button className="chip" title="Leave the RLS tool (Esc)" onClick={() => setZoneRlsMode(zoneId, false)}>
          Close
        </button>
      </div>
    );
  }

  return (
    <>
      <div className="routeBar rlsBar">
        <strong>RLS</strong>
        <select className="picker" value={tool.mode} onChange={(e) => mode(e.target.value as RlsMode)} aria-label="RLS mode">
          <option value="manual">RLS manual</option>
          <option value="auto">RLS auto</option>
          <option value="set">RLS set</option>
        </select>
        <select
          className="picker"
          value={tool.sheetId ?? ""}
          onChange={(e) => patchZoneRls(zoneId, chooseSheet(tool, e.target.value || null))}
          aria-label="Library sheet to copy"
        >
          <option value="">{sheets === null ? "Reading…" : sheets.length === 0 ? "No sheets in the library" : "Choose a sheet…"}</option>
          {sheets?.map((s) => (
            <option key={s.id} value={s.id}>
              {s.sheet_number} {s.sheet_name} ({s.viewports} viewport{s.viewports === 1 ? "" : "s"})
            </option>
          ))}
        </select>
        <span className="routeResult">{tool.roomIds.length === 0 ? "click rooms on the plan" : roomsText(tool.roomIds.length)}</span>
        <span className="rlsRooms">
          {tool.roomIds.map((id) => (
            <button key={id} className="chip" title="Take this room off" onClick={() => patchZoneRls(zoneId, { ...tool, roomIds: tool.roomIds.filter((r) => r !== id) })}>
              {name(id)} ×
            </button>
          ))}
        </span>
        <label title="Replace a sheet that already exists (off, it is skipped with a warning)">
          <input type="checkbox" checked={tool.overwrite} onChange={(e) => patchZoneRls(zoneId, { ...tool, overwrite: e.target.checked })} /> overwrite
        </label>
        <button className="chip on" disabled={busy || blocked !== null} title={blocked ?? "Queue a room layout sheet task per room, for the Revit session to make (the Tasks button)"} onClick={() => void make()}>
          {busy ? "Working…" : `Make ${tool.roomIds.length} task${tool.roomIds.length === 1 ? "" : "s"}`}
        </button>
        {notice ? <span className="meta">{notice}</span> : null}
        <a className="chip" href={`/tasks/?project=${encodeURIComponent(project ?? "")}`} title="See the tasks and what Revit did with them">
          Tasks page
        </a>
        <button className="chip" title="Leave the RLS tool (Esc)" onClick={() => setZoneRlsMode(zoneId, false)}>
          Close
        </button>
      </div>
      {record ? (
        <div className="rlsPreview">
          <SheetPreview record={record} />
        </div>
      ) : null}
    </>
  );
}

/** The zone's tool as it is now, for an async answer that arrives after the render that asked. */
function getZoneTool(zoneId: string) {
  return getState().zones.find((z) => z.id === zoneId)?.rls ?? null;
}
