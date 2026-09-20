// Pan, zoom and double-click-to-fit on one zone's plan.
//
// Imperative listeners on the overlay `<svg>` rather than React props, and for
// the reason the view rect is not React state: a pan fires on every pointer
// move and each one only has to reach the renderer. Going through a synthetic
// event and a state write would re-render the page per frame.
//
// **Both ends of a drag are converted by the RENDERER, and this is the bug
// this file exists to not have again.** `view` is the raw rect; what is on
// screen is that rect widened to the canvas's aspect ratio. Converting with
// the raw rect (`dx / width * view.w`) gets one axis right and the other wrong
// by whatever letterboxing added, so the plan slides out from under the pointer
// horizontally while tracking it perfectly vertically — which reads as a mouse
// problem rather than a projection one.

import { commitView, handleOf } from "./zoneRegistry.js";
import { clearSelection, closePickList, getState, openPickList, select } from "./store.js";
import type { Pick as PlanPick } from "../../renderer/seam.js";

/** Pointer travel below which a press-release is a CLICK, not a pan. Zero
 *  would make selection feel broken: a click on a trackpad drifts a pixel or
 *  two. */
export const CLICK_SLOP_PX = 4;

/**
 * What this zone's click and hover may reach: the stack, filtered by the
 * zone's selection filter.
 *
 * Drawn AND ticked. A layer that is off is not selectable whatever the filter
 * says, because a click must never resolve to something nobody can see — that
 * was true of the layer toggles before the filter existed and is the half of
 * the rule the filter does not replace.
 */
function pickable(zoneId: string, x: number, y: number): PlanPick[] {
  const handle = handleOf(zoneId);
  const zone = getState().zones.find((z) => z.id === zoneId);
  if (!handle || !zone) return [];
  return handle.renderer
    .pickAllAt(x, y)
    .filter((p) => zone.pickable[p.kind] && (p.kind === "room" ? zone.showRooms : zone.layers[entityOf(p.kind)]));
}

/** The layer a pick kind is drawn by. `room` has none — it is the base layer,
 *  behind its own toggle — so callers check that separately. */
function entityOf(kind: Exclude<PlanPick["kind"], "room">): "doors" | "windows" | "ffe" | "spaces" | "ceilings" | "floors" {
  switch (kind) {
    case "door":
      return "doors";
    case "window":
      return "windows";
    case "item":
      return "ffe";
    case "space":
      return "spaces";
    case "ceiling":
      return "ceilings";
    case "floor":
      return "floors";
  }
}

/** How an entry reads in the pick list: its type name, then what the element
 *  calls itself. The KIND leads because "which of the things under the pointer
 *  is this" is the question the list exists to answer. */
function labelOf(p: PlanPick): string {
  switch (p.kind) {
    case "room":
      return p.room.name || p.room.id;
    case "door":
      return p.door.type_name || p.door.id;
    case "window":
      return p.window.type_name || p.window.id;
    case "item":
      return (p.item.category ? `${p.item.category} · ` : "") + (p.item.type_name || p.item.id);
    case "space":
      return p.space.name || p.space.id;
    case "ceiling":
      return p.ceiling.type_name || p.ceiling.id;
    case "floor":
      return p.floor.type_name || p.floor.id;
  }
}

/** The id of whatever a pick found. */
function idOf(pick: PlanPick): string {
  switch (pick.kind) {
    case "room":
      return pick.room.id;
    case "door":
      return pick.door.id;
    case "window":
      return pick.window.id;
    case "item":
      return pick.item.id;
    case "space":
      return pick.space.id;
    case "ceiling":
      return pick.ceiling.id;
    case "floor":
      return pick.floor.id;
  }
}

