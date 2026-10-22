// The assign view: put a blueprint revision in another room, see what the server's resolver does with
// it, and keep the choice.
//
// **Doors orient, the person decides.** A rectangle fits a blueprint two ways a half turn apart and
// shape cannot choose; the doors the room already has (they are in the model) can, so the rotations
// that fit are listed best-by-doors first and the best is used unless the person picks another. A lot
// of resolving is manual by design, so any mapping can be made by hand, edge by edge, which is also the
// only way into a room whose edge count differs.
//
// **Nothing is computed here.** The placements, findings, candidates and the doors as mapped all come
// back from `GET .../resolve`; this file chooses what to ask and draws the answer. Saving stores the
// REQUEST (blueprint, pinned revision, room, mapping), never the placements: they are worked out again
// on every read, from the room as it stands.
//
// Several rooms at once is `AssignMany`, which asks the same resolver about each and lets the person
// tick. **Per-item overrides** (leave an item out, nudge it, turn it) are this room's own and are kept with
// its assignment; they ride the resolve request, so the drawing is still the server's answer. Not here yet:
// dragging, rotating or scaling a box over the main plan.

import { useCallback, useEffect, useMemo, useState } from "react";

import { AssignCanvas } from "./AssignCanvas.js";
import { AssignMany } from "./AssignMany.js";
import * as api from "./api.js";
import { adjust, describe, encode, overrideFor, reset, sameOverrides } from "./overrides.js";
import type { AssignmentRow, Blueprint, Finding, ItemOverride, Mapping, ResolveResponse, RoomRow } from "./types.js";

export interface AssignProps {
  project: string;
  blueprint: Blueprint;
  /** The saved revision being placed. A draft or a pending one is not assignable. */
  revision: number;
  onClose(): void;
}

const message = (e: unknown): string => String((e as Error).message ?? e);

const KIND_TEXT: Record<string, string> = {
  plain: "fits by shape",
  manual: "mapped by hand",
  mirror_image: "mirror image of the room",
  no_fit: "does not fit by shape",
  edge_count_differs: "different number of edges",
};

function sameMapping(a: Mapping, b: Mapping): boolean {
  return a.shift === b.shift && (a.edge_map ?? []).join(",") === (b.edge_map ?? []).join(",");
}

function findingText(f: Finding, name: (id: string) => string): string {
  switch (f.kind) {
    case "outside":
      return `${name(f.item)} is outside the room (it was inside the blueprint's)`;
    case "overlap":
      return `${name(f.a)} and ${name(f.b)} overlap (they did not in the blueprint)`;
    case "follow_broken":
      return `${name(f.item)}: ${f.reason}`;
    case "corner_anchored":
      return `${name(f.item)} had no wall beside it when captured; its position is approximate`;
    case "override_unused":
      return `a change to item ${f.item} was not used: this revision does not place that item`;
    default:
      return f.message;
  }
}

