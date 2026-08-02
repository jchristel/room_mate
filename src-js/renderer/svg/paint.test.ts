// The golden-file guard on the SVG export.
//
// Definition of done item 3 for the WebGL work is "the SVG export produces the
// same document it produces today". A one-time manual diff proves that on the
// day it is run and guarantees nothing afterwards, which is the whole reason
// this is a test: P1 moved ~600 lines of untested rendering code between files,
// and P3-P6 replace the live renderer underneath it. Every one of those steps
// can silently change what an export contains.
//
// Regenerate deliberately, never reflexively:  npm test -- -u
// A changed golden file in a diff is a claim that every previously exported
// .svg was wrong, and it should be read that way in review.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { fittedBounds } from "../geometry.js";
import type { Room } from "../types.js";
import { paintLevel, type CullUnit, type PaintOptions } from "./paint.js";

const SVG_NS = "http://www.w3.org/2000/svg";

function fixture(name: string): Room[] {
  // `import.meta.dirname`, not a URL built from `import.meta.url`: under Vite's
  // transform the latter resolves against the module graph rather than the file
  // system, and on Windows produced `C:\src-js\...` — a path that exists
  // nowhere.
  const path = resolve(import.meta.dirname, "..", "fixtures", `${name}.rooms.json`);
  return JSON.parse(readFileSync(path, "utf8")) as Room[];
}

const houseA = fixture("house-a-level-00");
const edgeCases = fixture("edge-cases");

/** Paint into a detached <svg> and serialize, exactly as `buildLevelSvgFile`
 *  does — minus the style block and paper background, which are pure CSS-variable
 *  reads and carry no geometry decision. */
function paintToString(rooms: Room[], opts: PaintOptions = {}): string {
  const fitted = fittedBounds(rooms);
  if (!fitted) throw new Error("fixture has no drawable geometry");
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("viewBox", `${fitted.x} ${fitted.y} ${fitted.w} ${fitted.h}`);
  paintLevel(svg, rooms, fitted, opts);
  // The same call `buildLevelSvgFile` makes. Serializing here rather than
  // asserting on the DOM is the point: what ships is the SERIALIZED text, and
  // escaping bugs only exist in that form.
  return new XMLSerializer().serializeToString(svg);
}

describe("paintLevel golden output", () => {
  it("matches the golden file for House A level 00", () => {
    expect(paintToString(houseA)).toMatchFileSnapshot("./__golden__/house-a-level-00.svg");
  });

  it("matches the golden file with labels off", () => {
    // Labels off must OMIT the <text> nodes, not style them away -- an exported
    // file then simply has none.
    expect(paintToString(houseA, { showLabels: false })).toMatchFileSnapshot(
      "./__golden__/house-a-level-00.no-labels.svg",
    );
  });

  it("matches the golden file for the edge-case set", () => {
    // Holes, the three label states, escaping, a concave ring, a sub-pixel room
    // and a room with no geometry at all.
    expect(paintToString(edgeCases)).toMatchFileSnapshot("./__golden__/edge-cases.svg");
  });

  it("matches the golden file with a colour plan, errors and a search active", () => {
    // Every appearance branch at once, because they compose and the composition
    // is what a single-branch test cannot see.
    const colours = ["#66c2a5", "#fc8d62", "#8da0cb"];
    return expect(
      paintToString(edgeCases, {
        colourFor: (r) => colours[r.id.length % colours.length]!,
        errorRoomIds: new Set(["two-holes", "concave"]),
        showErrors: true,
        searchActive: true,
        matchRoomIds: new Set(["plain", "concave"]),
      }),
    ).toMatchFileSnapshot("./__golden__/edge-cases.all-states.svg");
  });
});

describe("paintLevel structure", () => {
  const fitted = fittedBounds(edgeCases)!;

  function paint(opts: PaintOptions = {}): SVGElement {
    const svg = document.createElementNS(SVG_NS, "svg");
    paintLevel(svg, edgeCases, fitted, opts);
    return svg;
  }

  it("skips a room with no loops entirely", () => {
    const svg = paint();
    const titles = [...svg.querySelectorAll("polygon.room title")].map((t) => t.textContent);
    expect(titles).not.toContain("Geometryless");
  });

  it("draws one hole polygon per void", () => {
    // Two holes on one room. A single-hole fixture cannot catch an off-by-one
    // in the loops[1..] slice.
    expect(paint().querySelectorAll("polygon.hole")).toHaveLength(2);
  });

  it("renders nothing for a present-but-empty label, and a fallback for an absent one", () => {
    // The distinction the whole three-state rule exists for. Same visual
    // outcome would be a bug in one direction or the other.
    const texts = [...paint().querySelectorAll("text.label")].map((t) => t.textContent);
    expect(texts).not.toContain("Has A Name Anyway");
    expect(texts).toContain("Fallback To Name");
  });

  it("falls back to the id when both label and name are missing", () => {
    const texts = [...paint().querySelectorAll("text.label")].map((t) => t.textContent);
    expect(texts).toContain("id-only");
  });

  it("puts every label after every polygon, because paint order is z-order", () => {
    // SVG has no reliable z-index. If a label were emitted next to its own
    // polygon, a later room's fill would paint over it.
    const kinds = [...paint().children]
      .map((el) => el.tagName)
      .filter((t) => t === "polygon" || t === "text");
    expect(kinds.lastIndexOf("polygon")).toBeLessThan(kinds.indexOf("text"));
  });

  it("collects one cull unit per drawn room, with the polygon first", () => {
    // `applyHighlight` and `applySelection` both index nodes[0] directly.
    const cullUnits: CullUnit[] = [];
    const svg = paint({ cullUnits });
    expect(cullUnits).toHaveLength(svg.querySelectorAll("polygon.room").length);
    for (const u of cullUnits) expect(u.nodes[0]!.getAttribute("class")).toMatch(/^room/);
  });

  it("keeps the label as the LAST node of its cull unit", () => {
    // The tail position is what lets the labels-off path simply shorten the
    // unit instead of renumbering it.
    const cullUnits: CullUnit[] = [];
    paint({ cullUnits });
    const withLabel = cullUnits.find((u) => u.room.id === "plain")!;
    expect(withLabel.nodes.at(-1)!.tagName).toBe("text");
  });

  it("still produces well-formed units when labels are off", () => {
    const cullUnits: CullUnit[] = [];
    paint({ cullUnits, showLabels: false });
    expect(cullUnits.every((u) => u.nodes.length >= 1)).toBe(true);
    expect(cullUnits.every((u) => u.nodes[0]!.tagName === "polygon")).toBe(true);
  });

  it("omits text nodes entirely when labels are off", () => {
    expect(paint({ showLabels: false }).querySelectorAll("text")).toHaveLength(0);
  });

  it("applies an inline fill only when a colour plan resolves one", () => {
    const withPlan = paint({ colourFor: () => "#abcdef" });
    const first = withPlan.querySelector("polygon.room") as SVGElement;
    expect(first.style.fill).toBe("rgb(171, 205, 239)");

    const noPlan = paint().querySelector("polygon.room") as SVGElement;
    expect(noPlan.style.fill).toBe("");
  });

  it("gives every room a <title> for the browser's native tooltip", () => {
    // P4 replaces this with a DOM tooltip driven by the hover pick, because
    // WebGL has no equivalent. Recorded here so its disappearance is a failing
    // test rather than a silent regression.
    const rooms = paint().querySelectorAll("polygon.room");
    for (const r of rooms) expect(r.querySelector("title")).not.toBeNull();
  });
});