export function wirePlanGestures(
  svg: SVGSVGElement,
  zoneId: string,
  tip: { current: HTMLDivElement | null },
): () => void {
  let dragging = false;
  let movedFar = false;
  let last = { x: 0, y: 0 };

  /** Where the press landed. The pick uses THESE coordinates, not the live
   *  ones: `setPointerCapture` retargets every later event to the svg, and by
   *  pointerup the pointer may have drifted within the slop. */
  let downAt = { x: 0, y: 0 };
  /** What the press landed ON, for the footprints: the areas overlay stamps
   *  each path with its group key, so a click on one resolves without a second
   *  hit test. Rooms are picked from raw coordinates instead — `paintLevel` is
   *  shared verbatim with the SVG export, and an export has no business
   *  carrying selection plumbing. */
  let downNode: Element | null = null;

  const onPointerDown = (e: PointerEvent) => {
    // A new press supersedes an open list: it is about to answer the same
    // question somewhere else.
    closePickList();
    dragging = true;
    movedFar = false;
    downAt = { x: e.clientX, y: e.clientY };
    downNode = e.target instanceof Element ? e.target : null;
    last = { x: e.clientX, y: e.clientY };
    svg.setPointerCapture(e.pointerId);
  };

  const onPointerUp = (e: PointerEvent) => {
    dragging = false;
    svg.releasePointerCapture?.(e.pointerId);
    if (movedFar) return; // a pan, not a click
    const handle = handleOf(zoneId);
    if (!handle) return;
    // Footprints win over everything beneath them, because that is what an
    // areas-on plan is SHOWING: the rooms below are ghosted to 0.16 precisely
    // so the overlay reads as the subject. Selecting a room under one means
    // turning that zone's overlay off, which is per zone.
    const areaKey = downNode?.closest?.(".area-poly")?.getAttribute("data-area-key");
    if (areaKey) {
      select("area", areaKey, zoneId);
      downNode = null;
      return;
    }
    downNode = null;
    const hits = pickable(zoneId, downAt.x, downAt.y);
    // ONE match selects it; SEVERAL open the list. Empty space clears the
    // selection, which is how a reader deselects without a second control.
    //
    // **The defaults DO list on a click straight onto an element**, because a
    // door, window or item sits inside a room and both kinds start ticked. That
    // is the filter's whole job: a reader who wants one-click FF&E unticks
    // Rooms in that zone, and one comparing an item against its room leaves
    // both on. Empty floor inside a room still selects the room directly --
    // only the room is under the pointer there.
    if (hits.length === 0) {
      clearSelection();
      return;
    }
    if (hits.length === 1) {
      select(hits[0]!.kind, idOf(hits[0]!), zoneId);
      return;
    }
    openPickList({
      zoneId,
      at: { x: downAt.x, y: downAt.y },
      entries: hits.map((p) => ({ kind: p.kind, id: idOf(p), label: labelOf(p) })),
    });
  };

  /**
   * Hover: the mark, and the tooltip that replaced the SVG `<title>`.
   *
   * Throttled to an animation frame. A pointermove fires far more often than
   * the screen updates, and an unthrottled pick plus DOM write per event is
   * real work for a result nobody sees.
   */
  let hoverRaf = 0;
  let hoverAt: { x: number; y: number } | null = null;

  const showTip = (text: string | null, at: { x: number; y: number } | null) => {
    const el = tip.current;
    if (!el) return;
    if (!text || !at) {
      el.classList.add("hidden");
      return;
    }
    const rect = svg.getBoundingClientRect();
    el.textContent = text;
    el.style.left = `${at.x - rect.left}px`;
    el.style.top = `${at.y - rect.top}px`;
    el.classList.remove("hidden");
  };

  const clearHover = () => {
    if (hoverRaf) cancelAnimationFrame(hoverRaf);
    hoverRaf = 0;
    hoverAt = null;
    handleOf(zoneId)?.renderer.setHover(null);
    showTip(null, null);
  };

  const updateHover = (e: PointerEvent) => {
    hoverAt = { x: e.clientX, y: e.clientY };
    if (hoverRaf) return;
    hoverRaf = requestAnimationFrame(() => {
      hoverRaf = 0;
      const at = hoverAt;
      const handle = handleOf(zoneId);
      if (!at || !handle) return;
      // Hover marks the FIRST entry of the same filtered stack a click would
      // use, so hover and click can never disagree about what is under the
      // pointer.
      const first = pickable(zoneId, at.x, at.y)[0] ?? null;
      handle.renderer.setHover(first ? { kind: first.kind, id: idOf(first) } : null);
      const room = first?.kind === "room" ? first.room : null;
      // The same text the `<title>` carried -- name, else id.
      showTip(room ? room.name || room.id : null, room ? at : null);
    });
  };

  const onPointerMove = (e: PointerEvent) => {
    if (!dragging) {
      updateHover(e);
      return;
    }
    const handle = handleOf(zoneId);
    if (!handle) return;
    if (!movedFar && Math.hypot(e.clientX - downAt.x, e.clientY - downAt.y) > CLICK_SLOP_PX) movedFar = true;
    // Both ends in WORLD units, from the renderer, and subtracted. Both read
    // the same view — nothing has committed yet — so the difference is exact
    // rather than an approximation of it.
    const to = handle.renderer.toWorld(e.clientX, e.clientY);
    const from = handle.renderer.toWorld(last.x, last.y);
    last = { x: e.clientX, y: e.clientY };
    if (!to || !from) return;
    const v = { ...handle.view };
    v.x -= to.x - from.x;
    v.y -= to.y - from.y;
    commitView(zoneId, v, getState().linkViews);
    // A pan is not a hover. Dropping the mark here also stops the tooltip
    // trailing the pointer across a drag, which reads as a stuck overlay.
    clearHover();
  };

  const onWheel = (e: WheelEvent) => {
    e.preventDefault();
    const handle = handleOf(zoneId);
    if (!handle) return;
    // The anchor comes from the renderer for the reason the pan's ends do: the
    // point under the cursor is only findable through the aspect-corrected
    // view. Scaling the RAW view about that point is still right — the fit is
    // centre-preserving and uniform, so the corrected view scales by the same
    // factor about the same point.
    const m = handle.renderer.toWorld(e.clientX, e.clientY);
    if (!m) return;
    const v = { ...handle.view };
    const k = e.deltaY > 0 ? 1.1 : 0.9;
    v.x = m.x - (m.x - v.x) * k;
    v.y = m.y - (m.y - v.y) * k;
    v.w *= k;
    v.h *= k;
    commitView(zoneId, v, getState().linkViews);
  };

  const onDoubleClick = () => {
    const handle = handleOf(zoneId);
    if (handle) commitView(zoneId, { ...handle.fitted }, getState().linkViews);
  };

  // The pointer LEAVING must clear both, or the last hovered room stays lit
  // while the cursor is somewhere else entirely.
  const onPointerLeave = () => clearHover();

  svg.addEventListener("pointerleave", onPointerLeave);
  svg.addEventListener("pointerdown", onPointerDown);
  svg.addEventListener("pointerup", onPointerUp);
  svg.addEventListener("pointermove", onPointerMove);
  svg.addEventListener("wheel", onWheel, { passive: false });
  svg.addEventListener("dblclick", onDoubleClick);

  return () => {
    clearHover();
    svg.removeEventListener("pointerleave", onPointerLeave);
    svg.removeEventListener("pointerdown", onPointerDown);
    svg.removeEventListener("pointerup", onPointerUp);
    svg.removeEventListener("pointermove", onPointerMove);
    svg.removeEventListener("wheel", onWheel);
    svg.removeEventListener("dblclick", onDoubleClick);
  };
}
