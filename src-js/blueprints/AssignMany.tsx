// Assign one blueprint revision to several rooms in one go.
//
// **The solver decides, the person ticks.** "Check" asks the server to place the blueprint in every
// ticked room at the best fit (the same resolver as one room, rooms and doors read once) and lists
// a verdict for each. Rooms that fit with nothing to look at start ticked; the rest start unticked
// but can be ticked, because a verdict is advice and not a gate. Clicking a room's name previews
// it on the canvas through the single-room read, so there is one drawing rule and not two.
//
// Best fit only. A mapping by hand is a decision about one room and stays in the single-room view,
// which is where a room the solver cannot place is sent.

import { useMemo, useState } from "react";

import * as api from "./api.js";
import type { AssignmentRow, Blueprint, RoomRow, RoomSolve, Verdict } from "./types.js";

export interface AssignManyProps {
  project: string;
  blueprint: Blueprint;
  revision: number;
  rooms: RoomRow[] | null;
  levels: Map<string, string>;
  assigned: AssignmentRow[];
  /** Show this room on the canvas. */
  onPreview(room: RoomRow): void;
  /** Open this room in the single-room view, to map it by hand. */
  onOpen(room: RoomRow): void;
  /** Assignments changed: read them again. */
  onAssigned(message: string): void;
}

const message = (e: unknown): string => String((e as Error).message ?? e);
const MARK: Record<Verdict, string> = { clear: "✓", check: "!", cannot: "✗" };

function detail(s: RoomSolve): string {
  const bits = [s.reason];
  if (s.placed > 0 || s.items > 0) bits.push(`${s.placed}/${s.items} placed`);
  if (s.outside > 0) bits.push(`${s.outside} outside`);
  if (s.overlap > 0) bits.push(`${s.overlap} overlap`);
  if (s.door_gap_mm !== null && s.door_gap_mm !== undefined) bits.push(`doors ${Math.round(s.door_gap_mm)} mm apart`);
  return bits.join(" · ");
}

export function AssignMany({ project, blueprint, revision, rooms, levels, assigned, onPreview, onOpen, onAssigned }: AssignManyProps) {
  const [filter, setFilter] = useState("");
  const [ticked, setTicked] = useState<Set<string>>(new Set());
  const [solved, setSolved] = useState<Map<string, RoomSolve>>(new Map());
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const shown = useMemo(() => {
    const f = filter.trim().toLowerCase();
    return (rooms ?? []).filter((r) => !f || r.name.toLowerCase().includes(f)).slice(0, 200);
  }, [rooms, filter]);
  const assignedTo = useMemo(() => new Map(assigned.map((a) => [a.room.room_id, a])), [assigned]);

  const toggle = (id: string) =>
    setTicked((t) => {
      const n = new Set(t);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  const tickShown = () => setTicked((t) => new Set([...t, ...shown.map((r) => r.id)]));
  const clear = () => {
    setTicked(new Set());
    setSolved(new Map());
  };

  const check = async () => {
    setBusy(true);
    setError(null);
    const ids = [...ticked];
    const r = await api.solve(project, blueprint.id, revision, ids);
    setBusy(false);
    if (!r.ok) return setError(r.error);
    setSolved(new Map(r.value.map((s) => [s.room_id, s])));
    // Start from what the solver is content with; the person adjusts.
    setTicked(new Set(r.value.filter((s) => s.verdict === "clear").map((s) => s.room_id)));
  };

  const assignTicked = async () => {
    const ids = [...ticked];
    const replacing = ids.filter((id) => assignedTo.has(id)).length;
    const ask = `Assign "${blueprint.name}" r${revision} to ${ids.length} room${ids.length === 1 ? "" : "s"}?` + (replacing ? ` ${replacing} already ha${replacing === 1 ? "s" : "ve"} an assignment, which this replaces.` : "");
    if (!window.confirm(ask)) return;
    setBusy(true);
    setError(null);
    const r = await api.assignMany(project, blueprint.id, revision, ids, note.trim() || undefined);
    setBusy(false);
    if (!r.ok) return setError(r.error);
    const failed = r.value.filter((o) => !o.assigned);
    const done = r.value.length - failed.length;
    if (failed.length) setError(failed.map((f) => `${rooms?.find((x) => x.id === f.room_id)?.name ?? f.room_id}: ${f.error ?? "failed"}`).join("; "));
    setTicked(new Set(failed.map((f) => f.room_id)));
    setNote("");
    onAssigned(`Assigned r${revision} to ${done} room${done === 1 ? "" : "s"}${failed.length ? `, ${failed.length} failed` : ""}.`);
  };

  const counts = { clear: 0, check: 0, cannot: 0 };
  solved.forEach((s) => (counts[s.verdict] += 1));

  return (
    <>
      <h3>Rooms</h3>
      {!rooms ? <p className="hint">Reading the rooms…</p> : null}
      <input type="search" placeholder="Filter rooms by name" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Filter rooms" />
      <div className="actions">
        <button type="button" onClick={tickShown} disabled={shown.length === 0}>
          Tick the {shown.length} shown
        </button>
        <button type="button" onClick={clear} disabled={ticked.size === 0 && solved.size === 0}>
          Clear
        </button>
      </div>
      <ul className="picklist many">
        {shown.map((r) => {
          const s = solved.get(r.id);
          const a = assignedTo.get(r.id);
          return (
            <li key={r.id} className={s ? s.verdict : ""}>
              <label>
                <input type="checkbox" checked={ticked.has(r.id)} onChange={() => toggle(r.id)} />
              </label>
              <button type="button" onClick={() => (s?.verdict === "cannot" ? onOpen(r) : onPreview(r))} title={s ? detail(s) : undefined}>
                {s ? `${MARK[s.verdict]} ` : ""}
                {a ? "● " : ""}
                {r.name}
                <span>{levels.get(r.level_id) ?? ""}</span>
              </button>
            </li>
          );
        })}
      </ul>
      {rooms && rooms.length > shown.length ? <p className="hint">Showing {shown.length}; type to narrow. ● has an assignment.</p> : null}

      <div className="actions">
        <button type="button" disabled={busy || ticked.size === 0} onClick={() => void check()}>
          {busy ? "Working…" : `Check ${ticked.size} room${ticked.size === 1 ? "" : "s"}`}
        </button>
      </div>
      {error ? <p className="banner error">{error}</p> : null}

      {solved.size > 0 ? (
        <>
          <h3>What the solver found</h3>
          <p className="sub">
            ✓ {counts.clear} fit · ! {counts.check} to look at · ✗ {counts.cannot} cannot be placed. Fitting rooms are ticked; click a name to see it, and a ✗ opens in the single-room view to map by hand.
          </p>
          <ul className="findings">
            {[...solved.values()].map((s) => (
              <li key={s.room_id} className={s.verdict}>
                <span>
                  {MARK[s.verdict]} <b>{s.room_name}</b>: {detail(s)}
                </span>
              </li>
            ))}
          </ul>
          <h3>Assign</h3>
          <label className="stack">
            Note (optional)
            <input type="text" value={note} onChange={(e) => setNote(e.target.value)} />
          </label>
          <div className="actions">
            <button type="button" disabled={busy || ticked.size === 0} onClick={() => void assignTicked()}>
              Assign to {ticked.size} room{ticked.size === 1 ? "" : "s"}
            </button>
          </div>
          <p className="hint">Each room takes the best fit, worked out again whenever it is read; rooms already assigned are replaced.</p>
        </>
      ) : null}
    </>
  );
}
