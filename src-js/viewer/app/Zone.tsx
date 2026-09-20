// One zone: a toolbar, a plan, and the states a plan can be in instead.
//
// **The hybrid, in one component.** A `<canvas>` under an `<svg>`: the canvas
// is the WebGL plan, the svg is the overlay that carries pointer handling, the
// selection and hover marks and (later) the areas footprints. The renderer is
// handed the svg as its overlay and keeps its viewBox in step with the
// projection — without that the overlay is in pixel space and every mark lands
// in the top-left corner.
//
// The renderer is created ONCE per zone and released on unmount. Both calls
// matter and they are not the same: `dispose()` clears the level, `destroy()`
// releases the WebGL context, and only the second is what the browser's cap
// counts. A zone closed without it eventually blanks a different zone.

import { useEffect, useMemo, useRef } from "react";

import { levelLabel, levelsForPayload, pickerOrder, resolveLevel, roomsOnLevel } from "../levels.js";
import { elementsOnStorey, type ElementOf } from "./layers.js";
import { onStoreysChanged } from "./poll.js";
import { fittedBounds, GlPlanRenderer, type PlanRendererInstance } from "./planRenderer.js";
import { setZoneLevel, type ZoneRow } from "./store.js";
import { useViewer } from "./useViewer.js";
import { wirePlanGestures } from "./gestures.js";
import { register, unregister, type ZoneHandle } from "./zoneRegistry.js";

/** Above this many rooms, show a busy panel while the plan is built.
 *
 *  A ROOM COUNT, not a duration, and that is forced rather than chosen: the
 *  build is synchronous, so a "show it if this takes 150 ms" timer can never
 *  fire — nothing runs during the block, including timers. Measured: House A
 *  (12) and the showcase levels (~50) are imperceptible, the 299-room synthetic
 *  is under ~100 ms, and `big-plate` at 5,046 takes ~1.3 s. Nothing real sits
 *  near the line. */
const BUSY_ROOM_THRESHOLD = 1000;

