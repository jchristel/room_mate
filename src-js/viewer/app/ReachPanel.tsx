// The reach tool's bar: the start rooms, the walk, the grid, and the way to run it.
//
// Mounted inside a zone, beside the route bar, and renders nothing while the tool
// is off. Like the route bar it owns its read, because the read exists only while
// the tool does -- but unlike the route's it is NOT automatic: a reach is a
// whole-storey computation, and re-running it on every keystroke in the walk field
// would ask the server a question the reader has not finished typing. Any change to
// what is asked clears the answer (so the map never shows a stale one) and "Measure"
// asks again.
//
// Clicking a room adds it as a start point and clicking it again removes it, through
// the same choke point as the route tool (`store.select`), so a plan click, a grid
// row and a search chip all work without knowing this tool exists.

import { useEffect, useMemo, useState } from "react";

import { bodyAfterSaveReach, connectionsUrl, whyReachNotSavable } from "../connections.js";
import { describeReach, draftOf, problem, scopeKey, type ReachMethod, type ReachState } from "../reach.js";
import { saveConnections } from "./connectionsApi.js";
import { abortReach, currentReach, measureReach } from "./reachMeasure.js";
import { patchZoneReach, select, setZoneReachMode, type ZoneRow } from "./store.js";
import { useViewer } from "./useViewer.js";

export function ReachPanel({ zone }: { zone: ZoneRow }) {
  const { scope, payload, toolFocus, connections } = useViewer();
  const reach = zone.reach;
  const zoneId = zone.id;
  const active = reach !== null;
  const [name, setName] = useState("");
  const [notice, setNotice] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const key = scopeKey(scope);

  /** The zone's CURRENT reach state: an answer lands after the reader may have
   *  clicked again, so it is patched into what is there now and not what the
   *  closure saw. */
  const current = (): ReachState | null => currentReach(zoneId);
  const edit = (patch: Partial<ReachState>, keepAnswer = false) => {
    const now = current();
    if (!now) return;
    abortReach(zoneId);
    setNotice(null);
    patchZoneReach(zoneId, { ...now, ...patch, ...(keepAnswer ? {} : { result: { state: "idle" } }) });
  };

  // Leaving a scope forgets the start rooms: a room id means a room in THAT project.
  useEffect(() => {
    if (!active) return;
    abortReach(zoneId);
    const now = current();
    if (now) patchZoneReach(zoneId, { ...now, sources: [], result: { state: "idle" } });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, key]);

  useEffect(() => () => abortReach(zoneId), [zoneId]);

  // Escape leaves the tool, like every other transient thing on this page.
  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && toolFocus === zoneId) setZoneReachMode(zoneId, false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active, toolFocus, zoneId]);

  const rooms = useMemo(() => new Map((payload?.rooms ?? []).map((r) => [r.id, r])), [payload]);

  if (!reach) return null;
  const why = problem(reach);

  const doc = connections.projectId === scope.projectId ? connections.doc : null;
  const url = connectionsUrl(scope);
  const notSavable = whyReachNotSavable(name, reach.sources.length) ?? (why ? why : null);
  const save = async () => {
    const now = current();
    const draft = now ? draftOf(now, name.trim()) : null;
    if (!doc || !url || !draft) return;
    setSaving(true);
    const refused = await saveConnections(url, scope.projectId, doc, bodyAfterSaveReach(doc, draft));
    setSaving(false);
    setNotice(refused ?? `Saved "${draft.name}".`);
    if (!refused) setName("");
  };

  const running = reach.result.state === "loading";
  return (
    <div className="routeBar reachBar">
      <strong>Reach</strong>
      {reach.sources.length === 0 ? <span className="meta">click a room to start from</span> : null}
      {reach.sources.map((s) => {
        const room = rooms.get(s.roomId);
        return (
          <button
            key={s.roomId}
            className="chip"
            title={`${s.roomId}${s.at ? " (point set)" : ""} -- click to remove`}
            onClick={() => select("room", s.roomId, zoneId)}
          >
            {room?.name || s.roomId} ×
          </button>
        );
      })}
      <input
        className="zoneName widthInput"
        inputMode="decimal"
        value={reach.limit}
        onChange={(e) => edit({ limit: e.target.value })}
        aria-label="Walk length in metres"
        title="How far a walk reaches, in metres (30 for a 30 m hose reel)"
        placeholder="walk m"
      />
      <input
        className="zoneName widthInput"
        inputMode="decimal"
        value={reach.spacing}
        onChange={(e) => edit({ spacing: e.target.value })}
        aria-label="Grid spacing in metres"
        title="Grid spacing in metres, 0.1 to 10. Finer is slower; a grid over 600,000 cells is refused."
        placeholder="grid m"
      />
      <select
        className="picker"
        value={reach.method}
        onChange={(e) => edit({ method: e.target.value as ReachMethod })}
        title="exact: the exact walk to each cell. grid: Dijkstra over the grid cells, the lattice approximation (slightly longer, depends on the grid)."
      >
        <option value="exact">exact</option>
        <option value="grid">grid (Dijkstra)</option>
      </select>
      <label title="Treat the closed walls inside open zones as open: a cut says where a person routes, and a coverage question may not be bound by it.">
        <input type="checkbox" checked={reach.ignoreCuts} onChange={(e) => edit({ ignoreCuts: e.target.checked })} /> ignore cuts
      </label>
      <label title="How opaque the heat map is. Lower shows more of the plan beneath it.">
        opacity{" "}
        <input
          type="range"
          min={10}
          max={100}
          value={reach.opacity}
          onChange={(e) => edit({ opacity: Number(e.target.value) }, true)}
          aria-label="Heat map opacity"
        />
      </label>
      <button className="chip on" disabled={why !== null || running} title={why ?? "Measure the walk from these rooms"} onClick={() => void measureReach(zoneId, scope)}>
        {running ? "Measuring…" : "Measure"}
      </button>
      <input
        className="zoneName"
        value={name}
        onChange={(e) => setName(e.target.value)}
        aria-label="Name to save this reach under"
        placeholder="name to save"
      />
      <button
        className="chip"
        disabled={notSavable !== null || !doc || saving}
        title={notSavable ?? (doc ? "Save this reach for the project (the request, not the map)" : "Reading the saved reaches…")}
        onClick={() => void save()}
      >
        {saving ? "Saving…" : "Save reach"}
      </button>
      {notice ? <span className="meta">{notice}</span> : null}
      <span className="routeResult">{describeReach(reach.result)}</span>
      {reach.result.state === "done" && reach.result.body.note ? <span className="meta">{reach.result.body.note}</span> : null}
      <button className="chip" title="Leave the reach tool (Esc)" onClick={() => setZoneReachMode(zoneId, false)}>
        Close
      </button>
    </div>
  );
}
