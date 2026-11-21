// A drawing of a sheet: its outline and a labelled box per viewport, coloured by what the viewport is
// (plan, elevation, 3D, schedule), with the elevation lines and each 3D view's camera drawn on the plan. Geometry is
// `previewOf`; this only draws it.

import { useRef, useState } from "react";

import { KIND_LABEL, previewOf, type PortKind } from "./preview.js";
import type { SheetDrawing } from "./types.js";
import { MAX_ZOOM, useZoomPan } from "./useZoomPan.js";

const KINDS: PortKind[] = ["plan", "elevation", "three_d", "schedule", "other"];

/** Smaller than this (paper mm) is a click, not a move. */
const MOVED_MM = 0.3;

export interface PreviewProps {
  record: SheetDrawing;
  /** The room's outline and its doors (the model's own frame, mm), drawn on the plan viewport, under the lines. */
  room?: { outline: readonly (readonly [number, number])[]; doors: readonly (readonly (readonly [number, number])[])[] };
  /** When given, each 3D view's camera has a grip at each end: called with the view's index (from 1) and the heading the
   * grips now make, a unit vector in the model's frame, when a grip is let go. Nothing is changed here. */
  onAim?: (index: number, heading: [number, number]) => void;
  /** When given, the boxes can be dragged: called with the viewport's index in `record.sheet.view_ports` and its new
   * centre, paper mm from the bottom left (y up), rounded to 0.1, when a drag ends. Nothing is moved here: the caller
   * puts the centre in the record and draws again. */
  onMove?: (port: number, centre: [number, number]) => void;
}

