// The zones' IMPERATIVE half: the renderer each one owns, and the view rect it
// is showing.
//
// **A view is not React state, and that is a performance decision with a
// measurement behind it.** A pan writes a new rect on every pointer move; held
// in the store, each of those would re-render every component on the page to
// change a number that only the renderer reads. The old page mutates
// `zone.view` and calls `setView`, and this keeps that: the store holds which
// zones exist and which level each shows (things a component renders), and the
// rect lives here, beside the renderer that consumes it.
//
// It is also what lets one zone's gesture reach another. "Link views" hands the
// same rect to every zone, which is a cross-component write with no common
// parent — a context would have to re-render the whole tree to do it.

import type { Rect } from "../../renderer/types.js";
import type { PlanRendererInstance } from "./planRenderer.js";

export interface ZoneHandle {
  readonly id: string;
  renderer: PlanRendererInstance;
  /** What the zone is showing. Mutated by pan and zoom. */
  view: Rect;
  /** The frame the level fits in — what a double-click returns to, and what
   *  every repaint that must not move the plan re-uses. */
  fitted: Rect;
}

const handles = new Map<string, ZoneHandle>();

export function register(handle: ZoneHandle): void {
  handles.set(handle.id, handle);
}

export function unregister(id: string): void {
  handles.delete(id);
}

export function handleOf(id: string): ZoneHandle | undefined {
  return handles.get(id);
}

export function allHandles(): ZoneHandle[] {
  return [...handles.values()];
}

/**
 * Show `view` in one zone, or in every zone when views are linked.
 *
 * Linked, each zone gets the SAME rect rather than recomputing the gesture's
 * anchor per zone — that is what "locked together" means, and recomputing
 * would make two zones drift apart under a zoom.
 */
export function commitView(originId: string, view: Rect, linked: boolean): void {
  if (!linked) {
    const zone = handles.get(originId);
    if (zone) {
      zone.view = view;
      zone.renderer.setView(view);
    }
    return;
  }
  for (const zone of handles.values()) {
    zone.view = { ...view };
    zone.renderer.setView(zone.view);
  }
}
