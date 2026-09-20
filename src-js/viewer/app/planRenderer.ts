// The renderer bundle, typed.
//
// **Reached through the global, not imported.** `static/vendor/renderer.bundle.js`
// is an IIFE that publishes `PlanRenderer`, and both pages load it with a
// classic `<script>`. Importing `src-js/renderer/` from here instead would
// bundle Pixi a second time into `viewer.js` — two copies of a WebGL renderer
// in one page, and two committed artifacts that can disagree about how a plan
// is drawn.
//
// The TYPES are imported directly, which costs nothing: `import type` is erased,
// so the seam's contract is checked at build time while the runtime still comes
// from the one bundle.

import type { GlRendererOptions } from "../../renderer/gl/renderer.js";
import type { PlanRenderer as PlanRendererSeam } from "../../renderer/seam.js";
import type { Rect, Room } from "../../renderer/types.js";

/** What this page uses from the bundle. A narrow view of a wider surface, for
 *  the reason the bundle's own index gives: every name here is an unchecked
 *  boundary, so it grows one slice at a time. */
interface RendererBundle {
  GlPlanRenderer: new (canvas: HTMLCanvasElement, opts?: GlRendererOptions) => PlanRendererSeam & {
    ready: Promise<void>;
    destroy(): void;
  };
  fittedBounds(rooms: readonly Room[]): Rect | null;
}

const bundle = (globalThis as unknown as { PlanRenderer?: RendererBundle }).PlanRenderer;
if (!bundle) {
  // A missing bundle is a page that loads its scripts in the wrong order, and
  // the symptom otherwise is a viewer that draws nothing — indistinguishable
  // from a payload problem, which cost real time to work out once already.
  throw new Error("renderer bundle missing: /vendor/renderer.bundle.js must load before this module");
}

export const { GlPlanRenderer, fittedBounds } = bundle;
export type PlanRendererInstance = InstanceType<RendererBundle["GlPlanRenderer"]>;
