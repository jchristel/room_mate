// The approve flow: what moving a blueprint's assignments to another revision would change, room by room, and the way to
// let it through (plan section 7, build step 6).
//
// **The list is the server's dry run** (`GET .../impact`, `service::blueprint_impact`): this page ticks and words, and has
// no rule of its own about what a room needs. Approving does three things in order, and stops where one fails: accept the
// revision when it is still pending, re-pin each ticked room at it (keeping the room's own mapping, nudges and additions),
// and queue an `update_room` task for each. **A room whose nudge is held is not tickable**: it needs a person in the
// assign view first, because carrying a nudge over a recipe that moved is exactly the silent mistake the hold exists for.
// Nothing here writes to Revit; the tasks wait for a session, and the plug-in does not know the `update_room` kind yet.

import { useEffect, useMemo, useState } from "react";

import * as api from "./api.js";
import { canTick, isOverridden, planText, STATUS_TEXT, tickable } from "./approve.js";
import type { AssignmentRow, Blueprint, BlueprintImpact, RoomImpact, UpdateMode } from "./types.js";

export interface ImpactProps {
  project: string;
  blueprint: Blueprint;
  /** An accepted revision, or a pushed one still waiting for review. */
  target: { revision?: number; pending?: number };
  /** Accept the pending revision (the page reloads the blueprint); resolves whether it was accepted. */
  onAccept(pendingId: number): Promise<boolean>;
  onDone(message: string): void;
  onClose(): void;
}

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

const MODE_TEXT: Record<UpdateMode, string> = {
  locations: "locations only (move)",
  items: "items (add, move)",
};

