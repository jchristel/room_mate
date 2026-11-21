// A drawing of a sheet: its outline and a labelled box per viewport, coloured by what the viewport is
// (plan, elevation, 3D, schedule), with the elevation lines and each 3D view's camera drawn on the plan. Geometry is
// `previewOf`; this only draws it.

import { useRef, useState } from "react";

import { KIND_LABEL, previewOf, type PortKind } from "./preview.js";
import type { SheetRecord } from "./types.js";

const KINDS: PortKind[] = ["plan", "elevation", "three_d", "schedule", "other"];

/** Smaller than this (paper mm) is a click, not a move. */
const MOVED_MM = 0.3;

export interface PreviewProps {
  record: Pick<SheetRecord, "size_mm" | "sheet">;
  /** When given, the boxes can be dragged: called with the viewport's index in `record.sheet.view_ports` and its new
   * centre, paper mm from the bottom left (y up), rounded to 0.1, when a drag ends. Nothing is moved here: the caller
   * puts the centre in the record and draws again. */
  onMove?: (port: number, centre: [number, number]) => void;
}

export function SheetPreview({ record, onMove }: PreviewProps) {
  const svg = useRef<SVGSVGElement>(null);
  const [drag, setDrag] = useState<{ box: number; from: { x: number; y: number }; dx: number; dy: number } | null>(null);
  const preview = previewOf(record);
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
  return (
    <figure className={`preview${onMove ? " editable" : ""}`}>
      <svg ref={svg} viewBox={`-5 -5 ${preview.width + 10} ${preview.height + 10}`} role="img" aria-label="Sheet preview">
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
        {preview.lines.map((l, i) => (
          <g key={`line${i}`} className="elevation-line">
            <line x1={l.x1} y1={l.y1} x2={l.x2} y2={l.y2} />
            <text x={(l.x1 + l.x2) / 2} y={(l.y1 + l.y2) / 2} fontSize={fontSize} textAnchor="middle" dominantBaseline="central">
              {l.label}
            </text>
          </g>
        ))}
        {preview.arrows.map((a, i) => (
          <g key={`camera${i}`} className="camera-arrow">
            <line x1={a.x1} y1={a.y1} x2={a.x2} y2={a.y2} />
            <polygon points={a.head.map((h) => `${h.x},${h.y}`).join(" ")} />
            <text x={a.x1} y={a.y1} fontSize={fontSize} textAnchor="middle" dominantBaseline="central">
              {a.label}
            </text>
          </g>
        ))}
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
