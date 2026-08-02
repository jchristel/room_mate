// The SVG painter. Appends grid + room polygons + labels for one level into an
// `<svg>`, framed to `fitted`.
//
// Reads nothing from a zone or a module global — the colour plan, error set and
// toggles all arrive as arguments — which is what lets the live view and the
// `.svg` export share one body. From P6 this is the EXPORT-ONLY painter and the
// live view is WebGL, but the sharing constraint does not relax: the export's
// output is the thing a golden test pins, and the GL renderer is expected to
// agree with it about every appearance decision (see ../appearance.ts).

import { holeClassName, resolveRoomAppearance, roomClassName } from "../appearance.js";
import { centroid, loopBox, pointsAttr, roomBBox } from "../geometry.js";
import type { AppearanceContext, Extent, Rect, Room, RoomAppearance } from "../types.js";

const SVG_NS = "http://www.w3.org/2000/svg";

/** One room's cull unit: its precomputed flipped bbox plus every node that has
 *  to hide and show together. */
export interface CullUnit extends Extent {
  room: Room;
  /** `nodes[0]` is ALWAYS the outer polygon — `applyHighlight` and
   *  `applySelection` both index it directly. Holes follow, and the label is
   *  always LAST, so dropping the label pass only shortens the tail. */
  nodes: SVGElement[];
  hidden: boolean;
}

export interface PaintOptions extends AppearanceContext {
  /** When supplied, one unit per room is collected into it so the live view can
   *  cull on pan/zoom. The export passes none: an exported file needs every
   *  room, and a detached node has no view to cull against. */
  cullUnits?: CullUnit[] | null | undefined;
  showLabels?: boolean | undefined;
}

function line(doc: Document, x1: number, y1: number, x2: number, y2: number): SVGElement {
  const l = doc.createElementNS(SVG_NS, "line");
  l.setAttribute("x1", String(x1));
  l.setAttribute("y1", String(y1));
  l.setAttribute("x2", String(x2));
  l.setAttribute("y2", String(y2));
  return l;
}

function titleEl(doc: Document, text: string): SVGElement {
  const t = doc.createElementNS(SVG_NS, "title");
  t.textContent = text;
  return t;
}

/**
 * A room's label: the primary field, then any further fields stacked smaller in
 * the accent colour. Returns `null` when there is nothing to draw.
 *
 * The three-state `label` rule lives here (see `Room.label`): absent falls back
 * to name then id; present-but-empty renders nothing at all.
 *
 * The font size is fitted to the room's OWN bbox, with `baseFont` as a ceiling.
 * 0.6 is the mono glyph aspect ratio; 0.9/0.8 are margins. Note for P3: a
 * bitmap font scales differently, so re-derive this rather than porting the
 * arithmetic across.
 */
export function addLabel(svg: SVGElement, room: Room, baseFont: number): SVGElement | null {
  const fields =
    room.label !== undefined ? room.label : [room.name || room.id].filter(Boolean);
  if (fields.length === 0) return null;

  const outer = room.loops?.[0];
  if (!outer) return null;

  const box = loopBox(outer);
  const longest = fields.reduce((n, f) => Math.max(n, String(f).length), 1);
  const widthLimited = (box.w * 0.9) / longest / 0.6;
  const heightLimited = (box.h * 0.8) / (1 + 0.7 * (fields.length - 1));
  const fontSize = Math.min(baseFont, widthLimited, heightLimited);
  // `!(x > 0)` rather than `x <= 0` so NaN is rejected too.
  if (!(fontSize > 0)) return null;

  const doc = svg.ownerDocument;
  const c = centroid(outer);
  const label = doc.createElementNS(SVG_NS, "text");
  label.setAttribute("class", "label");
  label.setAttribute("x", String(c.x));
  label.setAttribute("y", String(c.y));
  label.setAttribute("font-size", String(fontSize));

  fields.forEach((text, i) => {
    const span = doc.createElementNS(SVG_NS, "tspan");
    span.setAttribute("x", String(c.x));
    span.textContent = text;
    if (i > 0) {
      span.setAttribute("class", "tag");
      span.setAttribute("dy", "1.2em");
      span.setAttribute("font-size", String(fontSize * 0.7));
    }
    label.appendChild(span);
  });

  svg.appendChild(label);
  return label;
}

export function paintLevel(
  svg: SVGElement,
  rooms: readonly Room[],
  fitted: Rect,
  opts: PaintOptions = {},
): void {
  const { cullUnits = null, showLabels = true } = opts;
  const doc = svg.ownerDocument;
  const baseFont = Math.max(fitted.w, fitted.h) * 0.02;

  const g = doc.createElementNS(SVG_NS, "g");
  g.setAttribute("class", "grid");
  const step = 5;
  for (let gx = Math.ceil(fitted.x / step) * step; gx < fitted.x + fitted.w; gx += step)
    g.appendChild(line(doc, gx, fitted.y, gx, fitted.y + fitted.h));
  for (let gy = Math.ceil(fitted.y / step) * step; gy < fitted.y + fitted.h; gy += step)
    g.appendChild(line(doc, fitted.x, gy, fitted.x + fitted.w, gy));
  svg.appendChild(g);

  // Keyed by room across the two passes, so the label (pass 2) joins the same
  // unit as the polygons (pass 1), and so the appearance decision is made ONCE
  // per room rather than once per pass — `colourFor` is caller-supplied and
  // resolving a colour plan twice for every room is real work at plate scale.
  const units = new Map<Room, CullUnit>();
  const appearances = new Map<Room, RoomAppearance>();

  // TWO passes, not one: every room's polygons first, then every room's label.
  // SVG has no reliable z-index — paint order is DOM order — so appending all
  // polygons before any label is what guarantees labels sit on top.
  for (const room of rooms) {
    const loops = room.loops;
    const outerLoop = loops?.[0];
    if (!loops || !outerLoop) continue;

    const appearance = resolveRoomAppearance(room, opts);
    appearances.set(room, appearance);

    const outer = doc.createElementNS(SVG_NS, "polygon");
    outer.setAttribute("points", pointsAttr(outerLoop));
    outer.setAttribute("class", roomClassName(appearance));
    // Inline style, not a presentation attribute: an inline fill beats the
    // `.room` CSS rule, a `fill=` attribute loses to it.
    if (appearance.fill !== null) outer.style.fill = appearance.fill;
    outer.appendChild(titleEl(doc, room.name || room.id));
    svg.appendChild(outer);

    const nodes: SVGElement[] = [outer];
    for (let i = 1; i < loops.length; i++) {
      const hole = doc.createElementNS(SVG_NS, "polygon");
      hole.setAttribute("points", pointsAttr(loops[i]!));
      hole.setAttribute("class", holeClassName(appearance));
      svg.appendChild(hole);
      nodes.push(hole);
    }
    if (cullUnits) units.set(room, { room, ...roomBBox(room), nodes, hidden: false });
  }

  // Labels off skips the pass entirely. Omitting elements beats styling them
  // away: an exported file then simply has no `<text>` nodes.
  if (showLabels) {
    for (const room of rooms) {
      if (!room.loops?.[0]) continue;
      const label = addLabel(svg, room, baseFont);
      if (!label) continue;
      if (appearances.get(room)?.dim) label.classList.add("dim");
      units.get(room)?.nodes.push(label);
    }
  }

  if (cullUnits) for (const u of units.values()) cullUnits.push(u);
}
