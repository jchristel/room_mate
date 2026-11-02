// Band 1's adjacency block: pick a room, see what it shares a wall with.
//
// React owns the controls, the fetch and the selection; `RoomGraph` owns the
// canvas (`../adjacency/`, where the layout's rules are written down and
// tested). What this component adds is the lifecycle: create the graph once the
// panel has a box to measure, feed it data, focus and tier, and destroy it.
//
// A hidden canvas measures zero, so the graph is created when the block OPENS
// rather than on mount — and that is also why the block is on-demand rather
// than part of the poll.

import { useEffect, useRef, useState } from "react";

import { RoomGraph, type GraphFocus } from "../adjacency/canvas.js";
import { tierNames, type AdjacencyPayload } from "../adjacency/view.js";
import { areaKey, tierLabel } from "../areas.js";
import { fetchJson } from "./api.js";
import { getState, select } from "./store.js";
import { useViewer } from "./useViewer.js";

/** Feet to millimetres. The wire is feet; the control is mm, because that is
 *  what a UK/EU hospital job is drawn in. */
const FT_TO_MM = 304.8;

/** How long the wall-gap slider must rest before it asks the server. `/adjacency`
 *  re-derives every shared wall on each call — the expensive read the endpoint is
 *  on-demand for — and a drag across the range is ninety steps. */
const WALL_SETTLE_MS = 250;

export function AdjacencyBand({ open, onToggle }: { open: boolean; onToggle: () => void }) {
  const { payload, selection, areas, scope } = useViewer();
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const graphRef = useRef<RoomGraph | null>(null);
  const [depth, setDepth] = useState("2");
  const [tier, setTier] = useState(0);
  const [tiers, setTiers] = useState<string[]>([]);
  /** The reader's own tolerance, or null while the SERVER's derived default
   *  applies. Assigning a number is what takes ownership: from then on the
   *  request sends it explicitly and the project's declared thickness no
   *  longer reaches the graph until the scope changes. */
  const [wallMm, setWallMm] = useState<number | null>(null);
  /** `wallMm` once the slider has rested — what the request actually sends,
   *  while the readout follows the thumb. */
  const [wallQuery, setWallQuery] = useState<number | null>(null);
  const [serverWallMm, setServerWallMm] = useState<number | null>(null);
  /** Bumped when data lands, so the meta line is recomputed from the graph that
   *  now holds it rather than the empty one the focus was first set on. */
  const [dataVersion, setDataVersion] = useState(0);
  const [meta, setMeta] = useState("no room or area selected");

  // Create on OPEN, destroy on close: a hidden canvas measures zero, and a
  // graph laid out against a zero box draws nothing and stays that way.
  useEffect(() => {
    if (!open) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    // A click on a node sets the PAGE selection, which re-centres the graph and
    // marks the room on every plan showing it. The graph does not own the
    // selection — it reads and writes it, which is what makes plan → graph and
    // graph → plan the same one-way flow twice rather than two views trying to
    // stay in sync.
    const graph = new RoomGraph(canvas, (id, kind) => select(kind, id, null));
    graphRef.current = graph;
    // MEASURE AFTER LAYOUT. At this point in the effect the block has been
    // added but not laid out — the canvas still reports the HTML default of
    // 300x150. A frame later it is real, and a ResizeObserver keeps it real
    // through region drags and window resizes.
    const frame = requestAnimationFrame(() => graph.resize());
    const observer = new ResizeObserver(() => graph.resize());
    observer.observe(canvas);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      graph.destroy();
      graphRef.current = null;
    };
  }, [open]);

  useEffect(() => {
    if (wallMm === wallQuery) return;
    const timer = setTimeout(() => setWallQuery(wallMm), WALL_SETTLE_MS);
    return () => clearTimeout(timer);
  }, [wallMm, wallQuery]);

  // Fetch when the block is open and the scope or tolerance moves. A superseded
  // request is ABORTED, not merely ignored: ignoring it still leaves the browser
  // holding a connection for an answer nobody will read.
  useEffect(() => {
    const projectId = scope.projectId;
    if (!open || !projectId) return;
    const abort = new AbortController();
    void (async () => {
      const params = new URLSearchParams();
      if (scope.building) params.set("building", scope.building);
      if (scope.milestone) params.set("milestone", scope.milestone);
      if (wallQuery != null) params.set("wall_max", String(wallQuery / FT_TO_MM));
      const qs = params.toString();
      let data: AdjacencyPayload | null = null;
      try {
        data = await fetchJson<AdjacencyPayload>(
          `/projects/${encodeURIComponent(projectId)}/adjacency${qs ? `?${qs}` : ""}`,
          abort.signal,
        );
      } catch {
        // A 400 is a caller fault (an out-of-range tolerance) and a 204 is an
        // empty store; neither is worth a modal. Clear and let the canvas say so.
      }
      if (abort.signal.aborted) return;
      // The tolerance the server says it APPLIED, reflected without taking
      // ownership: `wallMm` stays null, so the next request still omits it and a
      // change to the project's `[areas] max_wall_thickness` still reaches the
      // viewer.
      if (typeof data?.wall_max === "number" && Number.isFinite(data.wall_max)) {
        setServerWallMm(Math.round(data.wall_max * FT_TO_MM));
      }
      graphRef.current?.setData(data);
      setTiers(tierNames(data));
      setDataVersion((v) => v + 1);
    })();
    return () => abort.abort();
  }, [open, scope.projectId, scope.building, scope.milestone, wallQuery, payload]);

  // Focus, depth and tier follow their controls and the page selection.
  useEffect(() => {
    const graph = graphRef.current;
    if (!graph) return;
    graph.setDepth(depth === "all" ? Infinity : Number(depth));
    graph.setTierDepth(tier);
    graph.setFocus(focusOf(graph));
    setMeta(metaLine(graph));
  }, [depth, tier, selection, areas, open, dataVersion]);

  const shownWall = wallMm ?? serverWallMm ?? 0;

  return (
    <div className={`result-band${open ? " open" : ""}`} id="adjBand">
      <button className="band-head" id="adjHead" onClick={onToggle}>
        {open ? "▾" : "▸"} Adjacency
      </button>
      {open ? (
        <div className="band-body" id="adjBody">
          <div id="adjBar">
            <label>
              Depth{" "}
              {/* A client-side ring cap over data already held, so the list can
                  run as deep as the graph does. 2 stays the default — a whole
                  level is a hairball — but "All" is a legitimate question. */}
              <select className="picker" value={depth} onChange={(e) => setDepth(e.target.value)}>
                {["1", "2", "3", "4", "5", "6", "8"].map((d) => (
                  <option key={d} value={d}>
                    {d}
                  </option>
                ))}
                <option value="all">All</option>
              </select>
            </label>
            {tiers.length ? (
              <label>
                Colour{" "}
                <select className="picker" value={tier} onChange={(e) => setTier(Number(e.target.value))}>
                  {tiers.map((name, i) => (
                    <option key={name} value={i}>
                      {name}
                    </option>
                  ))}
                </select>
              </label>
            ) : null}
            <label
              id="adjWallLabel"
              title="Largest gap still counted as a shared wall. Opens at the project's declared value — 0 where rooms are drawn to wall centrelines, the wall thickness where they are drawn to finish faces. Drag only to probe a different tolerance."
            >
              Wall gap
              <input
                type="range"
                min={0}
                max={900}
                step={10}
                value={Math.min(Math.max(shownWall, 0), 900)}
                onChange={(e) => setWallMm(Number(e.target.value))}
              />
              {/* The readout carries the server's exact figure while the thumb
                  only approximates it, which is the honest way round: the input
                  lands on step multiples (1.5 ft is 457.2 mm) and a project may
                  declare a thickness past 900 mm. */}
              <span className="count">
                {shownWall} mm{wallMm == null && serverWallMm != null ? " · project default" : ""}
              </span>
            </label>
            <span className="count" id="adjMeta">
              {meta}
            </span>
          </div>
          <canvas id="adjCanvas" ref={canvasRef} />
        </div>
      ) : null}
    </div>
  );
}

