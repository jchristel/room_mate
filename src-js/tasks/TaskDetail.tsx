// One task, as a person checks what Revit was or will be told, and what the session said.

import { useEffect, useState } from "react";

import { KIND_TEXT, STATE_TEXT, UPDATE_MODE_TEXT, canDismiss, canResend, findingText, needsAttention, shortTime } from "./model.js";
import type { Task, ViewportMovesBody } from "./types.js";
import { SheetPreview } from "../library/SheetPreview.js";
import { movesBody, sheetStatus, taskSheetPreviews, type Centres } from "./sheetPreviews.js";

export interface DetailProps {
  task: Task;
  busy: boolean;
  onDismiss(): void;
  onResend(): void;
  /** Save where viewports were moved (`POST .../viewports`); the page shows the task the server answers with. */
  onMoveViewports(body: ViewportMovesBody): void;
}

const yes = (b: boolean): string => (b ? "yes" : "no");

export function TaskDetail({ task, busy, onDismiss, onResend, onMoveViewports }: DetailProps) {
  const t = task;
  const populate = t.kind === "populate_room";
  const update = t.kind === "update_room";
  // centres a person has dragged and not saved, by viewport key; gone when the task is read again
  const [draft, setDraft] = useState<Centres>(new Map());
  useEffect(() => setDraft(new Map()), [t.id, t.updated_at]);
  const previews = taskSheetPreviews(t, draft);
  const fit = sheetStatus(t, draft);
  // only a task still waiting in the queue: a session holds the others and a finished one is history
  const editable = t.state === "queued" && (t.kind === "create_rls" || t.kind === "create_rls_set");
  const body = movesBody(t, draft);
  const moved = (t.rls?.viewport_moves?.length ?? 0) > 0;
  return (
    <div className="detail">
      <div className="toolbar">
        <strong>{t.room.name}</strong>
        <span className={`badge ${t.state} ${needsAttention(t.state) ? "attention" : ""}`}>{STATE_TEXT[t.state]}</span>
        <span className="sub">{KIND_TEXT[t.kind]}</span>
        <span className="spacer" />
        <button type="button" disabled={busy || !canResend(t.state)} onClick={onResend}>
          Resend
        </button>
        <button type="button" className="danger" disabled={busy || !canDismiss(t.state)} onClick={onDismiss}>
          Dismiss
        </button>
      </div>

      <dl className="facts">
        <dt>Model</dt>
        <dd>{t.model_id}</dd>
        <dt>Room</dt>
        <dd>
          {t.room.name} · id {t.room.room_id} · {t.room.number ? `number ${t.room.number}` : "no number"}
          {t.rls_set && t.rls_set.rooms.length > 1 ? ` (lead of a set with ${t.rls_set.rooms.slice(1).map((r) => r.name).join(", ")})` : ""}
        </dd>
        {t.rls_set ? (
          <>
            <dt>Set</dt>
            <dd>
              {t.rls_set.sheets.length} {t.rls_set.sheet_size.toUpperCase()} sheet{t.rls_set.sheets.length === 1 ? "" : "s"},{" "}
              {t.rls_set.sheets.reduce((n, s) => n + s.views.length, 0)} views
            </dd>
          </>
        ) : null}
        <dt>Phase</dt>
        <dd>{t.phase}</dd>
        {populate || update ? (
          <>
            <dt>Blueprint</dt>
            <dd>{t.blueprint ? `${t.blueprint.name}, revision ${t.blueprint.revision}` : "—"}</dd>
            {update ? (
              <>
                <dt>Update</dt>
                <dd>{t.update_mode ? UPDATE_MODE_TEXT[t.update_mode] : "—"}</dd>
                <dt>Dry run</dt>
                <dd>{yes(t.dry_run)}</dd>
              </>
            ) : (
              <>
                <dt>Overwrite layout</dt>
                <dd>{yes(t.overwrite_layout)}</dd>
              </>
            )}
          </>
        ) : (
          <>
            <dt>Library sheet</dt>
            <dd>{t.library_sheet_id ?? "—"}</dd>
          </>
        )}
        {update || t.rls_set ? null : (
          <>
            <dt>Create RLS</dt>
            <dd>
              {yes(t.create_rls)}
              {t.create_rls ? ` · overwrite existing: ${yes(t.overwrite_rls)}` : ""}
            </dd>
          </>
        )}
        <dt>Made</dt>
        <dd>
          {shortTime(t.created_at)} · last change {shortTime(t.updated_at)}
        </dd>
      </dl>

      {previews.length > 0 ? (
        <>
          <h3>{previews.length === 1 ? "Sheet preview" : `Sheet previews (${previews.length})`}</h3>
          <p className="hint">
            {t.rls_set
              ? "The sheets as the set lays them out. Schedule and 3D sizes are estimates: Revit decides them."
              : "The template's viewports on the paper the task asks for, where the template puts them. The plan, the elevations and the 3D views are sized for this room; schedules keep the template's size, which Revit decides."}
          </p>
          {fit.state === "clipping" ? (
            <p className="banner error">
              ⚠ {fit.clipped} viewport{fit.clipped === 1 ? "" : "s"} past the sheet's edge (outlined in red). Revit will place {fit.clipped === 1 ? "it" : "them"} there.
            </p>
          ) : null}
          <div className="sheet-previews">
            {previews.map((p, i) => (
              <section key={i}>
                <div className="sub">{p.title}</div>
                <SheetPreview
                  record={p.record}
                  {...(editable && {
                    onMove: (port: number, centre: [number, number]) => {
                      const key = p.keys[port];
                      if (key) setDraft((cur) => new Map(cur).set(key, centre));
                    },
                  })}
                />
              </section>
            ))}
          </div>
          {editable ? (
            <div className="toolbar">
              <span className="hint">Drag a viewport to move it on the sheet; its size stays.</span>
              <span className="spacer" />
              <button type="button" disabled={busy || draft.size === 0} onClick={() => setDraft(new Map())}>
                Discard moves
              </button>
              {t.kind === "create_rls" && moved ? (
                <button type="button" disabled={busy} onClick={() => onMoveViewports({ moves: [] })}>
                  Back to the template's places
                </button>
              ) : null}
              <button type="button" disabled={busy || !body} onClick={() => body && onMoveViewports(body)}>
                Save positions
              </button>
            </div>
          ) : null}
        </>
      ) : null}

      <h3>Findings</h3>
      {t.findings.length === 0 ? <p className="hint">None.</p> : null}
      <ul className="findings">
        {t.findings.map((f, i) => (
          <li key={i} className={f.kind}>
            {findingText(f, t)}
          </li>
        ))}
      </ul>

      <h3>Attempts</h3>
      {t.attempts.length === 0 ? <p className="hint">No Revit session has taken this task yet.</p> : null}
      <ol className="attempts">
        {t.attempts.map((a, i) => (
          <li key={i}>
            <div>
              {shortTime(a.started_at)} · {a.status ? <i className={`badge ${a.status} ${needsAttention(a.status) ? "attention" : ""}`}>{STATE_TEXT[a.status]}</i> : "no report yet"}
              {a.finished_at ? ` · ${shortTime(a.finished_at)}` : ""}
            </div>
            {(a.messages ?? []).length > 0 ? (
              <ul>
                {a.messages?.map((m, j) => (
                  <li key={j}>{m}</li>
                ))}
              </ul>
            ) : null}
          </li>
        ))}
      </ol>

      {t.sheets.length > 0 ? (
        <>
          {t.rls_set ? (
            <>
              <h3>Template sheet</h3>
              <p className="hint">The set's views take their scales and the plan's framing from this library sheet; the sheets themselves are laid out as the set says above.</p>
            </>
          ) : (
            <>
              <h3>Sheets to make ({t.sheets.length})</h3>
              <p className="hint">The source sheet's number, name and id are not used for the new sheets: the Revit side generates those from the room. Tags are not in the layout: the rules below say where they go.</p>
            </>
          )}
          <table>
            <thead>
              <tr>
                <th>Source number</th>
                <th>Name</th>
                <th>Viewports</th>
              </tr>
            </thead>
            <tbody>
              {t.sheets.map((s) => (
                <tr key={s.revit_sheet_id + s.document_title}>
                  <td>{s.sheet_number}</td>
                  <td>{s.sheet_name ?? ""}</td>
                  <td>{s.sheet.view_ports?.length ?? 0}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      ) : null}

      {t.tag_rules.length > 0 ? (
        <>
          <h3>Tag rules ({t.tag_rules.length})</h3>
          <p className="hint">One rule per tagged type code, viewport type and view side, the median of the tags it was learned from. Which tag is placed is the model's RLS settings, so a rule names none. Offsets are in millimetres on the sheet, in the element's own frame (along its facing, then to its left).</p>
          <table>
            <thead>
              <tr>
                <th>Tagged</th>
                <th>Viewport</th>
                <th>Head (along, left)</th>
                <th>Leader</th>
                <th>Samples</th>
              </tr>
            </thead>
            <tbody>
              {t.tag_rules.map((r) => (
                <tr key={r.id}>
                  <td>{r.key.type_code || r.key.tagged_category}</td>
                  <td>{r.key.viewport_type}{r.key.view_side ? ` (${r.key.view_side})` : ""}</td>
                  <td>{r.placement.head.x.toFixed(1)}, {r.placement.head.y.toFixed(1)}</td>
                  <td>{r.placement.has_leader ? "yes" : "no"}</td>
                  <td>{r.sample_count}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      ) : null}

      {t.items.length > 0 ? (
        <>
          <h3>Items to place ({t.items.length})</h3>
          <p className="hint">Positions are in the target model's own frame, millimetres.</p>
          <table>
            <thead>
              <tr>
                <th>Family</th>
                <th>Type</th>
                <th>Placement</th>
                <th>x</th>
                <th>y</th>
                <th>Facing</th>
              </tr>
            </thead>
            <tbody>
              {t.items.map((i) => (
                <tr key={i.item_id}>
                  <td>{i.family_name ?? "—"}</td>
                  <td>{i.type_name}</td>
                  <td>{i.placement_type === "wall_device" ? "wall device" : "free"}</td>
                  <td>{Math.round(i.x_mm)}</td>
                  <td>{Math.round(i.y_mm)}</td>
                  <td>{Math.round((Math.atan2(i.facing_y, i.facing_x) * 180) / Math.PI)}°</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      ) : null}
    </div>
  );
}
