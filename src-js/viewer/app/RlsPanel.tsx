// The RLS tool's bar: which library sheet to copy, the rooms picked (click rooms on the plan), and the button that
// queues a `create_rls` task per room for the Revit session. Mounted inside a zone beside the blueprint bar and
// renders nothing while the tool is off.
//
// Nothing is asked about a phase (the views take their room's) or about where things go (the server works that
// out from the walls). The preview under the bar is the sheet's layout, so a person knows what is being copied.

import { useEffect, useState } from "react";

import { SheetPreview } from "../../library/SheetPreview.js";
import { chooseSheet, madeText, roomsText, whyNot } from "../rlsTool.js";
import { getLibrarySheet, listLibrarySheets, makeRlsTasks, type LibrarySheet, type LibrarySheetRecord } from "./rlsData.js";
import { patchZoneRls, setZoneRlsMode, type ZoneRow } from "./store.js";
import { useViewer } from "./useViewer.js";

export function RlsPanel({ zone }: { zone: ZoneRow }) {
  const { scope, payload, toolFocus } = useViewer();
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

  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && toolFocus === zoneId) setZoneRlsMode(zoneId, false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active, toolFocus, zoneId]);

  if (!tool) return null;
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

  return (
    <>
      <div className="routeBar rlsBar">
        <strong>RLS</strong>
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
        <button className="chip on" disabled={busy || blocked !== null} title={blocked ?? "Queue a room layout sheet task per room, for the Revit session to make (RLS create)"} onClick={() => void make()}>
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
