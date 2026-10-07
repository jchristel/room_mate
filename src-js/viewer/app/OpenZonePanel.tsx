// The open-zone editor's bar: the rooms picked so far, the name, and what is
// already saved.
//
// One bar per zone, mounted inside the zone like the route bar, and exclusive
// with it: both claim a room pick. It owns the reads and writes of the project's
// connections document, which exist only while an editor is open.
//
// **Everything about picking is the page's own.** A click, a pick-list entry, a
// grid row or a search chip arrives as an ordinary room pick and `select()`
// hands it here, so pan, zoom, search and level changes all work between picks
// for the reason they do for the route: the working set is room ids in the
// store, not screen positions. The bulk gesture is search: type a query, then
// add every match in one click.

import { useEffect, useMemo } from "react";

import {
  addMembers,
  connectionsUrl,
  DEFAULT_LEVEL_COST_FT,
  editOf,
  newEdit,
  removeMember,
  whyNotSavable,
  zonesAfterDelete,
  zonesAfterSave,
  type ConnectionsDoc,
  type SavedZone,
  type ZoneKind,
} from "../connections.js";
import { patchZoneEdit, setConnections, setZoneEditMode, type ZoneRow } from "./store.js";
import { useViewer } from "./useViewer.js";

/** Members shown as chips before "+N more". A zone can hold hundreds. */
const CHIPS = 10;

type Reply = { ok: true; body: ConnectionsDoc } | { ok: false; status: number; message: string };

async function request(url: string, init?: RequestInit): Promise<Reply> {
  try {
    const res = await fetch(url, { cache: "no-store", ...init });
    if (!res.ok) return { ok: false, status: res.status, message: (await res.text()).trim() || `${url} -> ${res.status}` };
    return { ok: true, body: (await res.json()) as ConnectionsDoc };
  } catch (err) {
    return { ok: false, status: 0, message: `Could not reach ${url}: ${String(err)}` };
  }
}

