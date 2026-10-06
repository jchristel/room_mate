// The route tool's bar: the two endpoints, the answer, and the way to leave.
//
// Mounted inside the header, as a full-width row, because the page's grid has
// three fixed rows and a fourth would have changed every layout rule that counts
// them. It renders nothing while the tool is off.
//
// **It also owns the two reads**, which is unusual for a panel and deliberate:
// the reads exist only while the tool does, and keeping them beside the bar that
// shows their answers means there is one place that knows when to ask. The
// first read is the cheap summary that says which rooms no door reaches (what a
// click is checked against); the second runs when both ends are placed.
//
// **Pan, zoom and search are the page's own and are untouched.** Nothing here
// captures the pointer or the keyboard apart from Escape: the endpoints are room
// ids in the store, not screen positions, so moving the view, changing a level
// or searching between the two clicks cannot lose either one. Search is carried
// further: while a query is active the bar lists the matching rooms that can be
// used, so an endpoint can be chosen without finding it on the plan first.

import { useEffect, useMemo } from "react";

import type { Level, Room } from "../../renderer/types.js";
import { connectivityUrl, describeResult, type RoutePath, type RouteState } from "../route.js";
import { clearRoute, patchRoute, select, setRouteMode, setZoneLevel } from "./store.js";
import { useViewer } from "./useViewer.js";

/** How many search matches the bar offers. A search can match thousands of
 *  rooms; the bar is a shortcut, not a results list, and the plan already
 *  highlights the rest. */
const MATCH_CHIPS = 8;

type Read = { ok: true; body: unknown } | { ok: false; message: string };

/** GET one connectivity read. A 204 is "nothing pushed" and a 400 carries the
 *  server's own sentence (an ambiguous room id says which models), which is
 *  worth showing as it is. */
async function read(url: string, signal: AbortSignal): Promise<Read> {
  try {
    const res = await fetch(url, { cache: "no-store", signal });
    if (res.status === 204) return { ok: true, body: null };
    if (!res.ok) return { ok: false, message: (await res.text()).trim() || `${url} -> ${res.status}` };
    return { ok: true, body: await res.json() };
  } catch (err) {
    return { ok: false, message: signal.aborted ? "" : `Could not read ${url}: ${String(err)}` };
  }
}

interface Summary {
  isolated?: { room_id: string }[];
  path?: RoutePath | null;
}