export function Assign({ project, blueprint, revision, onClose }: AssignProps) {
  const [rooms, setRooms] = useState<RoomRow[] | null>(null);
  const [levels, setLevels] = useState<Map<string, string>>(new Map());
  const [assigned, setAssigned] = useState<AssignmentRow[]>([]);
  const [filter, setFilter] = useState("");
  const [room, setRoom] = useState<RoomRow | null>(null);
  const [mapping, setMapping] = useState<Mapping>({});
  const [overrides, setOverrides] = useState<ItemOverride[]>([]);
  const [step, setStep] = useState(50);
  const [result, setResult] = useState<ResolveResponse | null>(null);
  const [byHand, setByHand] = useState<number[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [showTags, setShowTags] = useState(false);
  const [many, setMany] = useState(false);
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const bpEdges = (blueprint.revisions.find((r) => r.number === revision)?.outline.coarse.length ?? 0) || 0;

  const refreshAssigned = useCallback(
    () => api.listAssignments(project).then(setAssigned).catch((e: unknown) => setError(message(e))),
    [project],
  );

  useEffect(() => {
    api
      .listRooms(project)
      .then((r) => {
        setRooms(r.rooms);
        setLevels(new Map(r.levels.map((l) => [l.id, l.name])));
      })
      .catch((e: unknown) => setError(message(e)));
    void refreshAssigned();
  }, [project, refreshAssigned]);

  // Ask the server again whenever what is asked changes. A stale answer is dropped.
  useEffect(() => {
    if (!room) return;
    let live = true;
    setBusy(true);
    api
      .resolve(project, blueprint.id, room.id, revision, mapping, encode(overrides))
      .then((r) => {
        if (!live) return;
        setResult(r);
        setError(null);
      })
      .catch((e: unknown) => {
        if (!live) return;
        setResult(null);
        setError(message(e));
      })
      .finally(() => live && setBusy(false));
    return () => {
      live = false;
    };
  }, [project, blueprint.id, room, revision, mapping, overrides]);

  const choose = (r: RoomRow) => {
    setRoom(r);
    setSelected(null);
    setByHand(null);
    setNotice(null);
    const existing = assigned.find((a) => a.room.room_id === r.id && a.blueprint_id === blueprint.id);
    setMapping(existing?.mapping ?? {});
    setOverrides(existing?.overrides ?? []);
    if (existing?.mapping.edge_map) setByHand(existing.mapping.edge_map);
  };

  /** Where each blueprint edge goes now, as a list to start a by-hand mapping from. */
  const currentMap = (): number[] => {
    const k = result?.outline.coarse.length ?? 0;
    if (!result || k === 0) return Array.from({ length: bpEdges }, (_, i) => i);
    if (mapping.edge_map) return mapping.edge_map;
    if (result.fit.kind === "edge_count_differs") return Array.from({ length: bpEdges }, (_, i) => i % k);
    return Array.from({ length: bpEdges }, (_, i) => (i + result.fit.shift) % k);
  };

  const startByHand = () => {
    const m = currentMap();
    setByHand(m);
    setMapping({ edge_map: m });
  };

  const setEdge = (i: number, to: number) => {
    const next = (byHand ?? currentMap()).slice();
    next[i] = to;
    setByHand(next);
    setMapping({ edge_map: next });
  };

  const save = async () => {
    if (!room) return;
    setBusy(true);
    const r = await api.assign(project, blueprint.id, revision, room.id, mapping, note.trim() || undefined, overrides);
    setBusy(false);
    if (!r.ok) return setError(r.error);
    setError(null);
    setNotice(`Assigned r${revision} to ${room.name}.`);
    setNote("");
    await refreshAssigned();
  };

  const remove = async (a: AssignmentRow) => {
    if (!window.confirm(`Remove the assignment of "${a.room_name}"? The blueprint is untouched.`)) return;
    const r = await api.deleteAssignment(project, a.id);
    if (!r.ok) return setError(r.error);
    setNotice(`Removed the assignment of ${a.room_name}.`);
    await refreshAssigned();
  };

  const names = useMemo(() => new Map(result?.placements.map((p) => [p.item_id, p.type_name]) ?? []), [result]);
  const nameOf = (id: string): string => names.get(id) ?? `item ${id}`;
  const ofBlueprint = assigned.filter((a) => a.blueprint_id === blueprint.id);
  const assignedRoomIds = new Set(assigned.map((a) => a.room.room_id));
  const shown = useMemo(() => {
    const f = filter.trim().toLowerCase();
    return (rooms ?? []).filter((r) => !f || r.name.toLowerCase().includes(f)).slice(0, 80);
  }, [rooms, filter]);
  const roomEdges = result?.outline.coarse.length ?? 0;
  const here = room ? assigned.find((a) => a.room.room_id === room.id) : undefined;
  const saved =
    !!here && here.blueprint_id === blueprint.id && here.revision === revision && sameMapping(here.mapping, mapping) && sameOverrides(here.overrides ?? [], overrides);
  const changed = useMemo(() => new Set(overrides.filter((o) => !o.exclude).map((o) => o.item_id)), [overrides]);
  // Items the blueprint holds that this room leaves out, named from the blueprint (they are not in the answer).
  const itemsOfRevision = blueprint.revisions.find((r) => r.number === revision)?.items ?? [];
  const leftOut = overrides.filter((o) => o.exclude);
  const selectedPlaced = selected ? result?.placements.find((p) => p.item_id === selected) : undefined;
  const selectedOverride = selected ? overrideFor(overrides, selected) : undefined;
  const auto = mapping.shift === undefined && !mapping.edge_map;

  return (
    <>
      <main className="assign">
        <div className="toolbar">
          <strong>{blueprint.name}</strong>
          <span className="sub">
            r{revision} placed in {room ? room.name : "…"}
            {result ? ` · ${KIND_TEXT[result.fit.kind] ?? result.fit.kind}` : ""}
            {result ? ` · ${result.placements.length} items` : ""}
          </span>
          <label>
            <input type="checkbox" checked={showTags} onChange={(e) => setShowTags(e.target.checked)} /> FF&amp;E tags
          </label>
          <span className="legend">
            <i className="bar" /> room's door <i className="bar mapped" /> blueprint's door
          </span>
          <button type="button" style={{ marginLeft: "auto" }} onClick={onClose}>
            Back to the blueprint
          </button>
        </div>
        {error ? <p className="banner error">{error}</p> : notice ? <p className="banner">{notice}</p> : <div />}
        {result ? (
          <AssignCanvas result={result} selected={selected} changed={changed} showTags={showTags} onSelect={setSelected} />
        ) : (
          <p className="hint empty">{room ? (busy ? "Placing…" : "") : "Choose a room on the right to place this blueprint in."}</p>
        )}
      </main>

      <aside className="inspector">
        <div className="actions">
          <button type="button" className={many ? "" : "on"} onClick={() => setMany(false)}>
            One room
          </button>
          <button type="button" className={many ? "on" : ""} onClick={() => setMany(true)}>
            Several rooms
          </button>
        </div>
        {many ? (
          <AssignMany
            project={project}
            blueprint={blueprint}
            revision={revision}
            rooms={rooms}
            levels={levels}
            assigned={assigned}
            onPreview={choose}
            onOpen={(r) => {
              setMany(false);
              choose(r);
            }}
            onAssigned={(m) => {
              setNotice(m);
              void refreshAssigned();
            }}
          />
        ) : null}
        {many ? null : (
          <>
        <h3>Room</h3>
        {!rooms ? <p className="hint">Reading the rooms…</p> : null}
        <input type="search" placeholder="Filter rooms by name" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Filter rooms" />
        <ul className="picklist">
          {shown.map((r) => (
            <li key={r.id}>
              <button type="button" className={room?.id === r.id ? "on" : ""} onClick={() => choose(r)}>
                {assignedRoomIds.has(r.id) ? "● " : ""}
                {r.name}
                <span>{levels.get(r.level_id) ?? ""}</span>
              </button>
            </li>
          ))}
        </ul>
        {rooms && rooms.length > shown.length ? <p className="hint">Showing {shown.length}; type to narrow. ● has an assignment.</p> : null}
          </>
        )}

        {result && !many ? (
          <>
            <h3>Mapping</h3>
            <p className="sub">
              {result.outline.coarse.length} room edges, {bpEdges} blueprint edges
              {result.fit.cost !== null ? ` · shape cost ${result.fit.cost.toFixed(2)}` : ""}
            </p>
            <div className="actions">
              <button type="button" className={auto ? "on" : ""} onClick={() => { setByHand(null); setMapping({}); }}>
                Best fit
              </button>
              <button type="button" className={byHand ? "on" : ""} onClick={startByHand}>
                By hand
              </button>
            </div>
            {result.candidates.length > 0 ? (
              <fieldset>
                <legend>Rotations that fit</legend>
                {result.candidates.map((c) => (
                  <button
                    type="button"
                    key={c.shift}
                    className={!byHand && result.fit.shift === c.shift && result.fit.kind !== "manual" ? "on" : ""}
                    onClick={() => { setByHand(null); setMapping({ shift: c.shift }); }}
                  >
                    turn {c.shift} · {c.door_gap_mm === null ? "no doors to compare" : `doors ${Math.round(c.door_gap_mm)} mm apart`} · cost {c.cost.toFixed(2)}
                  </button>
                ))}
              </fieldset>
            ) : null}
            {byHand ? (
              <fieldset>
                <legend>Blueprint edge → room edge</legend>
                {byHand.map((to, i) => (
                  <label key={i} className="num">
                    edge {i}
                    <select value={to} onChange={(e) => setEdge(i, Number(e.target.value))}>
                      {Array.from({ length: roomEdges }, (_, j) => (
                        <option key={j} value={j}>
                          room edge {j}
                        </option>
                      ))}
                    </select>
                  </label>
                ))}
                <p className="hint">Nothing about a mapping by hand is judged: the items go where it says.</p>
              </fieldset>
            ) : result.fit.kind === "edge_count_differs" ? (
              <p className="note">The room has a different number of edges, so nothing is placed until the edges are mapped by hand.</p>
            ) : null}

            <h3>Findings</h3>
            {result.findings.length === 0 ? <p className="hint">None: nothing the room changes about the layout.</p> : null}
            <ul className="findings">
              {result.findings.map((f, i) => {
                const id = f.kind === "outside" || f.kind === "follow_broken" || f.kind === "corner_anchored" ? f.item : f.kind === "overlap" ? f.a : null;
                return (
                  <li key={i} className={f.kind}>
                    {id ? (
                      <button type="button" onClick={() => setSelected(id)}>
                        {findingText(f, nameOf)}
                      </button>
                    ) : (
                      findingText(f, nameOf)
                    )}
                  </li>
                );
              })}
            </ul>
            {result.fit.kind === "mirror_image" ? <p className="hint">This room is the other hand of the blueprint. Assign a blueprint authored for the other hand; nothing is mirrored for you.</p> : null}
            {selectedPlaced ? (
              <fieldset className="item-changes">
                <legend>{selectedPlaced.type_name}</legend>
                <p className="hint">{selectedPlaced.rule}</p>
                <label className="num">
                  step (mm)
                  <input type="number" min={1} max={2000} value={step} onChange={(e) => setStep(Math.max(1, Math.round(Number(e.target.value)) || 1))} />
                </label>
                <div className="pad" role="group" aria-label="Move this item in this room">
                  <button type="button" aria-label="Move up" onClick={() => setOverrides((l) => adjust(l, selectedPlaced.item_id, { dy_mm: step }))}>↑</button>
                  <button type="button" aria-label="Move left" onClick={() => setOverrides((l) => adjust(l, selectedPlaced.item_id, { dx_mm: -step }))}>←</button>
                  <button type="button" aria-label="Move right" onClick={() => setOverrides((l) => adjust(l, selectedPlaced.item_id, { dx_mm: step }))}>→</button>
                  <button type="button" aria-label="Move down" onClick={() => setOverrides((l) => adjust(l, selectedPlaced.item_id, { dy_mm: -step }))}>↓</button>
                </div>
                <div className="actions">
                  <button type="button" onClick={() => setOverrides((l) => adjust(l, selectedPlaced.item_id, { rotate_deg: 90 }))}>Turn 90° left</button>
                  <button type="button" onClick={() => setOverrides((l) => adjust(l, selectedPlaced.item_id, { rotate_deg: -90 }))}>Turn 90° right</button>
                </div>
                <div className="actions">
                  <button type="button" className="danger" onClick={() => { setOverrides((l) => adjust(l, selectedPlaced.item_id, { exclude: true })); setSelected(null); }}>
                    Leave out of this room
                  </button>
                  <button type="button" disabled={!selectedOverride} onClick={() => setOverrides((l) => reset(l, selectedPlaced.item_id))}>
                    Put back as the blueprint has it
                  </button>
                </div>
                {selectedOverride ? <p className="note">{describe(selectedOverride)}</p> : null}
              </fieldset>
            ) : selected ? (
              <p className="note">
                <b>{nameOf(selected)}</b>
              </p>
            ) : null}
            {leftOut.length > 0 ? (
              <fieldset className="item-changes">
                <legend>Left out of this room</legend>
                <ul className="findings">
                  {leftOut.map((o) => (
                    <li key={o.item_id}>
                      {itemsOfRevision.find((i) => i.id === o.item_id)?.type_name ?? `item ${o.item_id}`}
                      <button type="button" onClick={() => setOverrides((l) => reset(l, o.item_id))}>
                        put back
                      </button>
                    </li>
                  ))}
                </ul>
              </fieldset>
            ) : null}
            {overrides.length > 0 ? <p className="hint">{overrides.length} change{overrides.length === 1 ? "" : "s"} to single items, kept with this assignment when you assign.</p> : null}

            <h3>Assign</h3>
            <label className="stack">
              Note (optional)
              <input type="text" value={note} onChange={(e) => setNote(e.target.value)} />
            </label>
            <div className="actions">
              <button type="button" disabled={busy || saved || result.placements.length === 0} onClick={() => void save()}>
                {here ? (saved ? "Assigned" : "Replace this room's assignment") : "Assign to this room"}
              </button>
            </div>
            {here && here.blueprint_id !== blueprint.id ? <p className="hint">This room is assigned to another blueprint; assigning replaces it.</p> : null}
          </>
        ) : null}

        <h3>Rooms with this blueprint</h3>
        {ofBlueprint.length === 0 ? <p className="hint">None yet.</p> : null}
        <ul className="findings">
          {ofBlueprint.map((a) => (
            <li key={a.id}>
              <button type="button" onClick={() => { const r = rooms?.find((x) => x.id === a.room.room_id); if (r) choose(r); }}>
                {a.room_name} · r{a.revision}
                {a.newer_revision ? ` (r${a.current_revision} is current)` : ""}
              </button>
              <button type="button" className="danger" onClick={() => void remove(a)}>
                remove
              </button>
            </li>
          ))}
        </ul>
      </aside>
    </>
  );
}
