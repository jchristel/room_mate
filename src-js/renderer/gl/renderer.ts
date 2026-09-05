// The WebGL plan renderer, and now the only one.
//
// Reading order, because the layering is the design:
//
//   grid   -> one LineBatch, 0.5px, solid
//   fills  -> one FillBatch, earcut with true holes, per-vertex colour
//   hover  -> a small FillBatch, one room, rebuilt on hover change
//   holes  -> one LineBatch, 1px, dashed 4/3
//   lines  -> one LineBatch, 1.5px (3px for a search match), solid
//   doors  -> one FillBatch, every door glyph on the level
//   labels -> BitmapText, one container per room
//
// Five draw calls plus the labels, whatever the room OR door count. That is the whole
// point: the fitted view — what the viewer shows on load, and the case viewport
// culling could never help, because nothing is off screen to hide — stops
// scaling with the plate.
//
// WEBGL, NOT WEBGPU. Pixi v8 offers both, but a WebGPU path needs every shader
// written a second time in WGSL, and the shaders here are the least
// transferable part of the renderer. `preference: "webgl"` until there is a
// measured reason to pay for the second copy.

import { Application, Container } from "pixi.js";
import { resolveRoomAppearance } from "../appearance.js";
import { flip, pointsAttr } from "../geometry.js";
import type { HighlightState, PaintRequest, Pick, PlanRenderer } from "../seam.js";
import type { Door, Item, Rect, Room, WindowOpening } from "../types.js";
import { parseColour, readPalette, withAlpha, type PlanPalette, type Rgba } from "./colour.js";
import { FillBatch, type FillMesh, type VertexRange } from "./fills.js";
import { buildLabels, type RoomLabel } from "./labels.js";
import { LineBatch, ringSegments, type LineMesh, type Segment } from "./lines.js";
import { buildDoorGlyph } from "./doorGlyph.js";
import { buildWindowGlyph } from "./windowGlyph.js";
import { buildItemGlyph } from "./itemGlyph.js";
import { DoorIndex, RoomIndex, type PickableDoor } from "./spatial.js";
import { fitViewToAspect, labelTransform } from "./viewport.js";

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
/** The door footprint's fill, as an alpha over the ink colour. Light enough
 *  that a room's label still reads through a door drawn over it, dark enough to
 *  register as a solid object rather than a smudge. */
const DOOR_RECT_ALPHA = 0.25;
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
   *  what decides if hover may change its colour. See `#drawHover`. */
  appearanceIsPlanFill: boolean;
}

/** One drawn door: what it is, and where its pieces live in the shared buffer
 *  so selection can recolour them without rebuilding the level. */
interface DoorEntry {
  door: Door;
  rect: VertexRange | null;
  glyph: VertexRange | null;
}

/** One drawn window. The same three fields a door entry carries, because a
 *  window is drawn the same way: a footprint and a mark, each a range in the
 *  shared buffer so selection can recolour without a rebuild. */
interface ItemEntry {
  item: Item;
  rect: VertexRange | null;
  glyph: VertexRange | null;
}

/** One drawn window. The same three fields a door entry carries, because a
 *  window is drawn the same way: a footprint and a mark, each a range in the
 *  window mesh. */
interface WindowEntry {
  window: WindowOpening;
  rect: VertexRange | null;
  glyph: VertexRange | null;
}

export interface GlRendererOptions {
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
  readonly #themeRoot: HTMLElement;
  readonly #overlay: SVGElement | null;
  readonly #resolution: number;

  #app: Application | null = null;
  #ready: Promise<void>;
  #root = new Container();
  #observer: ResizeObserver | null = null;

  #grid: LineMesh | null = null;
  #fills: FillMesh | null = null;
  #hoverMesh: FillMesh | null = null;
  #doorMesh: FillMesh | null = null;
  #windowMesh: FillMesh | null = null;
  #ffeMesh: FillMesh | null = null;
  #holeLines: LineMesh | null = null;
  #outlines: LineMesh | null = null;
  #labelContainer: Container | null = null;
  #labels: RoomLabel[] = [];

