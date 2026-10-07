// The connections editor's bar: what is being drawn, and what is already saved.
//
// One bar per zone, mounted inside the zone like the route bar, and exclusive
// with it: both claim a room pick. It draws two kinds of record. An OPEN ZONE is
// a set of rooms declared to be one open space. A VERTICAL LINK is exactly two
// rooms on different levels, joined by hand, one floor-to-floor hop per link.
// The bar owns the reads and writes of the project's connections document, which
// exist only while an editor is open.
//
// **Everything about picking is the page's own.** A click, a pick-list entry, a
// grid row or a search chip arrives as an ordinary room pick and `select()`
// hands it here, so pan, zoom, search and level changes all work between picks
// for the reason they do for the route: the working set is room ids in the
// store, not screen positions. That is also what makes a link possible: pick the
// lower room, change the zone's level picker, pick the upper room.

import { useEffect, useMemo } from "react";

import {
  addMembers,
  bodyAfterDeleteLink,
  bodyAfterDeleteZone,
  bodyAfterSave,
  connectionsUrl,
  DEFAULT_LEVEL_COST_FT,
  editOf,
  editOfLink,
  newEdit,
  removeMember,
  whyNotSavable,
  type ConnectionsDoc,
  type DocBody,
  type EditKind,
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
  const levels = payload?.levels ?? [];
  const doc = connections.projectId === scope.projectId ? connections.doc : null;

  if (!edit) return null;
  const isLink = edit.kind === "link";

  // What the pure rules cannot see, because they hold room ids and not levels: a
  // link's two rooms must be on different levels, and a link that skips a level
  // is allowed but worth saying (a lift is a run of floor-to-floor hops).
  const levelOf = (id: string) => rooms.get(id)?.level_id;
  const [first, second] = edit.members;
  const sameLevel = isLink && first && second && levelOf(first.room_id) === levelOf(second.room_id);
  const skipped = (() => {
    if (!isLink || !first || !second || sameLevel) return 0;
    const elev = new Map(levels.map((l) => [l.id, l.elevation ?? 0]));
    const used = new Set((payload?.rooms ?? []).map((r) => r.level_id));
    const [ea, eb] = [elev.get(levelOf(first.room_id) ?? "") ?? 0, elev.get(levelOf(second.room_id) ?? "") ?? 0];
    const [lo, hi] = [Math.min(ea, eb), Math.max(ea, eb)];
    return levels.filter((l) => used.has(l.id) && (l.elevation ?? 0) > lo && (l.elevation ?? 0) < hi).length;
  })();
  const reason =
    whyNotSavable(edit) ?? (sameLevel ? "Both rooms are on one level: change the level picker before the second pick." : null);

  const send = async (body: DocBody, afterOk: () => void) => {
    if (!url || !doc) return;
    patchZoneEdit(zoneId, { ...edit, saving: true, error: null });
    const r = await request(url, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ base: doc.taken_at, ...body }),
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

  const save = () => send(bodyAfterSave(doc!, edit), () => patchZoneEdit(zoneId, newEdit(edit.kind)));
  const removeZone = (id: string, name: string) => {
    if (!window.confirm(`Delete the open zone "${name}"? Routes that used it will stop using it.`)) return;
    void send(bodyAfterDeleteZone(doc!, id), () =>
      patchZoneEdit(zoneId, edit.id === id ? newEdit(edit.kind) : { ...edit, saving: false }),
    );
  };
  const removeLink = (id: string) => {
    if (!window.confirm("Delete this vertical link? Routes that used it will stop using it.")) return;
    void send(bodyAfterDeleteLink(doc!, id), () =>
      patchZoneEdit(zoneId, edit.id === id ? newEdit(edit.kind) : { ...edit, saving: false }),
    );
  };
  const nameOf = (id: string) => rooms.get(id)?.name || id;
  const levelName = (id: string) => levels.find((l) => l.id === levelOf(id))?.name;

  const matchCount = !isLink && search.active && search.matches ? search.matches.size : 0;
  const shown = edit.members.slice(0, CHIPS);
  const changeKind = (kind: EditKind) => patchZoneEdit(zoneId, { ...newEdit(kind), name: edit.name });

  return (
    <div className="routeBar zoneBar">
      <strong>{isLink ? "Vertical link" : "Open zone"}</strong>
      <select
        className="picker"
        value={edit.kind}
        title="Open area: shared walls between the rooms are open. Vertical link: two rooms on different levels, joined floor to floor."
        onChange={(e) => changeKind(e.target.value as EditKind)}
        aria-label="What to draw"
      >
        <option value="open">Open area</option>
        <option value="link">Vertical link</option>
      </select>
      {isLink ? (
        <>
          <span>
            From:{" "}
            <strong>{first ? nameOf(first.room_id) : "click a room…"}</strong>
            {first && levelName(first.room_id) ? <span className="routeLevel"> · {levelName(first.room_id)}</span> : null}
          </span>
          <span aria-hidden="true">↕</span>
          <span>
            To: <strong>{second ? nameOf(second.room_id) : first ? "change level, then click a room…" : "—"}</strong>
            {second && levelName(second.room_id) ? <span className="routeLevel"> · {levelName(second.room_id)}</span> : null}
          </span>
          <input
            className="zoneName zoneCost"
            placeholder={`ft (${DEFAULT_LEVEL_COST_FT})`}
            title="Walking-equivalent feet for this hop. Blank uses the default."
            value={edit.costFt}
            onChange={(e) => patchZoneEdit(zoneId, { ...edit, costFt: e.target.value, error: null })}
            aria-label="Cost of this hop in feet"
          />
          {skipped > 0 ? (
            <span className="routeNotice">
              This skips {skipped} level{skipped === 1 ? "" : "s"}. A lift or stair is a run of floor-to-floor links.
            </span>
          ) : null}
        </>
      ) : (
        <>
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
        </>
      )}
      {matchCount > 0 ? (
        <button
          className="ctl"
          title="Add every room matching the search to this open zone"
          onClick={() => patchZoneEdit(zoneId, addMembers(edit, search.matches!))}
        >
          Add {matchCount} search match{matchCount === 1 ? "" : "es"}
        </button>
      ) : null}
      <button
        className="ctl"
        onClick={() => void save()}
        disabled={reason !== null || !doc || edit.saving}
        title={reason ?? (isLink ? "Save this link" : "Save this open zone")}
      >
        {edit.saving ? "Saving…" : edit.id ? "Save changes" : isLink ? "Save link" : "Save zone"}
      </button>
      <button
        className="ctl"
        onClick={() => patchZoneEdit(zoneId, newEdit(edit.kind))}
        disabled={edit.members.length === 0 && edit.id === null && edit.name === ""}
      >
        {edit.id ? "Cancel edit" : "Clear"}
      </button>
      <button className="ctl" onClick={() => setZoneEditMode(zoneId, false)} title="Leave the editor (Esc)">
        Done
      </button>
      {edit.error ? <span className="routeNotice">{edit.error}</span> : null}
      {connections.error ? <span className="routeNotice">Could not read the saved connections: {connections.error}</span> : null}
      {reason && edit.members.length > 0 ? <span className="routeLevel">{reason}</span> : null}
      {isLink ? null : (
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
      )}
      {doc && (doc.zones.length > 0 || doc.links.length > 0) ? (
        <span className="routeMatches savedZones">
          Saved:
          {doc.zones.map((z) => (
            <span key={z.id} className="savedZone">
              <button
                className={`chip${edit.kind === "open" && edit.id === z.id ? " on" : ""}`}
                title="Load this open zone to change it"
                onClick={() => patchZoneEdit(zoneId, editOf(z))}
              >
                {z.name} ({z.rooms.length})
              </button>
              <button className="link" title="Delete this open zone" onClick={() => removeZone(z.id, z.name)}>
                delete
              </button>
            </span>
          ))}
          {doc.links.map((l) => (
            <span key={l.id} className="savedZone">
              <button
                className={`chip${edit.kind === "link" && edit.id === l.id ? " on" : ""}`}
                title="Load this vertical link to change it"
                onClick={() => patchZoneEdit(zoneId, editOfLink(l))}
              >
                ↕ {nameOf(l.a.room_id)} ↔ {nameOf(l.b.room_id)}
              </button>
              <button className="link" title="Delete this vertical link" onClick={() => removeLink(l.id)}>
                delete
              </button>
            </span>
          ))}
        </span>
      ) : null}
    </div>
  );
}
