// One zone's slice of the route: the polyline on its level, and a mark on each
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

import { markerPoint, polylinePoints, segmentsOnLevel } from "../route.js";
import { handleOf } from "./zoneRegistry.js";
import { useViewer } from "./useViewer.js";

export function RouteOverlay({ zoneId, levelId }: { zoneId: string; levelId: string | null }) {
  const { route, payload } = useViewer();
  if (!route || levelId === null) return null;

  const path = route.result.state === "done" ? route.result.path : null;
  const segments = segmentsOnLevel(path, levelId);
  const fitted = handleOf(zoneId)?.fitted;
  const r = fitted ? Math.max(fitted.w, fitted.h) * 0.011 : 1;

  const mark = (roomId: string | null, which: "start" | "end") => {
    const room = roomId ? payload?.rooms?.find((x) => x.id === roomId) : null;
    if (!room || room.level_id !== levelId) return null;
    const at = markerPoint(room);
    return at ? <circle key={which} className={`route-mark ${which}`} cx={at.x} cy={-at.y} r={r} /> : null;
  };

  return (
    <g className="route-overlay">
      {segments.map((s, i) =>
        s.points.length >= 2 ? <polyline key={i} className="route-line" points={polylinePoints(s.points)} /> : null,
      )}
      {mark(route.start, "start")}
      {mark(route.end, "end")}
    </g>
  );
}
