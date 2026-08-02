// The bundle's single public surface — the one thing `static/index.html` may
// reach for, published as the global `PlanRenderer`.
//
// Kept deliberately narrow. `index.html` is still ~4,000 lines of inline
// JavaScript and TypeScript cannot see those call sites, so every name exported
// here is an unchecked boundary. A small surface is the only thing that limits
// how much can go wrong across it; the `.d.ts` Vite emits at least describes it.
//
// The migration direction (CODING-CONVENTIONS.md, "`static/`"): each frontend
// change moves the module it touches into `src-js/`. This file grows as that
// happens — it is not meant to stay this size, but it is meant to stay
// deliberate.

export { resolveRoomAppearance, roomClassName, holeClassName } from "./appearance.js";
export {
  bounds,
  centroid,
  fittedBounds,
  flip,
  loopBox,
  pointsAttr,
  roomBBox,
} from "./geometry.js";
export { addLabel, paintLevel } from "./svg/paint.js";
export { GlPlanRenderer } from "./gl/renderer.js";
export { RoomIndex } from "./gl/spatial.js";
export { parseColour, readPalette, withAlpha } from "./gl/colour.js";

export type { PaintOptions } from "./svg/paint.js";
export type { GlRendererOptions } from "./gl/renderer.js";
export type { HighlightState, PaintRequest, PlanRenderer } from "./seam.js";
export type {
  AppearanceContext,
  ClassificationTier,
  Extent,
  Loop,
  Point2D,
  PropertyValue,
  Rect,
  Room,
  RoomAppearance,
  Size,
} from "./types.js";

/** Build stamp, so a stale committed bundle is visible from the console rather
 *  than inferred from behaviour. CI gates on a rebuild-and-diff, but a human
 *  debugging a checkout wants to ask the page directly. */
export const version = "1.0.0";
