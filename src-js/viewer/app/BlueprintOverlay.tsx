// The blueprint tool's ghost on the plan: the room it is laid in, ringed, and the resolver's
// placements drawn as dashed boxes. SVG over the plan like the route and the footprints, in the
// plan's world coordinates with Y flipped (the resolver answers in the project's plan frame, so
// nothing is converted here).

import { ghostCorners, type BlueprintToolState } from "../blueprintTool.js";
import { useViewer } from "./useViewer.js";

export function BlueprintOverlay({ levelId, tool }: { levelId: string | null; tool: BlueprintToolState | null }) {
  const { payload } = useViewer();
  if (!tool || tool.result.state !== "done" || levelId === null || !tool.roomId) return null;
  const room = payload?.rooms?.find((r) => r.id === tool.roomId);
  // The ghost belongs to the room's storey: on another level's plan it would float over nothing.
  if (!room || room.level_id !== levelId) return null;
  const ring = room.loops?.[0]?.points;
  const flagged = new Set<string>();
  for (const f of tool.result.body.findings as { kind: string; item?: string; a?: string; b?: string }[]) {
    if (f.item) flagged.add(f.item);
    if (f.a) flagged.add(f.a);
    if (f.b) flagged.add(f.b);
  }
  return (
    <g className="blueprint-overlay">
      {ring && ring.length >= 3 ? <path className="bp-room" d={`M${ring.map((p) => `${p.x},${-p.y}`).join("L")}Z`} /> : null}
      {tool.result.body.placements.map((p) => {
        const box = ghostCorners(p);
        const front = { x: p.centre.x + p.facing.x * (p.size_mm[0] / 2), y: p.centre.y + p.facing.y * (p.size_mm[0] / 2) };
        return (
          <g key={p.item_id} className={`bp-item${p.class === "wall_device" ? " wall" : ""}${flagged.has(p.item_id) ? " flagged" : ""}`}>
            <title>{p.type_name}</title>
            <polygon points={box.map((q) => `${q.x},${-q.y}`).join(" ")} />
            <line x1={p.centre.x} y1={-p.centre.y} x2={front.x} y2={-front.y} />
          </g>
        );
      })}
    </g>
  );
}
