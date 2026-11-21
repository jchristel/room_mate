// What a task's sheets look like, as drawings the library's `SheetPreview` can draw: the sheets a set task lays out,
// or the sheet(s) a task copies. Pure, tested in sheetPreviews.test.ts.
//
// A create-RLS task copies its template's layout onto paper of the size it names, so its drawing is the template's
// viewports on that paper, each sized for the task's room (`fitToRoom`) and centred where the template had it. A set task's sheets are its own layout (each view's centre and size), turned into
// the viewport shape the library preview reads. Nothing here places a view: the positions are the task's or the template's.

import type { SheetRecord, ViewportInfo } from "../library/types.js";
import { kindOf, previewOf } from "../library/preview.js";
import type { RlsPlan, RlsSetPlan, SetSheetView, Task, ViewportMovesBody } from "./types.js";

export interface TaskSheetPreview {
  title: string;
  record: Pick<SheetRecord, "size_mm" | "sheet">;
  /** Each viewport's key, in the order of `record.sheet.view_ports`: what a move is saved against. */
  keys: (string | undefined)[];
}

/** Viewport centres a person set, by key (`taskSheetPreviews`), paper mm from the bottom left, y up. */
export type Centres = ReadonlyMap<string, [number, number]>;

const templateKey = (viewId: unknown): string | undefined => (typeof viewId === "number" ? `vp:${viewId}` : undefined);
const setKey = (sheet: number, view: number): string => `set:${sheet}:${view}`;

/** `p` with its box (size kept) centred on `centre`; its centre point follows, which the elevation lines and cameras drawn on the plan read. */
function placedAt(p: ViewportInfo, centre: [number, number]): ViewportInfo {
  const box = p.bounding_box;
  if (!box) return p;
  const [w, h] = [box.max_x - box.min_x, box.max_y - box.min_y];
  return {
    ...p,
    bounding_box: { min_x: centre[0] - w / 2, max_x: centre[0] + w / 2, min_y: centre[1] - h / 2, max_y: centre[1] + h / 2 },
    centre_point: { x: centre[0], y: centre[1] },
  };
}

/** A size token's paper, landscape, mm (the server's `paper_of`). */
const PAPER_MM: Record<string, [number, number]> = {
  a0: [1189, 841],
  a1: [841, 594],
  a2: [594, 420],
  a3: [420, 297],
};

export function paperOf(size: string | undefined): { width_mm: number; height_mm: number } | undefined {
  const p = PAPER_MM[(size ?? "").trim().toLowerCase()];
  return p ? { width_mm: p[0], height_mm: p[1] } : undefined;
}

const VP_TYPE: Record<SetSheetView["kind"], string> = {
  plan: "floor plan",
  elevation: "elevation",
  schedule: "schedule",
  three_d: "three d",
};

function setViewport(view: SetSheetView, plan: RlsSetPlan): ViewportInfo {
  const [cx, cy] = view.centre_mm;
  const [w, h] = view.size_mm;
  let scale: number | undefined;
  if (view.kind === "plan") scale = plan.plan?.scale;
  if (view.kind === "elevation") {
    scale = plan.markers
      ?.find((m) => m.room_id === view.room_id)
      ?.views.find((v) => v.marker_index === view.marker_index)?.scale;
  }
  return {
    vp_type: VP_TYPE[view.kind],
    bounding_box: { min_x: cx - w / 2, min_y: cy - h / 2, max_x: cx + w / 2, max_y: cy + h / 2 },
    centre_point: { x: cx, y: cy },
    view: { view_scale: scale, marker_index: view.marker_index },
  } as ViewportInfo;
}

type Box = NonNullable<ViewportInfo["bounding_box"]>;

/** `box` grown (or shrunk) about its centre to `w` x `h`, so a view keeps the place the template gave it. */
function resized(box: Box, w: number, h: number): Box {
  const cx = (box.min_x + box.max_x) / 2;
  const cy = (box.min_y + box.max_y) / 2;
  return { min_x: cx - w / 2, max_x: cx + w / 2, min_y: cy - h / 2, max_y: cy + h / 2 };
}

type Vec = [number, number, number];

const unit = (v: Vec): Vec => {
  const n = Math.hypot(...v) || 1;
  return [v[0] / n, v[1] / n, v[2] / n];
};

/**
 * How wide and tall (model mm) a box of `size` looks from a camera with these directions: the extent of its eight
 * corners along the view's right and up, which is what the plug-in crops the new 3D view to (duHast `_crop_to_box`).
 * Only the box's size matters, not where it stands, and a plan turned with its box and camera gives the same answer.
 */
export function boxExtent(forward: Vec, up: Vec, size: Vec): [number, number] {
  const f = unit(forward);
  const u = unit(up);
  const r: Vec = [f[1] * u[2] - f[2] * u[1], f[2] * u[0] - f[0] * u[2], f[0] * u[1] - f[1] * u[0]];
  const along = (axis: Vec) => Math.abs(axis[0]) * size[0] + Math.abs(axis[1]) * size[1] + Math.abs(axis[2]) * size[2];
  return [along(r), along(u)];
}

