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

import { RouteExtras } from "./RouteExtras.js";

import type { Level, Room } from "../../renderer/types.js";
import {
  type Clearance,
  connectivityUrl,
  describeClearance,
  describeResult,
  parseHeight,
  parseWidth,
  type PlanPoint,
  type RouteMethod,
  type RoutePath,
  type RouteState,
} from "../route.js";
import { clearZoneRoute, patchZoneRoute, select, setZoneLevel, setZoneRouteMode, type ZoneRow } from "./store.js";
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
  methods?: RouteMethod[];
  isolated?: { room_id: string }[];
  path?: RoutePath | null;
  clearance?: Clearance;
}

export function RoutePanel({ zone }: { zone: ZoneRow }) {
  const { scope, payload, search, toolFocus, connections } = useViewer();
  // A saved or deleted open zone changes which rooms a door-or-zone route can
  // reach, so it is a reason to ask the server again.
  const connectionsVersion = connections.projectId === scope.projectId ? (connections.doc?.taken_at ?? "") : "";
  const route = zone.route;
  const zoneId = zone.id;
  const patchRoute = (patch: Partial<RouteState>) => patchZoneRoute(zoneId, patch);
  const active = route !== null;
  const revision = payload?.revision ?? "";
  const scopeKey = `${scope.projectId}|${scope.building}|${scope.milestone}`;
  const start = route?.start ?? null;
  const method = route?.method ?? null;
  const width = parseWidth(route?.width ?? "");
  const height = parseHeight(route?.height ?? "");
  const end = route?.end ?? null;
  const startAt = route?.startAt ?? null;
  const endAt = route?.endAt ?? null;
  // The points as text, so an effect depends on WHERE and not on the identity of a
  // fresh object.
  const pointsKey = `${startAt?.x},${startAt?.y}|${endAt?.x},${endAt?.y}`;

  // Leaving a scope forgets the endpoints: a room id means a room in THAT
  // project, building and milestone.
  useEffect(() => {
    if (active) {
      patchRoute({ start: null, end: null, startAt: null, endAt: null, unreachable: null, notice: null, result: { state: "idle" } });
    }
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
      patchRoute({ unreachable: new Set(isolated.map((x) => x.room_id)), methods: (r.body as Summary | null)?.methods ?? [] });
    });
    return () => ac.abort();
    // `scope` is spread into the key; the object itself changes identity freely.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, scopeKey, revision, connectionsVersion]);

  // The route, once both ends are placed.
  useEffect(() => {
    if (!active || !start || !end) return;
    // A width that is not a number is not sent; the bar says so beside the field.
    const url = connectivityUrl(
      scope,
      start,
      end,
      method,
      { from: startAt, to: endAt },
      {},
      width === "bad" ? null : width,
      height === "bad" ? null : height,
    );
    if (!url) return;
    const ac = new AbortController();
    patchRoute({ result: { state: "loading" } });
    void read(url, ac.signal).then((r) => {
      if (ac.signal.aborted) return;
      if (!r.ok) return patchRoute({ result: { state: "error", message: r.message } });
      const body = r.body as Summary | null;
      const path = body?.path;
      patchRoute({
        result: path
          ? { state: "done", path, ...(body?.clearance ? { clearance: body.clearance } : {}) }
          : { state: "error", message: "The server returned no route." },
      });
    });
    return () => ac.abort();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, start, end, method, width, height, pointsKey, scopeKey, revision, connectionsVersion]);

  // Escape leaves the tool, like every other transient thing on this page.
  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && toolFocus === zoneId) setZoneRouteMode(zoneId, false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active, toolFocus, zoneId]);

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
    if (room?.level_id) setZoneLevel(zoneId, room.level_id);
  };

  return (
    <div className="routeBar">
      <strong>Route</strong>
      <Slot label="Start" roomId={start} at={startAt} onResetPoint={() => patchRoute({ startAt: null })} rooms={rooms} levels={levels} next={start === null} onShow={() => showOnPlan(start)} onClear={() => select("room", start ?? "", zoneId)} />
      <span aria-hidden="true">→</span>
      <Slot label="End" roomId={end} at={endAt} onResetPoint={() => patchRoute({ endAt: null })} rooms={rooms} levels={levels} next={start !== null && end === null} onShow={() => showOnPlan(end)} onClear={() => select("room", end ?? "", zoneId)} />
      <span className="routeResult">{describeResult(route.result)}</span>
      <input
        className="zoneName widthInput"
        inputMode="numeric"
        placeholder="width mm (0 = none)"
        value={route.width}
        onChange={(e) => patchRoute({ width: e.target.value })}
        aria-label="Width of the object that has to make the trip, in millimetres"
        title="The width in mm of what has to make the trip (a bed, a trolley, plant). A door or open wall narrower than this is not passable. Door widths are estimates: the footprint less 150 mm for the frame. Corridors inside rooms are not checked."
      />
      <input
        className="zoneName widthInput"
        inputMode="numeric"
        placeholder="height mm (0 = none)"
        value={route.height}
        onChange={(e) => patchRoute({ height: e.target.value })}
        aria-label="Height of the object that has to make the trip, in millimetres"
        title="The height in mm of what has to make the trip. A door lower than this, or a ROOM whose clear height is lower, is not passable. Door heights are estimates (a height property, else the size in the type name); room heights come from the project's room height property (Settings, Routing). Anything unreadable is let through and counted."
      />
      {width === "bad" ? <span className="routeNotice">Width must be a number of millimetres.</span> : null}
      {height === "bad" ? <span className="routeNotice">Height must be a number of millimetres.</span> : null}
      {route.result.state === "done" && describeClearance(route.result.clearance, (id) => rooms.get(id)?.name || id) ? (
        <span className="routeLevel" title="Door widths are estimates; level changes and doors with no footprint are not checked">
          {describeClearance(route.result.clearance, (id) => rooms.get(id)?.name || id)}
        </span>
      ) : null}
      {route.result.state === "done" && route.result.path.note ? (
        <span className="routeNotice">{route.result.path.note}</span>
      ) : null}
      {route.methods.length > 1 ? (
        <select
          className="picker"
          value={method ?? ""}
          title={methodTitle(route.methods, method)}
          onChange={(e) => patchRoute({ method: e.target.value || null })}
          aria-label="Routing method"
        >
          <option value="">Default ({route.methods[0]!.name})</option>
          {route.methods.map((m) => (
            <option key={m.id} value={m.id} title={m.summary}>
              {m.name}
            </option>
          ))}
        </select>
      ) : null}
      <Notice route={route} rooms={rooms} />
      {matches ? (
        <span className="routeMatches" title="Rooms matching the search that can be used as an endpoint">
          {matches.total === 0 ? (
            <em>no usable search match</em>
          ) : (
            matches.usable.map((r) => (
              <button key={r.id} className="chip" onClick={() => select("room", r.id, zoneId)}>
                {r.name || r.id}
              </button>
            ))
          )}
          {matches.total > matches.usable.length ? <em>+{matches.total - matches.usable.length} more</em> : null}
        </span>
      ) : null}
      <RouteExtras zone={zone} route={route} />
      <button className="ctl" onClick={() => clearZoneRoute(zoneId)} disabled={start === null && end === null}>
        Clear
      </button>
      <button className="ctl" onClick={() => setZoneRouteMode(zoneId, false)} title="Leave the route tool (Esc)">
        Done
      </button>
    </div>
  );
}

function Slot({
  label,
  roomId,
  at,
  onResetPoint,
  rooms,
  levels,
  next,
  onShow,
  onClear,
}: {
  label: string;
  roomId: string | null;
  at: PlanPoint | null;
  onResetPoint: () => void;
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
      {at ? (
        <>
          <span className="routeLevel"> · at your click</span>
          <button className="link" onClick={onResetPoint} title="Use the room's centre instead of the spot you clicked">
            centre
          </button>
        </>
      ) : null}
      <button className="link" onClick={onShow} title="Show this room's level in this zone">
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

/** The tooltip for the method picker: what the chosen method is and where it
 *  comes from, so a choice is never a bare label. The server lists its default
 *  method first. */
function methodTitle(methods: readonly RouteMethod[], chosen: string | null): string {
  const m = methods.find((x) => x.id === chosen) ?? methods[0];
  return m ? `${m.summary}\n\nSource: ${m.reference}` : "How the route is computed inside rooms";
}
