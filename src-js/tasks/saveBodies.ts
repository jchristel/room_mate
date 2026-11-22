// What a person's unsaved changes to a task's sheets are, and what to send to keep them: the viewports they moved
// (`Centres`, by key) and the headings they gave the 3D views (`Aims`). Pure, tested in sheetPreviews.test.ts.

import type { Aims } from "./camera.js";
import type { CameraAimsBody, Task, ViewportMovesBody } from "./types.js";

export type { Aims };

/** Viewport centres a person set, by key (`taskSheetPreviews`), paper mm from the bottom left, y up. */
export type Centres = ReadonlyMap<string, [number, number]>;

/** A create-RLS viewport's key: the TEMPLATE viewport's id, which is what a move is saved against. */
export const templateKey = (viewId: unknown): string | undefined => (typeof viewId === "number" ? `vp:${viewId}` : undefined);
/** A set's viewport key: its sheet and its place on it. */
export const setKey = (sheet: number, view: number): string => `set:${sheet}:${view}`;

/** The centres a `create_rls` task already holds (`rls.viewport_moves`), by key. */
export function savedCentres(task: Task): Map<string, [number, number]> {
  const out = new Map<string, [number, number]>();
  for (const m of task.rls?.viewport_moves ?? []) out.set(`vp:${m.view_id}`, m.centre_mm);
  return out;
}

/**
 * What to send to save `aims`: every heading the task keeps (what it holds with the aims over it), or null when there is
 * nothing to save. `{ cameras: [] }` puts every camera back to the template's direction.
 */
export function aimsBody(task: Task, aims: Aims): CameraAimsBody | null {
  if (aims.size === 0) return null;
  const merged = new Map<number, [number, number]>();
  for (const c of task.rls?.three_ds ?? []) if (c.direction) merged.set(c.index, c.direction);
  for (const [index, d] of aims) merged.set(index, d);
  return { cameras: [...merged].map(([index, direction]) => ({ index, direction })) };
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
