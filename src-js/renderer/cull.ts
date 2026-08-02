// Viewport culling: hide rooms whose bbox falls outside the current view, show
// them again when they re-enter.
//
// SVG clips but does not cull — every element still costs per frame — so at a
// few thousand rooms a zoomed-in view is unusable without this. Measured on
// `big-plate`: 16.5 ms/frame with culling against 912 ms without
// (Superseded/HANDOVER-culling-disable-switch.md).
//
// Note for P3: this same index feeds the GL pick. The cull and the pick are the
// same question — "what geometry overlaps this screen region" — and the
// handover is explicit that a Flatbush R-tree does both, rather than Pixi's
// culler or its scene-graph hit-testing, which are CPU walks over the display
// list.

import type { CullUnit } from "./svg/paint.js";
import type { Rect } from "./types.js";

/** Fraction of the view kept loaded beyond each edge, so panning does not pop
 *  edges into existence at the boundary. */
const MARGIN = 0.2;

export interface CullOptions {
  /** Kill switch, mirroring `index.html`'s `CULL_ENABLED`. Kept so culling's
   *  value can be re-measured against later rendering changes without deleting
   *  the feature — which is exactly what it was used for once already. */
  enabled?: boolean | undefined;
}

/**
 * Toggle `display` on each unit's nodes for the given view.
 *
 * Only touches a unit whose on/off state actually CHANGES, so steady-state
 * panning costs the handful of rooms crossing the boundary rather than the
 * whole level.
 */
export function cull(units: readonly CullUnit[], view: Rect, opts: CullOptions = {}): void {
  const { enabled = true } = opts;
  if (units.length === 0) return;

  // The restore loop is the load-bearing half of the kill switch: flipping the
  // flag mid-session must not strand rooms with `display: none` left over from
  // the last enabled pass. Guarded here rather than at the call sites so every
  // current and future caller is covered by one branch.
  if (!enabled) {
    for (const u of units) {
      if (u.hidden) {
        for (const n of u.nodes) n.style.display = "";
        u.hidden = false;
      }
    }
    return;
  }

  const mx = view.w * MARGIN;
  const my = view.h * MARGIN;
  const x0 = view.x - mx;
  const y0 = view.y - my;
  const x1 = view.x + view.w + mx;
  const y1 = view.y + view.h + my;

  for (const u of units) {
    const off = u.maxX < x0 || u.minX > x1 || u.maxY < y0 || u.minY > y1;
    if (off !== u.hidden) {
      const d = off ? "none" : "";
      for (const n of u.nodes) n.style.display = d;
      u.hidden = off;
    }
  }
}
