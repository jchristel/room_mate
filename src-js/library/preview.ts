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

export interface Preview {
  /** The sheet rectangle. When the paper size was not measured it is the viewports' extent plus a margin. */
  width: number;
  height: number;
  measured: boolean;
  boxes: PreviewBox[];
}

const MARGIN_MM = 20;

function labelOf(p: ViewportInfo, kind: PortKind): string {
  const scale = p.view?.view_scale;
  const name = KIND_LABEL[kind];
  return scale ? `${name} 1:${scale}` : name;
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
    boxes: boxes.map((b) => ({ x: b.minX, y: height - b.maxY, w: b.maxX - b.minX, h: b.maxY - b.minY, kind: b.kind, label: b.label })),
  };
}
