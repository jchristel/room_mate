// The left pane: the project's tasks, newest first, narrowed by model and by state.

import { KIND_TEXT, STATES, STATE_TEXT, countByState, modelsOf, needsAttention, newestFirst, roomsLabel, shortTime } from "./model.js";
import type { SheetStatus } from "./sheetPreviews.js";
import type { TaskState, TaskSummary } from "./types.js";

export interface ListProps {
  rows: readonly TaskSummary[] | null;
  selectedId: string | null;
  model: string;
  states: ReadonlySet<TaskState>;
  /** What each task's sheets look like, once read; a task not in it has not been looked at (or has no sheets). */
  status: ReadonlyMap<string, SheetStatus>;
  onlyWarnings: boolean;
  onOnlyWarnings(on: boolean): void;
  onModel(model: string): void;
  onToggleState(state: TaskState): void;
  onSelect(id: string): void;
  onMake(): void;
}

export function TaskList({ rows, selectedId, model, states, status, onlyWarnings, onOnlyWarnings, onModel, onToggleState, onSelect, onMake }: ListProps) {
  const all = rows ?? [];
  const counts = countByState(model ? all.filter((r) => r.model_id === model) : all);
  const warns = (id: string) => status.get(id)?.state === "clipping";
  const inModel = model ? all.filter((r) => r.model_id === model) : all;
  const warnings = inModel.filter((r) => warns(r.id)).length;
  const shown = newestFirst(all.filter((r) => (!model || r.model_id === model) && (states.size === 0 || states.has(r.state)) && (!onlyWarnings || warns(r.id))));
  const models = modelsOf(all);
  return (
    <aside className="left" aria-label="Tasks">
      <div className="actions">
        <button type="button" onClick={onMake}>
          Make tasks
        </button>
      </div>
      {models.length > 1 ? (
        <label className="stack">
          Model
          <select value={model} onChange={(e) => onModel(e.target.value)}>
            <option value="">All models</option>
            {models.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
        </label>
      ) : null}
      <div className="chips" role="group" aria-label="States">
        {STATES.map((s) => (
          <button key={s} type="button" className={`chip ${s} ${states.has(s) ? "on" : ""}`} aria-pressed={states.has(s)} onClick={() => onToggleState(s)}>
            {STATE_TEXT[s]} {counts[s]}
          </button>
        ))}
        <button
          type="button"
          className={`chip warning ${onlyWarnings ? "on" : ""}`}
          aria-pressed={onlyWarnings}
          title="Tasks whose sheets have a viewport past the paper's edge"
          onClick={() => onOnlyWarnings(!onlyWarnings)}
        >
          ⚠ sheet warnings {warnings}
        </button>
      </div>
      {rows === null ? <p className="hint">Reading the tasks…</p> : null}
      {rows && shown.length === 0 ? <p className="hint">{rows.length === 0 ? "No tasks yet. Make some from assignments." : "No task matches."}</p> : null}
      <ul>
        {shown.map((t) => (
          <li key={t.id}>
            <button type="button" className={t.id === selectedId ? "on" : ""} aria-pressed={t.id === selectedId} onClick={() => onSelect(t.id)}>
              <b>
                {warns(t.id) ? (
                  <span className="warn-icon" role="img" aria-label="A viewport is past the sheet's edge" title={`${status.get(t.id)?.clipped} viewport(s) past the sheet's edge`}>
                    ⚠{" "}
                  </span>
                ) : null}
                {roomsLabel(t)}
              </b>
              <span>
                <i className={`badge ${t.state} ${needsAttention(t.state) ? "attention" : ""}`}>{STATE_TEXT[t.state]}</i> {KIND_TEXT[t.kind]}
                {t.blueprint ? ` · ${t.blueprint.name} r${t.blueprint.revision}` : ""}
              </span>
              <span>{shortTime(t.updated_at)}</span>
            </button>
          </li>
        ))}
      </ul>
    </aside>
  );
}
