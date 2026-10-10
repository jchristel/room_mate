// Every shown saved route, on one zone's level, each in its own colour.
//
// SVG over the plan like the draft route, and for the same overlay rule: a stroke
// with no fill adds pixels without removing any. Each line sits on a paper-coloured
// halo, so where two routes share a corridor the one underneath still reads as a
// line rather than disappearing into the one on top. Drawn BELOW the draft route,
// which is the thing being asked about.

import { polylinePoints, routeDrawingsOnLevel } from "../route.js";
import { handleOf } from "./zoneRegistry.js";
import { useViewer } from "./useViewer.js";

export function SavedRoutesOverlay({ zoneId, levelId }: { zoneId: string; levelId: string | null }) {
  const { scope, connections, savedRoutes } = useViewer();
  if (levelId === null || savedRoutes.shown.size === 0) return null;
  const doc = connections.projectId === scope.projectId ? connections.doc : null;
  if (!doc) return null;

  const fitted = handleOf(zoneId)?.fitted;
  const r = fitted ? Math.max(fitted.w, fitted.h) * 0.004 : 0.5;

  const drawings = routeDrawingsOnLevel(
    doc.routes
      .filter((route) => savedRoutes.shown.has(route.id))
      .map((route) => {
        const result = savedRoutes.results[route.id];
        return { id: route.id, name: route.name, colour: route.colour, path: result?.state === "done" ? result.path : null };
      }),
    levelId,
  );

  return (
    <g className="saved-routes-overlay">
      {drawings.map((d) => (
        <g key={d.id} className="saved-route" data-route={d.id}>
          {d.lines.map((points, i) => (
            <g key={i}>
              <polyline className="saved-route-halo" points={polylinePoints(points)} />
              <polyline className="saved-route-line" style={{ stroke: d.colour }} points={polylinePoints(points)} />
            </g>
          ))}
          {d.start ? <circle className="saved-route-end" style={{ fill: d.colour }} cx={d.start.x} cy={-d.start.y} r={r} /> : null}
          {d.end ? (
            <rect
              className="saved-route-end"
              style={{ fill: d.colour }}
              x={d.end.x - r}
              y={-d.end.y - r}
              width={r * 2}
              height={r * 2}
            />
          ) : null}
        </g>
      ))}
    </g>
  );
}