/**
 * The template 3D viewport sized for the task's footprint: the new view is cropped tight round its section box, whose
 * plan is the task's plan crop and whose height is the template's, seen along the template's camera. The paper size
 * is that extent over the view scale plus whatever the template's box had beyond its own crop (about 6 mm on House
 * A's sheets). The export gives a 3D view no scale (0), so the sheet's plan scale stands in for it: 50H sheets
 * draw their 3D view at the same scale as the plan. Null when any of those is unknown.
 */
function threeDBox(p: ViewportInfo, rls: RlsPlan, scale: number | undefined): Box | null {
  const [v, box, crop, footprint] = [p.view, p.bounding_box, p.view?.bounding_box, rls.plan_crop];
  const [lo, hi, f, u] = [v?.section_box_min, v?.section_box_max, v?.forward_direction, v?.up_direction];
  if (!box || !crop || !footprint || !lo || !hi || !f || !u || !scale) return null;
  const [w, h] = boxExtent(
    [f.x, f.y, f.z ?? 0],
    [u.x, u.y, u.z ?? 0],
    [footprint.max[0] - footprint.min[0], footprint.max[1] - footprint.min[1], (hi.z ?? 0) - (lo.z ?? 0)],
  );
  const margin = [box.max_x - box.min_x - (crop.max_x - crop.min_x) / scale, box.max_y - box.min_y - (crop.max_y - crop.min_y) / scale];
  return resized(box, w / scale + margin[0]!, h / scale + margin[1]!);
}

/**
 * The template's viewports sized for THIS room, each still centred where the template put it. The plan follows the
 * task's crop and an elevation the width of its new line; both change by the model difference over the view scale,
 * added to the template's own box, so what the template drew round the view (title, annotation crop) is kept. An
 * elevation's height is the template's (it is cropped to the template's height), and a schedule's size is Revit's.
 * The plan's crop, the elevation lines (and their crop's width) and each 3D camera are replaced with the task's so the lines and arrows the
 * preview draws on the plan agree with the boxes. A 3D view is sized by `threeDBox`.
 */
export function fitToRoom(sheet: SheetRecord["sheet"], rls: RlsPlan | undefined): SheetRecord["sheet"] {
  if (!rls || !sheet.view_ports) return sheet;
  const lines = new Map<number, [[number, number], [number, number]]>();
  for (const m of rls.markers ?? []) for (const v of m.views) lines.set(v.view_id, [v.line_start, v.line_end]);
  const cameras = [...(rls.three_ds ?? [])].sort((a, b) => a.index - b.index);
  let threeD = 0;
  const planScale = sheet.view_ports.find((p) => kindOf(p.vp_type) === "plan")?.view?.view_scale || undefined;
  const view_ports = sheet.view_ports.map((p) => {
    const kind = kindOf(p.vp_type);
    const scale = p.view?.view_scale;
    const box = p.bounding_box;
    if (kind === "three_d") {
      const cam = cameras.find((c) => c.index === threeD++);
      if (!cam || !p.view) return p;
      const sized = threeDBox(p, rls, scale || planScale);
      // the preview draws a camera from its section box and direction: put the box where the task looks, half a
      // diagonal (the eye's distance) across, so the arrow starts at the eye and ends at the room
      const [dx, dy] = [cam.looks_at[0] - cam.eye[0], cam.looks_at[1] - cam.eye[1]];
      const half = Math.hypot(dx, dy) / Math.SQRT2;
      const [lo, hi] = [
        { x: cam.looks_at[0] - half, y: cam.looks_at[1] - half },
        { x: cam.looks_at[0] + half, y: cam.looks_at[1] + half },
      ];
      return {
        ...p,
        ...(sized && { bounding_box: sized }),
        view: { ...p.view, forward_direction: { x: dx, y: dy, z: 0 }, section_box_min: lo, section_box_max: hi },
      };
    }
    if (kind === "plan" && rls.plan_crop && p.view?.bounding_box) {
      const [old, crop] = [p.view.bounding_box, rls.plan_crop];
      const view = { ...p.view, bounding_box: { min_x: crop.min[0], min_y: crop.min[1], max_x: crop.max[0], max_y: crop.max[1] } };
      if (!box || !scale) return { ...p, view };
      const dw = (crop.max[0] - crop.min[0] - (old.max_x - old.min_x)) / scale;
      const dh = (crop.max[1] - crop.min[1] - (old.max_y - old.min_y)) / scale;
      return { ...p, view, bounding_box: resized(box, box.max_x - box.min_x + dw, box.max_y - box.min_y + dh) };
    }
    const line = kind === "elevation" ? lines.get((p as { view_id?: number }).view_id ?? NaN) : undefined;
    if (line && p.view) {
      const [[x0, y0], [x1, y1]] = line;
      const view = { ...p.view, line_start: { x: x0, y: y0 }, line_end: { x: x1, y: y1 } };
      // the crop's width is the line's length (its height is the template's), kept about its own middle
      const crop = p.view.bounding_box;
      if (crop) {
        const half = Math.hypot(x1 - x0, y1 - y0) / 2;
        const mid = (crop.min_x + crop.max_x) / 2;
        view.bounding_box = { ...crop, min_x: mid - half, max_x: mid + half };
      }
      const [a, b] = [p.view.line_start, p.view.line_end];
      if (!box || !scale || !a || !b) return { ...p, view };
      const dw = (Math.hypot(x1 - x0, y1 - y0) - Math.hypot(b.x - a.x, b.y - a.y)) / scale;
      return { ...p, view, bounding_box: resized(box, box.max_x - box.min_x + dw, box.max_y - box.min_y) };
    }
    return p;
  });
  return { ...sheet, view_ports };
}

