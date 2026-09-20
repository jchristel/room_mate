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
import { getState } from "./store.js";

/** Pointer travel below which a press-release is a CLICK, not a pan. Zero
 *  would make selection feel broken: a click on a trackpad drifts a pixel or
 *  two. Selection itself arrives in B5; the threshold is here because it is
 *  what tells a gesture from a click. */
export const CLICK_SLOP_PX = 4;

export function wirePlanGestures(svg: SVGSVGElement, zoneId: string): () => void {
  let dragging = false;
  let movedFar = false;
  let last = { x: 0, y: 0 };

  const onPointerDown = (e: PointerEvent) => {
    dragging = true;
    movedFar = false;
    last = { x: e.clientX, y: e.clientY };
    svg.setPointerCapture(e.pointerId);
  };

  const onPointerUp = (e: PointerEvent) => {
    dragging = false;
    svg.releasePointerCapture?.(e.pointerId);
  };

  const onPointerMove = (e: PointerEvent) => {
    if (!dragging) return;
    const handle = handleOf(zoneId);
    if (!handle) return;
    if (!movedFar && Math.hypot(e.clientX - last.x, e.clientY - last.y) > CLICK_SLOP_PX) movedFar = true;
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

  svg.addEventListener("pointerdown", onPointerDown);
  svg.addEventListener("pointerup", onPointerUp);
  svg.addEventListener("pointermove", onPointerMove);
  svg.addEventListener("wheel", onWheel, { passive: false });
  svg.addEventListener("dblclick", onDoubleClick);

  return () => {
    svg.removeEventListener("pointerdown", onPointerDown);
    svg.removeEventListener("pointerup", onPointerUp);
    svg.removeEventListener("pointermove", onPointerMove);
    svg.removeEventListener("wheel", onWheel);
    svg.removeEventListener("dblclick", onDoubleClick);
  };
}
