// The RLS tool's picks on the plan: each room chosen is ringed and tinted, on its own storey. SVG over the plan
// in feet with Y flipped, like the blueprint overlay; it never takes a click, which belongs to the room pick.
//
// In auto mode every room of the storey is drawn by what the solver made of it: green, yellow or red by fit, a
// dashed ring for a person's override, and a faded wash over a room that already has a task (the room search's way
// of saying "not this one"). The room last clicked is outlined heavier.

import { categoryOf, doneRoomIds, type RlsToolState } from "../rlsTool.js";
import { useViewer } from "./useViewer.js";

export function RlsOverlay({ levelId, tool }: { levelId: string | null; tool: RlsToolState | null }) {
  const { payload, roomTasks } = useViewer();
  if (!tool || levelId === null) return null;
  const points = (id: string) => {
    const room = payload?.rooms?.find((r) => r.id === id);
    const ring = room?.loops?.[0]?.points;
    return room && room.level_id === levelId && ring && ring.length >= 3 ? ring.map((p) => `${p.x},${-p.y}`).join(" ") : null;
  };
  if (tool.mode === "auto") {
    const done = doneRoomIds(roomTasks);
    const rooms = (payload?.rooms ?? []).filter((r) => r.level_id === levelId);
    return (
      <g className="rls-overlay">
        {rooms.map((r) => {
          const pts = points(r.id);
          if (!pts) return null;
          const faded = done.has(r.id) && !tool.included.includes(r.id);
          const cat = faded ? null : categoryOf(tool, r.id);
          const cls = faded ? "rls-faded" : cat ? `rls-${cat}` : "rls-unsolved";
          return <polygon key={r.id} className={`${cls}${tool.focus === r.id ? " rls-focus" : ""}`} points={pts} />;
        })}
      </g>
    );
  }
  if (tool.roomIds.length === 0) return null;
  return (
    <g className="rls-overlay">
      {tool.roomIds.map((id) => {
        const pts = points(id);
        return pts ? <polygon key={id} className="rls-room" points={pts} /> : null;
      })}
    </g>
  );
}
