// The open-zone editor's layers on the plan: the zone being drawn, the zones
// already saved, and the rooms no door reaches.
//
// SVG over the plan like the route and the footprints, and for the same rule in
// STRATEGY-BROWSER: a translucent fill adds colour without removing the room
// outline or its label beneath it. World coordinates, Y flipped, inside the
// viewBox the renderer keeps in step with the projection.
//
// The other two layers exist to answer "what still needs adding, and where":
// saved zones are tinted apart by index so two neighbours read as two, and a
// room nobody can walk to is ringed dotted, which is step 1's worklist drawn
// where the work is. The zone being drawn stays the strongest fill and paints
// last, so it is never hidden by a neighbour it overlaps.

import type { CSSProperties } from "react";

import { roomsInOtherZones, type ZoneEdit } from "../connections.js";
import { markerPoint } from "../route.js";
import { useViewer } from "./useViewer.js";

/** Hues cycle past this many saved zones; two zones sharing one hue are still
 *  told apart by the outline beneath them. */
const HUES = [205, 35, 150, 290, 5, 75];

export function OpenZoneOverlay({ levelId, edit }: { levelId: string | null; edit: ZoneEdit | null }) {
  const { payload, scope, connections } = useViewer();
  if (!edit || levelId === null) return null;

  const doc = connections.projectId === scope.projectId ? connections.doc : null;
  const isolated = connections.projectId === scope.projectId ? (connections.isolated ?? null) : null;
  const members = new Set(edit.members.map((m) => m.room_id));
  // A link has no zone to compare against; its overlay is the two picked rooms.
  const others = edit.kind === "open" ? roomsInOtherZones(doc, edit.id) : roomsInOtherZones(doc, null);
  const onLevel = (payload?.rooms ?? []).filter((r) => r.level_id === levelId);

  const pathOf = (r: (typeof onLevel)[number]) => {
    const ring = r.loops?.[0]?.points;
    if (!ring || ring.length < 3) return null;
    return `M${ring.map((p) => `${p.x},${-p.y}`).join("L")}Z`;
  };

  return (
    <g className="open-zone-overlay">
      {onLevel.map((r) => {
        const index = others.get(r.id);
        const d = index === undefined || members.has(r.id) ? null : pathOf(r);
        if (!d) return null;
        const style = { "--oz-hue": HUES[index! % HUES.length] } as CSSProperties;
        return <path key={`s${r.id}`} className="open-zone-saved" style={style} d={d} />;
      })}
      {isolated
        ? onLevel.map((r) => {
            const d = isolated.has(r.id) && !members.has(r.id) && !others.has(r.id) ? pathOf(r) : null;
            return d ? <path key={`u${r.id}`} className="open-zone-unreachable" d={d} /> : null;
          })
        : null}
      {onLevel.map((r) => {
        const d = members.has(r.id) ? pathOf(r) : null;
        // The hub and a room held for a cut are told apart from the plain members.
        const role = r.id === edit.hub ? " hub" : r.id === edit.cutFrom ? " held" : "";
        return d ? <path key={r.id} className={`open-zone-room${role}`} d={d} /> : null;
      })}
      {/* A closed wall: a line between the two rooms, crossed in the middle, so it
          reads as "no way through here" on a plan where the rooms are filled. */}
      {edit.kind === "open"
        ? edit.cuts.map((c) => {
            const [ra, rb] = [onLevel.find((r) => r.id === c.a), onLevel.find((r) => r.id === c.b)];
            const [pa, pb] = [ra && markerPoint(ra), rb && markerPoint(rb)];
            if (!pa || !pb) return null;
            const [mx, my] = [(pa.x + pb.x) / 2, -(pa.y + pb.y) / 2];
            const s = Math.max(Math.hypot(pa.x - pb.x, pa.y - pb.y) * 0.08, 0.4);
            return (
              <g key={`${c.a}|${c.b}`} className="open-zone-cut">
                <line x1={pa.x} y1={-pa.y} x2={pb.x} y2={-pb.y} />
                <line x1={mx - s} y1={my - s} x2={mx + s} y2={my + s} />
                <line x1={mx - s} y1={my + s} x2={mx + s} y2={my - s} />
              </g>
            );
          })
        : null}
    </g>
  );
}