export function Zone({ zone }: { zone: ZoneRow }) {
  const { payload, status, layers, showRooms, showLabels, appearance, spacesModel, layersRevision } = useViewer();
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const svgRef = useRef<SVGSVGElement | null>(null);
  const handleRef = useRef<ZoneHandle | null>(null);
  const busyRef = useRef<HTMLDivElement | null>(null);
  /** The level this zone last PAINTED, so a repaint can tell a level switch
   *  (which refits) from a new payload on the same level (which must keep the
   *  reader's pan and zoom). */
  const paintedLevel = useRef<string | null>(null);

  // Create the renderer once. StrictMode runs this twice in development, so the
  // cleanup has to be complete — a leaked context is invisible until the
  // browser starts killing them.
  useEffect(() => {
    const canvas = canvasRef.current;
    const svg = svgRef.current;
    if (!canvas || !svg) return;
    const renderer: PlanRendererInstance = new GlPlanRenderer(canvas, { overlay: svg });
    const handle: ZoneHandle = {
      id: zone.id,
      renderer,
      view: { x: 0, y: 0, w: 100, h: 100 },
      fitted: { x: 0, y: 0, w: 100, h: 100 },
    };
    handleRef.current = handle;
    register(handle);
    const unwire = wirePlanGestures(svg, zone.id);
    return () => {
      unwire();
      unregister(zone.id);
      handleRef.current = null;
      paintedLevel.current = null;
      renderer.dispose();
      renderer.destroy();
    };
  }, [zone.id]);

  const levels = payload ? levelsForPayload(payload) : [];
  // The zone's level, resolved against what the payload actually has. Written
  // back to the store when it moves, so the picker and (later) the storey scope
  // of every element read agree with what was drawn.
  const levelId = resolveLevel(levels, zone.levelId);
  useEffect(() => {
    if (levelId !== zone.levelId) setZoneLevel(zone.id, levelId);
  }, [levelId, zone.id, zone.levelId]);

  // MEMOISED, and it is not a micro-optimisation: this array is the paint
  // effect's dependency, so recomputing it on every render would tear down and
  // rebuild the plan whenever any unrelated state moved -- toggling "Link
  // views" would re-triangulate 3,000 RHH rooms in every zone. With the memo a
  // repaint happens when the payload or the level does, which is the old
  // page's rule.
  const rooms = useMemo(() => (payload ? roomsOnLevel(payload, levelId) : []), [payload, levelId]);
  const levelName = levels.find((l) => l.id === levelId)?.name ?? levelId ?? "";

  // Paint. The dependency list is what decides a repaint, and `payload` changes
  // identity only when the poll saw a new revision — so a quiet system never
  // repaints, exactly as the old page's revision check achieves.
  useEffect(() => {
    const handle = handleRef.current;
    if (!handle) return;
    const renderer = handle.renderer;

    const draw = () => {
      // Clearing the drawing and clearing the pick index behind it are ONE
      // operation. Every early return below used to skip the reset in the old
      // page: switching to a level with no rooms left the previous level's
      // units live against nothing drawn, so a click resolved to a room that
      // was not on screen.
      renderer.dispose();
      if (!rooms.length) return;

      const refit = paintedLevel.current !== levelId;
      if (refit) {
        const fitted = fittedBounds(rooms);
        if (!fitted) return;
        handle.fitted = fitted;
        handle.view = { ...fitted };
        renderer.setView(handle.view);
      }
      paintedLevel.current = levelId;
      // Every layer is resolved against THIS zone's storey, by name and
      // elevation -- `elementsOnStorey`. A layer switched off contributes an
      // empty list rather than being left out, so the renderer clears what it
      // drew last time.
      const on = <E extends keyof ElementOf>(entity: E): readonly ElementOf[E][] =>
        layers[entity] ? elementsOnStorey(entity, levelId).kept : [];
      const build = () =>
        renderer.paint(rooms, handle.fitted, {
          showLabels,
          showRooms,
          appearance,
          doors: on("doors"),
          showDoors: layers.doors,
          windows: on("windows"),
          showWindows: layers.windows,
          ffe: on("ffe"),
          showFfe: layers.ffe,
          spaces: on("spaces"),
          showSpaces: layers.spaces,
          ceilings: on("ceilings"),
          showCeilings: layers.ceilings,
          floors: on("floors"),
          showFloors: layers.floors,
        });
      // Below the threshold the build runs INLINE: deferring every level by a
      // frame would make the common case worse to save a flash nobody sees.
      if (rooms.length < BUSY_ROOM_THRESHOLD || !busyRef.current) {
        build();
        return;
      }
      // Showing the panel only changes the DOM, and the browser will not PAINT
      // that until the task yields — so the panel goes up, the frame is handed
      // back, and the build runs on the next one.
      const busy = busyRef.current;
      busy.classList.remove("hidden");
      requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          try {
            build();
          } finally {
            busy.classList.add("hidden");
          }
        }),
      );
    };

    // The GL context is created asynchronously; painting before it exists draws
    // nothing and looks exactly like a broken payload.
    void renderer.ready.then(draw);
  }, [payload, levelId, rooms, layers, showRooms, showLabels, appearance, spacesModel, layersRevision]);

  // The element reads only hold the storeys that WERE on screen, so a level
  // switch (or this zone appearing at all) has to ask again. A no-op when the
  // storeys did not actually move -- see `onStoreysChanged`.
  useEffect(() => {
    void onStoreysChanged();
  }, [payload, levelId]);

  return (
    <div className="zone">
      <div className="zone-toolbar">
        <select
          className={`picker levelSelect${levels.length > 1 ? "" : " hidden"}`}
          value={levelId ?? ""}
          onChange={(e) => setZoneLevel(zone.id, e.target.value)}
        >
          {payload
            ? pickerOrder(levels).map((l) => (
                <option key={l.id} value={l.id}>
                  {levelLabel(payload, l)}
                </option>
              ))
            : null}
        </select>
        <span className="meta">
          {payload
            ? `${levelName || "—"} · ${rooms.length} room${rooms.length === 1 ? "" : "s"} · v${payload.schema_version}`
            : status}
        </span>
      </div>
      <div className="zone-canvas">
        <canvas className="plan-gl" ref={canvasRef} />
        <svg className="plan" ref={svgRef} xmlns="http://www.w3.org/2000/svg" />
        <ZoneEmpty payload={!!payload} rooms={rooms.length} levels={levels.length} levelName={levelName} />
        <div className="plan-busy hidden" ref={busyRef}>
          <span>Drawing plan…</span>
        </div>
      </div>
    </div>
  );
}

/** The three states a zone shows instead of a plan, in the old page's words.
 *
 *  They are different answers and a reader acts on each differently: nothing
 *  has ever been pushed, the pickers exclude everything, or this one level is
 *  empty. The level is NAMED — with three zones open, which one is empty is the
 *  question, and this panel is the only place that answers it. */
function ZoneEmpty({
  payload,
  rooms,
  levels,
  levelName,
}: {
  payload: boolean;
  rooms: number;
  levels: number;
  levelName: string;
}) {
  if (rooms > 0) return null;
  if (!payload) {
    return (
      <div className="empty">
        <strong>No rooms received yet</strong>
        <span>
          POST room JSON to <code>/rooms</code>, then this updates.
        </span>
      </div>
    );
  }
  if (!levels) {
    return (
      <div className="empty">
        <strong>No rooms in this scope</strong>
        <span>The data has no levels to draw — check the project, building and milestone pickers.</span>
      </div>
    );
  }
  return (
    <div className="empty">
      <strong>{levelName || "This level"} has no rooms</strong>
      <span>The model arrived; this level is empty. Pick another level above.</span>
    </div>
  );
}