export function Impact({ project, blueprint, target, onAccept, onDone, onClose }: ImpactProps) {
  const [impact, setImpact] = useState<BlueprintImpact | null>(null);
  const [assignments, setAssignments] = useState<AssignmentRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [picked, setPicked] = useState<ReadonlySet<string>>(new Set());
  const [phase, setPhase] = useState("");
  const [mode, setMode] = useState<UpdateMode>("locations");
  const [dryRun, setDryRun] = useState(true);
  const [busy, setBusy] = useState(false);
  const [lines, setLines] = useState<string[]>([]);

  const key = `${target.revision ?? ""}/${target.pending ?? ""}`;
  useEffect(() => {
    let live = true;
    setImpact(null);
    setError(null);
    setPicked(new Set());
    // The rows carry each room's own mapping, nudges and additions, which approving sends again unchanged.
    Promise.all([api.impact(project, blueprint.id, target), api.listAssignments(project)])
      .then(([r, rows]) => {
        if (!live) return;
        setImpact(r);
        setAssignments(rows.filter((a) => a.blueprint_id === blueprint.id));
      })
      .catch((e: unknown) => live && setError(message(e)));
    return () => {
      live = false;
    };
    // `target` is rebuilt by the caller on every render; `key` is what it says.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project, blueprint.id, key]);

  const byId = useMemo(() => new Map(assignments.map((a) => [a.id, a] as const)), [assignments]);
  const toggle = (id: string) =>
    setPicked((p) => {
      const n = new Set(p);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  const rooms: RoomImpact[] = impact?.rooms ?? [];
  const ready = picked.size > 0 && phase.trim() !== "" && !busy;
  const pending = target.pending !== undefined;

  const approve = async () => {
    if (!impact) return;
    setBusy(true);
    setError(null);
    const out: string[] = [];
    if (target.pending !== undefined) {
      if (!(await onAccept(target.pending))) {
        setBusy(false);
        return setError("The revision could not be accepted, so nothing was changed.");
      }
      out.push(`Accepted as r${impact.target.revision}.`);
    }
    const queued: string[] = [];
    for (const r of rooms.filter((x) => picked.has(x.assignment_id))) {
      const a = byId.get(r.assignment_id);
      if (!a) continue;
      const sent = await api.assign(project, blueprint.id, impact.target.revision, a.room.room_id, a.mapping, a.note ?? undefined, a.overrides ?? [], a.additions ?? [], a.room.model_id);
      if (!sent.ok) out.push(`${r.room_name}: not moved to r${impact.target.revision}: ${sent.error}`);
      else queued.push(r.assignment_id);
    }
    if (queued.length > 0) {
      const made = await api.queueUpdates(project, queued, phase.trim(), mode, dryRun);
      if (!made.ok) out.push(`The update tasks were not queued: ${made.error}`);
      else
        for (const m of made.value) out.push(m.refused ? `${m.room_name}: ${m.refused}` : `${m.room_name}: ${dryRun ? "dry-run " : ""}update task queued.`);
    }
    setLines(out);
    setPicked(new Set());
    setBusy(false);
    onDone(`Moved ${queued.length} room${queued.length === 1 ? "" : "s"} to r${impact.target.revision}.`);
  };

  return (
    <main className="impact">
      <div className="toolbar">
        <strong>{blueprint.name}</strong>
        <span className="sub">
          {pending ? "a pending revision" : `r${target.revision}`} in the rooms it is assigned to
          {impact ? ` · ${rooms.length} room${rooms.length === 1 ? "" : "s"}` : ""}
        </span>
        <button type="button" style={{ marginLeft: "auto" }} onClick={onClose}>
          Back to the blueprint
        </button>
      </div>
      {error ? <p className="banner error">{error}</p> : null}
      {!impact && !error ? <p className="hint">Reading the rooms…</p> : null}
      {impact?.outline.changed ? (
        <p className="banner attention">
          The blueprint's own outline changed ({impact.outline.old_edges} to {impact.outline.new_edges} edges), so every room's fit is worked out again.
        </p>
      ) : null}
      {impact ? (
        <>
          <p className="hint">A dry run: nothing is changed until you approve. Nothing is removed, retyped or replaced here; those need a record of what Revit placed, which does not exist yet.</p>
          <div className="actions">
            <button type="button" onClick={() => setPicked(tickable(rooms, byId, false))}>
              Tick all with changes
            </button>
            <button type="button" onClick={() => setPicked(tickable(rooms, byId, true))} title="Leaves out rooms that carry their own nudges, leave-outs or additions">
              Tick all not overridden
            </button>
            <button type="button" onClick={() => setPicked(new Set())}>
              Tick none
            </button>
          </div>
          <ul className="impact-rooms">
            {rooms.map((r) => {
              const own = isOverridden(byId.get(r.assignment_id));
              return (
                <li key={r.assignment_id} className={`room ${r.status}`}>
                  <label>
                    <input type="checkbox" disabled={!canTick(r.status)} checked={picked.has(r.assignment_id)} onChange={() => toggle(r.assignment_id)} />
                    <b>{r.room_name}</b> <span className="sub">from r{r.pinned_revision}</span>
                    {own ? <span className="badge" title="This room has nudges, leave-outs or additions of its own"> its own changes</span> : null}
                  </label>
                  <div className="sub">
                    {STATUS_TEXT[r.status]}
                    {r.status === "changes" || r.status === "no_change" ? ` · ${planText(r.plan_counts)}` : ""}
                    {r.reason ? ` · ${r.reason}` : ""}
                  </div>
                  {r.held.length > 0 ? <p className="note">Held nudges: {r.held.join(", ")}. Open the room in the assign view and keep or drop each.</p> : null}
                  {r.item_changes.length > 0 || r.plan.length > 0 ? (
                    <details>
                      <summary>Why, and what</summary>
                      {r.item_changes.length > 0 ? (
                        <ul className="findings">
                          {r.item_changes.map((c) => (
                            <li key={c.item_id}>
                              {c.type_name}: {c.changes.map((k) => k.replace("_", " ")).join(", ")}
                              {c.moved_mm !== undefined ? ` (${Math.round(c.moved_mm)} mm)` : ""}
                              {c.matched === "by_position" ? " (paired by place)" : ""}
                            </li>
                          ))}
                        </ul>
                      ) : null}
                      {r.plan.length > 0 ? (
                        <ul className="findings">
                          {r.plan.map((p, i) => (
                            <li key={i}>
                              {p.action}: {p.verdict.type_name} ({p.verdict.kind.replace("_", " ")}
                              {p.verdict.distance_mm !== undefined ? `, ${Math.round(p.verdict.distance_mm)} mm` : ""})
                            </li>
                          ))}
                        </ul>
                      ) : null}
                    </details>
                  ) : null}
                </li>
              );
            })}
          </ul>
          {rooms.length === 0 ? <p className="hint">This blueprint is not assigned to any room.</p> : null}

          <h3>Approve</h3>
          <label className="stack">
            Phase to work in (the name Revit knows it by)
            <input type="text" value={phase} onChange={(e) => setPhase(e.target.value)} placeholder="New Construction" />
          </label>
          <label className="stack">
            What to update
            <select value={mode} onChange={(e) => setMode(e.target.value as UpdateMode)}>
              {(Object.keys(MODE_TEXT) as UpdateMode[]).map((m) => (
                <option key={m} value={m}>
                  {MODE_TEXT[m]}
                </option>
              ))}
            </select>
          </label>
          <label>
            <input type="checkbox" checked={dryRun} onChange={(e) => setDryRun(e.target.checked)} /> Dry run: the task reports what it would change and writes nothing
          </label>
          <div className="actions">
            <button type="button" disabled={!ready} onClick={() => void approve()}>
              {pending ? "Accept the revision and update" : "Update"} {picked.size} room{picked.size === 1 ? "" : "s"}
            </button>
          </div>
          {lines.length > 0 ? (
            <ul className="findings">
              {lines.map((l, i) => (
                <li key={i}>{l}</li>
              ))}
            </ul>
          ) : null}
        </>
      ) : null}
    </main>
  );
}
