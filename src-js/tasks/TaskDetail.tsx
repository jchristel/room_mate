// One task, as a person checks what Revit was or will be told, and what the session said.

import { KIND_TEXT, STATE_TEXT, UPDATE_MODE_TEXT, canDismiss, canResend, findingText, needsAttention, shortTime } from "./model.js";
import type { Task } from "./types.js";

export interface DetailProps {
  task: Task;
  busy: boolean;
  onDismiss(): void;
  onResend(): void;
}

const yes = (b: boolean): string => (b ? "yes" : "no");

export function TaskDetail({ task, busy, onDismiss, onResend }: DetailProps) {
  const t = task;
  const populate = t.kind === "populate_room";
  const update = t.kind === "update_room";
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
        </dd>
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
        {update ? null : (
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
          <h3>Sheets to make ({t.sheets.length})</h3>
          <p className="hint">The source sheet's number, name and id are not used for the new sheets: the Revit side generates those from the room. Tags are not in the layout: the rules below say where they go.</p>
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
          <p className="hint">One rule per tagged family and type, tag type and viewport type, the median of the tags it was learned from. Offsets are in millimetres on the sheet, in the element's own frame (along its facing, then to its left).</p>
          <table>
            <thead>
              <tr>
                <th>Tagged</th>
                <th>Tag</th>
                <th>Viewport</th>
                <th>Head (along, left)</th>
                <th>Leader</th>
                <th>Samples</th>
              </tr>
            </thead>
            <tbody>
              {t.tag_rules.map((r) => (
                <tr key={r.id}>
                  <td>{r.key.tagged_family} : {r.key.tagged_type}</td>
                  <td>{r.key.tag_family} : {r.key.tag_type}</td>
                  <td>{r.key.viewport_type}</td>
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
