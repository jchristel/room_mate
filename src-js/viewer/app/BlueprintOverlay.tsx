// The blueprint tool's ghost on the plan: the room it is laid in, ringed; the resolver's placements
// drawn as dashed boxes; and, when the person is laying the blueprint by hand, the BOX with its
// handles (drag the body to move it, a side to stretch it along that side's axis, the green corner
// knob to turn it).
//
// SVG over the plan like the route and the footprints, in the plan's world coordinates with Y
// flipped. **The viewer's world is feet and the blueprint's numbers are millimetres**, so this file
// converts at its edge and the tool's own maths stays in millimetres. The box is drawn from the
// client's copy of the transform (`boxRing`) so it follows the pointer at once; the placements inside
// it are the server's answer and catch up a moment later.
//
// Each handle owns its pointer (captures it and stops the event at itself), because the plan's
// gestures are on the same svg and a press would otherwise be a pan -- the route marks' rule.

import { useEffect, useRef } from "react";

import {
  boxCentre,
  boxHandles,
  boxRing,
  ghostCorners,
  moveBox,
  rotateBox,
  stretchBox,
  type BlueprintToolState,
  type BoxFrame,
  type HandleKind,
  type PlanXY,
} from "../blueprintTool.js";
import { planPointOf } from "../route.js";
import { currentTool } from "./blueprintData.js";
import { patchZoneBlueprint } from "./store.js";
import { handleOf } from "./zoneRegistry.js";
import { useViewer } from "./useViewer.js";

const MM_PER_FT = 304.8;
/** Millimetres to the feet the plan draws in. */
const ft = (p: PlanXY): PlanXY => ({ x: p.x / MM_PER_FT, y: p.y / MM_PER_FT });
const svgPoints = (ring: readonly PlanXY[]): string => ring.map((p) => `${p.x / MM_PER_FT},${-p.y / MM_PER_FT}`).join(" ");

/** The room's outer loop in millimetres, or null when it has none. */
export function roomRingMm(room: { loops?: { points: { x: number; y: number }[] }[] } | undefined): PlanXY[] | null {
  const ring = room?.loops?.[0]?.points;
  return ring && ring.length >= 3 ? ring.map((p) => ({ x: p.x * MM_PER_FT, y: p.y * MM_PER_FT })) : null;
}

export function BlueprintOverlay({ zoneId, levelId, tool }: { zoneId: string; levelId: string | null; tool: BlueprintToolState | null }) {
  const { payload, appearance } = useViewer();
  // The box's colour is `[appearance] blueprint_box`; the stylesheet's blue is the default.
  const boxColour = appearance?.blueprint_box ?? undefined;
  if (!tool || levelId === null || !tool.roomId) return null;
  const room = payload?.rooms?.find((r) => r.id === tool.roomId);
  // The ghost belongs to the room's storey: on another level's plan it would float over nothing.
  if (!room || room.level_id !== levelId) return null;
  const roomRing = roomRingMm(room);
  const done = tool.result.state === "done" ? tool.result.body : null;
  const flagged = new Set<string>();
  for (const f of (done?.findings ?? []) as { kind: string; item?: string; a?: string; b?: string }[]) {
    if (f.item) flagged.add(f.item);
    if (f.a) flagged.add(f.a);
    if (f.b) flagged.add(f.b);
  }
  const box = tool.frame && tool.outline && roomRing ? boxRing(tool.outline, roomRing, tool.frame) : null;
  const handles = box && tool.frame && roomRing ? boxHandles(box, boxCentre(roomRing, tool.frame), tool.frame.rotate_deg) : null;

  return (
    <g className="blueprint-overlay">
      {roomRing ? <polygon className="bp-room" points={svgPoints(roomRing)} /> : null}
      {box ? <polygon className="bp-box" points={svgPoints(box)} {...(boxColour ? { style: { stroke: boxColour, fill: boxColour } } : {})} /> : null}
      {done?.placements.map((p) => {
        const corners = ghostCorners(p);
        const front = { x: p.centre.x + p.facing.x * (p.size_mm[0] / 2), y: p.centre.y + p.facing.y * (p.size_mm[0] / 2) };
        const [c, f] = [ft(p.centre), ft(front)];
        return (
          <g key={p.item_id} className={`bp-item${p.class === "wall_device" ? " wall" : ""}${flagged.has(p.item_id) ? " flagged" : ""}`}>
            <title>{p.type_name}</title>
            <polygon points={svgPoints(corners)} />
            <line x1={c.x} y1={-c.y} x2={f.x} y2={-f.y} />
          </g>
        );
      })}
      {handles && roomRing && tool.frame ? (
        <>
          {(["move", "rotate", "u+", "u-", "v+", "v-"] as HandleKind[]).map((kind) => (
            <Handle key={kind} zoneId={zoneId} kind={kind} at={handles[kind]} roomRing={roomRing} />
          ))}
        </>
      ) : null}
    </g>
  );
}

/** One grab point of the box. */
function Handle({ zoneId, kind, at, roomRing }: { zoneId: string; kind: HandleKind; at: PlanXY; roomRing: readonly PlanXY[] }) {
  const grab = useRef<SVGLineElement>(null);
  const ring = useRef(roomRing);
  ring.current = roomRing;

  useEffect(() => {
    const el = grab.current;
    if (!el) return;
    let start: { frame: BoxFrame; from: PlanXY } | null = null;
    const where = (e: PointerEvent): PlanXY | null => {
      const p = planPointOf(handleOf(zoneId)?.renderer.toWorld(e.clientX, e.clientY));
      return p ? { x: p.x * MM_PER_FT, y: p.y * MM_PER_FT } : null;
    };
    const apply = (to: PlanXY) => {
      const now = currentTool(zoneId);
      if (!now || !start) return;
      const centre = boxCentre(ring.current, start.frame);
      const next =
        kind === "move"
          ? moveBox(start.frame, start.from, to)
          : kind === "rotate"
            ? rotateBox(start.frame, centre, start.from, to)
            : stretchBox(start.frame, kind[0] === "u" ? "u" : "v", centre, start.from, to);
      patchZoneBlueprint(zoneId, { ...now, frame: next });
    };
    const move = (e: PointerEvent) => {
      const p = where(e);
      if (p) apply(p);
    };
    const end = (e: PointerEvent) => {
      el.releasePointerCapture?.(e.pointerId);
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", end);
      el.removeEventListener("pointercancel", end);
      start = null;
    };
    const down = (e: PointerEvent) => {
      e.stopPropagation();
      e.preventDefault();
      const now = currentTool(zoneId);
      const p = where(e);
      if (!now?.frame || !p) return;
      start = { frame: now.frame, from: p };
      el.setPointerCapture?.(e.pointerId);
      el.addEventListener("pointermove", move);
      el.addEventListener("pointerup", end);
      el.addEventListener("pointercancel", end);
    };
    el.addEventListener("pointerdown", down);
    return () => el.removeEventListener("pointerdown", down);
  }, [zoneId, kind]);

  const p = ft(at);
  return (
    <line
      ref={grab}
      className={`bp-handle ${kind}`}
      x1={p.x}
      y1={-p.y}
      x2={p.x + 0.001}
      y2={-p.y}
      aria-label={kind === "move" ? "Drag to move the box" : kind === "rotate" ? "Drag to turn the box" : "Drag to stretch the box"}
    />
  );
}
