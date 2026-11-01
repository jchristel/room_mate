// The sheet preview's geometry: the sheet's outline and a box per viewport, in paper millimetres with y
// pointing down (SVG), so the component only draws. Pure, tested in preview.test.ts.

import type { SheetRecord, ViewportInfo } from "./types.js";

export type PortKind = "plan" | "elevation" | "three_d" | "schedule" | "other";

export const KIND_LABEL: Record<PortKind, string> = {
  plan: "Plan",
  elevation: "Elevation",
  three_d: "3D",
  schedule: "Schedule",
  other: "Other",
};

export function kindOf(vpType: string | undefined): PortKind {
  switch ((vpType ?? "").trim().toLowerCase()) {
    case "floor plan":
      return "plan";
    case "elevation":
      return "elevation";
    case "three d":
      return "three_d";
    case "schedule":
      return "schedule";
    default:
      return "other";
  }
}

export interface PreviewBox {
  x: number;
  y: number;
  w: number;
  h: number;
  kind: PortKind;
  label: string;
}

/** An elevation's line drawn on the plan viewport, with the index its view has on its marker. */
export interface PreviewLine {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  label: string;
}

export interface Preview {
  /** The sheet rectangle. When the paper size was not measured it is the viewports' extent plus a margin. */
  width: number;
  height: number;
  measured: boolean;
  boxes: PreviewBox[];
  lines: PreviewLine[];
}

const MARGIN_MM = 20;

function labelOf(p: ViewportInfo, kind: PortKind): string {
  const scale = p.view?.view_scale;
  const name = KIND_LABEL[kind];
  let label = scale ? `${name} 1:${scale}` : name;
  if (kind === "schedule" && typeof p.view?.total_number_of_rows === "number") label += ` · ${p.view.total_number_of_rows} items`;
  if (kind === "elevation" && typeof p.view?.marker_index === "number") label += ` · #${p.view.marker_index}`;
  return label;
}

/**
 * The elevation lines on the plan viewport, in paper mm with y up. The plan's crop box (model mm) is the
 * region its viewport shows, so a model point lands at the viewport's centre plus its offset from the crop's
 * centre divided by the view scale. Empty when the plan has no crop, scale or place, or no elevation has a line.
 */
export function elevationLines(ports: ViewportInfo[]): PreviewLine[] {
  const plan = ports.find((p) => kindOf(p.vp_type) === "plan" && p.view?.bounding_box && p.view.view_scale);
  if (!plan) return [];
  const crop = plan.view!.bounding_box!;
  const scale = plan.view!.view_scale!;
  const at = plan.centre_point ?? (plan.bounding_box && { x: (plan.bounding_box.min_x + plan.bounding_box.max_x) / 2, y: (plan.bounding_box.min_y + plan.bounding_box.max_y) / 2 });
  if (!at) return [];
  const cx = (crop.min_x + crop.max_x) / 2;
  const cy = (crop.min_y + crop.max_y) / 2;
  const onSheet = (p: { x: number; y: number }) => ({ x: at.x + (p.x - cx) / scale, y: at.y + (p.y - cy) / scale });
  const lines: PreviewLine[] = [];
  for (const p of ports) {
    const v = p.view;
    if (kindOf(p.vp_type) !== "elevation" || !v?.line_start || !v.line_end) continue;
    const a = onSheet(v.line_start);
    const b = onSheet(v.line_end);
    lines.push({ x1: a.x, y1: a.y, x2: b.x, y2: b.y, label: typeof v.marker_index === "number" ? String(v.marker_index) : "?" });
  }
  return lines;
}

/** The preview of a sheet, or null when it has nothing to draw: no viewport has an outline and no paper size is known. */
export function previewOf(record: Pick<SheetRecord, "size_mm" | "sheet">): Preview | null {
  const ports = (record.sheet.view_ports ?? []).filter((p) => p.bounding_box);
  const boxes = ports.map((p) => {
    const b = p.bounding_box!;
    const kind = kindOf(p.vp_type);
    return { minX: b.min_x, maxX: b.max_x, minY: b.min_y, maxY: b.max_y, kind, label: labelOf(p, kind) };
  });
  let width = record.size_mm?.width_mm ?? 0;
  let height = record.size_mm?.height_mm ?? 0;
  const measured = width > 0 && height > 0;
  if (!measured) {
    if (boxes.length === 0) return null;
    width = Math.max(...boxes.map((b) => b.maxX)) + MARGIN_MM;
    height = Math.max(...boxes.map((b) => b.maxY)) + MARGIN_MM;
  }
  return {
    width,
    height,
    measured,
    lines: elevationLines(record.sheet.view_ports ?? []).map((l) => ({ ...l, y1: height - l.y1, y2: height - l.y2 })),
    boxes: boxes.map((b) => ({ x: b.minX, y: height - b.maxY, w: b.maxX - b.minX, h: b.maxY - b.minY, kind: b.kind, label: b.label })),
  };
}
