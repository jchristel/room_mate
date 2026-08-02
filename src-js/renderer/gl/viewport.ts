// Aspect-ratio fitting, extracted as a pure function because getting it wrong
// is invisible in code review and obvious on screen.
//
// An SVG `viewBox` does NOT stretch its contents. `preserveAspectRatio` defaults
// to `xMidYMid meet`: scale uniformly by whichever axis is more constrained, and
// centre the slack. Every part of this viewer's coordinate handling assumes that
// — `zone.view`, `fittedBounds`, the areas overlay, the selection marks — because
// all of them were written against an `<svg>` that behaves that way.
//
// A GL projection has no such default. Mapping the view rect straight onto clip
// space stretches the drawing to whatever shape the canvas is, which on a wide
// short zone makes every room visibly the wrong proportions. It also silently
// desynchronises the GL layer from the SVG overlay drawn on top of it, so a
// selection outline lands beside the room it names.
//
// This is the one place that correction lives. The projection uniform, the label
// transform and the pick all read it, so they cannot drift apart.

import type { Rect } from "../types.js";

/**
 * Widen `view` to the canvas's aspect ratio, keeping its centre — the rect that
 * is actually visible once `meet` letterboxing is applied.
 *
 * Returns `view` unchanged for a degenerate canvas or view, so a zone that has
 * not been laid out yet cannot produce NaN coordinates.
 */
export function fitViewToAspect(view: Rect, canvasW: number, canvasH: number): Rect {
  if (canvasW <= 0 || canvasH <= 0 || view.w <= 0 || view.h <= 0) return view;
  // `meet` fits INSIDE, so the smaller scale wins and the other axis gains
  // slack. `slice` would take the larger and crop the plan instead.
  const scale = Math.min(canvasW / view.w, canvasH / view.h);
  const w = canvasW / scale;
  const h = canvasH / scale;
  // xMidYMid: split the slack evenly, so the view's centre stays centred.
  return { x: view.x + view.w / 2 - w / 2, y: view.y + view.h / 2 - h / 2, w, h };
}
