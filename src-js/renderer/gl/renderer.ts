// The WebGL plan renderer: the second implementation of the P2 seam.
//
// Reading order, because the layering is the design:
//
//   grid   -> one LineBatch, 0.5px, solid
//   fills  -> one FillBatch, earcut with true holes, per-vertex colour
//   holes  -> one LineBatch, 1px, dashed 4/3
//   lines  -> one LineBatch, 1.5px (3px for a search match), solid
//   labels -> BitmapText, one container per room
//
// Four draw calls plus the labels, whatever the room count. That is the whole
// point: the fitted view — what the viewer shows on load, and where the cull
// culls nothing — stops scaling with the plate.
//
// WEBGL, NOT WEBGPU. Pixi v8 offers both, but a WebGPU path needs every shader
// written a second time in WGSL, and the shaders here are the least
// transferable part of the renderer. `preference: "webgl"` until there is a
// measured reason to pay for the second copy.

import { Application, Container } from "pixi.js";
import { resolveRoomAppearance } from "../appearance.js";
import { flip, pointsAttr } from "../geometry.js";
import type { HighlightState, PaintRequest, PlanRenderer } from "../seam.js";
import type { Rect, Room } from "../types.js";
import { parseColour, readPalette, withAlpha, type PlanPalette, type Rgba } from "./colour.js";
import { FillBatch, type FillMesh, type VertexRange } from "./fills.js";
import { buildLabels, type RoomLabel } from "./labels.js";
import { LineBatch, ringSegments, type LineMesh, type Segment } from "./lines.js";
import { RoomIndex } from "./spatial.js";

/** Stroke widths, in CSS pixels — the same numbers the stylesheet uses, so the
 *  two renderers can be compared without converting anything. */
const W_GRID = 0.5;
const W_OUTLINE = 1.5;
const W_MATCH = 3;
const W_HOLE = 1;
/** `stroke-dasharray: 4 3` on `.hole`. */
const HOLE_DASH: readonly [number, number] = [4, 3];
/** `.dim { opacity: 0.15 }`. */
const DIM_ALPHA = 0.15;
/** `svg.plan.areas-active` ghosting. */
const GHOST_ROOMS = 0.16;
const GHOST_LABELS = 0.22;
/** Grid spacing in world units, as `paintLevel` uses. */
const GRID_STEP = 5;
const SVG_NS = "http://www.w3.org/2000/svg";

interface RoomEntry {
  room: Room;
  fill: VertexRange | null;
  outline: { start: number; count: number } | null;
  appearanceFill: Rgba;
  /** Whether a colour plan resolved a literal fill for this room — which is
   *  what decides if hover may change its colour. See `#drawMarks`. */
  appearanceIsPlanFill: boolean;
  /** The plan's fill as CSS, for re-applying inline on a hover mark. */
  appearanceCss: string | null;
}

export interface GlRendererOptions {
  cullEnabled?: (() => boolean) | undefined;
  /** Element whose custom properties carry the palette. Defaults to
   *  `document.documentElement`. */
  themeRoot?: HTMLElement | undefined;
  /**
   * The SVG layer sitting above the canvas, where the SMALL things are drawn.
   *
   * This is the hybrid, in one argument. Selection and hover are exactly ONE
   * room each, and drawing them here rather than in GL keeps the stylesheet as
   * the place their appearance is defined and avoids re-uploading a vertex
   * buffer on every click and every pointer move. The areas overlay already
   * lives on this element and is not touched by the renderer at all.
   */
  overlay?: SVGElement | undefined;
  /**
   * Backing-store scale. Defaults to `devicePixelRatio`, which is what should
   * ship.
   *
   * Overridable because the POC measured at DPR 1 and a retina display roughly
   * 4x's fill cost, so "what does this cost at DPR 2" is a question that has to
   * be answerable on a DPR 1 development machine rather than assumed.
   */
  resolution?: number | undefined;
}

