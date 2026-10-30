// A drawing of a sheet: its outline and a labelled box per viewport, coloured by what the viewport is
// (plan, elevation, 3D, schedule). Geometry is `previewOf`; this only draws it.

import { KIND_LABEL, previewOf, type PortKind } from "./preview.js";
import type { SheetRecord } from "./types.js";

const KINDS: PortKind[] = ["plan", "elevation", "three_d", "schedule", "other"];

export function SheetPreview({ record }: { record: Pick<SheetRecord, "size_mm" | "sheet"> }) {
  const preview = previewOf(record);
  if (!preview) return <p className="hint">Nothing to draw: the export has no viewport outlines and the paper size was not measured.</p>;
  const used = KINDS.filter((k) => preview.boxes.some((b) => b.kind === k));
  const fontSize = Math.max(preview.width, preview.height) / 45;
  return (
    <figure className="preview">
      <svg viewBox={`-5 -5 ${preview.width + 10} ${preview.height + 10}`} role="img" aria-label="Sheet preview">
        <rect className="paper" x={0} y={0} width={preview.width} height={preview.height} />
        {preview.boxes.map((b, i) => (
          <g key={i} className={`port ${b.kind}`}>
            <rect x={b.x} y={b.y} width={b.w} height={b.h} />
            <text x={b.x + b.w / 2} y={b.y + b.h / 2} fontSize={fontSize} textAnchor="middle" dominantBaseline="central">
              {b.label}
            </text>
          </g>
        ))}
      </svg>
      <figcaption className="hint">
        {preview.measured ? `${Math.round(preview.width)} x ${Math.round(preview.height)} mm` : "Paper size not measured: outline drawn round the viewports"}
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
