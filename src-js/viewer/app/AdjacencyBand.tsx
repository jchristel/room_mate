// Band 1's adjacency block: pick a room, see what it shares a wall with.
//
// **`graph.js` is WRAPPED, not ported.** It is the page's second renderer — a
// canvas, where the plan is WebGL and the overlays are SVG — and rewriting it
// is a separate project with its own history (`HANDOVER-adjacency.md`). What
// this component owns is its lifecycle: create the graph once the panel has a
// box to measure, feed it data, focus and tier, and tear it down.
//
// A hidden canvas measures zero, so the graph is created when the block OPENS
// rather than on mount — the old page's `ensureGraph` rule, and the reason the
// block is on-demand rather than part of the poll.

import { useEffect, useRef, useState } from "react";

import { areaKey, tierLabel } from "../areas.js";
import { fetchJson } from "./api.js";
import { getState, select } from "./store.js";
import { useViewer } from "./useViewer.js";

/** Feet to millimetres. The wire is feet; the control is mm, because that is
 *  what a UK/EU hospital job is drawn in. */
const FT_TO_MM = 304.8;

/** What `createRoomGraph` returns, as far as this component uses it. */
interface RoomGraph {
  setData(data: unknown): void;
  setFocus(focus: { kind: string; id: string; depth?: number } | null): void;
  setDepth(d: number): void;
  setTierDepth(d: number): void;
  tierNames(): string[];
  shownCount(): number;
  focusDegree(): number;
  groupDepth(): number | null;
  resize?(): void;
}

type GraphFactory = (canvas: HTMLCanvasElement, opts: { onSelect: (id: string, kind: string) => void }) => RoomGraph;

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
  const [serverWallMm, setServerWallMm] = useState<number | null>(null);
  const [meta, setMeta] = useState("no room or area selected");

  // Create on OPEN, destroy on close: a hidden canvas measures zero, and a
  // graph laid out against a zero box draws nothing and stays that way.
  useEffect(() => {
    if (!open) return;
    const canvas = canvasRef.current;
    const factory = (globalThis as unknown as { createRoomGraph?: GraphFactory }).createRoomGraph;
    if (!canvas || !factory) return;
    const graph = factory(canvas, {
      // A click on a node sets the PAGE selection, which re-centres the graph
      // and marks the room on every plan showing it. The graph does not own the
      // selection — it reads and writes it, which is what makes plan → graph
      // and graph → plan the same one-way flow twice rather than two views
      // trying to stay in sync.
      onSelect: (id, kind) => select(kind === "area" ? "area" : "room", id, null),
    });
    graphRef.current = graph;
    // MEASURE AFTER LAYOUT. `createRoomGraph` sizes its backing store from the
    // canvas's box, and at this point in the effect the block has been added
    // but not laid out -- the canvas still reports the HTML default of
    // 300x150, so the graph draws into a box that is not the one on screen.
    // A frame later it is real, and a ResizeObserver keeps it real through
    // region drags and window resizes.
    requestAnimationFrame(() => graph.resize?.());
    const observer = new ResizeObserver(() => graph.resize?.());
    observer.observe(canvas);
    // No teardown call: `createRoomGraph` returns no destroy, and the canvas
    // it drew on is unmounted with this block — a second open builds a fresh
    // graph against a fresh canvas.
    return () => {
      observer.disconnect();
      graphRef.current = null;
    };
  }, [open]);

  // Fetch when the block is open and the scope or tolerance moves.
  useEffect(() => {
    const projectId = scope.projectId;
    if (!open || !projectId) return;
    let cancelled = false;
    void (async () => {
      const params = new URLSearchParams();
      if (scope.building) params.set("building", scope.building);
      if (scope.milestone) params.set("milestone", scope.milestone);
      if (wallMm != null) params.set("wall_max", String(wallMm / FT_TO_MM));
      const qs = params.toString();
      try {
        const data = await fetchJson<{ wall_max?: number }>(
          `/projects/${encodeURIComponent(projectId)}/adjacency${qs ? `?${qs}` : ""}`,
        );
        if (cancelled) return;
        // The tolerance the server says it APPLIED, reflected without taking
        // ownership: `wallMm` stays null, so the next request still omits it
        // and a change to the project's `[areas] max_wall_thickness` still
        // reaches the viewer.
        if (typeof data.wall_max === "number" && Number.isFinite(data.wall_max)) {
          setServerWallMm(Math.round(data.wall_max * FT_TO_MM));
        }
        graphRef.current?.setData(data);
        setTiers(graphRef.current?.tierNames() ?? []);
      } catch {
        // A 400 is a caller fault (an out-of-range tolerance) and a 204 is an
        // empty store; neither is worth a modal. Clear and let the canvas say so.
        if (!cancelled) graphRef.current?.setData(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, scope.projectId, scope.building, scope.milestone, wallMm, payload]);

  // Focus, depth and tier follow their controls and the page selection.
  useEffect(() => {
    const graph = graphRef.current;
    if (!graph) return;
    graph.setDepth(depth === "all" ? Infinity : Number(depth));
    graph.setTierDepth(tier);
    graph.setFocus(focusOf(graph));
    setMeta(metaLine(graph));
  }, [depth, tier, selection, areas, open]);

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
function focusOf(graph: RoomGraph): { kind: string; id: string; depth?: number } | null {
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