export function RoutePanel() {
  const { route, scope, payload, search, zones } = useViewer();
  const active = route !== null;
  const revision = payload?.revision ?? "";
  const scopeKey = `${scope.projectId}|${scope.building}|${scope.milestone}`;
  const start = route?.start ?? null;
  const end = route?.end ?? null;

  // Leaving a scope forgets the endpoints: a room id means a room in THAT
  // project, building and milestone.
  useEffect(() => {
    if (active) patchRoute({ start: null, end: null, unreachable: null, notice: null, result: { state: "idle" } });
  }, [active, scopeKey]);

  // Which rooms no door reaches. Refreshed when the rooms change, but the
  // endpoints stay: a push moves the graph, not what a reader has chosen.
  useEffect(() => {
    if (!active) return;
    const url = connectivityUrl(scope, null, null);
    if (!url) return;
    const ac = new AbortController();
    void read(url, ac.signal).then((r) => {
      if (ac.signal.aborted) return;
      if (!r.ok) return patchRoute({ result: { state: "error", message: r.message } });
      const isolated = (r.body as Summary | null)?.isolated ?? [];
      patchRoute({ unreachable: new Set(isolated.map((x) => x.room_id)) });
    });
    return () => ac.abort();
    // `scope` is spread into the key; the object itself changes identity freely.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, scopeKey, revision]);

  // The route, once both ends are placed.
  useEffect(() => {
    if (!active || !start || !end) return;
    const url = connectivityUrl(scope, start, end);
    if (!url) return;
    const ac = new AbortController();
    patchRoute({ result: { state: "loading" } });
    void read(url, ac.signal).then((r) => {
      if (ac.signal.aborted) return;
      if (!r.ok) return patchRoute({ result: { state: "error", message: r.message } });
      const path = (r.body as Summary | null)?.path;
      patchRoute({ result: path ? { state: "done", path } : { state: "error", message: "The server returned no route." } });
    });
    return () => ac.abort();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, start, end, scopeKey, revision]);

  // Escape leaves the tool, like every other transient thing on this page.
  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setRouteMode(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active]);

  const rooms = useMemo(() => new Map((payload?.rooms ?? []).map((r) => [r.id, r])), [payload]);
  const levels = payload?.levels ?? [];

  const matches = useMemo(() => {
    if (!route || !search.active || !search.matches) return null;
    const usable: Room[] = [];
    let total = 0;
    for (const r of payload?.rooms ?? []) {
      if (!search.matches.has(r.id)) continue;
      if (route.unreachable?.has(r.id)) continue;
      total += 1;
      if (usable.length < MATCH_CHIPS) usable.push(r);
    }
    return { usable, total };
  }, [route, search.active, search.matches, payload]);

  if (!route) return null;

  const showOnPlan = (roomId: string | null) => {
    const room = roomId ? rooms.get(roomId) : null;
    if (room?.level_id && zones[0]) setZoneLevel(zones[0].id, room.level_id);
  };

  return (
    <div id="routeBar">
      <strong>Route</strong>
      <Slot label="Start" roomId={start} rooms={rooms} levels={levels} next={start === null} onShow={() => showOnPlan(start)} onClear={() => select("room", start ?? "")} />
      <span aria-hidden="true">→</span>
      <Slot label="End" roomId={end} rooms={rooms} levels={levels} next={start !== null && end === null} onShow={() => showOnPlan(end)} onClear={() => select("room", end ?? "")} />
      <span className="routeResult">{describeResult(route.result)}</span>
      <Notice route={route} rooms={rooms} />
      {matches ? (
        <span className="routeMatches" title="Rooms matching the search that can be used as an endpoint">
          {matches.total === 0 ? (
            <em>no usable search match</em>
          ) : (
            matches.usable.map((r) => (
              <button key={r.id} className="chip" onClick={() => select("room", r.id)}>
                {r.name || r.id}
              </button>
            ))
          )}
          {matches.total > matches.usable.length ? <em>+{matches.total - matches.usable.length} more</em> : null}
        </span>
      ) : null}
      <button className="ctl" onClick={clearRoute} disabled={start === null && end === null}>
        Clear
      </button>
      <button className="ctl" onClick={() => setRouteMode(false)} title="Leave the route tool (Esc)">
        Done
      </button>
    </div>
  );
}

function Slot({
  label,
  roomId,
  rooms,
  levels,
  next,
  onShow,
  onClear,
}: {
  label: string;
  roomId: string | null;
  rooms: ReadonlyMap<string, Room>;
  levels: readonly Level[];
  next: boolean;
  onShow: () => void;
  onClear: () => void;
}) {
  if (roomId === null) {
    return (
      <span className={`routeSlot empty${next ? " next" : ""}`}>
        {label}: {next ? "click a room…" : "—"}
      </span>
    );
  }
  const room = rooms.get(roomId);
  const level = room ? levels.find((l) => l.id === room.level_id)?.name : null;
  return (
    <span className="routeSlot">
      {label}: <strong>{room?.name || roomId}</strong>
      {level ? <span className="routeLevel"> · {level}</span> : null}
      <button className="link" onClick={onShow} title="Show this room's level in the first zone">
        show
      </button>
      <button className="link" onClick={onClear} title="Clear this endpoint">
        ×
      </button>
    </span>
  );
}

function Notice({ route, rooms }: { route: RouteState; rooms: ReadonlyMap<string, Room> }) {
  const n = route.notice;
  if (!n) return null;
  if (n.kind === "loading") return <span className="routeNotice">Still reading door connections — try again in a moment.</span>;
  const name = rooms.get(n.roomId)?.name || n.roomId;
  return (
    <span className="routeNotice">
      {name} has no door connection, so it cannot be a start or an end. Pick a room a door leads to.
    </span>
  );
}