/**
 * The graph's focus, from the one page selection.
 *
 * A room is itself a node; an AREA is a group of nodes, so the graph also needs
 * the tier depth to aggregate to. That depth comes from the areas result when
 * it is loaded, and otherwise from the tier the graph is already at — a click
 * on a graph node means the group is one the graph itself derived, and those
 * can outrun `/areas`, which drops hierarchy-excluded rooms.
 */
function focusOf(graph: RoomGraph): GraphFocus | null {
  const { selection, areas } = getState();
  if (!selection) return null;
  if (selection.kind === "room") return { kind: "room", id: selection.id };
  if (selection.kind !== "area") return null;
  const group = areas?.groups.find((g) => areaKey(g) === selection.id);
  const depth = group ? group.path.length - 1 : graph.groupDepth();
  if (depth == null) return null;
  return { kind: "area", id: selection.id, depth };
}

/** What the block says about the focus. Both counts come from the GRAPH, not
 *  from the payload: once the view is aggregated a degree counts GROUPS, which
 *  the raw room edges cannot answer. */
function metaLine(graph: RoomGraph): string {
  const { selection, areas, payload } = getState();
  const focus = focusOf(graph);
  if (!focus || !selection) return "no room or area selected";
  const name =
    focus.kind === "area"
      ? // The FULL tier path, not the leaf: two departments under different
        // parents can carry the same leaf label, and this line is the one place
        // with room to tell them apart.
        (areas?.groups.find((g) => areaKey(g) === focus.id)?.path.map(tierLabel).join(" / ") ?? focus.id)
      : (payload?.rooms?.find((r) => r.id === focus.id)?.name ?? focus.id);
  const from = selection.zoneId ? ` · from ${selection.zoneId}` : "";
  return `${name} · ${graph.focusDegree()} adjacent · ${graph.shownCount()} shown${from}`;
}