export function SheetPreview({ record, room, onAim, onMove }: PreviewProps) {
  const svg = useRef<SVGSVGElement>(null);
  const [drag, setDrag] = useState<{ box: number; from: { x: number; y: number }; dx: number; dy: number } | null>(null);
  const [aim, setAim] = useState<{ index: number; end: "tail" | "tip"; x: number; y: number } | null>(null);
  const preview = previewOf(record);
  const zoom = useZoomPan(svg, preview);
  if (!preview) return <p className="hint">Nothing to draw: the export has no viewport outlines and the paper size was not measured.</p>;
  const used = KINDS.filter((k) => preview.boxes.some((b) => b.kind === k));
  const fontSize = Math.max(preview.width, preview.height) / 45;
  // a label shrinks to fit its own box (monospace: a character is about 0.6 of the size wide), never grows past the sheet's
  const fit = (label: string, w: number, h: number) => Math.min(fontSize, (w * 0.9) / (Math.max(label.length, 1) * 0.6), h * 0.45);
  // a pointer in the drawing's own units (paper mm, y down), whatever size the page draws it at
  const at = (e: { clientX: number; clientY: number }) => {
    const m = svg.current?.getScreenCTM();
    if (!m) return null;
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(m.inverse());
    return { x: p.x, y: p.y };
  };
  const tenth = (v: number) => Math.round(v * 10) / 10;

  const grip = Math.max(preview.width, preview.height) / 80 / zoom.k;
  const frame = preview.frame;
  const path = (ring: readonly (readonly [number, number])[]) => (frame ? ring.map((p) => frame.toPaper(p).join(",")).join(" ") : "");
  const arrowOf = (a: (typeof preview.arrows)[number]) => {
    const [tail, tip] = [{ x: a.x1, y: a.y1 }, a.head[0]!];
    if (aim?.index !== a.index) return { tail, tip, shaft: { x: a.x2, y: a.y2 }, head: a.head };
    // a grip is being dragged: the arrow is redrawn between the grips as they are now
    const [t, h] = aim.end === "tail" ? [{ x: aim.x, y: aim.y }, tip] : [tail, { x: aim.x, y: aim.y }];
    const len = Math.hypot(h.x - t.x, h.y - t.y) || 1;
    const size = Math.max(2, len * 0.12);
    const [ux, uy] = [(h.x - t.x) / len, (h.y - t.y) / len];
    const [bx, by] = [h.x - ux * size, h.y - uy * size];
    return { tail: t, tip: h, shaft: { x: bx, y: by }, head: [h, { x: bx - uy * size * 0.5, y: by + ux * size * 0.5 }, { x: bx + uy * size * 0.5, y: by - ux * size * 0.5 }] };
  };
  return (
    <figure className={`preview${onMove ? " editable" : ""}${zoom.k > 1 ? " zoomed" : ""}`}>
      <div className="zoom-bar" role="group" aria-label="Zoom">
        <button type="button" disabled={zoom.k >= MAX_ZOOM} onClick={() => zoom.zoomBy(1.5)} aria-label="Zoom in" title="Zoom in (or Ctrl/Cmd + wheel)">
          +
        </button>
        <button type="button" disabled={zoom.k <= 1} onClick={() => zoom.zoomBy(1 / 1.5)} aria-label="Zoom out" title="Zoom out">
          −
        </button>
        <button type="button" disabled={zoom.k === 1} onClick={zoom.fit} title="Show the whole sheet">
          Fit
        </button>
        {zoom.k > 1 ? <span className="hint">{Math.round(zoom.k * 100)}% · drag the paper to move</span> : null}
      </div>
      <svg
        ref={svg}
        viewBox={`${zoom.box!.x} ${zoom.box!.y} ${zoom.box!.w} ${zoom.box!.h}`}
        role="img"
        aria-label="Sheet preview"
        {...zoom.handlers}
      >
        <rect className="paper" x={0} y={0} width={preview.width} height={preview.height} />
        {preview.boxes.map((b, i) => (
          <g
            key={i}
            className={`port ${b.kind}${b.clipped ? " clipped" : ""}${drag?.box === i ? " dragging" : ""}`}
            transform={drag?.box === i ? `translate(${drag.dx} ${drag.dy})` : undefined}
            {...(onMove && {
              onPointerDown: (e: React.PointerEvent<SVGGElement>) => {
                const from = at(e);
                if (!from) return;
                e.stopPropagation();
                e.currentTarget.setPointerCapture(e.pointerId);
                setDrag({ box: i, from, dx: 0, dy: 0 });
              },
              onPointerMove: (e: React.PointerEvent<SVGGElement>) => {
                const p = drag?.box === i ? at(e) : null;
                if (p && drag) setDrag({ ...drag, dx: p.x - drag.from.x, dy: p.y - drag.from.y });
              },
              onPointerUp: () => {
                if (drag?.box !== i) return;
                if (Math.hypot(drag.dx, drag.dy) >= MOVED_MM) onMove(b.port, [tenth(b.x + b.w / 2 + drag.dx), tenth(preview.height - (b.y + b.h / 2 + drag.dy))]);
                setDrag(null);
              },
              onPointerCancel: () => setDrag(null),
            })}
          >
            <rect x={b.x} y={b.y} width={b.w} height={b.h} />
            {b.crop ? <rect className="view-crop" x={b.crop.x} y={b.crop.y} width={b.crop.w} height={b.crop.h} /> : null}
            {b.annotation ? <rect className="annotation-crop" x={b.annotation.x} y={b.annotation.y} width={b.annotation.w} height={b.annotation.h} /> : null}
            <text x={b.x + b.w / 2} y={b.y + b.h / 2} fontSize={fit(b.label, b.w, b.h)} textAnchor="middle" dominantBaseline="central">
              {b.label}
            </text>
          </g>
        ))}
        {room && frame ? (
          <g className="room" aria-label="Room">
            {room.doors.map((d, i) => (
              <polygon key={`door${i}`} className="door" points={path(d)} />
            ))}
            <polygon className="room-outline" points={path(room.outline)} />
          </g>
        ) : null}
        {preview.lines.map((l, i) => (
          <g key={`line${i}`} className="elevation-line">
            <line x1={l.x1} y1={l.y1} x2={l.x2} y2={l.y2} />
            <text x={(l.x1 + l.x2) / 2} y={(l.y1 + l.y2) / 2} fontSize={fontSize} textAnchor="middle" dominantBaseline="central">
              {l.label}
            </text>
          </g>
        ))}
        {preview.arrows.map((a, i) => {
          const s = arrowOf(a);
          // a grip at each end: dragging either sets the heading (the arrow's direction), the other end stays
          const gripOf = (end: "tail" | "tip", p: { x: number; y: number }) => (
            <circle
              key={end}
              className={`grip ${end}${aim?.index === a.index && aim.end === end ? " dragging" : ""}`}
              cx={p.x}
              cy={p.y}
              r={grip}
              onPointerDown={(e) => {
                e.stopPropagation();
                e.currentTarget.setPointerCapture(e.pointerId);
                setAim({ index: a.index, end, x: p.x, y: p.y });
              }}
              onPointerMove={(e) => {
                const q = aim?.index === a.index && aim.end === end ? at(e) : null;
                if (q) setAim({ index: a.index, end, x: q.x, y: q.y });
              }}
              onPointerUp={() => {
                if (!(aim?.index === a.index && aim.end === end) || !frame) return setAim(null);
                const [t, h] = [frame.toModel(s.tail.x, s.tail.y), frame.toModel(s.tip.x, s.tip.y)];
                const len = Math.hypot(h[0] - t[0], h[1] - t[1]);
                setAim(null);
                if (len > 1e-6) onAim?.(a.index, [h[0] / len, h[1] / len]);
              }}
              onPointerCancel={() => setAim(null)}
            />
          );
          return (
            <g key={`camera${i}`} className="camera-arrow">
              <line x1={s.tail.x} y1={s.tail.y} x2={s.shaft.x} y2={s.shaft.y} />
              <polygon points={s.head.map((h) => `${h.x},${h.y}`).join(" ")} />
              <text x={s.tail.x} y={s.tail.y} fontSize={fontSize} textAnchor="middle" dominantBaseline="central">
                {a.label}
              </text>
              {onAim && frame ? [gripOf("tail", s.tail), gripOf("tip", s.tip)] : null}
            </g>
          );
        })}
      </svg>
      <figcaption className="hint">
        {preview.measured ? `${Math.round(preview.width)} x ${Math.round(preview.height)} mm` : "Paper size not measured: outline drawn round the viewports"}
        {preview.boxes.some((b) => b.annotation) ? " · dashed: view crop (short) and annotation crop (long), centred in the box" : ""}
        {preview.boxes.some((b) => b.clipped) ? " · red outline: past the paper's edge" : ""}
        {" · "}
        {used.map((k) => (
          <span key={k} className={`key ${k}`}>
            {KIND_LABEL[k]}
          </span>
        ))}
      </figcaption>
    </figure>
  );
}
