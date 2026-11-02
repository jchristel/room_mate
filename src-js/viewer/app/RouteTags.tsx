// The route tags on the plan: each hop's length, and in brackets the walk so far.
//
// SVG text in the overlay, drawn over the route lines. The tags themselves are
// `routeTagsOnLevel`'s (shared with the SVG export); this only places them. Sized in
// the plan's own unit from the level's fitted frame, so they scale with the plan as
// the room labels do and a tag stays beside the line it describes at every zoom.

import { routeTagFont, type RouteTag } from "../route.js";
import { handleOf } from "./zoneRegistry.js";

export function RouteTags({ zoneId, tags }: { zoneId: string; tags: readonly RouteTag[] }) {
  if (tags.length === 0) return null;
  const fitted = handleOf(zoneId)?.fitted;
  const font = fitted ? routeTagFont(fitted) : 1;
  return (
    <g className="route-tags" fontSize={font}>
      {tags.map((t, i) => (
        <text key={i} className="route-tag" x={t.at.x} y={-t.at.y} data-route={t.routeId} style={{ strokeWidth: font * 0.22 }}>
          {t.text}
        </text>
      ))}
    </g>
  );
}
