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
  bodyAfterSave,
  connectionsUrl,
  DEFAULT_LEVEL_COST_FT,
  newEdit,
  clearHub,
  removeCut,
  removeMember,
  setTool,
  whyNotSavable,
  type ConnectionsDoc,
  type DocBody,
  type EditKind,
} from "../connections.js";
import { connectivityUrl } from "../route.js";
import { put, request } from "./connectionsApi.js";
import { StackPanel } from "./StackPanel.js";
import { patchZoneEdit, setConnections, setIsolated, setZoneEditMode, type ZoneRow } from "./store.js";
import { useViewer } from "./useViewer.js";

/** Members shown as chips before "+N more". A zone can hold hundreds. */
const CHIPS = 10;

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

  // The rooms no door reaches: what a new zone is for. Asked again when the
  // document changes, because a saved zone is exactly what removes a room from it.
  const revision = payload?.revision ?? "";
  const version = connections.doc?.taken_at ?? "";
  useEffect(() => {
    const connectivity = scope.projectId ? connectivityUrl(scope, null, null) : null;
    if (!active || !connectivity) return;
    const ac = new AbortController();
    void fetch(connectivity, { cache: "no-store", signal: ac.signal })
      .then((res) => (res.ok && res.status !== 204 ? res.json() : null))
      .then((body: { isolated?: { room_id: string }[] } | null) => {
        if (ac.signal.aborted || !body) return;
        setIsolated(scope.projectId, new Set((body.isolated ?? []).map((x) => x.room_id)));
      })
      .catch(() => {});
    return () => ac.abort();
    // `scope` is spread into the key; the object itself changes identity freely.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, url, revision, version]);

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
    const r = await put(url, doc.taken_at, body);
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
  const nameOf = (id: string) => rooms.get(id)?.name || id;
  const levelName = (id: string) => levels.find((l) => l.id === levelOf(id))?.name;

  const matchCount = !isLink && search.active && search.matches ? search.matches.size : 0;
  const shown = edit.members.slice(0, CHIPS);
  const changeKind = (kind: EditKind) => patchZoneEdit(zoneId, { ...newEdit(kind), name: edit.name });

  const kindSelect = (
    <select
      className="picker"
      value={edit.kind}
      title="Open area: shared walls between the rooms are open. Vertical link: two rooms on different levels, joined floor to floor. Stack view: the rooms joined by vertical links as a column of levels, with the next hop suggested."
      onChange={(e) => changeKind(e.target.value as EditKind)}
      aria-label="What to draw"
    >
      <option value="open">Open area</option>
      <option value="link">Vertical link</option>
      <option value="stack">Stack view</option>
    </select>
  );

  if (edit.kind === "stack") {
    return (
      <div className="routeBar zoneBar">
        <strong>Stack view</strong>
        {kindSelect}
        <button className="ctl" onClick={() => setZoneEditMode(zoneId, false)} title="Leave the editor (Esc)">
          Done
        </button>
        {connections.error ? <span className="routeNotice">Could not read the saved connections: {connections.error}</span> : null}
        <StackPanel zone={zone} />
      </div>
    );
  }

  return (
    <div className="routeBar zoneBar">
      <strong>{isLink ? "Vertical link" : "Open zone"}</strong>
      {kindSelect}
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
      {isLink ? null : (
        <>
          <button
            className={`ctl${edit.tool === "cut" ? " on" : ""}`}
            disabled={edit.members.length < 2}
            aria-pressed={edit.tool === "cut"}
            onClick={() => patchZoneEdit(zoneId, setTool(edit, "cut"))}
            title="Keep the wall between two rooms of this area closed, so a route cannot go that way: click one room, then the room next to it. Click the same pair again to open it."
          >
            Cut
          </button>
          <button
            className={`ctl${edit.tool === "hub" ? " on" : ""}`}
            disabled={edit.members.length < 2}
            aria-pressed={edit.tool === "hub"}
            onClick={() => patchZoneEdit(zoneId, setTool(edit, "hub"))}
            title="Make the other rooms connect only through one room, such as a corridor with bays along it, so a route cannot cut through the bays"
          >
            Only through…
          </button>
          {edit.tool === "cut" ? (
            <span className="routeNotice">
              {edit.cutFrom
                ? `Now click the room next to ${nameOf(edit.cutFrom)}`
                : "Click a room, then the room next to it, to close the wall between them"}
            </span>
          ) : edit.tool === "hub" ? (
            <span className="routeNotice">Click the room the others connect through</span>
          ) : null}
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
          {edit.hub ? (
            <button
              className="chip hub"
              title="Stop connecting through this room"
              onClick={() => patchZoneEdit(zoneId, clearHub(edit))}
            >
              only through {nameOf(edit.hub)} ×
            </button>
          ) : null}
          {edit.cuts.map((c) => (
            <button
              key={`${c.a}|${c.b}`}
              className="chip cut"
              title="Open this wall again"
              onClick={() => patchZoneEdit(zoneId, removeCut(edit, c))}
            >
              {nameOf(c.a)} | {nameOf(c.b)} ×
            </button>
          ))}
        </span>
      )}
    </div>
  );
}
