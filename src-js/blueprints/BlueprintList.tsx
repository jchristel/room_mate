// The left pane: the project's blueprints, one selected at a time, and what can be done with the
// selected one. Import opens a panel in place of the list's footer. There is no "new from a room": a blueprint
// comes only from Revit; its sheets are kept in the sheet library, bound to it (plan decision 27).

import { useState } from "react";

import { Chip, Chips, ListPane, Row, RowList, SearchBox } from "../ui/ListPane.js";
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
  /** Whether the selected blueprint is in use; the button flips it. */
  inUse: boolean;
  onToggleUse(): void;
}

export function BlueprintList({ rows, selectedId, busy, onSelect, onImport, onExport, onRename, inUse, onToggleUse }: ListProps) {
  const [onlyAttention, setOnlyAttention] = useState(false);
  const [text, setText] = useState("");
  const attention = rows?.filter(blueprintNeedsAttention).length ?? 0;
  const shown = rows ? filterBlueprints(onlyAttention ? rows.filter(blueprintNeedsAttention) : rows, text) : rows;
  return (
    <ListPane
      label="Blueprints"
      actions={
        <button type="button" onClick={onImport}>
          Import
        </button>
      }
      search={<SearchBox value={text} onChange={setText} placeholder="Filter: name or room" label="Filter blueprints" />}
      filters={
        <Chips label="Filters">
          <Chip kind="attention" on={onlyAttention} onClick={() => setOnlyAttention(!onlyAttention)}>
            needs attention {attention}
          </Chip>
        </Chips>
      }
      footer={
        selectedId ? (
          <>
            <button type="button" disabled={busy} onClick={onExport}>
              Export
            </button>
            <button type="button" disabled={busy} onClick={onRename}>
              Rename
            </button>
            <button type="button" disabled={busy} onClick={onToggleUse} title="A blueprint is never deleted. Not in use: no new assignments, its sheets are not suggested; everything bound to it stays.">
              {inUse ? "Mark not in use" : "Mark in use"}
            </button>
          </>
        ) : null
      }
    >
      {rows === null ? <p className="hint">Reading the blueprints…</p> : null}
      {rows && rows.length > 0 && shown?.length === 0 ? <p className="hint">No blueprint matches.</p> : null}
      {rows?.length === 0 ? <p className="hint">No blueprints yet. A blueprint is pushed from Revit, and its room layout sheets go to the sheet library bound to it; Import reads an export of one (the room alone).</p> : null}
      <RowList>
        {shown?.map((b) => (
          <Row key={b.id} on={b.id === selectedId} onClick={() => onSelect(b.id)}>
            <b>
              {b.name}
              {blueprintNeedsAttention(b) ? (
                <i className="badge attention" title={attentionReasons(b).join("; ")}>
                  needs attention
                </i>
              ) : null}
            </b>
            <span>
              {b.in_use ? "" : "not in use · "}
              {b.items} items · {b.sheets > 0 ? `${b.sheets} sheet${b.sheets === 1 ? "" : "s"}` : "no sheets"} · r{b.current_revision}
              {b.pending > 0 ? ` · ${b.pending} pending` : ""}
            </span>
          </Row>
        ))}
      </RowList>
    </ListPane>
  );
}
