// One zone's own route: the polyline on its level, and a mark on each
// endpoint that is on it.
//
// SVG over the plan, like the area footprints, and for the rule in
// STRATEGY-BROWSER ("what belongs on the overlay"): a stroke with no fill adds
// pixels without removing any, so it composites harmlessly over the rooms and
// their labels. It draws in world coordinates, Y flipped, inside the viewBox the
// renderer keeps in step with the projection, so it pans and zooms with the plan
// for free and needs no handler of its own.
//
// Marks are placed from the rooms the page already holds rather than from the
// route result, so a start with no end yet still shows where it is.
//
// **The marks can be dragged** to adjust where the route begins or ends. A mark owns
// its own pointer (it captures it and stops the event at itself), because the plan's
// gestures sit on the same `<svg>` and would otherwise take the press for a pan or a
// pick. The mark follows the pointer while it is held, and the route is asked again
// ONCE, on release: re-routing on every move would be a request per pixel. A point
// dropped outside its room is moved onto the room's outline by the server and the
// answer says so, so the drag is held to its own room and does not change which rooms
// the route joins.

import { useEffect, useRef, useState } from "react";

import {
  markerPoint,
  MM_PER_FT,
  planPointOf,
  polylinePoints,
  segmentsOnLevel,
  type PlanPoint,
  type RouteState,
} from "../route.js";
import { patchZoneRoute } from "./store.js";
import { handleOf } from "./zoneRegistry.js";
import { useViewer } from "./useViewer.js";

type Which = "start" | "end";

export function RouteOverlay({
  zoneId,
  levelId,
  route,
}: {
  zoneId: string;
  levelId: string | null;
  route: RouteState | null;
}) {
  const { payload } = useViewer();
  // Where a mark is while it is held, so it follows the pointer between the press and
  // the release. Null otherwise.
  const [held, setHeld] = useState<{ which: Which; at: PlanPoint } | null>(null);
  if (!route || levelId === null) return null;

  const path = route.result.state === "done" ? route.result.path : null;
  const segments = segmentsOnLevel(path, levelId);
  const fitted = handleOf(zoneId)?.fitted;
  const r = fitted ? Math.max(fitted.w, fitted.h) * 0.0055 : 0.5;
  // The width the server applied, not the one typed: a half-typed number draws nothing.
  const askedMm = route.result.state === "done" ? route.result.clearance?.asked_mm : undefined;
  const bandFt = askedMm && askedMm > 0 ? askedMm / MM_PER_FT : null;

  const place = (roomId: string | null, which: Which) => {
    const room = roomId ? payload?.rooms?.find((x) => x.id === roomId) : null;
    if (!room || room.level_id !== levelId) return null;
    if (held?.which === which) return { room, at: held.at };
    // The spot the route really began or ended at (the server answers with it, moved
    // onto the room if it lay outside), else the spot clicked, else the room's middle.
    const asked = which === "start" ? route.startAt : route.endAt;
    const answered = path ? (which === "start" ? path.start : path.end) : undefined;
    const at = answered ?? asked ?? markerPoint(room);
    return at ? { room, at } : null;
  };

  const mark = (roomId: string | null, which: Which) => {
    const placed = place(roomId, which);
    if (!placed) return null;
    return (
      <DraggableMark
        key={which}
        zoneId={zoneId}
        which={which}
        at={placed.at}
        r={r}
        onHold={(at) => setHeld({ which, at })}
        onDrop={(at) => {
          setHeld(null);
          if (at) patchZoneRoute(zoneId, which === "start" ? { startAt: at } : { endAt: at });
        }}
      />
    );
  };

  return (
    <g className="route-overlay">
      {/* The object's own width, at true size, under the line: a corridor narrower than
          it shows as a band wider than the rooms around it. Absent for no width. */}
      {bandFt !== null
        ? segments.map((s, i) =>
            s.points.length >= 2 ? (
              <polyline key={`band${i}`} className="route-band" style={{ strokeWidth: bandFt }} points={polylinePoints(s.points)} />
            ) : null,
          )
        : null}
      {segments.map((s, i) =>
        s.points.length >= 2 ? <polyline key={i} className="route-line" points={polylinePoints(s.points)} /> : null,
      )}
      {mark(route.start, "start")}
      {mark(route.end, "end")}
    </g>
  );
}

/** A route's start or end: the mark you see, and a wider invisible one you grab. */
function DraggableMark({
  zoneId,
  which,
  at,
  r,
  onHold,
  onDrop,
}: {
  zoneId: string;
  which: Which;
  at: PlanPoint;
  r: number;
  onHold: (at: PlanPoint) => void;
  onDrop: (at: PlanPoint | null) => void;
}) {
  const grab = useRef<SVGLineElement>(null);
  // Read through refs so the listeners are attached once and still see the latest.
  const callbacks = useRef({ onHold, onDrop });
  callbacks.current = { onHold, onDrop };

  useEffect(() => {
    const el = grab.current;
    if (!el) return;
    let last: PlanPoint | null = null;
    const where = (e: PointerEvent) => planPointOf(handleOf(zoneId)?.renderer.toWorld(e.clientX, e.clientY));
    const move = (e: PointerEvent) => {
      const p = where(e);
      if (!p) return;
      last = p;
      callbacks.current.onHold(p);
    };
    const end = (e: PointerEvent) => {
      el.releasePointerCapture?.(e.pointerId);
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", end);
      el.removeEventListener("pointercancel", cancel);
      callbacks.current.onDrop(last);
    };
    const cancel = (e: PointerEvent) => {
      last = null;
      end(e);
    };
    const down = (e: PointerEvent) => {
      // The plan's gestures are on the same svg: stop the press here, or it is a pan.
      e.stopPropagation();
      e.preventDefault();
      last = null;
      el.setPointerCapture?.(e.pointerId);
      el.addEventListener("pointermove", move);
      el.addEventListener("pointerup", end);
      el.addEventListener("pointercancel", cancel);
    };
    el.addEventListener("pointerdown", down);
    return () => {
      el.removeEventListener("pointerdown", down);
    };
  }, [zoneId]);

  return (
    <>
      {which === "start" ? (
        <circle className="route-mark start" cx={at.x} cy={-at.y} r={r} />
      ) : (
        <circle className="route-mark end" cx={at.x} cy={-at.y} r={r} />
      )}
      {/* A very short line with a wide, invisible, non-scaling stroke and round caps:
          a dot-shaped grab target that stays the same size on screen however far the
          plan is zoomed. (A circle of no radius has no stroke to hit.) */}
      <line
        ref={grab}
        className="route-mark-grab"
        x1={at.x}
        y1={-at.y}
        x2={at.x + r * 0.01}
        y2={-at.y}
        data-grab={which}
        aria-label={`Drag to move the route's ${which}`}
      />
    </>
  );
}
