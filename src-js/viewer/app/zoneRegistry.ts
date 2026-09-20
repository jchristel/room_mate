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

import { closePickList, getState } from "./store.js";
import { fittedBounds } from "./planRenderer.js";
import { UNKNOWN_LEVEL } from "../levels.js";
import { viewCentredOn } from "../panTo.js";
import type { Rect, Room } from "../../renderer/types.js";
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
  // A pan or a zoom moves the plan out from under an open pick list, whose
  // entries name what was at a point that is no longer there.
  closePickList();
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

/**
 * Bring a room into view in every zone that draws it (G3).
 *
 * **Per zone, each deciding on its own**, because two zones may be showing
 * different storeys at different scales and a room that is already on screen in
 * one of them must not be dragged around to satisfy the other. A zone showing
 * another storey is not moved at all: there is nothing there to centre.
 *
 * **Linked views are the exception, and they have to be.** "Link views" means
 * one rect for every zone, so deciding per zone would immediately break the
 * invariant the reader switched on — the first commit broadcasts and the next
 * zone's answer fights it. Linked, the decision is taken once, from the first
 * zone that shows the room, and every zone follows it.
 *
 * Nothing here zooms: `viewCentredOn` keeps the view's size, and a zone where
 * the room is already wholly visible gets no commit at all.
 */
export function panToRoom(room: Room): void {
  const { zones, linkViews } = getState();
  // The room's own extent, with `fittedBounds`' 4% margin — which is wanted
  // here: a room flush against the frame edge reads as clipped, so the margin
  // is what "wholly inside" should mean rather than an artefact of reusing it.
  const target = fittedBounds([room]);
  if (!target) return; // a room with no loops: selectable, not locatable
  const storey = room.level_id || UNKNOWN_LEVEL;
  for (const zone of zones) {
    if (zone.levelId !== storey) continue;
    const handle = handles.get(zone.id);
    if (!handle) continue;
    const next = viewCentredOn(handle.view, target);
    if (!next) {
      // Already visible here. Under linked views that settles it for every
      // zone, since they all show the same rect.
      if (linkViews) return;
      continue;
    }
    commitView(zone.id, next, linkViews);
    if (linkViews) return;
  }
}
