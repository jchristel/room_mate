// The revision timeline across the top.
//
// Revisions only ever accumulate; "current" is a marker on one of them and moving it is the one
// way to change which a new assignment uses. A revision that has ARRIVED (an import, later a
// Revit push) sits on the line as pending until a person accepts it or rejects it: it is not part
// of the history, and nothing pins it.

import type { Blueprint, PendingRevision } from "./types.js";

export interface TimelineProps {
  blueprint: Blueprint;
  viewing: number;
  busy: boolean;
  onView(number: number): void;
  onSetCurrent(number: number): void;
  onViewPending(p: PendingRevision | null): void;
  viewingPending: number | null;
  onDecide(pendingId: number, accept: boolean): void;
}

const when = (stamp: string): string => stamp.replace("T", " ").slice(0, 16);

export function Timeline({ blueprint, viewing, busy, onView, onSetCurrent, onViewPending, viewingPending, onDecide }: TimelineProps) {
  const pending = blueprint.pending ?? [];
  const rejected = blueprint.rejected ?? [];
  return (
    <nav className="timeline" aria-label="Revisions">
      <ol>
        {blueprint.revisions.map((r) => {
          const current = r.number === blueprint.current_revision;
          const on = viewingPending === null && r.number === viewing;
          return (
            <li key={r.number} className={`rev${on ? " on" : ""}${current ? " current" : ""}`}>
              <button type="button" onClick={() => onView(r.number)} title={r.note ?? undefined}>
                <b>r{r.number}</b>
                <span>{when(r.created_at)}</span>
                {r.note ? <em>{r.note}</em> : null}
                {current ? <span className="badge">current</span> : null}
              </button>
              {on && !current ? (
                <button type="button" className="action" disabled={busy} onClick={() => onSetCurrent(r.number)}>
                  Make current
                </button>
              ) : null}
            </li>
          );
        })}
        {pending.map((p) => (
          <li key={`p${p.pending_id}`} className={`rev pending${viewingPending === p.pending_id ? " on" : ""}`}>
            <button type="button" onClick={() => onViewPending(p)} title="Arrived and not yet reviewed. Not part of the history.">
              <b>pending</b>
              <span>{when(p.received_at)}</span>
              <em>{p.origin ?? "pushed"}</em>
            </button>
            {viewingPending === p.pending_id ? (
              <>
                <button
                  type="button"
                  className="action"
                  disabled={busy}
                  title={p.bundle ? "Its sheets and tags are applied too: sheets and tags no longer in it become generic." : undefined}
                  onClick={() => onDecide(p.pending_id, true)}
                >
                  {p.bundle ? "Accept, with its sheets and tags" : "Accept as new current"}
                </button>
                <button type="button" className="action danger" disabled={busy} onClick={() => onDecide(p.pending_id, false)}>
                  Reject
                </button>
              </>
            ) : null}
          </li>
        ))}
      </ol>
      {rejected.length > 0 ? <span className="rejected">{rejected.length} rejected, kept</span> : null}
    </nav>
  );
}