/** The centres a `create_rls` task already holds (`rls.viewport_moves`), by key. */
export function savedCentres(task: Task): Map<string, [number, number]> {
  const out = new Map<string, [number, number]>();
  for (const m of task.rls?.viewport_moves ?? []) out.set(`vp:${m.view_id}`, m.centre_mm);
  return out;
}

/**
 * The drawings for `task`, one per sheet it makes or copies, in order; empty when it has none. `draft` holds centres a
 * person has set and not saved; they go over the ones the task holds. A set's saved centres are in its views already.
 */
export function taskSheetPreviews(task: Task, draft?: Centres): TaskSheetPreview[] {
  const set = task.rls_set;
  if (set) {
    const size_mm = set.paper_mm ? { width_mm: set.paper_mm[0], height_mm: set.paper_mm[1] } : paperOf(set.sheet_size);
    return set.sheets.map((s, i) => {
      const keys = s.views.map((_, j) => setKey(i, j));
      const sheet = {
        view_ports: s.views.map((v, j) => {
          const moved = draft?.get(keys[j]!);
          return setViewport(moved ? { ...v, centre_mm: moved } : v, set);
        }),
      };
      return {
        title: `Sheet ${i + 1} of ${set.sheets.length} (${set.sheet_size.toUpperCase()})`,
        record: size_mm ? { size_mm, sheet } : { sheet },
        keys,
      };
    });
  }
  const asked = task.kind === "create_rls" ? paperOf(task.rls?.sheet_size) : undefined;
  const centres = new Map([...savedCentres(task), ...(draft ?? [])]);
  return task.sheets.map((s) => {
    const size_mm = asked ?? s.size_mm;
    const fitted = (task.kind === "create_rls" ? fitToRoom(s.sheet as SheetRecord["sheet"], task.rls) : s.sheet) as SheetRecord["sheet"];
    const keys = (fitted.view_ports ?? []).map((p) => templateKey((p as { view_id?: unknown }).view_id));
    // the viewport keeps the size the room gave it and goes where it was put
    const view_ports = fitted.view_ports?.map((p, i) => {
      const at = task.kind === "create_rls" && keys[i] ? centres.get(keys[i]!) : undefined;
      return at ? placedAt(p, at) : p;
    });
    const sheet = (view_ports ? { ...fitted, view_ports } : fitted) as SheetRecord["sheet"];
    return {
      title: `${s.sheet_number}${s.sheet_name ? ` ${s.sheet_name}` : ""}${asked ? ` on ${String(task.rls?.sheet_size).toUpperCase()}` : ""}`,
      record: size_mm ? { size_mm, sheet } : { sheet },
      keys,
    };
  });
}

/**
 * What to send to save `draft`: a create-RLS task's whole list of moved viewports (what it holds with the draft over
 * it), a set's changed views. Null when there is nothing to save. `[]` for a create-RLS task is the way to put every
 * viewport back, so that is `clearMoves`, not this.
 */
export function movesBody(task: Task, draft: Centres): ViewportMovesBody | null {
  if (draft.size === 0) return null;
  if (task.rls_set) {
    const set_moves = [...draft].flatMap(([key, centre_mm]) => {
      const m = /^set:(\d+):(\d+)$/.exec(key);
      return m ? [{ sheet: Number(m[1]), view: Number(m[2]), centre_mm }] : [];
    });
    return set_moves.length > 0 ? { set_moves } : null;
  }
  const merged = new Map([...savedCentres(task), ...draft]);
  const moves = [...merged].flatMap(([key, centre_mm]) => {
    const m = /^vp:(-?\d+)$/.exec(key);
    return m ? [{ view_id: Number(m[1]), centre_mm }] : [];
  });
  return moves.length > 0 ? { moves } : null;
}

/** Whether the task's sheets fit their paper: `clipping` when any viewport reaches past the edge. `unknown` for a task
 * with no sheet drawing or no measured paper, which says nothing either way. */
export interface SheetStatus {
  state: "ok" | "clipping" | "unknown";
  /** Viewports past the edge, over every sheet of the task. */
  clipped: number;
}

export function sheetStatus(task: Task, draft?: Centres): SheetStatus {
  let measured = 0;
  let clipped = 0;
  for (const p of taskSheetPreviews(task, draft)) {
    const preview = previewOf(p.record);
    if (!preview?.measured) continue;
    measured += 1;
    clipped += preview.boxes.filter((b) => b.clipped).length;
  }
  return { state: measured === 0 ? "unknown" : clipped > 0 ? "clipping" : "ok", clipped };
}
