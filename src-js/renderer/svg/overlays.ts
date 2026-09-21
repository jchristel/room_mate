// The element layers, drawn into the exported SVG.
//
// **The export used to stop at rooms and labels**, so an export with ceilings
// switched on silently left them out, and nothing in the file said so. The
// layers are drawn here from the SAME inputs the screen uses: the glyph builders
// (`../gl/*Glyph.ts`, GL-free by design) for doors, windows and FF&E, and every
// ring of every piece for the surfaces. The shape decisions therefore have one
// definition, and `../style.ts` holds the look; what is left here is only
// emitting, which was never the risk.
//
// **Paint order is the screen's**, because in SVG it is DOM order and there is
// no z-index to fix it later: floors below ceilings below spaces, over the
// rooms; then doors, windows and FF&E; labels last, drawn by the caller. The GL
// renderer states why each sits where it does.
//
// **Each layer is its own `<g id=…>`**, so the file opens in Illustrator or
// Inkscape with the layers named and separately switchable -- which is what an
// exported drawing is for. A layer with nothing to draw emits no group at all:
// an empty group would change the file for every level and every golden that
// never had one.

import { pointsAttr } from "../geometry.js";
import { buildDoorGlyph } from "../gl/doorGlyph.js";
import { buildItemGlyph } from "../gl/itemGlyph.js";
import { buildWindowGlyph } from "../gl/windowGlyph.js";
import type { Ceiling, Door, Floor, Item, Loop, Space, WindowOpening } from "../types.js";

const SVG_NS = "http://www.w3.org/2000/svg";

/** The element layers to draw. A layer switched off is simply absent or empty:
 *  the caller owns visibility, as it does for the screen. */
export interface OverlayLayers {
  doors?: readonly Door[] | undefined;
  windows?: readonly WindowOpening[] | undefined;
  ffe?: readonly Item[] | undefined;
  spaces?: readonly Space[] | undefined;
  ceilings?: readonly Ceiling[] | undefined;
  floors?: readonly Floor[] | undefined;
}

/** Glyph coordinates are computed, not read, so they carry float noise to 17
 *  digits. A ten-thousandth of a foot is 0.03 mm -- far below anything a plan
 *  can show -- and trimming it roughly halves an FF&E-heavy file. */
function num(n: number): string {
  return String(Math.round(n * 1e4) / 1e4);
}

/** A flat run of independent triangles as ONE path. One element per glyph part
 *  rather than one per triangle: a single fill has no hairline seams between
 *  its triangles, where separately filled ones would show every diagonal. */
function trianglesPath(tris: readonly number[]): string {
  let d = "";
  for (let i = 0; i + 5 < tris.length; i += 6) {
    d += `M${num(tris[i]!)} ${num(tris[i + 1]!)}L${num(tris[i + 2]!)} ${num(tris[i + 3]!)}L${num(tris[i + 4]!)} ${num(tris[i + 5]!)}Z`;
  }
  return d;
}

function group(svg: SVGElement, id: string): SVGElement {
  const g = svg.ownerDocument.createElementNS(SVG_NS, "g");
  g.setAttribute("id", id);
  return g;
}

function path(g: SVGElement, tris: readonly number[], cls: string): void {
  if (!tris.length) return;
  const p = g.ownerDocument.createElementNS(SVG_NS, "path");
  p.setAttribute("d", trianglesPath(tris));
  p.setAttribute("class", cls);
  g.appendChild(p);
}

function ring(g: SVGElement, loop: Loop, cls: string): void {
  if (!loop.points?.length) return;
  const p = g.ownerDocument.createElementNS(SVG_NS, "polygon");
  p.setAttribute("points", pointsAttr(loop));
  p.setAttribute("class", cls);
  g.appendChild(p);
}

function append(svg: SVGElement, g: SVGElement): void {
  if (g.childNodes.length) svg.appendChild(g);
}

/** Every ring of every piece. A ceiling or floor is a LIST of polygons whose
 *  pieces on RHH are genuinely disjoint, and a hole is a visible edge. */
function surface(svg: SVGElement, id: string, cls: string, elements: readonly Ceiling[] | undefined): void {
  const g = group(svg, id);
  for (const element of elements ?? []) {
    for (const piece of element.polygons ?? []) for (const loop of piece.loops ?? []) ring(g, loop, cls);
  }
  append(svg, g);
}

/** The ring layers: floors, ceilings, spaces. Painted over the rooms and under
 *  the glyphs. */
export function paintSurfaces(svg: SVGElement, layers: OverlayLayers): void {
  surface(svg, "floors", "floor", layers.floors);
  surface(svg, "ceilings", "ceiling", layers.ceilings);
  // The OUTER ring only, as on screen: a space's holes are not drawn there.
  const spaces = group(svg, "spaces");
  for (const space of layers.spaces ?? []) {
    const outer = space.loops?.[0];
    if (outer) ring(spaces, outer, "space");
  }
  append(svg, spaces);
}

/** The glyph layers: doors, windows, FF&E, in that order, over the rings. A
 *  glyph the builder cannot place (no footprint AND no insertion point) is not
 *  drawn -- drawing it at the origin would claim a position it does not have. */
export function paintGlyphs(svg: SVGElement, layers: OverlayLayers): void {
  const doors = group(svg, "doors");
  for (const door of layers.doors ?? []) {
    const glyph = buildDoorGlyph(door);
    if (!glyph) continue;
    path(doors, glyph.rect, "door-rect");
    path(doors, glyph.arrow.length ? glyph.arrow : glyph.cross, "door-mark");
  }
  append(svg, doors);

  const windows = group(svg, "windows");
  for (const window of layers.windows ?? []) {
    const glyph = buildWindowGlyph(window);
    if (!glyph) continue;
    path(windows, glyph.rect, "window-rect");
    path(windows, glyph.symbol.length ? glyph.symbol : glyph.cross, "window-mark");
  }
  append(svg, windows);

  const ffe = group(svg, "ffe");
  for (const item of layers.ffe ?? []) {
    const glyph = buildItemGlyph(item);
    if (!glyph) continue;
    path(ffe, glyph.rect, "ffe-rect");
    path(ffe, glyph.marker.concat(glyph.tick), "ffe-mark");
  }
  append(svg, ffe);
}
