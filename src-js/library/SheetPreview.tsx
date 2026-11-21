// A drawing of a sheet: its outline and a labelled box per viewport, coloured by what the viewport is
// (plan, elevation, 3D, schedule), with the elevation lines and each 3D view's camera drawn on the plan. Geometry is
// `previewOf`; this only draws it.

import { KIND_LABEL, previewOf, type PortKind } from "./preview.js";
import type { SheetRecord } from "./types.js";

const KINDS: PortKind[] = ["plan", "elevation", "three_d", "schedule", "other"];

export function SheetPreview({ record }: { record: Pick<SheetRecord, "size_mm" | "sheet"> }) {
  const preview = previewOf(record);
  if (!preview) return <p className="hint">Nothing to draw: the export has no viewport outlines and the paper size was not measured.</p>;
  const used = KINDS.filter((k) => preview.boxes.some((b) => b.kind === k));
  const fontSize = Math.max(preview.width, preview.height) / 45;
  // a label shrinks to fit its own box (monospace: a character is about 0.6 of the size wide), never grows past the sheet's
  const fit = (label: string, w: number, h: number) => Math.min(fontSize, (w * 0.9) / (Math.max(label.length, 1) * 0.6), h * 0.45);
  return (
    <figure className="preview">
      <svg viewBox={`-5 -5 ${preview.width + 10} ${preview.height + 10}`} role="img" aria-label="Sheet preview">
        <rect className="paper" x={0} y={0} width={preview.width} height={preview.height} />
        {preview.boxes.map((b, i) => (
          <g key={i} className={`port ${b.kind}${b.clipped ? " clipped" : ""}`}>
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
