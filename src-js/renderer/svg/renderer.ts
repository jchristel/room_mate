// The seam, implemented by the SVG code that has always drawn the plan.
//
// Nothing here is new behaviour. It is the existing paint/cull/highlight/select
// logic collected behind one interface, so that P3 can add a second
// implementation and the two can be compared on the same data in the same
// session. If this file changes what the viewer does, it is wrong.

import { cull as cullUnits } from "../cull.js";
import type { HighlightState, PaintRequest, PlanRenderer } from "../seam.js";
import type { Rect, Room } from "../types.js";
import { paintLevel, type CullUnit } from "./paint.js";

export interface SvgRendererOptions {
  /** Read at cull time rather than captured, so flipping the page's
   *  `CULL_ENABLED` from the console takes effect on the next pass instead of
   *  needing a reload. */
  cullEnabled?: (() => boolean) | undefined;
}

export class SvgPlanRenderer implements PlanRenderer {
  readonly #svg: SVGElement;
  readonly #cullEnabled: () => boolean;
  #units: CullUnit[] = [];
  #view: Rect = { x: 0, y: 0, w: 100, h: 100 };
  #selected: string | null = null;

  constructor(svg: SVGElement, opts: SvgRendererOptions = {}) {
    this.#svg = svg;
    this.#cullEnabled = opts.cullEnabled ?? (() => true);
  }

  paint(rooms: readonly Room[], fitted: Rect, opts: PaintRequest = {}): void {
    // Emptying the element and resetting the index are ONE operation and must
    // stay adjacent. Emptying without resetting leaves units pointing at
    // detached nodes, which is how a click once resolved to a room that was not
    // on screen and the cull spent its passes toggling `display` on nodes in no
    // document.
    this.#svg.replaceChildren();
    this.#units = [];
    paintLevel(this.#svg, rooms, fitted, { ...opts, cullUnits: this.#units });
    this.cull();
    // Painting built fresh nodes, so any selection class went with the old
    // ones. Re-applied from state the renderer holds rather than threaded
    // through `paintLevel`, which is shared with the export and must never
    // carry a selection stroke into a saved file.
    this.#applySelection();
  }

  setView(view: Rect): void {
    this.#view = view;
    this.#svg.setAttribute("viewBox", `${view.x} ${view.y} ${view.w} ${view.h}`);
  }

  cull(): void {
    cullUnits(this.#units, this.#view, { enabled: this.#cullEnabled() });
  }

  applyHighlight(state: HighlightState): void {
    // A state change over existing nodes, never a re-render — the fast path a
    // keystroke depends on.
    for (const u of this.#units) {
      const isMatch = !!(state.searchActive && state.matchRoomIds?.has(u.room.id));
      const dim = !!state.searchActive && !isMatch;
      u.nodes[0]!.classList.toggle("match", isMatch);
      for (const n of u.nodes) n.classList.toggle("dim", dim);
    }
  }

  setSelection(roomId: string | null): void {
    this.#selected = roomId;
    this.#applySelection();
  }

  setHover(_roomId: string | null): void {
    // Deliberately nothing. SVG rooms are real elements, so `.room:hover` in the
    // stylesheet already does this and doing it again here would fight the CSS.
    // The GL renderer has no elements to hover, so its implementation is where
    // the work lives — which is exactly why this is on the interface rather
    // than being something the page does for itself.
  }

  setAreasActive(on: boolean): void {
    this.#svg.classList.toggle("areas-active", on);
  }

  roomAt(clientX: number, clientY: number): Room | null {
    // Resolved through the DOM rather than by testing the point against every
    // room's geometry: the browser has already done this work, and it accounts
    // for what is actually on top. The GL renderer, having no nodes, does the
    // geometric version instead.
    const doc = this.#svg.ownerDocument;
    const node = doc.elementFromPoint(clientX, clientY);
    if (!node) return null;
    // `nodes.includes` rather than an id attribute stamped on the polygon:
    // `paintLevel` is shared with the SVG export, and an export has no business
    // carrying selection plumbing.
    return this.#units.find((u) => u.nodes.includes(node as SVGElement))?.room ?? null;
  }

  dispose(): void {
    this.#svg.replaceChildren();
    this.#units = [];
    this.#selected = null;
  }

  /** Escape hatch for the page's own selection pass, which must also walk the
   *  areas overlay — a layer this renderer does not own. Read-only by
   *  convention; mutating it from outside is how the index and the document
   *  drift apart. */
  get units(): readonly CullUnit[] {
    return this.#units;
  }

  #applySelection(): void {
    for (const u of this.#units)
      u.nodes[0]!.classList.toggle("selected", this.#selected !== null && u.room.id === this.#selected);
  }
}