export class GlPlanRenderer implements PlanRenderer {
  readonly #canvas: HTMLCanvasElement;
  readonly #cullEnabled: () => boolean;
  readonly #themeRoot: HTMLElement;
  readonly #overlay: SVGElement | null;
  readonly #resolution: number;

  #app: Application | null = null;
  #ready: Promise<void>;
  #root = new Container();
  #observer: ResizeObserver | null = null;

  #grid: LineMesh | null = null;
  #fills: FillMesh | null = null;
  #holeLines: LineMesh | null = null;
  #outlines: LineMesh | null = null;
  #labelContainer: Container | null = null;
  #labels: RoomLabel[] = [];

  #entries: RoomEntry[] = [];
  #index = new RoomIndex([]);
  #palette: PlanPalette;
  #view: Rect = { x: 0, y: 0, w: 100, h: 100 };
  #selected: string | null = null;
  #hovered: string | null = null;
  #areasActive = false;
  /** Retained so a repaint can reproduce exactly what is on screen. */
  #rooms: readonly Room[] = [];
  #fitted: Rect = { x: 0, y: 0, w: 100, h: 100 };
  #lastPaint: PaintRequest = {};
  #highlight: HighlightState = { searchActive: false, matchRoomIds: null };

  constructor(canvas: HTMLCanvasElement, opts: GlRendererOptions = {}) {
    this.#canvas = canvas;
    this.#cullEnabled = opts.cullEnabled ?? (() => true);
    this.#themeRoot = opts.themeRoot ?? document.documentElement;
    this.#overlay = opts.overlay ?? null;
    this.#resolution = opts.resolution ?? window.devicePixelRatio ?? 1;
    this.#palette = readPalette(this.#themeRoot);
    this.#ready = this.#init();
  }

  /** Resolves once the GL context exists. `paint` before this is a no-op that
   *  replays on ready, so callers never have to await construction. */
  get ready(): Promise<void> {
    return this.#ready;
  }

