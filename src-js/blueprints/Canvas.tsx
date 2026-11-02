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

import { useMemo, useState, type PointerEvent as ReactPointerEvent } from "react";

import { insertNode, moveNode, removeNode, snapNode } from "./boxEdit.js";
import { boundsOf, corners, doorSegment, edges, frameOf, pt, type Pt } from "./geometry.js";
import type { BlueprintDoor, BlueprintItem, Outline } from "./types.js";
import { TagLabel } from "./TagLabel.js";
import { itemTagLines } from "./tagText.js";
import { usePanZoom, type View } from "./view.js";

const s = (p: Pt): string => `${p[0]},${-p[1]}`;

export interface CanvasProps {
  outline: Outline;
  items: readonly BlueprintItem[];
  /** The room's doors: orientation only, drawn on their walls. */
  doors?: readonly BlueprintDoor[];
  selected: ReadonlySet<string>;
  showTags: boolean;
  /** The `[tags] ffe` fields the tag says (the main viewer's setting). */
  tagFields: readonly string[];
  showExcluded: boolean;
  onSelect(id: string | null, additive: boolean): void;
  /** The box's outline colour (`[appearance] blueprint_box`). */
  boxColour: string;
  /** Present when the box can be edited (not a pending revision). */
  box?: BoxEditing;
}

export interface BoxEditing {
  /** The nodes the person made, to write the items against. Resolves when the draft has been redrawn,
   *  so the box stays where it was dropped meanwhile. */
  onNodes(ring: Pt[]): Promise<void>;
  /** Say why a gesture did nothing. */
  onRefuse(text: string): void;
  /** Align a side of the box with a wall of the real room: both are picked on this canvas. */
  aligning: boolean;
  onAlign(boxEdge: number, wallEdge: number): void;
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

export function Canvas({ outline, items, doors = [], selected, showTags, tagFields, showExcluded, onSelect, boxColour, box }: CanvasProps) {
  const ring = useMemo(() => outline.coarse.map(pt), [outline]);
  const fine = useMemo(() => outline.fine.map(pt), [outline]);
  // The view is reset when the OUTLINE changes (another blueprint or revision), not on every edit.
  const fitted = useMemo(() => fitView(outline, items), [outline]); // eslint-disable-line react-hooks/exhaustive-deps
  const { view, svg, onPointerDown, onPointerMove, onPointerUp } = usePanZoom(fitted);

  // A node being dragged: the box is drawn from this while items stay where the committed outline puts them.
  const [drag, setDrag] = useState<{ i: number; ring: Pt[]; moved: boolean } | null>(null);
  // Aligning: the box side picked first, waiting for a wall.
  const [side, setSide] = useState<number | null>(null);
  const shownRing = drag?.ring ?? ring;
  const shownEs = useMemo(() => edges(drag?.ring ?? ring), [drag, ring]);
  const toPlan = (e: { clientX: number; clientY: number }): Pt | null => {
    const m = svg.current?.getScreenCTM();
    if (!m) return null;
    const q = new DOMPoint(e.clientX, e.clientY).matrixTransform(m.inverse());
    return [q.x, -q.y];
  };
  const commit = async (next: Pt[]) => {
    try {
      await box?.onNodes(next);
    } finally {
      setDrag(null);
    }
  };
  const nodeDown = (e: ReactPointerEvent<SVGCircleElement>, i: number) => {
    if (!box) return;
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag({ i, ring, moved: false });
  };
  const nodeMove = (e: ReactPointerEvent<SVGCircleElement>) => {
    if (!drag) return;
    const p = toPlan(e);
    if (!p) return;
    const at = e.altKey ? p : snapNode(ring, drag.i, p, view.w / 100);
    setDrag({ i: drag.i, ring: moveNode(ring, drag.i, at), moved: true });
  };
  const nodeUp = () => {
    if (drag?.moved) void commit(drag.ring);
    else setDrag(null);
  };

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
      <polygon className="outline" style={{ stroke: boxColour }} points={shownRing.map(s).join(" ")} />
      {box?.aligning
        ? fine.map((a, i) => {
            const b = fine[(i + 1) % fine.length] as Pt;
            return (
              <line
                key={`w${i}`}
                className={`wall-hit${side === null ? " off" : ""}`}
                x1={a[0]}
                y1={-a[1]}
                x2={b[0]}
                y2={-b[1]}
                onPointerDown={(e) => e.stopPropagation()}
                onClick={(e) => {
                  e.stopPropagation();
                  if (side === null) return box.onRefuse("Pick a side of the box first, then a wall of the room.");
                  box.onAlign(side, i);
                  setSide(null);
                }}
              >
                <title>{`room wall ${i}`}</title>
              </line>
            );
          })
        : null}
      {box
        ? shownRing.map((a, i) => {
            const b = shownRing[(i + 1) % shownRing.length] as Pt;
            return (
              <line
                key={`h${i}`}
                className={`edge-hit${side === i ? " picked" : ""}`}
                x1={a[0]}
                y1={-a[1]}
                x2={b[0]}
                y2={-b[1]}
                onPointerDown={(e) => e.stopPropagation()}
                onClick={(e) => {
                  e.stopPropagation();
                  if (box.aligning) setSide(i);
                }}
                onDoubleClick={(e) => {
                  e.stopPropagation();
                  const p = toPlan(e);
                  if (p && !box.aligning) void commit(insertNode(ring, i, p));
                }}
              >
                <title>{box.aligning ? `side ${i}: click to align it with a wall` : "double-click to add a node here"}</title>
              </line>
            );
          })
        : null}

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
              <TagLabel
                x={f.centre[0]}
                y={-f.centre[1]}
                fontSize={Math.min(Math.max(Math.min(...it.size_mm) / 5, view.w / 220), view.w / 70)}
                lines={itemTagLines(tagFields, it)}
              />
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

      {shownEs.map((e, i) => {
        const mid = [e.a[0] + (e.b[0] - e.a[0]) / 2 - e.n[0] * r * 4, e.a[1] + (e.b[1] - e.a[1]) / 2 - e.n[1] * r * 4] as const;
        return (
          <text key={`e${i}`} className="edge-no" x={mid[0]} y={-mid[1]} fontSize={r * 3.2} textAnchor="middle" dominantBaseline="central">
            {i}
          </text>
        );
      })}
      {shownRing.map((p, i) => (
        <circle
          key={`v${i}`}
          className={`vertex${box ? " node" : ""}`}
          style={{ stroke: boxColour }}
          cx={p[0]}
          cy={-p[1]}
          r={box ? r * 1.4 : r}
          onPointerDown={(e) => nodeDown(e, i)}
          onPointerMove={nodeMove}
          onPointerUp={nodeUp}
          onDoubleClick={(e) => {
            e.stopPropagation();
            if (!box) return;
            const next = removeNode(ring, i);
            if (next) void commit(next);
            else box.onRefuse("A box has at least 4 nodes.");
          }}
        >
          {box ? <title>{`node ${i}: drag to move (Alt: no snap), double-click to remove`}</title> : null}
        </circle>
      ))}
    </svg>
  );
}
