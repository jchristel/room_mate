// The reach heat map, drawn into the exported SVG.
//
// The same cells and the same colour rule as the screen (`../reach.ts`), so a
// file shows what the plan showed. What differs is only the emitting: the GL
// layer is one quad per cell, which in a file would be one element per cell and
// tens of thousands of them on a big plate. Here the walk is cut into BANDS and
// every band is ONE path of abutting squares -- one fill has no hairline seams
// between its squares, where separately filled ones would show every edge
// (`overlays.ts` does the same for glyph triangles).
//
// Banding costs a little colour resolution, and says so: a legend is drawn
// beside the plan, because a colour on paper means nothing without its scale.
// Absent when nothing is covered, like every other layer with nothing to draw.

import { REACH_ALPHA, reachColour } from "../reach.js";
import type { ReachLayer } from "../seam.js";
import type { Rect } from "../types.js";

const SVG_NS = "http://www.w3.org/2000/svg";

/** Steps of the walk, near to far. Enough to read as a gradient, few enough to
 *  keep the file small. */
const BANDS = 24;

const css = (c: readonly [number, number, number]) =>
  `rgb(${Math.round(c[0] * 255)},${Math.round(c[1] * 255)},${Math.round(c[2] * 255)})`;

/** The square of a cell, in the file's frame (y down, so the plan's y is negated),
 *  as an absolute move followed by relative lines. */
function square(x: number, y: number, s: number): string {
  return `M${x} ${y}h${s}v${s}h${-s}z`;
}

function el(doc: Document, name: string, attrs: Record<string, string | number>): SVGElement {
  const e = doc.createElementNS(SVG_NS, name);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
}

/** Whether `layer` has a single covered cell. */
function covered(layer: ReachLayer): boolean {
  return layer.distance_mm.some((mm) => mm >= 0);
}

/**
 * Draw `layer` into `svg`: the cells, then the legend. Nothing at all when no
 * cell is covered. `fitted` places the legend in the file's own units.
 */
export function paintReach(svg: SVGElement, layer: ReachLayer | undefined, fitted: Rect): void {
  if (!layer || !covered(layer)) return;
  const doc = svg.ownerDocument;
  const { origin, spacing_ft: s, cols, rows, distance_mm: d, limit_mm: limit } = layer;
  const alpha = layer.alpha ?? REACH_ALPHA;

  const paths: string[][] = Array.from({ length: BANDS }, () => []);
  for (let row = 0; row < rows; row++) {
    for (let col = 0; col < cols; col++) {
      const mm = d[row * cols + col];
      if (mm === undefined || mm < 0) continue;
      const t = limit > 0 ? Math.min(Math.max(mm / limit, 0), 1) : 0;
      const band = Math.min(BANDS - 1, Math.floor(t * BANDS));
      // The cell spans origin + (col, row) * s to the next corner; its top edge in
      // the file is the plan's far edge, negated.
      const x = Math.round((origin.x + col * s) * 1e4) / 1e4;
      const y = Math.round(-(origin.y + (row + 1) * s) * 1e4) / 1e4;
      paths[band]!.push(square(x, y, Math.round(s * 1e4) / 1e4));
    }
  }

  const group = el(doc, "g", { class: "reach", "fill-opacity": alpha, stroke: "none" });
  paths.forEach((cells, band) => {
    if (cells.length === 0) return;
    group.appendChild(el(doc, "path", { d: cells.join(""), fill: css(reachColour(((band + 0.5) / BANDS) * limit, limit)) }));
  });
  svg.appendChild(group);
  svg.appendChild(legend(doc, layer, fitted));
}

/** A colour bar from 0 to the limit, bottom left (the routes legend is top left). */
function legend(doc: Document, layer: ReachLayer, fitted: Rect): SVGElement {
  const u = fitted.w / 1600; // one pixel of the 1600 px export
  const w = 160 * u;
  const h = 12 * u;
  const x0 = fitted.x + 12 * u;
  const y0 = fitted.y + fitted.h - 12 * u - h;
  const g = el(doc, "g", { class: "reach-legend" });
  g.appendChild(
    el(doc, "rect", { x: x0 - 6 * u, y: y0 - 22 * u, width: w + 12 * u, height: h + 44 * u, fill: "#fff", "fill-opacity": 0.85 }),
  );
  const step = w / BANDS;
  for (let i = 0; i < BANDS; i++) {
    g.appendChild(
      el(doc, "rect", { x: x0 + i * step, y: y0, width: step + 0.01 * u, height: h, fill: css(reachColour(((i + 0.5) / BANDS) * layer.limit_mm, layer.limit_mm)) }),
    );
  }
  const text = (x: number, anchor: string, body: string) => {
    const t = el(doc, "text", { x, y: y0 - 6 * u, "font-size": 11 * u, "text-anchor": anchor, fill: "#222", class: "reach-legend-text" });
    t.textContent = body;
    return t;
  };
  const metres = layer.limit_mm / 1000;
  g.appendChild(text(x0, "start", "Walk 0 m"));
  g.appendChild(text(x0 + w, "end", `${Number.isInteger(metres) ? metres : metres.toFixed(1)} m`));
  return g;
}
