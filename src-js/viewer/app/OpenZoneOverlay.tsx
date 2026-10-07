// The rooms of the open zone being drawn, filled on the plan.
//
// SVG over the plan like the route and the footprints, and for the same rule in
// STRATEGY-BROWSER: a translucent fill adds colour without removing the room
// outline or its label beneath it. World coordinates, Y flipped, inside the
// viewBox the renderer keeps in step with the projection.

import type { ZoneEdit } from "../connections.js";
import { useViewer } from "./useViewer.js";

export function OpenZoneOverlay({ levelId, edit }: { levelId: string | null; edit: ZoneEdit | null }) {
  const { payload } = useViewer();
  if (!edit || levelId === null || edit.members.length === 0) return null;

  const members = new Set(edit.members.map((m) => m.room_id));
  const drawn = (payload?.rooms ?? []).filter((r) => r.level_id === levelId && members.has(r.id));
  return (
    <g className="open-zone-overlay">
      {drawn.map((r) => {
        const ring = r.loops?.[0]?.points;
        if (!ring || ring.length < 3) return null;
        const d = `M${ring.map((p) => `${p.x},${-p.y}`).join("L")}Z`;
        return <path key={r.id} className="open-zone-room" d={d} />;
      })}
    </g>
  );
}
