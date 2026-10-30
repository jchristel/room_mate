// The RLS tool's picks on the plan: each room chosen is ringed and tinted, on its own storey. SVG over the plan
// in feet with Y flipped, like the blueprint overlay; it never takes a click, which belongs to the room pick.

import type { RlsToolState } from "../rlsTool.js";
import { useViewer } from "./useViewer.js";

export function RlsOverlay({ levelId, tool }: { levelId: string | null; tool: RlsToolState | null }) {
  const { payload } = useViewer();
  if (!tool || levelId === null || tool.roomIds.length === 0) return null;
  return (
    <g className="rls-overlay">
      {tool.roomIds.map((id) => {
        const room = payload?.rooms?.find((r) => r.id === id);
        const ring = room?.loops?.[0]?.points;
        if (!room || room.level_id !== levelId || !ring || ring.length < 3) return null;
        return <polygon key={id} className="rls-room" points={ring.map((p) => `${p.x},${-p.y}`).join(" ")} />;
      })}
    </g>
  );
}
