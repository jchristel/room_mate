import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it } from "vitest";
import { fittedBounds } from "../geometry.js";
import type { Rect, Room } from "../types.js";
import { SvgPlanRenderer } from "./renderer.js";

const SVG_NS = "http://www.w3.org/2000/svg";

const rooms = JSON.parse(
  readFileSync(resolve(import.meta.dirname, "..", "fixtures", "edge-cases.rooms.json"), "utf8"),
) as Room[];
const fitted = fittedBounds(rooms)!;

let svg: SVGElement;
let r: SvgPlanRenderer;

beforeEach(() => {
  svg = document.createElementNS(SVG_NS, "svg");
  document.body.replaceChildren(svg);
  r = new SvgPlanRenderer(svg);
  r.setView(fitted);
  r.paint(rooms, fitted);
});

const drawnIds = () => r.units.map((u) => u.room.id);

describe("paint", () => {
  it("indexes exactly the rooms it drew", () => {
    expect(r.units).toHaveLength(svg.querySelectorAll("polygon.room").length);
    expect(drawnIds()).not.toContain("no-geometry");
  });

  it("replaces the previous level rather than appending to it", () => {
    // The failure this guards is specific: emptying the drawing without
    // emptying the index leaves units pointing at detached nodes, so a click
    // resolves to a room that is not on screen.
    const first = svg.querySelectorAll("polygon").length;
    r.paint(rooms, fitted);
    expect(svg.querySelectorAll("polygon")).toHaveLength(first);
    expect(r.units).toHaveLength(drawnIds().length);
  });

  it("leaves nothing indexed after painting an empty level", () => {
    r.paint([], fitted);
    expect(r.units).toHaveLength(0);
    expect(svg.querySelectorAll("polygon")).toHaveLength(0);
  });
});

describe("setView", () => {
  it("writes the viewBox and rebuilds no geometry", () => {
    // Pan and zoom are the per-frame path. Rebuilding here is the regression
    // the whole WebGL exercise exists to avoid.
    const before = svg.querySelectorAll("polygon")[0];
    const view: Rect = { x: 1, y: 2, w: 30, h: 40 };
    r.setView(view);
    expect(svg.getAttribute("viewBox")).toBe("1 2 30 40");
    expect(svg.querySelectorAll("polygon")[0]).toBe(before);
  });
});

describe("cull", () => {
  it("hides rooms outside the view and restores them when it widens", () => {
    r.setView({ x: -1000, y: -1000, w: 1, h: 1 });
    r.cull();
    expect(r.units.every((u) => u.hidden)).toBe(true);

    r.setView(fitted);
    r.cull();
    expect(r.units.some((u) => u.hidden)).toBe(false);
  });

  it("restores everything when the kill switch goes off mid-session", () => {
    // The load-bearing half of the switch: flipping it must not strand rooms
    // with display:none left over from the last enabled pass.
    let enabled = true;
    const killable = new SvgPlanRenderer(svg, { cullEnabled: () => enabled });
    killable.setView({ x: -1000, y: -1000, w: 1, h: 1 });
    killable.paint(rooms, fitted);
    expect(killable.units.every((u) => u.hidden)).toBe(true);

    enabled = false;
    killable.cull();
    expect(killable.units.every((u) => !u.hidden)).toBe(true);
    expect(killable.units.every((u) => u.nodes.every((n) => n.style.display === ""))).toBe(true);
  });
});

describe("applyHighlight", () => {
  it("marks matches and dims the rest without redrawing", () => {
    const before = svg.querySelectorAll("polygon")[0];
    r.applyHighlight({ searchActive: true, matchRoomIds: new Set(["plain"]) });

    const matched = r.units.filter((u) => u.nodes[0]!.classList.contains("match"));
    expect(matched.map((u) => u.room.id)).toEqual(["plain"]);
    expect(r.units.filter((u) => u.nodes[0]!.classList.contains("dim")).length).toBe(
      r.units.length - 1,
    );
    // Same node object: a keystroke must not re-upload a level.
    expect(svg.querySelectorAll("polygon")[0]).toBe(before);
  });

  it("dims a room's holes and label along with its outline", () => {
    r.applyHighlight({ searchActive: true, matchRoomIds: new Set(["plain"]) });
    const holed = r.units.find((u) => u.room.id === "two-holes")!;
    expect(holed.nodes.every((n) => n.classList.contains("dim"))).toBe(true);
  });

  it("clears every mark when the search ends", () => {
    r.applyHighlight({ searchActive: true, matchRoomIds: new Set(["plain"]) });
    r.applyHighlight({ searchActive: false, matchRoomIds: null });
    expect(
      r.units.every(
        (u) => !u.nodes[0]!.classList.contains("dim") && !u.nodes[0]!.classList.contains("match"),
      ),
    ).toBe(true);
  });
});

describe("setSelection", () => {
  it("marks exactly one room", () => {
    r.setSelection("plain");
    const marked = r.units.filter((u) => u.nodes[0]!.classList.contains("selected"));
    expect(marked.map((u) => u.room.id)).toEqual(["plain"]);
  });

  it("survives a repaint", () => {
    // renderLevel repaints on every poll tick that carries a new revision, and
    // a selection quietly vanishing on one of those is the bug this prevents.
    r.setSelection("plain");
    r.paint(rooms, fitted);
    expect(
      r.units.find((u) => u.room.id === "plain")!.nodes[0]!.classList.contains("selected"),
    ).toBe(true);
  });

  it("moves the mark, leaving none behind", () => {
    r.setSelection("plain");
    r.setSelection("concave");
    expect(
      r.units.filter((u) => u.nodes[0]!.classList.contains("selected")).map((u) => u.room.id),
    ).toEqual(["concave"]);
  });

  it("clears on null", () => {
    r.setSelection("plain");
    r.setSelection(null);
    expect(r.units.some((u) => u.nodes[0]!.classList.contains("selected"))).toBe(false);
  });

  it("marks only the outline, never the holes or the label", () => {
    // The stroke is what carries selection; putting it on a hole would draw a
    // dashed accent ring around a void.
    r.setSelection("two-holes");
    const u = r.units.find((x) => x.room.id === "two-holes")!;
    expect(u.nodes.slice(1).some((n) => n.classList.contains("selected"))).toBe(false);
  });
});

describe("setAreasActive", () => {
  it("toggles the class the ghosting rule keys on", () => {
    r.setAreasActive(true);
    expect(svg.classList.contains("areas-active")).toBe(true);
    r.setAreasActive(false);
    expect(svg.classList.contains("areas-active")).toBe(false);
  });
});

describe("dispose", () => {
  it("empties both the drawing and the index", () => {
    r.dispose();
    expect(svg.childNodes).toHaveLength(0);
    expect(r.units).toHaveLength(0);
  });

  it("drops the selection, so a later paint does not resurrect it", () => {
    r.setSelection("plain");
    r.dispose();
    r.paint(rooms, fitted);
    expect(r.units.some((u) => u.nodes[0]!.classList.contains("selected"))).toBe(false);
  });
});