  #entries: RoomEntry[] = [];
  #index = new RoomIndex([]);
  #doorIndex = new DoorIndex([]);
  #doorEntries: DoorEntry[] = [];
  #windowIndex = new DoorIndex([]);
  #windowEntries: WindowEntry[] = [];
  #ffeIndex = new DoorIndex([]);
  #ffeEntries: ItemEntry[] = [];
  #palette: PlanPalette;
  #view: Rect = { x: 0, y: 0, w: 100, h: 100 };
  #selected: string | null = null;
  #selectedDoor: string | null = null;
  #selectedWindow: string | null = null;
  #selectedItem: string | null = null;
  #hovered: string | null = null;
  #areasActive = false;
  /** Retained so a repaint can reproduce exactly what is on screen. */
  #rooms: readonly Room[] = [];
  #fitted: Rect = { x: 0, y: 0, w: 100, h: 100 };
  #lastPaint: PaintRequest = {};
  #highlight: HighlightState = { searchActive: false, matchRoomIds: null };

  constructor(canvas: HTMLCanvasElement, opts: GlRendererOptions = {}) {
    this.#canvas = canvas;
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
        // Compared in CSS pixels, on both sides. `renderer.width` is ALREADY
        // CSS pixels (see `#pushView`), so the old `/ resolution` here was
        // asking whether the parent was 1/DPR of the canvas -- a question that
        // is trivially "no" at DPR 1 and accidentally "yes" whenever a zone
        // happens to land on that ratio, which skipped the resize and left the
        // canvas at Pixi's default 800x600 inside it.
        if (w === app.renderer.width && h === app.renderer.height) return;
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

  /** The doors this paint was given, after the `showDoors` toggle. */
  #activeDoors(): readonly Door[] {
    const opts = this.#lastPaint;
    if (opts.showDoors === false) return [];
    return opts.doors ?? [];
  }

