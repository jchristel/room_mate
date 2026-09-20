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
import { clearSelection, getState, select } from "./store.js";
import type { Pick as PlanPick } from "../../renderer/seam.js";

/** Pointer travel below which a press-release is a CLICK, not a pan. Zero
 *  would make selection feel broken: a click on a trackpad drifts a pixel or
 *  two. */
export const CLICK_SLOP_PX = 4;

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

  const onPointerDown = (e: PointerEvent) => {
    dragging = true;
    movedFar = false;
    downAt = { x: e.clientX, y: e.clientY };
    last = { x: e.clientX, y: e.clientY };
    svg.setPointerCapture(e.pointerId);
  };

  const onPointerUp = (e: PointerEvent) => {
    dragging = false;
    svg.releasePointerCapture?.(e.pointerId);
    if (movedFar) return; // a pan, not a click
    const handle = handleOf(zoneId);
    if (!handle) return;
    const hit = handle.renderer.pickAt(downAt.x, downAt.y);
    // Empty space CLEARS the selection, which is how a reader deselects
    // without a second control.
    if (hit) select(hit.kind, idOf(hit), zoneId);
    else clearSelection();
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
      const room = handle.renderer.roomAt(at.x, at.y);
      handle.renderer.setHover(room ? { kind: "room", id: room.id } : null);
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
