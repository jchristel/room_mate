// The left pane: the project's blueprints, one selected at a time, and what can be done with the
// selected one. Import opens a panel in place of the list's footer. There is no "new from a room": a blueprint
// comes only from Revit; its sheets are kept in the sheet library, bound to it (plan decision 27).

import { useState } from "react";

import { attentionReasons, blueprintNeedsAttention, filterBlueprints } from "./common.js";
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
  const [onlyAttention, setOnlyAttention] = useState(false);
  const [text, setText] = useState("");
  const attention = rows?.filter(blueprintNeedsAttention).length ?? 0;
  const shown = rows ? filterBlueprints(onlyAttention ? rows.filter(blueprintNeedsAttention) : rows, text) : rows;
  return (
    <aside className="list" aria-label="Blueprints">
      <div className="actions">
        <button type="button" onClick={onImport}>
          Import
        </button>
      </div>
      <input type="search" placeholder="Filter: name or room" value={text} onChange={(e) => setText(e.target.value)} aria-label="Filter blueprints" />
      {attention > 0 ? (
        <label className="filter">
          <input type="checkbox" checked={onlyAttention} onChange={(e) => setOnlyAttention(e.target.checked)} /> Needs attention ({attention})
        </label>
      ) : null}
      {rows === null ? <p className="hint">Reading the blueprints…</p> : null}
      {rows && rows.length > 0 && shown?.length === 0 ? <p className="hint">No blueprint matches.</p> : null}
      {rows?.length === 0 ? <p className="hint">No blueprints yet. A blueprint is pushed from Revit, and its room layout sheets go to the sheet library bound to it; Import reads an export of one (the room alone).</p> : null}
      <ul>
        {shown?.map((b) => (
          <li key={b.id}>
            <button type="button" className={b.id === selectedId ? "on" : ""} aria-pressed={b.id === selectedId} onClick={() => onSelect(b.id)}>
              <b>
                {b.name}
                {blueprintNeedsAttention(b) ? (
                  <i className="attention" title={attentionReasons(b).join("; ")}>
                    needs attention
                  </i>
                ) : null}
              </b>
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