  async #init(): Promise<void> {
    const app = new Application();
    await app.init({
      canvas: this.#canvas,
      preference: "webgl",
      antialias: true,
      backgroundAlpha: 0, // the page's own background shows through
      // Size the backing store by DPR, the way static/graph.js already does.
      // The POC measured at DPR 1 and a retina display roughly 4x's fill cost,
      // so this is the number to report a measurement against.
      resolution: this.#resolution,
      autoDensity: true,
      // The plan only redraws when something changes -- a pan, a repaint, a
      // search keystroke -- so there is no animation to drive and a ticker would
      // spend a frame budget every 16 ms on a static page for nothing.
      //
      // `autoStart: false` at init rather than `app.stop()` afterwards, because
      // it is the documented way to never start a ticker in the first place.
      autoStart: false,
      sharedTicker: false,
    });
    app.stage.addChild(this.#root);
    this.#app = app;

    // Pixi's `autoDensity` writes inline width/height onto the canvas, which
    // beats the stylesheet's `width: 100%` -- so without this the plan sits in
    // an 800x600 box in the corner of the zone no matter how large the zone is.
    // Size to the parent, and keep doing so: zones are a CSS grid whose column
    // count changes with `+ zone`, and the window is resizable.
    const parent = this.#canvas.parentElement;
    if (parent) {
      const fit = () => {
        const w = Math.max(1, parent.clientWidth);
        const h = Math.max(1, parent.clientHeight);
        if (w === app.renderer.width / app.renderer.resolution && h === app.renderer.height / app.renderer.resolution)
          return;
        app.renderer.resize(w, h);
        this.#pushView();
        this.#render();
      };
      this.#observer = new ResizeObserver(fit);
      this.#observer.observe(parent);
      fit();
    }

    if (this.#rooms.length > 0) this.#rebuild();
  }

  // ---------------------------------------------------------------- seam ----

  paint(rooms: readonly Room[], fitted: Rect, opts: PaintRequest = {}): void {
    this.#rooms = rooms;
    this.#fitted = fitted;
    this.#lastPaint = opts;
    this.#highlight = {
      searchActive: !!opts.searchActive,
      matchRoomIds: opts.matchRoomIds ?? null,
    };
    // Re-read every paint: the theme can change under a running page, and a
    // cached palette would strand the plan in the old one.
    this.#palette = readPalette(this.#themeRoot);
    this.#index = new RoomIndex(rooms);
    if (this.#app) this.#rebuild();
  }

  setView(view: Rect): void {
    this.#view = view;
    this.#pushView();
    this.#render();
  }

  cull(): void {
    // Geometry is NOT culled, and that is the headline result rather than an
    // omission: fills, grid, holes and outlines are four draw calls whatever the
    // room count, so hiding some of them saves nothing measurable. Labels are
    // one scene object per room and do cost, so they are what the index culls.
    if (this.#labels.length === 0) return;

    const v = this.#view;
    const mx = v.w * 0.2;
    const my = v.h * 0.2;
    const x0 = v.x - mx;
    const y0 = v.y - my;
    const x1 = v.x + v.w + mx;
    const y1 = v.y + v.h + my;

    // ONLY WRITE WHAT CHANGES. Assigning `visible` on a Pixi object dirties the
    // scene graph whether or not the value differs, so a blind pass over every
    // label costs the same at a fitted view -- where nothing is off-screen and
    // the cull can achieve nothing -- as it does zoomed in. Measured at 5,046
    // rooms that was ~20 ms per frame of pure waste, which is most of a frame
    // budget spent to change nothing. Same shape as the SVG cull's rule for the
    // same reason: it too only toggles a unit whose state actually flips.
    let changed = false;
    const enabled = this.#cullEnabled();
    for (const l of this.#labels) {
      const want =
        !enabled || !(l.maxX < x0 || l.minX > x1 || l.maxY < y0 || l.minY > y1);
      if (l.node.visible !== want) {
        l.node.visible = want;
        changed = true;
      }
    }
    // Re-rendering an unchanged scene is the other half of the same waste.
    if (changed) this.#render();
  }

  applyHighlight(state: HighlightState): void {
    this.#highlight = state;
    // The fast path. Rewrites vertex colours and stroke widths in place and
    // re-uploads two buffers; it does NOT re-triangulate, re-index or rebuild
    // labels. A search can match thousands of rooms and a keystroke must not
    // re-upload a level.
    const fills = this.#fills;
    const outlines = this.#outlines;
    if (!fills || !outlines) return;

    for (const e of this.#entries) {
      const isMatch = !!(state.searchActive && state.matchRoomIds?.has(e.room.id));
      const dim = !!state.searchActive && !isMatch;
      if (e.fill) fills.recolour(e.fill, dim ? withAlpha(e.appearanceFill, DIM_ALPHA) : e.appearanceFill);
      if (e.outline) {
        const colour = isMatch ? this.#palette.accent : this.#palette.ink;
        outlines.restyle(
          e.outline,
          dim ? withAlpha(colour, DIM_ALPHA) : colour,
          isMatch ? W_MATCH : W_OUTLINE,
        );
      }
    }
    fills.commit();
    outlines.commit();
    this.#render();
  }

  setSelection(roomId: string | null): void {
    if (this.#selected === roomId) return;
    this.#selected = roomId;
    this.#drawMarks();
  }

  get selection(): string | null {
    return this.#selected;
  }

  /**
   * Force a render and return immediately, for verification.
   *
   * A WebGL drawing buffer is cleared once the compositor has presented it
   * (there is no `preserveDrawingBuffer` here, and turning it on would cost a
   * copy every frame for the rest of the product's life). So reading the canvas
   * from a later task always sees an empty buffer, whatever was drawn — which
   * is indistinguishable from a renderer that drew nothing, and cost real time
   * to work out once already. Anything measuring pixels must render and read in
   * ONE synchronous turn; this is the render half of that.
   */
  renderNow(): void {
    this.#render();
  }

  /** What the renderer currently holds. Exists because a GL renderer that draws
   *  nothing looks identical to one that was never given data, and telling those
   *  apart from the console is otherwise impossible. */
  debugState(): Record<string, unknown> {
    return {
      hasApp: !!this.#app,
      stageChildren: this.#root.children.length,
      rooms: this.#rooms.length,
      entries: this.#entries.length,
      indexed: this.#index.size,
      labels: this.#labels.length,
      layers: {
        grid: !!this.#grid,
        fills: !!this.#fills,
        holes: !!this.#holeLines,
        outlines: !!this.#outlines,
      },
      view: this.#view,
      rendererSize: this.#app ? [this.#app.renderer.width, this.#app.renderer.height] : null,
      resolution: this.#app?.renderer.resolution ?? null,
    };
  }

  setHover(roomId: string | null): void {
    if (this.#hovered === roomId) return;
    this.#hovered = roomId;
    this.#drawMarks();
  }

  setAreasActive(on: boolean): void {
    this.#areasActive = on;
    this.#fills?.setLayerAlpha(on ? GHOST_ROOMS : 1);
    this.#outlines?.setLayerAlpha(on ? GHOST_ROOMS : 1);
    this.#holeLines?.setLayerAlpha(on ? GHOST_ROOMS : 1);
    this.#grid?.setLayerAlpha(on ? GHOST_ROOMS : 1);
    if (this.#labelContainer) this.#labelContainer.alpha = on ? GHOST_LABELS : 1;
    this.#render();
  }

  roomAt(clientX: number, clientY: number): Room | null {
    // Viewport pixels -> flipped world, using the canvas's own rect so this is
    // correct under any CSS sizing.
    const r = this.#canvas.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return null;
    const wx = this.#view.x + ((clientX - r.left) / r.width) * this.#view.w;
    const wy = this.#view.y + ((clientY - r.top) / r.height) * this.#view.h;
    return this.#index.roomAt(wx, wy);
  }

  dispose(): void {
    this.#destroyLayers();
    this.#entries = [];
    this.#labels = [];
    this.#index = new RoomIndex([]);
    this.#rooms = [];
    this.#selected = null;
    this.#hovered = null;
    // The marks live in the SVG overlay, which this renderer does NOT own — so
    // clearing the canvas would leave a selection outline floating over an empty
    // plan. Remove them explicitly rather than trusting the caller.
    this.#overlay?.querySelector("g.plan-marks")?.remove();
    this.#render();
  }

  /** Release the WebGL context itself. Separate from `dispose()`, which clears
   *  a level: browsers cap live contexts (commonly ~16) and silently kill the
   *  oldest past the limit, so a zone that closes without calling this
   *  eventually blanks a DIFFERENT zone with no error anyone can act on. */
  destroy(): void {
    this.#observer?.disconnect();
    this.#observer = null;
    this.#destroyLayers();
    this.#app?.destroy(false, { children: true });
    this.#app = null;
  }

  // ------------------------------------------------------------ internals ----

  #destroyLayers(): void {
    this.#root.removeChildren();
    this.#grid?.destroy();
    this.#fills?.destroy();
    this.#holeLines?.destroy();
    this.#outlines?.destroy();
    this.#labelContainer?.destroy({ children: true });
    this.#grid = null;
    this.#fills = null;
    this.#holeLines = null;
    this.#outlines = null;
    this.#labelContainer = null;
  }

  #rebuild(): void {
    this.#destroyLayers();
    const rooms = this.#rooms;
    const opts = this.#lastPaint;
    const pal = this.#palette;
    if (rooms.length === 0) {
      this.#entries = [];
      this.#labels = [];
      this.#render();
      return;
    }

    const fitted = this.#fitted;
    const gridBatch = new LineBatch();
    const gridSegs: Segment[] = [];
    for (let gx = Math.ceil(fitted.x / GRID_STEP) * GRID_STEP; gx < fitted.x + fitted.w; gx += GRID_STEP)
      gridSegs.push({ x0: gx, y0: fitted.y, x1: gx, y1: fitted.y + fitted.h });
    for (let gy = Math.ceil(fitted.y / GRID_STEP) * GRID_STEP; gy < fitted.y + fitted.h; gy += GRID_STEP)
      gridSegs.push({ x0: fitted.x, y0: gy, x1: fitted.x + fitted.w, y1: gy });
    gridBatch.push(gridSegs, pal.rule, W_GRID);

    const fillBatch = new FillBatch();
    const outlineBatch = new LineBatch();
    const holeBatch = new LineBatch();
    const entries: RoomEntry[] = [];

    for (const room of rooms) {
      const loops = room.loops;
      if (!loops?.[0]) continue;

      // The SAME decision the SVG painter makes, from the same function. This
      // is what Decision 3 is for: one appearance resolution, two emitters.
      const a = resolveRoomAppearance(room, opts);
      const base: Rgba = a.fill !== null ? parseColour(a.fill) : a.error ? pal.error : pal.fill;
      const fillColour = a.dim ? withAlpha(base, DIM_ALPHA) : base;
      const strokeBase = a.match ? pal.accent : pal.ink;
      const strokeColour = a.dim ? withAlpha(strokeBase, DIM_ALPHA) : strokeBase;

      const fill = fillBatch.push(room, fillColour);
      const outline = outlineBatch.push(
        ringSegments(loops[0].points.map(flip)),
        strokeColour,
        a.match ? W_MATCH : W_OUTLINE,
      );
      for (let i = 1; i < loops.length; i++)
        holeBatch.push(
          ringSegments(loops[i]!.points.map(flip)),
          a.dim ? withAlpha(pal.ink, DIM_ALPHA) : pal.ink,
          W_HOLE,
        );

      entries.push({
        room,
        fill,
        outline,
        appearanceFill: base,
        appearanceIsPlanFill: a.fill !== null,
        appearanceCss: a.fill,
      });
    }
    this.#entries = entries;

    this.#grid = gridBatch.build();
    this.#fills = fillBatch.isEmpty ? null : fillBatch.build();
    // The dash is the whole reason holes get their own batch: `stroke-dasharray:
    // 4 3` on `.hole` is a visible feature, not incidental decoration.
    this.#holeLines = holeBatch.isEmpty ? null : holeBatch.build({ dash: HOLE_DASH });
    this.#outlines = outlineBatch.isEmpty ? null : outlineBatch.build();

    // Paint order is child order, and it mirrors the SVG document exactly:
    // grid behind, then fills, then the strokes that sit on them, then labels.
    if (this.#grid) this.#root.addChild(this.#grid.mesh);
    if (this.#fills) this.#root.addChild(this.#fills.mesh);
    if (this.#holeLines) this.#root.addChild(this.#holeLines.mesh);
    if (this.#outlines) this.#root.addChild(this.#outlines.mesh);

    if (opts.showLabels !== false) {
      const built = buildLabels(rooms, fitted, {
        ink: pal.ink,
        accent: pal.accent,
        fontFamily: getComputedStyle(this.#themeRoot).getPropertyValue("--mono").trim() || "monospace",
      });
      this.#labelContainer = built.container;
      this.#labels = built.labels;
      // A RENDER GROUP, and this is a budget decision rather than a tidy-up.
      //
      // Pan and zoom move this container, and without a render group Pixi
      // re-derives the world transform of every descendant each frame -- 5,046
      // label nodes at plate scale, measured at ~20 ms per frame, which is most
      // of a 16 ms budget spent recomputing positions that all moved by the same
      // amount. A render group makes the container's own transform the thing
      // that changes and leaves the children's local transforms alone.
      built.container.enableRenderGroup();
      this.#root.addChild(built.container);
    } else {
      this.#labels = [];
    }

    this.setAreasActive(this.#areasActive);
    this.#pushView();
    this.cull();
    // A repaint rebuilds `#entries`, so the marks point at rooms that no longer
    // exist. Re-derived here, which is the GL equivalent of `renderLevel`
    // re-applying the selection class after `paintLevel` rebuilt every node.
    this.#drawMarks();
  }

  /**
   * Redraw the selection and hover marks into the SVG overlay.
   *
   * ONE `<g>`, rebuilt whole. There are at most two polygons in it, so nothing
   * is gained by diffing — and a rebuilt group cannot get out of step with the
   * state that produced it, which a diffed one can.
   *
   * The marks are appended BEFORE the areas overlay if that exists, so
   * footprints keep drawing on top of a selected room exactly as they did when
   * the rooms were SVG. `renderAreasOverlay` owns that element and this must not
   * disturb it.
   */
  #drawMarks(): void {
    const overlay = this.#overlay;
    if (!overlay) return;
    const doc = overlay.ownerDocument;

    let g = overlay.querySelector<SVGGElement>("g.plan-marks");
    if (!g) {
      g = doc.createElementNS(SVG_NS, "g") as SVGGElement;
      g.setAttribute("class", "plan-marks");
      // Never a pointer target. The pick is answered by the spatial index from
      // raw coordinates, and a mark that swallowed clicks would make the
      // selected room unclickable — which reads as "selection is stuck".
      g.setAttribute("pointer-events", "none");
    }
    g.replaceChildren();

    const byId = (id: string | null) => (id ? this.#entries.find((e) => e.room.id === id) : undefined);

    // Hover first, selection second: the selection stroke must win where a room
    // is both, which is routine (you click what you are pointing at).
    const hovered = byId(this.#hovered);
    if (hovered && hovered.room.id !== this.#selected) {
      const p = this.#markPolygon(doc, hovered.room);
      if (p) {
        // `.room` for the shape, `hovered` for the fill swap. An inline
        // colour-plan fill is re-applied here on purpose: in SVG an inline fill
        // BEATS the `:hover` rule, so a room with an active colour plan does not
        // change colour on hover. Reproducing that means reproducing the
        // precedence, not just the hover colour.
        p.setAttribute("class", hovered.appearanceIsPlanFill ? "room hovered" : "room hovered plain");
        if (hovered.appearanceIsPlanFill && hovered.appearanceCss) p.style.fill = hovered.appearanceCss;
        g.appendChild(p);
      }
    }

    const selected = byId(this.#selected);
    if (selected) {
      const p = this.#markPolygon(doc, selected.room);
      if (p) {
        // Stroke only — `fill: none` in the stylesheet — so whatever the GL
        // layer painted underneath still shows. In SVG the selection class went
        // onto the room polygon itself and the fill came from the same element;
        // here the fill is a different technology, so the mark must not cover it.
        p.setAttribute("class", "room-selected-mark");
        g.appendChild(p);
      }
    }

    if (g.childNodes.length === 0) {
      g.remove();
      return;
    }
    // Always FIRST, so the areas overlay (appended last by renderAreasOverlay)
    // keeps drawing above the marks.
    if (overlay.firstChild !== g) overlay.insertBefore(g, overlay.firstChild);
  }

  #markPolygon(doc: Document, room: Room): SVGPolygonElement | null {
    const outer = room.loops?.[0];
    if (!outer) return null;
    const p = doc.createElementNS(SVG_NS, "polygon") as SVGPolygonElement;
    p.setAttribute("points", pointsAttr(outer));
    return p;
  }

  #pushView(): void {
    const app = this.#app;
    if (!app) return;
    const pxW = app.renderer.width;
    const pxH = app.renderer.height;
    const dpr = app.renderer.resolution;
    this.#grid?.setView(this.#view, pxW, pxH, dpr);
    this.#holeLines?.setView(this.#view, pxW, pxH, dpr);
    this.#outlines?.setView(this.#view, pxW, pxH, dpr);
    this.#fills?.setView(this.#view, pxW, pxH);
    // Labels live in world space, so the scene transform carries them. This is
    // the ONE place a container transform is used, and it is correct here
    // precisely because label text SHOULD scale with the view.
    if (this.#labelContainer) {
      const sx = pxW / dpr / this.#view.w;
      const sy = pxH / dpr / this.#view.h;
      this.#labelContainer.scale.set(sx, sy);
      this.#labelContainer.position.set(-this.#view.x * sx, -this.#view.y * sy);
    }
  }

  #render(): void {
    this.#app?.render();
  }
}
