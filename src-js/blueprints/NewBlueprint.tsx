// Make a blueprint from a room: pick the room, see what capture would keep, name it, create it.
//
// Capture is a READ (`GET .../blueprints/capture`): it stores nothing, so what is shown is exactly
// what will be saved and nothing is created until the button is pressed. What capture leaves out
// (components of another instance, items with no footprint) is shown as numbers, not hidden.

import { useEffect, useMemo, useState } from "react";

import * as api from "./api.js";
import type { Blueprint, CaptureResponse, RoomRow } from "./types.js";

export interface NewBlueprintProps {
  project: string;
  onCreated(b: Blueprint): void;
  onCancel(): void;
}

export function NewBlueprint({ project, onCreated, onCancel }: NewBlueprintProps) {
  const [rooms, setRooms] = useState<RoomRow[] | null>(null);
  const [levels, setLevels] = useState<Map<string, string>>(new Map());
  const [filter, setFilter] = useState("");
  const [room, setRoom] = useState<RoomRow | null>(null);
  const [captured, setCaptured] = useState<CaptureResponse | null>(null);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api
      .listRooms(project)
      .then((r) => {
        setRooms(r.rooms);
        setLevels(new Map(r.levels.map((l) => [l.id, l.name])));
      })
      .catch((e: unknown) => setError(String((e as Error).message ?? e)));
  }, [project]);

  const shown = useMemo(() => {
    const f = filter.trim().toLowerCase();
    return (rooms ?? []).filter((r) => !f || r.name.toLowerCase().includes(f)).slice(0, 80);
  }, [rooms, filter]);

  const pick = async (r: RoomRow) => {
    setRoom(r);
    setCaptured(null);
    setError(null);
    setBusy(true);
    try {
      const c = await api.capture(project, r.id);
      setCaptured(c);
      setName(c.room_name);
    } catch (e) {
      setError(String((e as Error).message ?? e));
    } finally {
      setBusy(false);
    }
  };

  const create = async () => {
    if (!captured || !room) return;
    setBusy(true);
    const r = await api.createBlueprint(
      project,
      name.trim(),
      // The room row names no model; the server resolved it and says so, and a source is a record.
      { model_id: captured.model_id, room_id: room.id, room_name: captured.room_name },
      { outline: captured.outline, items: captured.items },
    );
    setBusy(false);
    if (r.ok) onCreated(r.value);
    else setError(r.error);
  };

  return (
    <section className="panel">
      <h3>New blueprint from a room</h3>
      {error ? <p className="error">{error}</p> : null}
      {!rooms ? <p className="hint">Reading the rooms…</p> : null}
      {rooms && !captured ? (
        <>
          <input type="search" placeholder="Filter rooms by name" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Filter rooms" />
          <ul className="picklist">
            {shown.map((r) => (
              <li key={r.id}>
                <button type="button" disabled={busy} className={room?.id === r.id ? "on" : ""} onClick={() => void pick(r)}>
                  {r.name}
                  <span>{levels.get(r.level_id) ?? ""}</span>
                </button>
              </li>
            ))}
          </ul>
          {rooms.length > shown.length ? <p className="hint">Showing {shown.length}; type to narrow.</p> : null}
          {busy ? <p className="hint">Reading the room and its FF&amp;E…</p> : null}
        </>
      ) : null}
      {captured ? (
        <>
          <p className="sub">
            {captured.items.length} items from <b>{captured.room_name}</b>
            {captured.skipped.nested + captured.skipped.no_footprint > 0
              ? `; left out: ${captured.skipped.nested} components of other items, ${captured.skipped.no_footprint} with no footprint`
              : ""}
            .
          </p>
          <label>
            Name
            <input type="text" value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          <div className="actions">
            <button type="button" disabled={busy || !name.trim()} onClick={() => void create()}>
              Create blueprint
            </button>
            <button type="button" onClick={() => setCaptured(null)}>
              Choose another room
            </button>
          </div>
        </>
      ) : null}
      <div className="actions">
        <button type="button" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </section>
  );
}
