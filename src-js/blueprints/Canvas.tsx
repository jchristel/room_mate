// The centre of the page: the room's outline and what is in it, drawn square-on.
//
// **SVG, not the Pixi renderer the plan first named.** This draws ONE room: an outline and a few
// dozen to a few hundred rectangles, where the main viewer's GL renderer is built for thousands of
// rooms and 100,000 items. At this size SVG gives hit-testing, labels and accessibility for free,
// and an editor needs all three; the GL machinery would buy nothing and cost the selection logic.
//
// Y points UP on the plan and DOWN on the screen, so every point is flipped once, here, on its way
// to a string (`s`). Nothing outside this file knows about the flip.
//
// The outline is drawn as a polygon with a circle at each corner and each edge numbered, because
// the edge index is what a recipe refers to and what a person maps by hand when a room does not
// match. The fine outline sits behind it, dashed, since that is what "inside the room" is judged
// against.

import { useMemo } from "react";

import { boundsOf, corners, doorSegment, edges, frameOf, pt, type Pt } from "./geometry.js";
import type { BlueprintDoor, BlueprintItem, Outline } from "./types.js";
import { usePanZoom, type View } from "./view.js";

const s = (p: Pt): string => `${p[0]},${-p[1]}`;

export interface CanvasProps {
  outline: Outline;
  items: readonly BlueprintItem[];
  /** The room's doors: orientation only, drawn on their walls. */
  doors?: readonly BlueprintDoor[];
  selected: ReadonlySet<string>;
  showTags: boolean;
  showExcluded: boolean;
  onSelect(id: string | null, additive: boolean): void;
}

/** A view that holds the whole room and everything in it, with a margin. */
function fitView(outline: Outline, items: readonly BlueprintItem[]): View {
  const ring = outline.coarse.map(pt);
  const points: Pt[] = [...ring, ...outline.fine.map(pt)];
  for (const it of items) {
    const f = frameOf(it, ring);
    if (f) points.push(...corners(f, it.size_mm));
  }
  const b = boundsOf(points.length ? points : [[0, 0]]);
  const pad = Math.max(b.maxX - b.minX, b.maxY - b.minY, 1000) * 0.08;
  return { x: b.minX - pad, y: -(b.maxY + pad), w: b.maxX - b.minX + 2 * pad, h: b.maxY - b.minY + 2 * pad };
}

export function Canvas({ outline, items, doors = [], selected, showTags, showExcluded, onSelect }: CanvasProps) {
  const ring = useMemo(() => outline.coarse.map(pt), [outline]);
  const fine = useMemo(() => outline.fine.map(pt), [outline]);
  const es = useMemo(() => edges(ring), [ring]);
  // The view is reset when the OUTLINE changes (another blueprint or revision), not on every edit.
  const fitted = useMemo(() => fitView(outline, items), [outline]); // eslint-disable-line react-hooks/exhaustive-deps
  const { view, svg, onPointerDown, onPointerMove, onPointerUp } = usePanZoom(fitted);

  const r = view.w / 160; // corner circles and edge numbers scale with the zoom
  const shown = items.filter((i) => showExcluded || !i.excluded);
  const byId = new Map(items.map((i) => [i.id, i]));

  return (
    <svg
      ref={svg}
      className="canvas"
      viewBox={`${view.x} ${view.y} ${view.w} ${view.h}`}
      role="img"
      aria-label="Blueprint plan"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={() => {
        // A press that did not move is a click on the background: clear the selection.
        if (onPointerUp()) onSelect(null, false);
      }}
    >
      <polygon className="outline-fine" points={fine.map(s).join(" ")} />
      <polygon className="outline" points={ring.map(s).join(" ")} />

      {doors.map((d) => {
        const seg = doorSegment(ring, d);
        return seg ? <line key={`d${d.id}`} className="door" x1={seg[0][0]} y1={-seg[0][1]} x2={seg[1][0]} y2={-seg[1][1]} /> : null;
      })}

      {shown.map((it) => {
        const f = frameOf(it, ring);
        if (!f) return null;
        const box = corners(f, it.size_mm);
        const front: Pt = [f.centre[0] + Math.cos(f.angle) * (it.size_mm[0] / 2), f.centre[1] + Math.sin(f.angle) * (it.size_mm[0] / 2)];
        const on = selected.has(it.id);
        const cls = `item${it.class === "wall_device" ? " wall" : ""}${it.excluded ? " excluded" : ""}${on ? " selected" : ""}`;
        return (
          <g
            key={it.id}
            className={cls}
            onPointerDown={(e) => e.stopPropagation()}
            onClick={(e) => {
              e.stopPropagation();
              onSelect(it.id, e.shiftKey || e.metaKey || e.ctrlKey);
            }}
          >
            <title>{`${it.type_name}${it.excluded ? " (excluded)" : ""}`}</title>
            <polygon points={box.map(s).join(" ")} />
            <line className="facing" x1={f.centre[0]} y1={-f.centre[1]} x2={front[0]} y2={-front[1]} />
            {it.class === "wall_device" ? <circle className="device" cx={f.centre[0]} cy={-f.centre[1]} r={r * 0.9} /> : null}
            {showTags ? (
              <text
                className="tag"
                x={f.centre[0]}
                y={-f.centre[1]}
                fontSize={Math.min(Math.max(Math.min(...it.size_mm) / 5, view.w / 220), view.w / 70)}
                textAnchor="middle"
                dominantBaseline="central"
              >
                {it.type_name.length > 22 ? `${it.type_name.slice(0, 21)}…` : it.type_name}
              </text>
            ) : null}
          </g>
        );
      })}

      {/* Followers: a dashed line from a selected item to the one it follows, and back. */}
      {[...selected].map((id) => {
        const it = byId.get(id);
        const to = it?.follows ? byId.get(it.follows.item) : undefined;
        const [a, b] = [it && frameOf(it, ring), to && frameOf(to, ring)];
        if (!a || !b) return null;
        return <line key={`f${id}`} className="follow" x1={a.centre[0]} y1={-a.centre[1]} x2={b.centre[0]} y2={-b.centre[1]} />;
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
