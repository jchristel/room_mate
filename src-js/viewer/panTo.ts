// "Bring this room into view" — the one rule behind a grid row click (G3).
//
// Pure and out of the components for the reason `levels.ts` is: it is a small
// rule that several zones each apply to themselves, and it is only wrong in
// ways that are hard to see — a plan that jumps when it did not need to, or a
// room a reader was told is selected sitting just outside the frame.
//
// **A pan, never a zoom.** The view keeps its size, so the reader's scale is
// theirs: a row click is "show me where this is", not "fill the panel with
// it". Zoom-to-fit is a stated non-goal.

import type { Rect } from "../renderer/types.js";

/**
 * Where `view` must move so `target` is visible, or `null` for "it already is".
 *
 * **Null is the interesting half.** A zone where the room is wholly inside the
 * view does not move at all, so clicking row after row down a department keeps
 * the plan still for the ones already on screen and moves it only for the ones
 * that are not — which is what makes the pan readable rather than a lurch per
 * click.
 *
 * **Centred, not nudged to the nearest edge.** A room dragged just inside the
 * frame is technically visible and practically not, and the centre is also
 * what makes a second click on the same row a no-op: the answer does not
 * depend on where the room was, only on where it is. That is the answer to a
 * room LARGER than the view too — it can never be wholly inside, so every
 * click centres it, and centring an already-centred room moves nothing.
 */
export function viewCentredOn(view: Rect, target: Rect): Rect | null {
  const inside =
    target.x >= view.x &&
    target.y >= view.y &&
    target.x + target.w <= view.x + view.w &&
    target.y + target.h <= view.y + view.h;
  if (inside) return null;
  const next = {
    x: target.x + target.w / 2 - view.w / 2,
    y: target.y + target.h / 2 - view.h / 2,
    w: view.w,
    h: view.h,
  };
  // Already centred to within a rounding error: a room bigger than the view
  // fails the containment test for ever, and returning a rect here would let
  // repeated clicks on one row re-commit a view identical to the current one.
  const moved = Math.abs(next.x - view.x) > EPS * view.w || Math.abs(next.y - view.y) > EPS * view.h;
  return moved ? next : null;
}

/** How much of the view's own size counts as "did not move". Relative rather
 *  than absolute because the units are project feet and the view spans four
 *  orders of magnitude between a cupboard and RHH's site. */
const EPS = 1e-6;
