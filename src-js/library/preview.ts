// The sheet preview's geometry: the sheet's outline and a box per viewport, in paper millimetres with y
// pointing down (SVG), so the component only draws. Pure, tested in preview.test.ts.

import type { PaperBox, SheetRecord, ViewportInfo } from "./types.js";

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
  /** The box reaches past the paper's edge (only judged when the paper size is measured). */
  clipped?: boolean;
  /** The view crop and the annotation crop round it, drawn dashed, when the view has one on (see `cropRectangles`). */
  crop?: { x: number; y: number; w: number; h: number };
  annotation?: { x: number; y: number; w: number; h: number };
}

/** An elevation's line drawn on the plan viewport, with the index its view has on its marker. */
export interface PreviewLine {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  label: string;
}

/** A 3D view's camera drawn on the plan viewport: a shaft from where it stands to the middle of its section box,
 * and the arrow head at that end as a polygon. */
export interface PreviewArrow {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  head: { x: number; y: number }[];
  label: string;
}

export interface Preview {
  /** The sheet rectangle. When the paper size was not measured it is the viewports' extent plus a margin. */
  width: number;
  height: number;
  measured: boolean;
  boxes: PreviewBox[];
  lines: PreviewLine[];
  arrows: PreviewArrow[];
}

const MARGIN_MM = 20;

/** How far past the paper's edge a box may stand before it counts as clipping: drawing and rounding noise. */
export const CLIP_TOLERANCE_MM = 0.5;

function labelOf(p: ViewportInfo, kind: PortKind): string {
  const scale = p.view?.view_scale;
  const name = KIND_LABEL[kind];
  let label = scale ? `${name} 1:${scale}` : name;
  if (kind === "schedule" && typeof p.view?.total_number_of_rows === "number") label += ` · ${p.view.total_number_of_rows} items`;
  if (kind === "elevation" && typeof p.view?.marker_index === "number") label += ` · #${p.view.marker_index}`;
  return label;
}

/**
 * A viewport's view crop and, round it, its annotation crop, paper mm with y up; null when the view has no annotation
 * crop on or its crop size cannot be worked out (no scale, or no crop box). The crop's size is exact: its box in the
 * model (the plan's crop, an elevation's width and height) over the view scale. Where it sits is NOT exported, so it is
 * centred on the viewport's box, which is the crop plus a margin (measured on House A's Rumpus sheet: about 6 mm on
 * the plan, 15 mm wide and 39 mm tall on the elevations); the annotation crop is the crop pushed out by the export's
 * offsets, each side its own.
 */
export function cropRectangles(p: ViewportInfo): { view: PaperBox; annotation: PaperBox } | null {
  const [b, c, o, scale] = [p.bounding_box, p.view?.bounding_box, p.view?.annotation_crop_offsets, p.view?.view_scale];
  if (!b || !c || !o || !scale || !p.view?.annotation_crop_active) return null;
  const [hw, hh] = [(c.max_x - c.min_x) / scale / 2, (c.max_y - c.min_y) / scale / 2];
  const [cx, cy] = [(b.min_x + b.max_x) / 2, (b.min_y + b.max_y) / 2];
  const view = { min_x: cx - hw, max_x: cx + hw, min_y: cy - hh, max_y: cy + hh };
  return { view, annotation: { min_x: view.min_x - o.left, max_x: view.max_x + o.right, min_y: view.min_y - o.bottom, max_y: view.max_y + o.top } };
}

/**
 * The elevation lines on the plan viewport, in paper mm with y up. The plan's crop box (model mm) is the
 * region its viewport shows, so a model point lands at the viewport's centre plus its offset from the crop's
 * centre divided by the view scale. Empty when the plan has no crop, scale or place, or no elevation has a line.
 */
type Mapping = (p: { x: number; y: number }) => { x: number; y: number };

/**
 * Model mm to paper mm (y up) on the plan viewport: the plan's crop box (model mm) is the region its viewport shows,
 * so a model point lands at the viewport's centre plus its offset from the crop's centre divided by the view scale.
 * Null when the plan has no crop, scale or place.
 */
function planMapping(ports: ViewportInfo[]): { onSheet: Mapping; scale: number } | null {
  const plan = ports.find((p) => kindOf(p.vp_type) === "plan" && p.view?.bounding_box && p.view.view_scale);
  if (!plan) return null;
  const crop = plan.view!.bounding_box!;
  const scale = plan.view!.view_scale!;
  const at = plan.centre_point ?? (plan.bounding_box && { x: (plan.bounding_box.min_x + plan.bounding_box.max_x) / 2, y: (plan.bounding_box.min_y + plan.bounding_box.max_y) / 2 });
  if (!at) return null;
  const cx = (crop.min_x + crop.max_x) / 2;
  const cy = (crop.min_y + crop.max_y) / 2;
  return { onSheet: (p) => ({ x: at.x + (p.x - cx) / scale, y: at.y + (p.y - cy) / scale }), scale };
}