  /** The FF&E this paint was given, after the `showFfe` toggle. */
  #activeFfe(): readonly Item[] {
    const opts = this.#lastPaint;
    if (opts.showFfe === false) return [];
    return opts.ffe ?? [];
  }

  /** The windows this paint was given, after the `showWindows` toggle. */
  #activeWindows(): readonly WindowOpening[] {
    const opts = this.#lastPaint;
    if (opts.showWindows === false) return [];
    return opts.windows ?? [];
  }

  setView(view: Rect): void {
    this.#view = view;
    // THE OVERLAY NEEDS THE VIEWBOX TOO, and it is set from the RAW view rather
    // than the aspect-corrected one -- deliberately.
    //
    // The SVG layer above the canvas is not decoration: it carries the areas
    // overlay, the selection mark and the hover mark, all drawn in world
    // coordinates. An <svg> with no viewBox is in PIXEL space, so a footprint at
    // x=27, y=-50 lands in a few pixels at the top-left corner and reads as
    // "areas stopped working" or "the selection outline vanished" -- which is
    // exactly what it did, because the SVG renderer used to set this and the GL
    // one did not.
    //
    // Raw, not corrected, because SVG applies `preserveAspectRatio: xMidYMid
    // meet` to a viewBox itself -- the same uniform-scale-and-centre that
    // `fitViewToAspect` reproduces for GL. Feeding it the already-corrected rect
    // would apply the correction twice. The two layers land together precisely
    // because each is given the input its own coordinate system expects.
    this.#overlay?.setAttribute("viewBox", `${view.x} ${view.y} ${view.w} ${view.h}`);
    this.#pushView();
    this.#render();
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
      doors: this.#doorEntries.length,
      doorsIndexed: this.#doorIndex.size,
      windows: this.#windowEntries.length,
      windowsIndexed: this.#windowIndex.size,
      ffe: this.#ffeEntries.length,
      ffeIndexed: this.#ffeIndex.size,
      labels: this.#labels.length,
      layers: {
        grid: !!this.#grid,
        fills: !!this.#fills,
        holes: !!this.#holeLines,
        outlines: !!this.#outlines,
        doors: !!this.#doorMesh,
        windows: !!this.#windowMesh,
        ffe: !!this.#ffeMesh,
      },
      // PAINT ORDER, bottom-first — the answer to "what is covering what",
      // which is otherwise invisible from outside and cost a real bug once:
      // doors shared the room fill mesh, and the hover fill (which must sit
      // directly above that mesh) painted over every door in a hovered room.
      // A layer list is the cheapest way to see that without a screenshot.
      order: this.#root.children.map((c) => c.label || "?"),
      view: this.#view,
      rendererSize: this.#app ? [this.#app.renderer.width, this.#app.renderer.height] : null,
      resolution: this.#app?.renderer.resolution ?? null,
    };
  }

  setHover(roomId: string | null): void {
    if (this.#hovered === roomId) return;
    this.#hovered = roomId;
    this.#drawHover();
  }

  /**
   * The hover fill, as its own small GL mesh sitting between the plan and the
   * labels.
   *
   * One room, re-triangulated on each hover change — which sounds wasteful and
   * is not: it is a handful of triangles, built only when the hovered room
   * actually changes (`setHover` returns early otherwise) and behind an
   * animation-frame throttle in the page. The alternative, rewriting the shared
   * fill buffer's colours, would re-upload the whole level's vertices on every
   * pointer move.
   *
   * Added BELOW the label container on purpose. That is the entire reason this
   * is not drawn in the SVG overlay with the selection mark: the overlay is
   * above the canvas, so an opaque hover fill there covers the label of the room
   * being pointed at.
   */
  #drawHover(): void {
    this.#hoverMesh?.destroy();
    this.#hoverMesh = null;

    const entry = this.#hovered ? this.#entries.find((e) => e.room.id === this.#hovered) : undefined;
    // A room with a colour plan does NOT change on hover, and that is a
    // precedence being reproduced rather than an omission: in SVG the plan's
    // inline fill beat the `:hover` rule, so those rooms never highlighted.
    if (!entry || entry.appearanceIsPlanFill) {
      this.#render();
      return;
    }

    const batch = new FillBatch();
    // `.room.error:hover` resolves to the accent, everything else to
    // `--fill-hover`. Same two rules the stylesheet carries.
    const a = resolveRoomAppearance(entry.room, this.#lastPaint);
    const colour = a.error ? this.#palette.accent : this.#palette.fillHover;
    if (!batch.push(entry.room, colour)) {
      this.#render();
      return;
    }
    const mesh = batch.build();
    this.#hoverMesh = mesh;
    mesh.mesh.label = "hover";
    mesh.setLayerAlpha(this.#areasActive ? GHOST_ROOMS : 1);
    // Directly ABOVE the fills and below everything else — hover replaces a
    // room's FILL, nothing more. Sitting any higher covers that room's own
    // outline and its dashed hole strokes, which measurably lost ~420 ink pixels
    // when this went in just below the labels instead.
    //
    // In SVG this ordering was free: the stroke and the fill were the same
    // element, and a stroke always paints over its own fill. Split across two
    // meshes, the order has to be stated.
    const fills = this.#fills;
    const at = fills ? this.#root.getChildIndex(fills.mesh) + 1 : 0;
    this.#root.addChildAt(mesh.mesh, Math.min(at, this.#root.children.length));
    this.#pushView();
    this.#render();
  }

  setAreasActive(on: boolean): void {
    this.#areasActive = on;
    for (const mesh of this.#worldMeshes()) mesh.setLayerAlpha(on ? GHOST_ROOMS : 1);
    this.#outlines?.setLayerAlpha(on ? GHOST_ROOMS : 1);
    this.#holeLines?.setLayerAlpha(on ? GHOST_ROOMS : 1);
    this.#grid?.setLayerAlpha(on ? GHOST_ROOMS : 1);
    if (this.#labelContainer) this.#labelContainer.alpha = on ? GHOST_LABELS : 1;
    this.#render();
  }

  roomAt(clientX: number, clientY: number): Room | null {
    const p = this.toWorld(clientX, clientY);
    if (!p) return null;
    return this.#index.roomAt(p.x, p.y);
  }

  /**
   * Viewport pixels -> flipped world, or `null` when the canvas has no size.
   *
   * PUBLIC because the page pans and zooms in world units and cannot do this
   * conversion itself. `zone.view` is the RAW view; what is on screen is the
   * aspect-corrected one, and the two differ on whichever axis `meet` gave the
   * slack to. A drag converted with the raw view moves the plan by the wrong
   * number of world units on exactly that axis — the plan slides out from under
   * the pointer horizontally while tracking it perfectly vertically, which
   * reads as a mouse problem rather than a projection one.
   *
   * Everything that converts between screen and world goes through
   * `#effectiveView`, and this is how a caller outside the renderer joins that
   * rule instead of reimplementing it.
   */
  toWorld(clientX: number, clientY: number): { x: number; y: number } | null {
    // The canvas's own rect, so this is correct under any CSS sizing.
    const r = this.#canvas.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return null;
    const eff = this.#effectiveView();
    return {
      x: eff.x + ((clientX - r.left) / r.width) * eff.w,
      y: eff.y + ((clientY - r.top) / r.height) * eff.h,
    };
  }

  pickAt(clientX: number, clientY: number): Pick | null {
    const p = this.toWorld(clientX, clientY);
    if (!p) return null;
    // Doors first. A door glyph is drawn over the room it serves and is far
    // smaller, so a click inside one is a click on the door — resolving to the
    // room instead would make a door selectable only where it happens to poke
    // outside its own wall.
    const door = this.#doorIndex.doorAt(p.x, p.y);
    if (door) return { kind: "door", door };
    // Windows next, on the same argument and before rooms. Doors are tried
    // first only because where the two could overlap -- they hardly ever do,
    // an opening being one or the other -- a fixed order beats an ambiguous
    // one, and doors were here first.
    const window = this.#windowIndex.doorAt(p.x, p.y);
    if (window) return { kind: "window", window };

    // FF&E last of the three element layers and before rooms. An item sits
    // INSIDE a room rather than in its wall, so unlike an opening it competes
    // with the room over the same floor -- and it is the smaller, more specific
    // thing a reader aimed at, which is the rule the opening layers already
    // follow. It loses to an opening only where the two overlap, which is a
    // chair pushed against a door: the door is the fixed thing and the more
    // likely target.
    const item = this.#ffeIndex.doorAt(p.x, p.y) as Item | null;
    if (item) return { kind: "item", item };
    const room = this.#index.roomAt(p.x, p.y);
    return room ? { kind: "room", room } : null;
  }

  setDoorSelection(doorId: string | null): void {
    if (this.#selectedDoor === doorId) return;
    this.#selectedDoor = doorId;
    this.#drawMarks();
  }

  get doorSelection(): string | null {
    return this.#selectedDoor;
  }

  setItemSelection(itemId: string | null): void {
    if (this.#selectedItem === itemId) return;
    this.#selectedItem = itemId;
    this.#drawMarks();
  }

  get itemSelection(): string | null {
    return this.#selectedItem;
  }

  setWindowSelection(windowId: string | null): void {
    if (this.#selectedWindow === windowId) return;
    this.#selectedWindow = windowId;
    this.#drawMarks();
  }

  get windowSelection(): string | null {
    return this.#selectedWindow;
  }

  dispose(): void {
    this.#destroyLayers();
    this.#entries = [];
    this.#labels = [];
    this.#index = new RoomIndex([]);
    this.#doorIndex = new DoorIndex([]);
    this.#doorEntries = [];
    this.#windowIndex = new DoorIndex([]);
    this.#windowEntries = [];
    this.#ffeIndex = new DoorIndex([]);
    this.#ffeEntries = [];
    this.#rooms = [];
    this.#selected = null;
    this.#selectedDoor = null;
    this.#selectedWindow = null;
    this.#selectedItem = null;
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
    this.#hoverMesh?.destroy();
    this.#doorMesh?.destroy();
    this.#windowMesh?.destroy();
    this.#ffeMesh?.destroy();
    this.#holeLines?.destroy();
    this.#outlines?.destroy();
    this.#labelContainer?.destroy({ children: true });
    this.#grid = null;
    this.#fills = null;
    this.#hoverMesh = null;
    this.#doorMesh = null;
    this.#windowMesh = null;
    this.#ffeMesh = null;
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
      // Doors go with them. A level with no rooms has no doors to draw either,
      // and leaving the previous level's index in place would keep its doors
      // clickable over an empty plan.
      this.#doorEntries = [];
      this.#doorIndex = new DoorIndex([]);
      this.#windowEntries = [];
      this.#windowIndex = new DoorIndex([]);
      this.#ffeEntries = [];
      this.#ffeIndex = new DoorIndex([]);
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
      });
    }
    this.#entries = entries;

    // DOORS GET THEIR OWN MESH, which is a correction to the handover's
    // "append into the room vertex stream".
    //
    // Sharing the rooms' batch is what the brief asks for, and it draws
    // correctly — until a room is hovered. The hover fill is its own small mesh
    // inserted directly above the room fills (see `#drawHover`, which has to sit
    // exactly there so it does not cover outlines or hole strokes), so anything
    // sharing the room mesh is painted over by it. Hovering a room made every
    // door inside it vanish.
    //
    // The cost is ONE extra draw call for the whole door layer, not one per
    // door — which is what the brief's "no second draw call added per door"
    // actually asks for. Baking the orientation still matters for exactly the
    // reason it gave: a per-door matrix would mean a draw call each.
    //
    // The pick index is built from the SAME glyph objects that were just
    // drawn, so what you can click is by construction what you can see. Two
    // computations here — one for the vertices, one for the hit target — is
    // how the two come to disagree about a door in a diagonal wall.
    const doorBatch = new FillBatch();
    const pickable: PickableDoor[] = [];
    const doorEntries: DoorEntry[] = [];
    for (const door of this.#activeDoors()) {
      const glyph = buildDoorGlyph(door);
      // `null` means the door cannot be placed at all: no footprint AND no
      // insertion point. It is not drawn and not clickable, because there is
      // nowhere to put it — which is a strictly better outcome than drawing it
      // at the origin, where it would claim to be somewhere it is not.
      if (!glyph) continue;

      const rect = glyph.rect.length
        ? doorBatch.pushTriangles(glyph.rect, withAlpha(pal.ink, DOOR_RECT_ALPHA))
        : null;
      // Arrow and cross are mutually exclusive by construction, so one range
      // covers whichever was drawn.
      const marks = glyph.arrow.length ? glyph.arrow : glyph.cross;
      const glyphRange = marks.length ? doorBatch.pushTriangles(marks, pal.ink) : null;

      doorEntries.push({ door, rect, glyph: glyphRange });
      pickable.push({ door, ring: glyph.pickRing, box: glyph.pick });
    }
    this.#doorEntries = doorEntries;
    this.#doorIndex = new DoorIndex(pickable);
    this.#doorMesh = doorBatch.isEmpty ? null : doorBatch.build();

    // WINDOWS GET THEIR OWN MESH, for the reason doors got theirs and one more.
    // The hover fill sits directly above the room fills, so anything sharing
    // that batch is painted over by it -- that is the doors argument. The extra
    // reason is the toggles: doors and windows are turned on and off
    // independently, and a shared mesh would have to be rebuilt whenever either
    // moved. Two meshes is two draw calls for the whole plan, not two per
    // element, which is what the one-draw-call-per-layer rule actually asks.
    const windowBatch = new FillBatch();
    const pickableWindows: PickableDoor[] = [];
    const windowEntries: WindowEntry[] = [];
    for (const window of this.#activeWindows()) {
      const glyph = buildWindowGlyph(window);
      // `null` means no footprint AND no insertion point: nowhere to draw it,
      // and drawing it at the origin would claim a position it does not have.
      if (!glyph) continue;

      const rect = glyph.rect.length
        ? windowBatch.pushTriangles(glyph.rect, withAlpha(pal.ink, DOOR_RECT_ALPHA))
        : null;
      // Symbol and cross are mutually exclusive by construction, so one range
      // covers whichever was drawn.
      const marks = glyph.symbol.length ? glyph.symbol : glyph.cross;
      const glyphRange = marks.length ? windowBatch.pushTriangles(marks, pal.ink) : null;

      windowEntries.push({ window, rect, glyph: glyphRange });
      pickableWindows.push({ door: window, ring: glyph.pickRing, box: glyph.pick });
    }
    this.#windowEntries = windowEntries;
    this.#windowIndex = new DoorIndex(pickableWindows);
    this.#windowMesh = windowBatch.isEmpty ? null : windowBatch.build();

    // FF&E GETS A FOURTH MESH, on the two arguments the third was given plus
    // one of its own: a furnished level carries hundreds of items against tens
    // of openings, so this is the layer a reader toggles most and the one whose
    // rebuild cost most wants isolating from the others.
    const ffeBatch = new FillBatch();
    const pickableFfe: PickableDoor[] = [];
    const ffeEntries: ItemEntry[] = [];
    for (const item of this.#activeFfe()) {
      const glyph = buildItemGlyph(item);
      // `null` means no footprint AND no insertion point: nowhere to draw it.
      if (!glyph) continue;

      const rect = glyph.rect.length
        ? ffeBatch.pushTriangles(glyph.rect, withAlpha(pal.ink, DOOR_RECT_ALPHA))
        : null;
      // Marker and rectangle are mutually exclusive by construction; the tick
      // rides with whichever was drawn, so one range covers the marks.
      const marks = glyph.marker.concat(glyph.tick);
      const glyphRange = marks.length ? ffeBatch.pushTriangles(marks, pal.ink) : null;

      ffeEntries.push({ item, rect, glyph: glyphRange });
      pickableFfe.push({ door: item as unknown as Door, ring: glyph.pickRing, box: glyph.pick });
    }
    this.#ffeEntries = ffeEntries;
    this.#ffeIndex = new DoorIndex(pickableFfe);
    this.#ffeMesh = ffeBatch.isEmpty ? null : ffeBatch.build();

    this.#grid = gridBatch.build();
    this.#fills = fillBatch.isEmpty ? null : fillBatch.build();
    // The dash is the whole reason holes get their own batch: `stroke-dasharray:
    // 4 3` on `.hole` is a visible feature, not incidental decoration.
    this.#holeLines = holeBatch.isEmpty ? null : holeBatch.build({ dash: HOLE_DASH });
    this.#outlines = outlineBatch.isEmpty ? null : outlineBatch.build();

    // Paint order is child order, and it mirrors the SVG document exactly:
    // grid behind, then fills, then the strokes that sit on them, then labels.
    if (this.#grid) { this.#grid.mesh.label = "grid"; this.#root.addChild(this.#grid.mesh); }
    if (this.#fills) { this.#fills.mesh.label = "fills"; this.#root.addChild(this.#fills.mesh); }
    if (this.#holeLines) { this.#holeLines.mesh.label = "holes"; this.#root.addChild(this.#holeLines.mesh); }
    if (this.#outlines) { this.#outlines.mesh.label = "outlines"; this.#root.addChild(this.#outlines.mesh); }
    // Above the outlines, so a glyph is never cut by the wall line it sits in,
    // and above the hover mesh, so hovering a room cannot hide its doors. Below
    // the labels, because a door covering a room's name would trade one
    // unreadable thing for another.
    if (this.#doorMesh) { this.#doorMesh.mesh.label = "doors"; this.#root.addChild(this.#doorMesh.mesh); }
    // Windows above doors. They rarely overlap -- an opening is either one or
    // the other -- so the order decides almost nothing; where it does, the
    // window symbol reaching into a room is the thing that would be cut, and a
    // half-drawn symbol reads as a fault rather than as an overlap.
    if (this.#windowMesh) { this.#windowMesh.mesh.label = "windows"; this.#root.addChild(this.#windowMesh.mesh); }
    // FF&E BELOW the openings would be wrong and above them is right, which is
    // the opposite of what "the biggest layer goes at the back" would suggest.
    // An item is a thing standing in a room; an opening is a hole in the wall
    // around it. Where a chair meets a door the chair is in front of it in the
    // world, and drawing it behind would read as the door being on top of the
    // furniture. Below the labels, on the rule every element layer follows.
    if (this.#ffeMesh) { this.#ffeMesh.mesh.label = "ffe"; this.#root.addChild(this.#ffeMesh.mesh); }

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
      built.container.label = "labels";
      built.container.enableRenderGroup();
      this.#root.addChild(built.container);
    } else {
      this.#labels = [];
    }

    // ORDER MATTERS, and getting it wrong is not subtle: the projection uniforms
    // must be pushed BEFORE anything renders. `setAreasActive` renders, and
    // `cull` renders only when a visibility flag actually changed — so with
    // `#pushView` last, the final frame of a repaint was drawn with the previous
    // level's uniforms and the zone showed a black panel until the next pan
    // happened to call `setView`. "Level switch shows nothing until you drag" is
    // what that looks like from outside.
    this.#pushView();
    this.setAreasActive(this.#areasActive);
    // A repaint rebuilds `#entries`, so the marks point at rooms that no longer
    // exist. Re-derived here, which is the GL equivalent of `renderLevel`
    // re-applying the selection class after `paintLevel` rebuilt every node.
    this.#drawMarks();
    this.#drawHover();
    // Unconditional, because none of the calls above is guaranteed to have
    // rendered. A repaint that draws nothing is the one outcome this method
    // must never have.
    this.#render();
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
    // NOTE: hover is NOT drawn here. It is a FILL, and a fill drawn into this
    // overlay sits above the canvas — and therefore above the labels, which the
    // canvas draws. It would hide the very label the user is pointing at.
    //
    // The old SVG renderer had no such problem: hover recoloured the room
    // polygon itself, and labels were appended after every polygon, so they
    // stayed on top. Reproducing that ordering means the hover fill belongs in
    // the GL layer, underneath the label container — see `#drawHover`.
    //
    // Selection is different and does belong here: it is a STROKE with
    // `fill: none`, so it rings the room without covering anything.
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

    // A selected DOOR gets the same treatment, from the same group. Drawn after
    // the room mark so it wins where a door is selected inside a selected room,
    // which is the normal case — you select a door by clicking into a room.
    //
    // Its ring is the glyph's PICK ring, not the raw footprint: for a door with
    // no geometry that is the square the arrow was drawn in, so the mark lands
    // on the thing the user actually clicked. A mark derived from `loops` would
    // simply not appear for those doors, which reads as "selecting that door
    // does nothing".
    const door = this.#selectedDoor
      ? this.#doorEntries.find((e) => e.door.id === this.#selectedDoor)
      : undefined;
    if (door) {
      const glyph = buildDoorGlyph(door.door);
      if (glyph) {
        const p = doc.createElementNS(SVG_NS, "polygon") as SVGPolygonElement;
        // Already flipped, and the mark layer is in flipped space — so this is
        // written out rather than run through `pointsAttr`, which flips as it
        // formats and would put the mark at the mirror image of the door.
        p.setAttribute("points", glyph.pickRing.map((q) => `${q.x},${q.y}`).join(" "));
        p.setAttribute("class", "door-selected-mark");
        g.appendChild(p);
      }
    }

    // The same mark for a selected window, and deliberately the same CSS class:
    // it means "this is what you picked", which is one idea. A second style
    // would imply a second meaning.
    const window = this.#selectedWindow
      ? this.#windowEntries.find((e) => e.window.id === this.#selectedWindow)
      : undefined;
    if (window) {
      const glyph = buildWindowGlyph(window.window);
      if (glyph) {
        const p = doc.createElementNS(SVG_NS, "polygon") as SVGPolygonElement;
        p.setAttribute("points", glyph.pickRing.map((q) => `${q.x},${q.y}`).join(" "));
        p.setAttribute("class", "door-selected-mark");
        g.appendChild(p);
      }
    }

    // And for a selected item, same class again -- three element layers, one
    // idea. Its ring is the marker's, which for an item with no footprint (all
    // of them today) is the square the marker was drawn in, so the mark lands
    // exactly on what was clicked.
    const item = this.#selectedItem
      ? this.#ffeEntries.find((e) => e.item.id === this.#selectedItem)
      : undefined;
    if (item) {
      const glyph = buildItemGlyph(item.item);
      if (glyph) {
        const p = doc.createElementNS(SVG_NS, "polygon") as SVGPolygonElement;
        p.setAttribute("points", glyph.pickRing.map((q) => `${q.x},${q.y}`).join(" "));
        p.setAttribute("class", "door-selected-mark");
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

  /**
   * `zone.view` widened to the canvas's aspect ratio, which is what actually
   * gets projected.
   *
   * THE SVG VIEWBOX DOES NOT STRETCH. `preserveAspectRatio` defaults to
   * `xMidYMid meet`, so SVG scales uniformly by the smaller axis and letterboxes
   * the rest — the plan keeps its proportions whatever shape the zone is. A GL
   * projection that maps the view rect onto the full canvas instead stretches
   * the drawing, badly on a wide short zone, and every room comes out the wrong
   * shape.
   *
   * It is not only a cosmetic mismatch. `zone.view` is shared with the SVG
   * overlay — the selection and hover marks, and the whole areas overlay — which
   * letterboxes because it is real SVG. If GL stretched and SVG did not, the two
   * layers would disagree about where every room is, and a selection outline
   * would sit beside the room it names.
   *
   * So the aspect correction happens ONCE, here, and everything that converts
   * between world and screen goes through it: the projection uniform, the label
   * transform, and the pick.
   */
  #effectiveView(): Rect {
    const app = this.#app;
    if (!app) return this.#view;
    return fitViewToAspect(this.#view, app.renderer.width, app.renderer.height);
  }

  /**
   * Every mesh drawn in WORLD space, which is every mesh that needs the
   * projection uniforms and the ghosting alpha.
   *
   * **A list rather than a sequence of `?.` calls, because forgetting one is a
   * bug this codebase has now shipped twice.** A mesh that is built, added to
   * the scene and never handed `setView` renders against whatever uniforms were
   * last set: the windows layer drew nothing that way, and the FF&E layer drew
   * two large black rectangles over the plan. Both passed every unit test --
   * the geometry was provably correct in both cases -- and both were found in a
   * screenshot.
   *
   * The grid, holes and outlines are NOT here: they are line meshes and take a
   * fourth argument (the device pixel ratio, which strokes are measured in), so
   * folding them in would mean a wider signature for the sake of one loop. They
   * are the three that have never been forgotten, because they predate every
   * element layer.
   */
  #worldMeshes(): FillMesh[] {
    return [this.#fills, this.#hoverMesh, this.#doorMesh, this.#windowMesh, this.#ffeMesh].filter(
      (m): m is FillMesh => m !== null,
    );
  }

  #pushView(): void {
    const app = this.#app;
    if (!app) return;
    // `renderer.width` COUNTS CSS PIXELS, NOT DEVICE PIXELS. It is
    // `view.texture.frame.width` — the logical size — while the drawing buffer
    // is that times the resolution. Reading it the other way is invisible at
    // DPR 1 and wrong at every other DPR, which is exactly how it shipped: it
    // put CSS pixels into `uPxSize` (documented as device pixels, and the space
    // lines.ts measures stroke widths in, so every stroke and dash came out DPR
    // times too thick) and divided by the resolution a second time for the
    // labels (so each label sat at 1/DPR of its correct distance from the
    // canvas corner — a room's name several rooms away from its room).
    //
    // Both units are in the names below, because the two are the same number on
    // the machine most of this gets looked at on.
    const cssW = app.renderer.width;
    const cssH = app.renderer.height;
    const dpr = app.renderer.resolution;
    const devW = cssW * dpr;
    const devH = cssH * dpr;
    const eff = this.#effectiveView();
    this.#grid?.setView(eff, devW, devH, dpr);
    this.#holeLines?.setView(eff, devW, devH, dpr);
    this.#outlines?.setView(eff, devW, devH, dpr);
    for (const mesh of this.#worldMeshes()) mesh.setView(eff, devW, devH);
    // Labels live in world space, so the scene transform carries them. This is
    // the ONE place a container transform is used, and it is correct here
    // precisely because label text SHOULD scale with the view — unlike strokes,
    // which must not (see lines.ts).
    //
    // CSS pixels, because that is the unit Pixi's stage is in: the resolution is
    // applied below the stage, where it maps onto the drawing buffer. See
    // `labelTransform`, which has no DPR parameter for that reason.
    if (this.#labelContainer) {
      const t = labelTransform(eff, cssW);
      this.#labelContainer.scale.set(t.scale, t.scale);
      this.#labelContainer.position.set(t.x, t.y);
    }
  }

  #render(): void {
    this.#app?.render();
  }
}