export function OpenZonePanel({ zone }: { zone: ZoneRow }) {
  const { scope, payload, search, connections } = useViewer();
  const edit = zone.edit;
  const active = edit !== null;
  const url = connectionsUrl(scope);
  const zoneId = zone.id;

  // Read what is already saved, on opening and when the project changes.
  useEffect(() => {
    if (!active || !url) return;
    const ac = new AbortController();
    void request(url, { signal: ac.signal }).then((r) => {
      if (ac.signal.aborted) return;
      setConnections(
        r.ok
          ? { projectId: scope.projectId, doc: r.body, error: null }
          : { projectId: scope.projectId, doc: null, error: r.message },
      );
    });
    return () => ac.abort();
    // `scope` is spread into the key; the object itself changes identity freely.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, url]);

  // Escape leaves the editor, like every other transient thing on this page.
  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setZoneEditMode(zoneId, false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active, zoneId]);

  const rooms = useMemo(() => new Map((payload?.rooms ?? []).map((r) => [r.id, r])), [payload]);
  const doc = connections.projectId === scope.projectId ? connections.doc : null;

  if (!edit) return null;
  // A vertical zone is only a stair if it reaches two levels, which the pure rules
  // cannot see (they hold room ids, not levels), so the panel adds it.
  const levelsReached = new Set(edit.members.map((m) => rooms.get(m.room_id)?.level_id).filter(Boolean)).size;
  const reason =
    whyNotSavable(edit) ??
    (edit.kind === "vertical" && levelsReached < 2
      ? "A vertical zone needs rooms on at least two levels: change the level picker between picks."
      : null);

  const send = async (zones: unknown[], afterOk: () => void) => {
    if (!url || !doc) return;
    patchZoneEdit(zoneId, { ...edit, saving: true, error: null });
    const r = await request(url, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ base: doc.taken_at, zones }),
    });
    if (r.ok) {
      setConnections({ projectId: scope.projectId, doc: r.body, error: null });
      afterOk();
      return;
    }
    // A conflict means somebody saved since this was read: show theirs, keep ours.
    if (r.status === 409) {
      const fresh = await request(url);
      if (fresh.ok) setConnections({ projectId: scope.projectId, doc: fresh.body, error: null });
    }
    patchZoneEdit(zoneId, { ...edit, saving: false, error: r.message });
  };

  const save = () => send(zonesAfterSave(doc!, edit), () => patchZoneEdit(zoneId, newEdit()));
  const remove = (z: SavedZone) => {
    if (!window.confirm(`Delete the open zone "${z.name}"? Routes that used it will stop using it.`)) return;
    void send(zonesAfterDelete(doc!, z.id), () =>
      patchZoneEdit(zoneId, edit.id === z.id ? newEdit() : { ...edit, saving: false }),
    );
  };

  const matchCount = search.active && search.matches ? search.matches.size : 0;
  const shown = edit.members.slice(0, CHIPS);

  return (
    <div className="routeBar zoneBar">
      <strong>{edit.kind === "vertical" ? "Vertical zone" : "Open zone"}</strong>
      <select
        className="picker"
        value={edit.kind}
        title="Open: shared walls between the rooms are open. Vertical: a stair or lift, open within each level and joined between levels."
        onChange={(e) => patchZoneEdit(zoneId, { ...edit, kind: e.target.value as ZoneKind, error: null })}
        aria-label="Zone kind"
      >
        <option value="open">Open area</option>
        <option value="vertical">Vertical (stair / lift)</option>
      </select>
      <input
        className="zoneName"
        placeholder={edit.id ? "name" : "name this open zone"}
        value={edit.name}
        onChange={(e) => patchZoneEdit(zoneId, { ...edit, name: e.target.value, error: null })}
        aria-label="Open zone name"
      />
      <span>
        {edit.members.length} room{edit.members.length === 1 ? "" : "s"}
        {edit.members.length === 0 ? " — click rooms on the plan, or search and add every match" : ""}
      </span>
      {edit.kind === "vertical" ? (
        <input
          className="zoneName zoneCost"
          placeholder={`ft per level (${DEFAULT_LEVEL_COST_FT})`}
          title="Walking-equivalent feet for each level change. Blank uses the default."
          value={edit.levelCost}
          onChange={(e) => patchZoneEdit(zoneId, { ...edit, levelCost: e.target.value, error: null })}
          aria-label="Level cost in feet"
        />
      ) : null}
      {edit.kind === "vertical" && edit.members.length > 0 ? (
        <span className="routeLevel">
          {levelsReached} level{levelsReached === 1 ? "" : "s"}
        </span>
      ) : null}
      {matchCount > 0 ? (
        <button
          className="ctl"
          title="Add every room matching the search to this open zone"
          onClick={() => patchZoneEdit(zoneId, addMembers(edit, search.matches!))}
        >
          Add {matchCount} search match{matchCount === 1 ? "" : "es"}
        </button>
      ) : null}
      <button className="ctl" onClick={() => void save()} disabled={reason !== null || !doc || edit.saving} title={reason ?? "Save this open zone"}>
        {edit.saving ? "Saving…" : edit.id ? "Save changes" : "Save zone"}
      </button>
      <button
        className="ctl"
        onClick={() => patchZoneEdit(zoneId, newEdit())}
        disabled={edit.members.length === 0 && edit.id === null && edit.name === ""}
      >
        {edit.id ? "Cancel edit" : "Clear"}
      </button>
      <button className="ctl" onClick={() => setZoneEditMode(zoneId, false)} title="Leave the editor (Esc)">
        Done
      </button>
      {edit.error ? <span className="routeNotice">{edit.error}</span> : null}
      {connections.error ? <span className="routeNotice">Could not read the saved zones: {connections.error}</span> : null}
      {reason && edit.members.length > 0 ? <span className="routeLevel">{reason}</span> : null}
      <span className="routeMatches">
        {shown.map((m) => {
          const room = rooms.get(m.room_id);
          return (
            <button
              key={m.room_id}
              className="chip"
              title="Take this room out"
              onClick={() => patchZoneEdit(zoneId, removeMember(edit, m.room_id))}
            >
              {room?.name || m.room_id}
              {room ? "" : " (not in model)"} ×
            </button>
          );
        })}
        {edit.members.length > shown.length ? <em>+{edit.members.length - shown.length} more</em> : null}
      </span>
      {doc && doc.zones.length > 0 ? (
        <span className="routeMatches savedZones">
          Saved:
          {doc.zones.map((z) => (
            <span key={z.id} className="savedZone">
              <button
                className={`chip${edit.id === z.id ? " on" : ""}`}
                title="Load this open zone to change it"
                onClick={() => patchZoneEdit(zoneId, editOf(z))}
              >
                {z.kind === "vertical" ? "↕ " : ""}
                {z.name} ({z.rooms.length})
              </button>
              <button className="link" title="Delete this open zone" onClick={() => remove(z)}>
                delete
              </button>
            </span>
          ))}
        </span>
      ) : null}
    </div>
  );
}
