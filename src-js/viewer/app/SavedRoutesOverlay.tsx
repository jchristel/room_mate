// Every shown saved route, on one zone's level, each in its own colour.
//
// SVG over the plan like the draft route, and for the same overlay rule: a stroke
// with no fill adds pixels without removing any. Each line sits on a paper-coloured
// halo, so where two routes share a corridor the one underneath still reads as a
// line rather than disappearing into the one on top. Drawn BELOW the draft route,
// which is the thing being asked about.

import { polylinePoints, segmentsOnLevel } from "../route.js";
import { handleOf } from "./zoneRegistry.js";
import { useViewer } from "./useViewer.js";

export function SavedRoutesOverlay({ zoneId, levelId }: { zoneId: string; levelId: string | null }) {
  const { scope, connections, savedRoutes } = useViewer();
  if (levelId === null || savedRoutes.shown.size === 0) return null;
  const doc = connections.projectId === scope.projectId ? connections.doc : null;
  if (!doc) return null;

  const fitted = handleOf(zoneId)?.fitted;
  const r = fitted ? Math.max(fitted.w, fitted.h) * 0.004 : 0.5;

  return (
    <g className="saved-routes-overlay">
      {doc.routes
        .filter((route) => savedRoutes.shown.has(route.id))
        .map((route) => {
          const result = savedRoutes.results[route.id];
          const path = result?.state === "done" ? result.path : null;
          const segments = segmentsOnLevel(path, levelId);
          if (segments.length === 0 || !path) return null;
          // The ends are where the first and last segments begin and end, drawn only on
          // the level they are on.
          const first = path.segments[0];
          const last = path.segments[path.segments.length - 1];
          const startAt = first && first.level_id === levelId ? first.points[0] : null;
          const endAt = last && last.level_id === levelId ? last.points[last.points.length - 1] : null;
          return (
            <g key={route.id} className="saved-route" data-route={route.id}>
              {segments.map((s, i) =>
                s.points.length >= 2 ? (
                  <g key={i}>
                    <polyline className="saved-route-halo" points={polylinePoints(s.points)} />
                    <polyline className="saved-route-line" style={{ stroke: route.colour }} points={polylinePoints(s.points)} />
                  </g>
                ) : null,
              )}
              {startAt ? <circle className="saved-route-end" style={{ fill: route.colour }} cx={startAt.x} cy={-startAt.y} r={r} /> : null}
              {endAt ? (
                <rect
                  className="saved-route-end"
                  style={{ fill: route.colour }}
                  x={endAt.x - r}
                  y={-endAt.y - r}
                  width={r * 2}
                  height={r * 2}
                />
              ) : null}
            </g>
          );
        })}
    </g>
  );
}