/** The elevation lines on the plan viewport, in paper mm with y up. Empty when the plan cannot be mapped or no elevation has a line. */
export function elevationLines(ports: ViewportInfo[]): PreviewLine[] {
  const map = planMapping(ports);
  if (!map) return [];
  const lines: PreviewLine[] = [];
  for (const p of ports) {
    const v = p.view;
    if (kindOf(p.vp_type) !== "elevation" || !v?.line_start || !v.line_end) continue;
    const a = map.onSheet(v.line_start);
    const b = map.onSheet(v.line_end);
    lines.push({ x1: a.x, y1: a.y, x2: b.x, y2: b.y, label: typeof v.marker_index === "number" ? String(v.marker_index) : "?" });
  }
  return lines;
}

/**
 * Each 3D view's camera on the plan viewport, in paper mm with y up: an arrow along its direction in plan, ending at
 * the middle of its section box and starting half the box's diagonal back, which is where the view is seen from.
 * A view with no section box, or looking straight down, has none. Labelled `3D`, numbered when there are several.
 */
export function cameraArrows(ports: ViewportInfo[]): PreviewArrow[] {
  const map = planMapping(ports);
  if (!map) return [];
  const views = ports.filter((p) => kindOf(p.vp_type) === "three_d");
  const arrows: PreviewArrow[] = [];
  views.forEach((p, i) => {
    const v = p.view;
    const f = v?.forward_direction;
    const lo = v?.section_box_min;
    const hi = v?.section_box_max;
    if (!f || !lo || !hi) return;
    const len = Math.hypot(f.x, f.y);
    if (len < 1e-6) return;
    const [ux, uy] = [f.x / len, f.y / len];
    const back = Math.hypot(hi.x - lo.x, hi.y - lo.y) / 2;
    const centre = { x: (lo.x + hi.x) / 2, y: (lo.y + hi.y) / 2 };
    const tip = map.onSheet(centre);
    const tail = map.onSheet({ x: centre.x - ux * back, y: centre.y - uy * back });
    const size = Math.max(2, Math.hypot(tip.x - tail.x, tip.y - tail.y) * 0.12);
    // the head's two back corners, either side of the shaft
    const [bx, by] = [tip.x - ux * size, tip.y - uy * size];
    const [nx, ny] = [-uy * size * 0.5, ux * size * 0.5];
    arrows.push({
      x1: tail.x,
      y1: tail.y,
      x2: bx,
      y2: by,
      head: [tip, { x: bx + nx, y: by + ny }, { x: bx - nx, y: by - ny }],
      label: views.length > 1 ? `3D ${i + 1}` : "3D",
    });
  });
  return arrows;
}

function clips(b: { minX: number; maxX: number; minY: number; maxY: number }, width: number, height: number): boolean {
  const t = CLIP_TOLERANCE_MM;
  return b.minX < -t || b.minY < -t || b.maxX > width + t || b.maxY > height + t;
}

/** The preview of a sheet, or null when it has nothing to draw: no viewport has an outline and no paper size is known. */
export function previewOf(record: Pick<SheetRecord, "size_mm" | "sheet">): Preview | null {
  const ports = (record.sheet.view_ports ?? []).filter((p) => p.bounding_box);
  const boxes = ports.map((p) => {
    const b = p.bounding_box!;
    const kind = kindOf(p.vp_type);
    return { minX: b.min_x, maxX: b.max_x, minY: b.min_y, maxY: b.max_y, kind, label: labelOf(p, kind), crops: cropRectangles(p) };
  });
  let width = record.size_mm?.width_mm ?? 0;
  let height = record.size_mm?.height_mm ?? 0;
  const measured = width > 0 && height > 0;
  if (!measured) {
    if (boxes.length === 0) return null;
    width = Math.max(...boxes.map((b) => b.maxX)) + MARGIN_MM;
    height = Math.max(...boxes.map((b) => b.maxY)) + MARGIN_MM;
  }
  const rect = (r: PaperBox) => ({ x: r.min_x, y: height - r.max_y, w: r.max_x - r.min_x, h: r.max_y - r.min_y });
  return {
    width,
    height,
    measured,
    lines: elevationLines(record.sheet.view_ports ?? []).map((l) => ({ ...l, y1: height - l.y1, y2: height - l.y2 })),
    arrows: cameraArrows(record.sheet.view_ports ?? []).map((a) => ({
      ...a,
      y1: height - a.y1,
      y2: height - a.y2,
      head: a.head.map((h) => ({ x: h.x, y: height - h.y })),
    })),
    boxes: boxes.map((b) => ({
      x: b.minX,
      y: height - b.maxY,
      w: b.maxX - b.minX,
      h: b.maxY - b.minY,
      kind: b.kind,
      label: b.label,
      ...(measured && clips(b, width, height) && { clipped: true }),
      ...(b.crops && { crop: rect(b.crops.view), annotation: rect(b.crops.annotation) }),
    })),
  };
}
