// The adjacency graph's canvas: drawing, picking, the settle loop, sizing.
//
// **Imperative on purpose, beside a React page.** React owns the controls, the
// data and the selection (`AdjacencyBand.tsx`); this class owns the pixels. The
// layout settles over a run of frames, and re-rendering a component tree per
// frame — or an SVG node per room at depth "All" on an RHH level — is the cost
// STRATEGY-BROWSER's "continuous animation" rule sends a view to canvas to
// avoid. The same rule is why the plan is WebGL and its overlays SVG: three
// renderers on one page is a deliberate outcome.
//
// What canvas costs, and how each cost is paid here:
//   - no DOM, so hit-testing is by hand — a nearest-node scan (`pick`), free at
//     tens of nodes and needing no point-in-polygon;
//   - no CSS cascade, so colours are read from the resolved :root custom
//     properties (`readPalette`), as the SVG exporter does, so the graph cannot
//     fork tokens.css;
//   - no automatic HiDPI, so the backing store is sized by devicePixelRatio;
//   - no accessibility, no text selection and no export. Accepted for a graph;
//     raster export is out of scope in STRATEGY-BROWSER.

import { qualitative } from "../palette.js";
import { positions, ringRadius, seed, SETTLE_EPS, step, TAU, type Layout, type Placed } from "./layout.js";
import { buildView, colourKey, colourKeys, type AdjacencyPayload, type GraphView, type ViewNode } from "./view.js";

const LABEL_FONT = 11; // CSS px
/** Above this many nodes on screen, only the focus, ring 1 and whatever is
 *  hovered are labelled: past it the text is the noise rather than the data. */
const LABEL_BUDGET = 24;

/** The focus states both what to centre on and at what granularity: a room, or
 *  a hierarchy group at tier `depth`. */
export type GraphFocus = { kind: "room"; id: string } | { kind: "area"; id: string; depth: number };

interface Palette {
  ink: string;
  paper: string;
  accent: string;
  rule: string;
  fill: string;
}

function readPalette(): Palette {
  const cs = getComputedStyle(document.documentElement);
  const v = (name: string, fallback: string) => cs.getPropertyValue(name).trim() || fallback;
  return {
    ink: v("--ink", "#1a1a1a"),
    paper: v("--paper", "#faf8f3"),
    accent: v("--accent", "#b4532a"),
    rule: v("--rule", "#d8d2c4"),
    fill: v("--fill", "#efe9dc"),
  };
}

