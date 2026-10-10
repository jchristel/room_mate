// Saved routes, drawn into an exported level's SVG.
//
// Reads the same `RouteDrawing`s the plan overlay does (`routeDrawingsOnLevel`), so a
// file shows what the screen shows. What differs is only the units: the screen strokes
// in pixels with `vector-effect: non-scaling-stroke`, and a file has no zoom, so the
// widths are converted to the file's own units from its pixel size.
//
// Each route is a line on a paper-coloured halo (so routes sharing a corridor stay
// legible), a circle at its start and a square at its end, in its own colour. **A
// legend names every route drawn on the level**, because colour alone says nothing on
// paper. Open zones are not drawn: they are authoring, not the drawing.

import { MM_PER_FT, type RouteDrawing } from "../route.js";
import type { Rect } from "../../renderer/types.js";

const SVG_NS = "http://www.w3.org/2000/svg";

/** The screen's stroke widths, in pixels. */
const LINE_PX = 3.5;
const HALO_PX = 7;
const FONT_PX = 13;
const ROW_PX = 18;
const MARK_OPACITY = 0.55;

export interface RoutePalette {
  paper: string;
  ink: string;
}

function el(name: string, attrs: Record<string, string | number>): SVGElement {
  const e = document.createElementNS(SVG_NS, name);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
}

/** Draw `drawings` over an exported level, in the file's own coordinates (world Y is
 *  flipped, like every drawn shape). `widthPx` is the file's pixel width, which with
 *  the viewBox gives the size of a pixel. */
export function appendRoutes(
  svg: SVGElement,
  drawings: readonly RouteDrawing[],
  fitted: Rect,
  pal: RoutePalette,
  widthPx: number,
): void {
  if (drawings.length === 0) return;
  const px = fitted.w / widthPx;
  const mark = Math.max(fitted.w, fitted.h) * 0.004;
  const group = el("g", { class: "routes" });

  for (const d of drawings) {
    const route = el("g", { class: "route", "data-route": d.id });
    for (const line of d.lines) {
      const points = line.map((p) => `${p.x},${-p.y}`).join(" ");
      const common = { points, fill: "none", "stroke-linejoin": "round", "stroke-linecap": "round" };
      // The object's width at true size, under everything else of the route.
      if (d.bandFt !== null) {
        route.appendChild(el("polyline", { ...common, stroke: d.colour, "stroke-width": d.bandFt, "stroke-opacity": 0.28 }));
      }
      route.appendChild(el("polyline", { ...common, stroke: pal.paper, "stroke-width": HALO_PX * px, "stroke-opacity": 0.85 }));
      route.appendChild(el("polyline", { ...common, stroke: d.colour, "stroke-width": LINE_PX * px }));
    }
    const ends = { fill: d.colour, "fill-opacity": MARK_OPACITY, stroke: pal.paper, "stroke-opacity": MARK_OPACITY, "stroke-width": 1.5 * px };
    if (d.start) route.appendChild(el("circle", { ...ends, cx: d.start.x, cy: -d.start.y, r: mark }));
    if (d.end) route.appendChild(el("rect", { ...ends, x: d.end.x - mark, y: -d.end.y - mark, width: mark * 2, height: mark * 2 }));
    group.appendChild(route);
  }

  // The legend, top left, on a paper plate so it reads over the plan.
  const pad = 8 * px;
  const row = ROW_PX * px;
  // A route drawn for an object says how wide: the band's size means nothing otherwise.
  const label = (d: RouteDrawing) => (d.bandFt !== null ? `${d.name} (${Math.round(d.bandFt * MM_PER_FT)} mm wide)` : d.name);
  const longest = Math.max(...drawings.map((d) => label(d).length));
  const plateW = pad * 2 + 22 * px + longest * FONT_PX * 0.62 * px;
  const plateH = pad * 2 + row * drawings.length - (row - FONT_PX * px);
  const x0 = fitted.x + 12 * px;
  const y0 = fitted.y + 12 * px;
  const legend = el("g", { class: "routes-legend" });
  legend.appendChild(el("rect", { x: x0, y: y0, width: plateW, height: plateH, fill: pal.paper, "fill-opacity": 0.85, stroke: pal.ink, "stroke-opacity": 0.35, "stroke-width": px }));
  drawings.forEach((d, i) => {
    const y = y0 + pad + i * row;
    legend.appendChild(el("line", { x1: x0 + pad, y1: y + (FONT_PX * px) / 2, x2: x0 + pad + 16 * px, y2: y + (FONT_PX * px) / 2, stroke: d.colour, "stroke-width": LINE_PX * px, "stroke-linecap": "round" }));
    const text = el("text", { x: x0 + pad + 22 * px, y: y + FONT_PX * px * 0.85, "font-size": FONT_PX * px, "font-family": "monospace", fill: pal.ink });
    text.textContent = label(d);
    legend.appendChild(text);
  });
  group.appendChild(legend);
  svg.appendChild(group);
}
