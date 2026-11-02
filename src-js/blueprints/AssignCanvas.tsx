// The target room with the blueprint laid into it: the resolver's answer, drawn.
//
// **Nothing here works out where an item goes.** The server's resolver returns each item's centre,
// facing and size in the plan's frame, and this file draws exactly that. A second copy of the rule
// is how a preview and the result disagree (plan decision 2). What it does compute is the
// room's own edges, to put an edge number and a door on a wall.
//
// Two door marks, because they answer different questions: a SOLID bar is a door the room has (it is
// in the model); a DASHED bar, a little inside the wall, is where the blueprint's door lands under
// the chosen mapping. Orientation is right when they sit on each other.

import { useMemo } from "react";

import { boundsOf, corners, doorSegment, edges, pt, type Pt } from "./geometry.js";
import { TagLabel } from "./TagLabel.js";
import { itemTagLines } from "./tagText.js";
import type { BlueprintItem, Finding, ResolveResponse } from "./types.js";
import { usePanZoom, type View } from "./view.js";

const s = (p: Pt): string => `${p[0]},${-p[1]}`;

export interface AssignCanvasProps {
  result: ResolveResponse;
  selected: string | null;
  /** Items this room moved or turned: drawn so a person can see what they changed. */
  changed: ReadonlySet<string>;
  showTags: boolean;
  /** The `[tags] ffe` fields the tag says (the main viewer's setting), resolved against the blueprint's items. */
  tagFields: readonly string[];
  /** The blueprint's items by id: a placement carries only the type name, the properties are the item's. */
  itemsById: ReadonlyMap<string, BlueprintItem>;
  onSelect(id: string | null): void;
}

/** Item ids a finding is about, so the drawing can mark them. */
export function flaggedItems(findings: readonly Finding[]): Set<string> {
  const out = new Set<string>();
  for (const f of findings) {
    if (f.kind === "outside" || f.kind === "follow_broken" || f.kind === "corner_anchored") out.add(f.item);
    if (f.kind === "overlap") {
      out.add(f.a);
      out.add(f.b);
    }
  }
  return out;
}

function fitView(result: ResolveResponse): View {
  const points: Pt[] = [...result.outline.coarse.map(pt), ...result.outline.fine.map(pt)];
  for (const p of result.placements) {
    const c = pt(p.centre);
    const r = Math.max(...p.size_mm) / 2;
    points.push([c[0] - r, c[1] - r], [c[0] + r, c[1] + r]);
  }
  const b = boundsOf(points.length ? points : [[0, 0]]);
  const pad = Math.max(b.maxX - b.minX, b.maxY - b.minY, 1000) * 0.08;
  return { x: b.minX - pad, y: -(b.maxY + pad), w: b.maxX - b.minX + 2 * pad, h: b.maxY - b.minY + 2 * pad };
}

export function AssignCanvas({ result, selected, changed, showTags, tagFields, itemsById, onSelect }: AssignCanvasProps) {
  const ring = useMemo(() => result.outline.coarse.map(pt), [result.outline]);
  const fine = useMemo(() => result.outline.fine.map(pt), [result.outline]);
  const es = useMemo(() => edges(ring), [ring]);
  // Refit when the ROOM changes, not when the mapping does: turning a room's rotation over should not
  // throw away where the person had zoomed to.
  const fitted = useMemo(() => fitView(result), [result.outline]); // eslint-disable-line react-hooks/exhaustive-deps
  const { view, svg, onPointerDown, onPointerMove, onPointerUp } = usePanZoom(fitted);
  const flagged = useMemo(() => flaggedItems(result.findings), [result.findings]);
  const r = view.w / 160;

  return (
    <svg
      ref={svg}
      className="canvas"
      viewBox={`${view.x} ${view.y} ${view.w} ${view.h}`}
      role="img"
      aria-label="Blueprint placed in the room"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={() => {
        if (onPointerUp()) onSelect(null);
      }}
    >
      <polygon className="outline-fine" points={fine.map(s).join(" ")} />
      <polygon className="outline" points={ring.map(s).join(" ")} />

      {result.placements.map((p) => {
        const [fx, fy] = [p.facing.x, p.facing.y];
        const frame = { centre: pt(p.centre), angle: Math.atan2(fy, fx) };
        const box = corners(frame, p.size_mm);
        const front: Pt = [frame.centre[0] + fx * (p.size_mm[0] / 2), frame.centre[1] + fy * (p.size_mm[0] / 2)];
        const cls = `item${p.class === "wall_device" ? " wall" : ""}${flagged.has(p.item_id) ? " flagged" : ""}${selected === p.item_id ? " selected" : ""}${changed.has(p.item_id) ? " changed" : ""}`;
        return (
          <g
            key={p.item_id}
            className={cls}
            onPointerDown={(e) => e.stopPropagation()}
            onClick={(e) => {
              e.stopPropagation();
              onSelect(p.item_id);
            }}
          >
            <title>{`${p.type_name}\n${p.rule}`}</title>
            <polygon points={box.map(s).join(" ")} />
            <line className="facing" x1={frame.centre[0]} y1={-frame.centre[1]} x2={front[0]} y2={-front[1]} />
            {p.class === "wall_device" ? <circle className="device" cx={frame.centre[0]} cy={-frame.centre[1]} r={r * 0.9} /> : null}
            {showTags ? (
              <TagLabel
                x={frame.centre[0]}
                y={-frame.centre[1]}
                fontSize={Math.min(Math.max(Math.min(...p.size_mm) / 5, view.w / 220), view.w / 70)}
                lines={itemTagLines(tagFields, itemsById.get(p.item_id) ?? { id: p.item_id, type_name: p.type_name })}
              />
            ) : null}
          </g>
        );
      })}

      {result.doors.map((d) => {
        const seg = doorSegment(ring, d);
        return seg ? <line key={`d${d.id}`} className="door" x1={seg[0][0]} y1={-seg[0][1]} x2={seg[1][0]} y2={-seg[1][1]} /> : null;
      })}
      {result.mapped_doors.map((d) => {
        const seg = doorSegment(ring, d, r * 3);
        return seg ? <line key={`m${d.id}`} className="door mapped" x1={seg[0][0]} y1={-seg[0][1]} x2={seg[1][0]} y2={-seg[1][1]} /> : null;
      })}

      {es.map((e, i) => {
        const mid = [e.a[0] + (e.b[0] - e.a[0]) / 2 - e.n[0] * r * 4, e.a[1] + (e.b[1] - e.a[1]) / 2 - e.n[1] * r * 4] as const;
        return (
          <text key={`e${i}`} className="edge-no" x={mid[0]} y={-mid[1]} fontSize={r * 3.2} textAnchor="middle" dominantBaseline="central">
            {i}
          </text>
        );
      })}
      {ring.map((p, i) => (
        <circle key={`v${i}`} className="vertex" cx={p[0]} cy={-p[1]} r={r} />
      ))}
    </svg>
  );
}
