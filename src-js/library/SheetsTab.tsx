// The sheet library: the room layout sheets a person exported from Revit, kept by sheet identity.
// Left: the sheets. Right: the selected sheet (its room, its viewports, pushes waiting for a
// decision), or the suggestion panel that ranks sheets against a room.
//
// **The sheet body is duHast's export and opaque to the server**, so this page shows only what a
// person needs to choose a sheet (number, name, room, viewports); it does not draw the sheet.

import { useCallback, useEffect, useMemo, useState } from "react";

import * as api from "./api.js";
import { boundText, sheetNeedsAttention, sheetReasons, shortTime, sizeText, suggestionText, summaryText } from "./model.js";
import { SheetPreview } from "./SheetPreview.js";
import type { RoomRow, SheetRecord, SheetSummary, Suggestions } from "./types.js";

const message = (e: unknown): string => String((e as Error).message ?? e);
const POLL_MS = 8000;

export interface TabProps {
  project: string;
  rooms: readonly RoomRow[];
  onError(text: string | null): void;
  onNotice(text: string | null): void;
}

export function SheetsTab({ project, rooms, onError, onNotice }: TabProps) {
  const [rows, setRows] = useState<SheetSummary[] | null>(null);
  const [record, setRecord] = useState<SheetRecord | null>(null);
  const [onlyAttention, setOnlyAttention] = useState(false);
  const [suggesting, setSuggesting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [names, setNames] = useState<ReadonlyMap<string, string>>(new Map());

  const refresh = useCallback(() => api.listSheets(project).then(setRows).catch((e: unknown) => onError(message(e))), [project, onError]);

  // A new project starts clean; the list is read again so a push from Revit shows up.
  useEffect(() => {
    setRows(null);
    setRecord(null);
    setSuggesting(false);
    void refresh();
    api.listBlueprintNames(project).then(setNames).catch(() => setNames(new Map()));
    const timer = window.setInterval(() => void refresh(), POLL_MS);
    return () => window.clearInterval(timer);
  }, [project, refresh]);

  const open = async (id: string) => {
    onError(null);
    onNotice(null);
    setSuggesting(false);
    try {
      setRecord(await api.getSheet(project, id));
    } catch (e) {
      onError(message(e));
    }
  };

  // Run a write against the open sheet; show its text on failure and keep the page in step on success.
  const act = async (what: string, run: (r: SheetRecord) => Promise<api.Sent<SheetRecord | null>>) => {
    if (!record) return;
    setBusy(true);
    const r = await run(record);
    setBusy(false);
    if (!r.ok) return onError(r.error);
    onError(null);
    onNotice(what);
    if (r.value) setRecord(await api.getSheet(project, r.value.id));
    else setRecord(null);
    void refresh();
  };

  const shown = useMemo(() => (onlyAttention ? rows?.filter(sheetNeedsAttention) : rows) ?? null, [rows, onlyAttention]);
  const attention = rows?.filter(sheetNeedsAttention).length ?? 0;

  return (
    <div className="page">
      <aside className="left" aria-label="Sheets">
        <div className="actions">
          <button
            type="button"
            onClick={() => {
              setSuggesting(true);
              setRecord(null);
            }}
          >
            Suggest for a room
          </button>
        </div>
        {attention > 0 ? (
          <label className="filter">
            <input type="checkbox" checked={onlyAttention} onChange={(e) => setOnlyAttention(e.target.checked)} /> Needs attention ({attention})
          </label>
        ) : null}
        {rows === null ? <p className="hint">Reading the sheets…</p> : null}
        {rows?.length === 0 ? <p className="hint">The library is empty. Export sheets from Revit with the Sheets button.</p> : null}
        <ul>
          {shown?.map((s) => (
            <li key={s.id}>
              <button type="button" className={s.id === record?.id ? "on" : ""} aria-pressed={s.id === record?.id} onClick={() => void open(s.id)}>
                <b>
                  {s.sheet_number} {s.sheet_name}
                  {sheetNeedsAttention(s) ? (
                    <i className="badge attention" title={sheetReasons(s).join("; ")}>
                      needs attention
                    </i>
                  ) : null}
                </b>
                <span>
                  {s.document_title} · {sizeText(s.size_mm)} · {s.viewports} viewport{s.viewports === 1 ? "" : "s"} · {boundText(s.blueprint, names)}
                </span>
              </button>
            </li>
          ))}
        </ul>
      </aside>
      <main>
        {suggesting ? (
          <Suggest project={project} rooms={rooms} onError={onError} />
        ) : record ? (
          <SheetDetail
            record={record}
            names={names}
            busy={busy}
            onDecide={(pendingId, accept) => void act(accept ? "Push accepted." : "Push rejected.", (r) => api.decidePush(project, r.id, pendingId, r.taken_at, accept))}
            onDelete={() => {
              if (window.confirm(`Delete sheet ${record.sheet_number} from the library? Tasks already made keep their copy.`))
                void act("Sheet deleted.", async (r) => {
                  const d = await api.deleteSheet(project, r.id, r.taken_at);
                  return d.ok ? { ok: true as const, value: null } : d;
                });
            }}
          />
        ) : (
          <p className="hint empty">Choose a sheet on the left.</p>
        )}
      </main>
    </div>
  );
}

interface DetailProps {
  record: SheetRecord;
  names: ReadonlyMap<string, string>;
  busy: boolean;
  onDecide(pendingId: number, accept: boolean): void;
  onDelete(): void;
}

function SheetDetail({ record, names, busy, onDecide, onDelete }: DetailProps) {
  const ports = record.sheet.view_ports ?? [];
  return (
    <div className="detail">
      <div className="toolbar">
        <strong>
          {record.sheet_number} {record.sheet_name}
        </strong>
        <span className="spacer" />
        <button type="button" className="danger" disabled={busy} onClick={onDelete}>
          Delete
        </button>
      </div>
      <dl className="facts">
        <dt>Document</dt>
        <dd>
          {record.document_title} · Revit sheet {record.revit_sheet_id}
        </dd>
        <dt>Paper size</dt>
        <dd>{sizeText(record.size_mm)}</dd>
        <dt>Room blueprint</dt>
        <dd>{boundText(record.blueprint, names)}</dd>
        {record.room_key ? (
          <>
            <dt>Room key</dt>
            <dd>{record.room_key}</dd>
          </>
        ) : null}
        <dt>Phase</dt>
        <dd>{record.phase ?? "—"}</dd>
        <dt>Rooms covered</dt>
        <dd>{record.rooms && record.rooms.length > 0 ? record.rooms.join(", ") : "not said"}</dd>
        <dt>Room figures</dt>
        <dd>{summaryText(record.room_summary)}</dd>
        <dt>Last pushed</dt>
        <dd>{shortTime(record.received_at)}</dd>
      </dl>

      <h3>Preview</h3>
      <SheetPreview record={record} />

      <h3>Viewports ({ports.length})</h3>
      {ports.length === 0 ? <p className="hint">None. A sheet with no viewport has no layout to copy.</p> : null}
      {ports.length > 0 ? (
        <table>
          <thead>
            <tr>
              <th>Type</th>
              <th>Scale</th>
              <th>View</th>
            </tr>
          </thead>
          <tbody>
            {ports.map((p, i) => (
              <tr key={i}>
                <td>{p.vp_type ?? "?"}</td>
                <td>{p.view?.view_scale ? `1:${p.view.view_scale}` : "—"}</td>
                <td>{p.view?.name ?? "—"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}

      <h3>Pushes waiting ({record.pending.length})</h3>
      {record.pending.length === 0 ? <p className="hint">None.</p> : null}
      <ul className="findings">
        {record.pending.map((p) => (
          <li key={p.pending_id} className="pending">
            Sheet number changed to <b>{p.sheet_number}</b> {p.sheet_name} · {shortTime(p.received_at)}
            <span className="actions inline">
              <button type="button" disabled={busy} onClick={() => onDecide(p.pending_id, true)}>
                Accept
              </button>
              <button type="button" disabled={busy} onClick={() => onDecide(p.pending_id, false)}>
                Reject
              </button>
            </span>
          </li>
        ))}
      </ul>
      {record.rejected.length > 0 ? (
        <>
          <h3>Rejected ({record.rejected.length})</h3>
          <ul className="findings">
            {record.rejected.map((p) => (
              <li key={`${p.pending_id}-${p.rejected_at}`}>
                {p.sheet_number} {p.sheet_name} · rejected {shortTime(p.rejected_at)}
              </li>
            ))}
          </ul>
        </>
      ) : null}
    </div>
  );
}

function Suggest({ project, rooms, onError }: { project: string; rooms: readonly RoomRow[]; onError(text: string | null): void }) {
  const [roomId, setRoomId] = useState("");
  const [filter, setFilter] = useState("");
  const [answer, setAnswer] = useState<Suggestions | null>(null);
  const [busy, setBusy] = useState(false);
  const f = filter.trim().toLowerCase();
  const options = rooms.filter((r) => !f || r.name.toLowerCase().includes(f)).slice(0, 200);

  const ask = async () => {
    setBusy(true);
    onError(null);
    try {
      setAnswer(await api.suggest(project, roomId));
    } catch (e) {
      setAnswer(null);
      onError(message(e));
    }
    setBusy(false);
  };

  return (
    <div className="detail">
      <div className="toolbar">
        <strong>Suggest sheets for a room</strong>
      </div>
      <p className="hint">
        Ranks the library by how close the room figures of each sheet are to this one: area, proportions and item count, weighed equally. It offers; nothing is applied.
      </p>
      <input type="search" placeholder="Filter rooms" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Filter rooms" />
      <div className="actions">
        <select value={roomId} onChange={(e) => setRoomId(e.target.value)} aria-label="Room">
          <option value="">Choose a room…</option>
          {options.map((r) => (
            <option key={r.id} value={r.id}>
              {r.name}
            </option>
          ))}
        </select>
        <button type="button" disabled={busy || !roomId} onClick={() => void ask()}>
          {busy ? "Working…" : "Suggest"}
        </button>
      </div>
      {answer ? (
        <>
          <p>
            For <b>{answer.target.room_name}</b>: {answer.target.signature.area_m2.toFixed(1)} m2, {answer.target.signature.items} items, proportion {answer.target.signature.proportion.toFixed(2)}.
          </p>
          {answer.suggestions.length === 0 ? <p className="hint">No sheet in the library has room figures to compare.</p> : null}
          <table>
            <thead>
              <tr>
                <th>Sheet</th>
                <th>Made for</th>
                <th>Distance</th>
                <th>Why</th>
              </tr>
            </thead>
            <tbody>
              {answer.suggestions.map((s) => (
                <tr key={s.sheet_id}>
                  <td>
                    {s.sheet_number} {s.sheet_name}
                  </td>
                  <td>{s.room_name}</td>
                  <td>{s.distance.toFixed(2)}</td>
                  <td>{suggestionText(s)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {answer.unassigned > 0 ? <p className="hint">{answer.unassigned} sheet(s) have no room figures and cannot be ranked.</p> : null}
          {answer.unresolved.map((u) => (
            <p key={u.sheet_id} className="hint">
              {u.sheet_number}: {u.reason}
            </p>
          ))}
        </>
      ) : null}
    </div>
  );
}
