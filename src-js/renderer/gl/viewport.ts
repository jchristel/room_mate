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

/** Where the label container sits: CSS pixels per world unit, and the offset
 *  that lands the view's top-left corner on the canvas's. */
export interface LabelTransform {
  scale: number;
  x: number;
  y: number;
}

/**
 * The label container's transform, from the ALREADY aspect-corrected view.
 *
 * `cssW` is the canvas's width in CSS pixels, which is also the unit Pixi's
 * stage is in — the resolution is applied below the stage, when it maps onto
 * the drawing buffer.
 *
 * SO THERE IS NO DPR PARAMETER, and that is the property worth stating rather
 * than an omission. It shipped with one: `pxW / dpr / eff.w`, written against a
 * belief that `renderer.width` counts device pixels (it does not — it is
 * `texture.frame.width`, the logical size). At DPR 1 the two readings agree and
 * everything looks right, which is why it survived; at any other DPR every
 * label landed at 1/DPR of its correct distance from the canvas corner, so a
 * room's name sat several rooms away from the room. A signature with no DPR in
 * it cannot express that mistake again.
 *
 * One scale for both axes: `view` is already aspect-corrected, so the two
 * axes agree by construction, and taking each axis separately would stretch the
 * glyphs even with the geometry correct.
 */
export function labelTransform(view: Rect, cssW: number): LabelTransform {
  const scale = view.w > 0 ? cssW / view.w : 1;
  return { scale, x: -view.x * scale, y: -view.y * scale };
}
