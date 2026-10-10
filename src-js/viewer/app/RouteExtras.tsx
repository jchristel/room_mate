// What the route bar adds to a route once there is one: its steps, a way to save
// it, and the routes already saved.
//
// **Steps are a list because a route across levels cannot be seen at once.** A zone
// shows one level, so a route that climbs two storeys is mostly off screen; the list
// reads the whole of it in order, marks where the level changes, and a row takes the
// zone to that room.
//
// **A saved route is a request, not a path** (`connections.SavedRoute`): the rooms,
// where in them, the method and a colour. Saving writes the project's one
// connections document, so routes are shared like zones and links are. Which routes
// are drawn is the reader's own and lives in the page store.

import { useState } from "react";

import type { Level, Room } from "../../renderer/types.js";
import {
  bodyAfterSaveRoute,
  type ConnectionsDoc,
  connectionsUrl,
  nextRouteColour,
  whyRouteNotSavable,
  type DocBody,
} from "../connections.js";
import { parseWidth, routeRows, type RouteState } from "../route.js";
import { ColourInput } from "./ColourInput.js";
import { put, request } from "./connectionsApi.js";
import { panToRoom } from "./zoneRegistry.js";
import { setConnections, setSavedRouteShown, setZoneLevel, type ZoneRow } from "./store.js";
import { useViewer } from "./useViewer.js";

export function RouteExtras({ zone, route }: { zone: ZoneRow; route: RouteState }) {
  const { scope, payload, connections } = useViewer();
  const url = connectionsUrl(scope);
  const doc = connections.projectId === scope.projectId ? connections.doc : null;
  const [name, setName] = useState("");
  const [colour, setColour] = useState<string | null>(null);
  const [stepsOpen, setStepsOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const rooms = new Map((payload?.rooms ?? []).map((r) => [r.id, r]));
  const levels = payload?.levels ?? [];
  const path = route.result.state === "done" && route.result.path.found ? route.result.path : null;
  const rows = routeRows(path, (id) => rooms.get(id)?.level_id);

  const draft = {
    name,
    colour: colour ?? nextRouteColour(doc),
    from: route.start ?? "",
    to: route.end ?? "",
    fromAt: route.startAt,
    toAt: route.endAt,
    method: route.method,
    widthMm: parseWidth(route.width) === "bad" ? null : (parseWidth(route.width) as number | null),
  };
  const reason = whyRouteNotSavable(draft) ?? (path ? null : "Only a route that was found can be saved.");

  /** Write the document, and on a conflict show theirs and keep ours. */
  const send = async (body: DocBody | null, afterOk: (saved: ConnectionsDoc) => void) => {
    if (!url || !doc || !body) return;
    setSaving(true);
    setError(null);
    const r = await put(url, doc.taken_at, body);
    setSaving(false);
    if (r.ok) {
      setConnections({ projectId: scope.projectId, doc: r.body, error: null });
      afterOk(r.body);
      return;
    }
    if (r.status === 409) {
      const fresh = await request(url);
      if (fresh.ok) setConnections({ projectId: scope.projectId, doc: fresh.body, error: null });
    }
    setError(r.message);
  };

  const save = () =>
    send(doc ? bodyAfterSaveRoute(doc, draft) : null, (saved) => {
      const before = new Set(doc!.routes.map((x) => x.id));
      const added = saved.routes.find((x) => !before.has(x.id));
      // A route just saved is one the reader wants to see, and it is the same line as
      // the draft, so showing it costs nothing visible until the draft is cleared.
      if (added) setSavedRouteShown(added.id, true);
      setName("");
      setColour(null);
    });

  const showRoom = (roomId: string) => {
    const room = rooms.get(roomId);
    if (!room) return;
    if (room.level_id) setZoneLevel(zone.id, room.level_id);
    // The zone repaints on its new level first; the pan needs that view.
    window.setTimeout(() => panToRoom(room), 0);
  };

  return (
    <>
      {path ? (
        <>
          <button className="ctl" onClick={() => setStepsOpen((o) => !o)} aria-expanded={stepsOpen}>
            {stepsOpen ? "Hide steps" : `Steps (${rows.length})`}
          </button>
          <input
            className="zoneName"
            placeholder="name this route"
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-label="Route name"
          />
          <ColourInput value={draft.colour} onCommit={setColour} label="Colour for this route" />
          <button className="ctl" onClick={() => void save()} disabled={reason !== null || !doc || saving} title={reason ?? "Save this route for everyone on the project"}>
            {saving ? "Saving…" : "Save route"}
          </button>
        </>
      ) : null}
      {error ? <span className="routeNotice">{error}</span> : null}
      {stepsOpen && path ? <Steps rows={rows} rooms={rooms} levels={levels} onShow={showRoom} /> : null}
    </>
  );
}

function Steps({
  rows,
  rooms,
  levels,
  onShow,
}: {
  rows: ReturnType<typeof routeRows>;
  rooms: ReadonlyMap<string, Room>;
  levels: readonly Level[];
  onShow: (roomId: string) => void;
}) {
  return (
    <ol className="routeSteps">
      {rows.map((row, i) => {
        const room = rooms.get(row.roomId);
        const level = levels.find((l) => l.id === room?.level_id)?.name;
        const how =
          row.how === "start"
            ? "start"
            : row.how === "door"
              ? `door${row.doorId ? ` ${row.doorId}` : ""}`
              : row.how === "zone"
                ? "open zone"
                : "level change";
        return (
          <li key={`${i}-${row.roomId}`} className={row.levelChanged ? "levelChanged" : undefined}>
            <button className="link" onClick={() => onShow(row.roomId)} title="Show this room">
              {room?.name || row.roomId}
            </button>
            {level ? <span className="routeLevel"> · {level}</span> : null}
            <span className="routeLevel">
              {" "}
              — {how}
              {row.lengthFt > 0 ? `, ${Math.round(row.lengthFt)} ft` : ""}
            </span>
          </li>
        );
      })}
    </ol>
  );
}