export class RoomGraph {
  private readonly ctx: CanvasRenderingContext2D;
  private readonly palette: Palette;
  private payload: AdjacencyPayload | null = null;
  /** Null for room granularity, else the classification depth whose groups are
   *  the nodes. Set from the focus, because the focus is what states the
   *  granularity — selecting a footprint is a question about that tier. */
  private group: number | null = null;
  private view: GraphView = buildView(null, null);
  private focusId: string | null = null;
  private depth = 2;
  private tierDepth = 0;
  private keys: string[] = [];
  private layout: Layout = { placed: [], edges: [], maxWeight: 1, maxHop: 0 };
  private hoverId: string | null = null;
  private raf = 0;
  private cssW = 0;
  private cssH = 0;
  private readonly listeners: [string, (e: MouseEvent) => void][];

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly onSelect: (id: string, kind: "room" | "area") => void,
  ) {
    this.ctx = canvas.getContext("2d")!;
    this.palette = readPalette();
    this.listeners = [
      ["mousemove", (e) => this.hover(e)],
      ["mouseleave", () => this.setHover(null)],
      ["click", (e) => this.click(e)],
    ];
    for (const [type, fn] of this.listeners) canvas.addEventListener(type, fn as EventListener);
  }

  /** Stop the loop and let go of the canvas. The canvas unmounts with the panel,
   *  but a layout still settling would otherwise keep a rAF chain drawing into
   *  a detached element until it came to rest. */
  destroy(): void {
    cancelAnimationFrame(this.raf);
    this.raf = 0;
    for (const [type, fn] of this.listeners) this.canvas.removeEventListener(type, fn as EventListener);
  }

  setData(payload: AdjacencyPayload | null): void {
    this.payload = payload;
    this.keys = colourKeys(payload, this.tierDepth);
    this.view = buildView(payload, this.group);
    this.relayout();
  }

  setFocus(focus: GraphFocus | null): void {
    const id = focus?.id ?? null;
    const group = focus?.kind === "area" ? Math.max(0, focus.depth | 0) : null;
    if (id === this.focusId && group === this.group) return;
    this.focusId = id;
    if (group !== this.group) {
      this.group = group;
      this.view = buildView(this.payload, group);
    }
    this.relayout();
  }

  /** Infinity is legal: "as far as the graph goes". */
  setDepth(d: number): void {
    const next = d === Infinity ? Infinity : Math.max(1, d | 0);
    if (next === this.depth) return;
    this.depth = next;
    this.relayout();
  }

  setTierDepth(d: number): void {
    const next = Math.max(0, d | 0);
    if (next === this.tierDepth) return;
    this.tierDepth = next;
    this.keys = colourKeys(this.payload, next);
    this.redraw();
  }

  /** Nodes actually on screen — "how much of the level is this showing?" is the
   *  first question the depth cap raises. */
  shownCount(): number {
    return this.layout.placed.length;
  }

  /** How many nodes the focus touches, counted in the CURRENT granularity: once
   *  the view is aggregated the answer is a count of groups, which the raw room
   *  edges cannot give. */
  focusDegree(): number {
    return (this.focusId && this.view.neighbours.get(this.focusId)?.length) || 0;
  }

  /** The tier the view is aggregated to, or null for rooms. Keeps an area focus
   *  resolvable when the click came from a graph node rather than the plan. */
  groupDepth(): number | null {
    return this.group;
  }

  /** Size the backing store by devicePixelRatio, or every label is soft on a
   *  retina display. Called after layout and on every resize of the canvas. */
  resize(): void {
    const rect = this.canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    this.cssW = Math.max(rect.width, 1);
    this.cssH = Math.max(rect.height, 1);
    this.canvas.width = Math.round(this.cssW * dpr);
    this.canvas.height = Math.round(this.cssH * dpr);
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.draw();
  }

  // ---- loop ---------------------------------------------------------------

  private relayout(): void {
    this.layout = seed(this.view, this.focusId, this.depth);
    this.start();
  }

  /** Repaint without re-simulating — unless the loop is running, which will. */
  private redraw(): void {
    if (!this.raf) this.draw();
  }

  private start(): void {
    if (this.raf) return;
    const frame = () => {
      const radius = (hop: number) => ringRadius(hop, this.layout.maxHop, this.cssW, this.cssH);
      const peak = step(this.layout.placed, this.view.neighbours, radius);
      this.draw();
      // Settled: stop repainting a static picture.
      this.raf = peak < SETTLE_EPS ? 0 : requestAnimationFrame(frame);
    };
    this.raf = requestAnimationFrame(frame);
  }

  // ---- drawing ------------------------------------------------------------

  private colourFor(node: ViewNode): string {
    const k = colourKey(node.classification, this.tierDepth);
    return k == null ? this.palette.fill : qualitative("Set2", this.keys.indexOf(k));
  }

  private draw(): void {
    const { ctx, palette, cssW, cssH, layout } = this;
    positions(layout, cssW, cssH);
    ctx.clearRect(0, 0, cssW, cssH);

    if (!layout.placed.length) {
      ctx.fillStyle = palette.ink;
      ctx.globalAlpha = 0.45;
      ctx.font = `${LABEL_FONT + 1}px ui-monospace, monospace`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(
        this.focusId
          ? `selected ${this.group == null ? "room" : "area"} is not in this scope`
          : "click a room or an area on the plan",
        cssW / 2,
        cssH / 2,
      );
      ctx.globalAlpha = 1;
      return;
    }

    // Faint ring guides, so "one ring out" reads as a distance rather than
    // being inferred from where the nodes happen to sit.
    ctx.strokeStyle = palette.rule;
    ctx.lineWidth = 1;
    for (let h = 1; h <= layout.maxHop; h++) {
      ctx.beginPath();
      ctx.arc(cssW / 2, cssH / 2, ringRadius(h, layout.maxHop, cssW, cssH), 0, TAU);
      ctx.stroke();
    }

    // Edges first, so nodes sit on top. Only edges between two laid-out nodes
    // exist here — an edge to a room beyond the depth cap is not half-drawn.
    // Thickness encodes shared wall length: a 4m wall is a stronger
    // relationship than a 200mm corner touch and should look like one.
    for (const e of layout.edges) {
      const t = e.weight / layout.maxWeight;
      ctx.strokeStyle = palette.ink;
      ctx.globalAlpha = 0.18 + 0.42 * t;
      ctx.lineWidth = 1 + 3.5 * t;
      ctx.beginPath();
      ctx.moveTo(e.a.x, e.a.y);
      ctx.lineTo(e.b.x, e.b.y);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;

    ctx.font = `${LABEL_FONT}px ui-monospace, monospace`;
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    for (const p of layout.placed) {
      const isFocus = p.hop === 0;
      const isHover = p.node.id === this.hoverId;
      ctx.beginPath();
      ctx.arc(p.x, p.y, p.r + (isHover ? 2 : 0), 0, TAU);
      ctx.fillStyle = this.colourFor(p.node);
      ctx.fill();
      ctx.lineWidth = isFocus ? 3 : 1.5;
      ctx.strokeStyle = isFocus || isHover ? palette.accent : palette.ink;
      ctx.stroke();

      // Label the focus and ring 1 always; further rings only when the graph
      // is sparse enough for the text not to become the noise.
      if (isFocus || p.hop === 1 || layout.placed.length <= LABEL_BUDGET || isHover) {
        // An area node says how many rooms it stands for — without it a group
        // of one and a group of ninety are the same dot.
        const name = p.node.name || p.node.id;
        const text = p.node.rooms > 1 ? `${name} (${p.node.rooms})` : name;
        ctx.fillStyle = palette.ink;
        ctx.globalAlpha = isFocus || isHover ? 1 : 0.75;
        ctx.fillText(text.length > 22 ? `${text.slice(0, 21)}…` : text, p.x, p.y + p.r + 3);
        ctx.globalAlpha = 1;
      }
    }
  }

  // ---- interaction --------------------------------------------------------

  /** Nearest node within its own radius plus a little slack for fingers and
   *  trackpads. A linear scan: at tens of nodes this is nothing. */
  private pick(e: MouseEvent): Placed | null {
    const rect = this.canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    let best: Placed | null = null;
    let bestD = Infinity;
    for (const p of this.layout.placed) {
      const d = Math.hypot(p.x - x, p.y - y);
      if (d <= p.r + 6 && d < bestD) {
        best = p;
        bestD = d;
      }
    }
    return best;
  }

  private hover(e: MouseEvent): void {
    this.setHover(this.pick(e)?.node.id ?? null);
  }

  private setHover(id: string | null): void {
    if (id === this.hoverId) return;
    this.hoverId = id;
    this.canvas.style.cursor = id ? "pointer" : "default";
    this.redraw();
  }

  private click(e: MouseEvent): void {
    const hit = this.pick(e);
    // Clicking the focus is a no-op rather than a re-centre on itself. The kind
    // travels with the id: the page selection is kinded, and a group id handed
    // over as a room would name nothing.
    if (hit && hit.node.id !== this.focusId) this.onSelect(hit.node.id, this.group == null ? "room" : "area");
  }
}
