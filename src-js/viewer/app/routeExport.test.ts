import { describe, expect, it } from "vitest";

import type { RouteDrawing } from "../route.js";
import { appendRoutes } from "./routeExport.js";

const SVG_NS = "http://www.w3.org/2000/svg";
const fitted = { x: 0, y: -100, w: 160, h: 100 };
const pal = { paper: "#f3efe6", ink: "#111111" };

function draw(drawings: RouteDrawing[], width = 1600): Element {
  const svg = document.createElementNS(SVG_NS, "svg");
  appendRoutes(svg, drawings, fitted, pal, width);
  return svg;
}

const route = (over: Partial<RouteDrawing> = {}): RouteDrawing => ({
  id: "r1",
  name: "Kitchen to study",
  colour: "#d9480f",
  bandFt: null,
  lines: [
    [
      { x: 10, y: 20 },
      { x: 40, y: 20 },
    ],
  ],
  start: { x: 10, y: 20 },
  end: { x: 40, y: 20 },
  ...over,
});

describe("appendRoutes", () => {
  it("draws nothing, not even an empty group, when no route is on the level", () => {
    expect(draw([]).children).toHaveLength(0);
  });

  it("draws each route as a haloed line in its colour, with a circle at the start and a square at the end", () => {
    const svg = draw([route()]);
    const lines = svg.querySelectorAll("g.route polyline");
    expect(lines).toHaveLength(2);
    expect(lines[0]!.getAttribute("stroke")).toBe(pal.paper);
    expect(lines[1]!.getAttribute("stroke")).toBe("#d9480f");
    expect(svg.querySelectorAll("g.route circle")).toHaveLength(1);
    expect(svg.querySelectorAll("g.route rect")).toHaveLength(1);
  });

  it("draws the object's width at true size under the line, and nothing extra for no width", () => {
    const plain = draw([route()]).querySelectorAll("g.route polyline");
    expect(plain).toHaveLength(2);
    const banded = draw([route({ bandFt: 4 })]).querySelectorAll("g.route polyline");
    expect(banded).toHaveLength(3);
    expect(banded[0]!.getAttribute("stroke-width")).toBe("4");
    expect(banded[0]!.getAttribute("stroke")).toBe("#d9480f");
    expect(banded[0]!.getAttribute("stroke-opacity")).toBe("0.28");
  });

  it("flips Y like every drawn shape", () => {
    const svg = draw([route()]);
    expect(svg.querySelector("g.route polyline")!.getAttribute("points")).toBe("10,-20 40,-20");
    expect(svg.querySelector("g.route circle")!.getAttribute("cy")).toBe("-20");
  });

  it("sizes strokes to the file: 3.5 px of a 1600 px file over 160 units is 0.35 units", () => {
    const svg = draw([route()]);
    const [halo, line] = [...svg.querySelectorAll("g.route polyline")];
    expect(Number(line!.getAttribute("stroke-width"))).toBeCloseTo(0.35, 6);
    expect(Number(halo!.getAttribute("stroke-width"))).toBeCloseTo(0.7, 6);
    // A bigger file has smaller units per pixel, so the same pixel width is a thinner line.
    const big = draw([route()], 3200);
    expect(Number([...big.querySelectorAll("g.route polyline")][1]!.getAttribute("stroke-width"))).toBeCloseTo(0.175, 6);
  });

  it("leaves out an end that is not on this level", () => {
    const svg = draw([route({ start: null })]);
    expect(svg.querySelectorAll("g.route circle")).toHaveLength(0);
    expect(svg.querySelectorAll("g.route rect")).toHaveLength(1);
  });

  it("names every route in a legend, in the colour it is drawn in", () => {
    const svg = draw([route(), route({ id: "r2", name: "Bed to lift", colour: "#1c7ed6" })]);
    const names = [...svg.querySelectorAll("g.routes-legend text")].map((t) => t.textContent);
    expect(names).toEqual(["Kitchen to study", "Bed to lift"]);
    const swatches = [...svg.querySelectorAll("g.routes-legend line")].map((l) => l.getAttribute("stroke"));
    expect(swatches).toEqual(["#d9480f", "#1c7ed6"]);
  });

  it("says in the legend how wide a route's object is", () => {
    const names = [...draw([route({ bandFt: 3.937 })]).querySelectorAll("g.routes-legend text")].map((t) => t.textContent);
    expect(names).toEqual(["Kitchen to study (1200 mm wide)"]);
  });

  it("serialises to a standalone document with no script or external reference", () => {
    const text = new XMLSerializer().serializeToString(draw([route({ name: "A <b> & C" })]));
    expect(text).toContain("A &lt;b&gt; &amp; C");
    expect(text).not.toMatch(/<script|href=|url\(/);
  });
});
