// The left pane: the project's tasks, newest first, narrowed by model and by state.

import { KIND_TEXT, STATES, STATE_TEXT, countByState, modelsOf, needsAttention, newestFirst, roomsLabel, shortTime } from "./model.js";
import { Chip, Chips, ListPane, Row, RowList, SearchBox } from "../ui/ListPane.js";
import { MultiToggle, type Selection } from "../ui/Selection.js";
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
  filter: string;
  onFilter(text: string): void;
  /** Several tasks at once: a click ticks a task instead of opening it. */
  selection: Selection;
  /** What the list shows (`shownTasks`): the page works it out because the multiple-choice panel needs the same rows. */
  shown: readonly TaskSummary[];
}

export function TaskList({ rows, selectedId, model, states, status, onlyWarnings, onOnlyWarnings, onModel, onToggleState, onSelect, onMake, filter, onFilter, selection, shown }: ListProps) {
  const all = rows ?? [];
  const counts = countByState(model ? all.filter((r) => r.model_id === model) : all);
  const warns = (id: string) => status.get(id)?.state === "clipping";
  const inModel = model ? all.filter((r) => r.model_id === model) : all;
  const warnings = inModel.filter((r) => warns(r.id)).length;
  const models = modelsOf(all);
  const on = (id: string) => (selection.multi ? selection.picked.has(id) : id === selectedId);
  return (
    <ListPane
      label="Tasks"
      actions={
        <>
          <button type="button" onClick={onMake}>
            Make tasks
          </button>
          <MultiToggle selection={selection} />
        </>
      }
      search={<SearchBox value={filter} onChange={onFilter} placeholder="Filter: room, number, kind, blueprint" label="Filter tasks" />}
      filters={
        <>
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
          <Chips label="States">
            {STATES.map((s) => (
              <Chip key={s} kind={s} on={states.has(s)} onClick={() => onToggleState(s)}>
                {STATE_TEXT[s]} {counts[s]}
              </Chip>
            ))}
            <Chip kind="warning" on={onlyWarnings} title="Tasks whose sheets have a viewport past the paper's edge" onClick={() => onOnlyWarnings(!onlyWarnings)}>
              ⚠ sheet warnings {warnings}
            </Chip>
          </Chips>
        </>
      }
    >
      {rows === null ? <p className="hint">Reading the tasks…</p> : null}
      {rows && shown.length === 0 ? <p className="hint">{rows.length === 0 ? "No tasks yet. Make some from assignments." : "No task matches."}</p> : null}
      <RowList>
        {shown.map((t) => (
          <Row key={t.id} on={on(t.id)} onClick={() => (selection.multi ? selection.toggle(t.id) : onSelect(t.id))}>
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
          </Row>
        ))}
      </RowList>
    </ListPane>
  );
}
