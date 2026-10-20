// The left pane: the project's blueprints, one selected at a time, and what can be done with the
// selected one. Import opens a panel in place of the list's footer. There is no "new from a room": a blueprint
// comes only from Revit, with its sheets (plan decision 18).

import type { BlueprintSummary } from "./types.js";

export interface ListProps {
  rows: readonly BlueprintSummary[] | null;
  selectedId: string | null;
  busy: boolean;
  onSelect(id: string): void;
  onImport(): void;
  onExport(): void;
  onRename(): void;
  onDelete(): void;
}

export function BlueprintList({ rows, selectedId, busy, onSelect, onImport, onExport, onRename, onDelete }: ListProps) {
  return (
    <aside className="list" aria-label="Blueprints">
      <div className="actions">
        <button type="button" onClick={onImport}>
          Import
        </button>
      </div>
      {rows === null ? <p className="hint">Reading the blueprints…</p> : null}
      {rows?.length === 0 ? <p className="hint">No blueprints yet. A blueprint is pushed from Revit with its room layout sheets; Import reads an export of one.</p> : null}
      <ul>
        {rows?.map((b) => (
          <li key={b.id}>
            <button type="button" className={b.id === selectedId ? "on" : ""} aria-pressed={b.id === selectedId} onClick={() => onSelect(b.id)}>
              <b>{b.name}</b>
              <span>
                {b.items} items · {b.sheets > 0 ? `${b.sheets} sheet${b.sheets === 1 ? "" : "s"}` : "no sheets"} · r{b.current_revision}
                {b.pending > 0 ? ` · ${b.pending} pending` : ""}
              </span>
            </button>
          </li>
        ))}
      </ul>
      {selectedId ? (
        <div className="actions">
          <button type="button" disabled={busy} onClick={onExport}>
            Export
          </button>
          <button type="button" disabled={busy} onClick={onRename}>
            Rename
          </button>
          <button type="button" className="danger" disabled={busy} onClick={onDelete}>
            Delete
          </button>
        </div>
      ) : null}
    </aside>
  );
}
