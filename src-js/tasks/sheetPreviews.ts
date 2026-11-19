// What a task's sheets look like, as drawings the library's `SheetPreview` can draw: the sheets a set task lays out,
// or the sheet(s) a task copies. Pure, tested in sheetPreviews.test.ts.
//
// A create-RLS task copies its template's layout onto paper of the size it names, so its drawing is the template's
// viewports on that paper. A set task's sheets are its own layout (each view's centre and size), turned into the
// viewport shape the library preview reads. Nothing here places a view: the positions are the task's.

import type { SheetRecord, ViewportInfo } from "../library/types.js";
import type { RlsSetPlan, SetSheetView, Task } from "./types.js";

export interface TaskSheetPreview {
  title: string;
  record: Pick<SheetRecord, "size_mm" | "sheet">;
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

/** The drawings for `task`, one per sheet it makes or copies, in order; empty when it has none. */
export function taskSheetPreviews(task: Task): TaskSheetPreview[] {
  const set = task.rls_set;
  if (set) {
    const size_mm = set.paper_mm ? { width_mm: set.paper_mm[0], height_mm: set.paper_mm[1] } : paperOf(set.sheet_size);
    return set.sheets.map((s, i) => {
      const sheet = { view_ports: s.views.map((v) => setViewport(v, set)) };
      return {
        title: `Sheet ${i + 1} of ${set.sheets.length} (${set.sheet_size.toUpperCase()})`,
        record: size_mm ? { size_mm, sheet } : { sheet },
      };
    });
  }
  const asked = task.kind === "create_rls" ? paperOf(task.rls?.sheet_size) : undefined;
  return task.sheets.map((s) => {
    const size_mm = asked ?? s.size_mm;
    return {
      title: `${s.sheet_number}${s.sheet_name ? ` ${s.sheet_name}` : ""}${asked ? ` on ${String(task.rls?.sheet_size).toUpperCase()}` : ""}`,
      record: size_mm ? { size_mm, sheet: s.sheet as SheetRecord["sheet"] } : { sheet: s.sheet as SheetRecord["sheet"] },
    };
  });
}
