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
import type { Ceiling, Door, Item, Room, Space, WindowOpening } from "../types.js";
import { paintLevel, type PaintOptions } from "./paint.js";
import { exportStyle } from "./style.js";

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

/** House A's level-00 doors, the ones that stand among these rooms. */
const houseADoors = (
  JSON.parse(readFileSync(resolve(import.meta.dirname, "..", "fixtures", "house-a.doors.json"), "utf8")) as Door[]
).filter((d) => d.level_id === houseA[0]!.level_id);

/**
 * One of every other element layer, built on House A's own rooms so the golden
 * stays in frame. Synthetic on purpose: what these guard is the EMITTING -- the
 * group, the class, the paint order -- and the glyph shapes already have their
 * own tests against real exports.
 */
function houseAOverlays() {
  const [a, b, c, d] = houseA;
  const at = (room: Room) => room.loops![0]!.points[0]!;
  const ceilings: Ceiling[] = [{ id: "c1", polygons: [{ loops: a!.loops! }, { loops: b!.loops! }] }];
  const floors: Ceiling[] = [{ id: "f1", polygons: [{ loops: c!.loops! }] }];
  const spaces: Space[] = [{ id: "s1", loops: d!.loops! }];
  const windows: WindowOpening[] = [
    { id: "w1", insertion_point: at(a!), through_wall_normal: { x: 0, y: 1 } } as WindowOpening,
  ];
  const ffe: Item[] = [{ id: "i1", insertion_point: at(b!), facing: { x: 1, y: 0 } }];
  return { doors: houseADoors, windows, ffe, spaces, ceilings, floors };
}

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

describe("paintLevel element layers", () => {
  it("matches the golden file with every element layer on", () => {
    // A changed golden here is a claim that every exported overlay was wrong.
    expect(houseADoors.length).toBeGreaterThan(0);
    expect(paintToString(houseA, houseAOverlays())).toMatchFileSnapshot("./__golden__/house-a-level-00.layers.svg");
  });

  function paintLayers(opts: PaintOptions = {}): SVGElement {
    const svg = document.createElementNS(SVG_NS, "svg");
    paintLevel(svg, houseA, fittedBounds(houseA)!, { ...houseAOverlays(), ...opts });
    return svg;
  }

  it("paints the layers in the screen's order, between the rooms and the labels", () => {
    // SVG has no z-index: DOM order IS paint order, so this is the whole rule.
    const order = [...paintLayers().children].map((el) => el.getAttribute("id") ?? el.tagName);
    const at = (name: string) => order.indexOf(name);
    expect(order.lastIndexOf("polygon")).toBeLessThan(at("floors"));
    const layers = ["floors", "ceilings", "spaces", "doors", "windows", "ffe"].map(at);
    expect(layers.every((i) => i >= 0)).toBe(true);
    expect([...layers].sort((x, y) => x - y)).toEqual(layers);
    expect(at("ffe")).toBeLessThan(order.indexOf("text"));
  });

  it("draws every piece of a ceiling, not just the first", () => {
    // RHH's multi-piece ceilings are genuinely disjoint; one piece loses 44%.
    expect(paintLayers().querySelectorAll("#ceilings polygon.ceiling")).toHaveLength(2);
  });

  it("emits no group for a layer with nothing to draw", () => {
    // An empty group would change every rooms-only export, and every golden.
    expect(paintLayers({ spaces: [] }).querySelector("#spaces")).toBeNull();
  });

  it("draws overlays alone with rooms off, and no labels with them", () => {
    const svg = paintLayers({ showRooms: false });
    expect(svg.querySelectorAll("polygon.room")).toHaveLength(0);
    expect(svg.querySelectorAll("text")).toHaveLength(0);
    expect(svg.querySelector("#doors")).not.toBeNull();
  });
});

describe("exportStyle", () => {
  const pal = { ink: "#111111", fill: "#dddddd", paper: "#ffffff", accent: "#c8102e", error: "#ff0000", rule: "#cccccc" };

  it("uses the screen's dashes, so a ceiling never exports with the floor's dot", () => {
    const css = exportStyle(pal);
    expect(css).toMatch(/\.ceiling \{[^}]*stroke-dasharray: 6 4/);
    expect(css).toMatch(/\.floor \{[^}]*stroke-dasharray: 2 8/);
    expect(css).toMatch(/\.space \{[^}]*stroke: #c8102e/);
  });

  it("applies a usable override and ignores an unusable one, as the screen does", () => {
    const css = exportStyle(pal, { doors: { line: "#00ff00", fill: "not a colour" }, rooms: { fill: "#abcdef" } });
    expect(css).toMatch(/\.door-mark \{ fill: #00ff00/);
    expect(css).toMatch(/\.door-rect \{ fill: #111111; fill-opacity: 0.25/);
    expect(css).toMatch(/\.room \{ fill: #abcdef/);
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

  // The cull-unit tests that used to sit here are gone with the cull. They
  // asserted an index `paintLevel` built for the LIVE SVG renderer -- nodes[0]
  // is the room polygon, the label is last -- and both the index and the
  // renderer that read it were deleted once the plan moved to WebGL. What they
  // protected (paint order, and labels being optional) is covered by the two
  // tests either side of this comment and by the golden files.

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
