import { describe, expect, it } from "vitest";

import {
  boxHandles,
  boxPoint,
  boxRing,
  chooseBlueprint,
  describeTool,
  ghostCorners,
  initialFrame,
  moveBox,
  newBlueprintTool,
  pickRoom,
  resolveUrl,
  ringMean,
  rotateBox,
  stepRotation,
  stretchBox,
  type BlueprintToolState,
  type BoxFrame,
} from "./blueprintTool.js";

const ready = (over: Partial<BlueprintToolState> = {}): BlueprintToolState => ({ ...newBlueprintTool(), blueprintId: "bp 1", roomId: "r9", ...over });

describe("the blueprint tool", () => {
  it("asks nothing until it has a blueprint and a room", () => {
    expect(resolveUrl("p", newBlueprintTool())).toBeNull();
    expect(resolveUrl("p", { ...newBlueprintTool(), blueprintId: "b" })).toBeNull();
    expect(resolveUrl("p", ready())).toBe("/projects/p/blueprints/bp%201/resolve?room_id=r9");
  });

  it("carries a revision and a rotation when they are chosen", () => {
    const url = resolveUrl("House A", ready({ revision: 2, shift: 3 }));
    expect(url).toBe("/projects/House%20A/blueprints/bp%201/resolve?room_id=r9&revision=2&shift=3");
  });

  it("moves the ghost to another room at the best fit, and leaves the same room alone", () => {
    const here = ready({ shift: 2, result: { state: "loading" } });
    expect(pickRoom(here, "r9")).toBe(here);
    const there = pickRoom(here, "r10");
    expect(there.roomId).toBe("r10");
    expect(there.shift).toBeNull();
    expect(there.result.state).toBe("idle");
  });

  it("forgets the rotation when the blueprint changes but keeps the room", () => {
    const s = chooseBlueprint(ready({ shift: 2, revision: 3 }), "other");
    expect(s).toMatchObject({ blueprintId: "other", roomId: "r9", shift: null, revision: null });
  });

  it("steps through the rotations that fit and wraps round", () => {
    const cands = [{ shift: 0 }, { shift: 2 }];
    const one = stepRotation(ready(), cands, 0, 1)!;
    expect(one.shift).toBe(2);
    expect(stepRotation(ready(), cands, 2, 1)!.shift).toBe(0);
    expect(stepRotation(ready(), cands, 0, -1)!.shift).toBe(2);
  });

  it("has nothing to step through with one rotation or none", () => {
    expect(stepRotation(ready(), [{ shift: 0 }], 0, 1)).toBeNull();
    expect(stepRotation(ready(), [], 0, 1)).toBeNull();
  });

  it("draws an item's footprint from its centre, facing and size", () => {
    // 1000 along +X, 400 across: a box from x 4500..5500, y 1800..2200.
    const c = ghostCorners({ centre: { x: 5000, y: 2000 }, facing: { x: 1, y: 0 }, size_mm: [1000, 400] });
    const xs = c.map((p) => Math.round(p.x)).sort((a, b) => a - b);
    const ys = c.map((p) => Math.round(p.y)).sort((a, b) => a - b);
    expect([xs[0], xs[3], ys[0], ys[3]]).toEqual([4500, 5500, 1800, 2200]);
  });

  it("turns the footprint with the facing", () => {
    const c = ghostCorners({ centre: { x: 0, y: 0 }, facing: { x: 0, y: 1 }, size_mm: [1000, 400] });
    const xs = c.map((p) => Math.round(p.x));
    const ys = c.map((p) => Math.round(p.y));
    expect(Math.max(...xs)).toBe(200);
    expect(Math.max(...ys)).toBe(500);
  });

  it("says where it is in the sequence of a person's choices", () => {
    expect(describeTool(newBlueprintTool(), null)).toBe("choose a blueprint");
    expect(describeTool({ ...newBlueprintTool(), blueprintId: "b" }, null)).toBe("click a room to lay it in");
    const done = ready({
      result: { state: "done", body: { revision: 1, room_name: "BED 02", fit: { kind: "plain", shift: 0 }, placements: [], findings: [{ kind: "outside" }], candidates: [] } },
    });
    expect(describeTool(done, "BED 02 01.02")).toBe("BED 02 01.02: 0 items, fits by shape, 1 to look at");
  });
});

const rect = (w: number, h: number, x = 0, y = 0) => [
  { x, y },
  { x: x + w, y },
  { x: x + w, y: y + h },
  { x, y: y + h },
];
const frame = (o: Partial<BoxFrame> = {}): BoxFrame => ({ dx_mm: 0, dy_mm: 0, rotate_deg: 0, sx: 1, sy: 1, ...o });
const close = (a: { x: number; y: number }, b: { x: number; y: number }, tol = 1e-6) => expect(Math.hypot(a.x - b.x, a.y - b.y)).toBeLessThan(tol);

describe("the box", () => {
  it("places points as the server does (the same numbers as Frame::place)", () => {
    // c0 (1000,1000), turned 90, stretched 2 by 0.5: a point 10 right of the pivot goes 20 up.
    const f = frame({ dx_mm: 100, dy_mm: -50, rotate_deg: 90, sx: 2, sy: 0.5 });
    const c0 = { x: 1000, y: 1000 };
    close(boxPoint({ x: 1010, y: 1000 }, c0, { x: 0, y: 0 }, f), { x: 0, y: 20 });
    close(boxPoint({ x: 1000, y: 1010 }, c0, { x: 0, y: 0 }, f), { x: -5, y: 0 });
  });

  it("is the blueprint's own outline when the box is the identity over the same room", () => {
    const bp = rect(6000, 4000);
    const ring = boxRing(bp, bp, frame());
    ring.forEach((p, i) => close(p, bp[i]!));
  });

  it("starts as a fit: stretched to fill a bigger room, centred on it", () => {
    const f = initialFrame(rect(6000, 4000), rect(7200, 4400, 1000, 500), null);
    expect(f.rotate_deg).toBe(0);
    expect(f.sx).toBeCloseTo(1.2);
    expect(f.sy).toBeCloseTo(1.1);
    const ring = boxRing(rect(6000, 4000), rect(7200, 4400, 1000, 500), f);
    const xs = ring.map((p) => p.x);
    const ys = ring.map((p) => p.y);
    expect([Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)].map(Math.round)).toEqual([1000, 8200, 500, 4900]);
  });

  it("starts turned the way the best fit turns it", () => {
    // Room edge 1 of a rectangle is vertical: blueprint edge 0 (horizontal) onto room edge 1 is a quarter turn.
    const f = initialFrame(rect(6000, 4000), rect(4000, 6000), 1);
    expect(Math.round(f.rotate_deg)).toBe(90);
    const ring = boxRing(rect(6000, 4000), rect(4000, 6000), f);
    const xs = ring.map((p) => p.x);
    const ys = ring.map((p) => p.y);
    expect([Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)].map((v) => Math.round(v) + 0)).toEqual([0, 4000, 0, 6000]);
  });

  it("moves with the pointer", () => {
    const f = moveBox(frame({ dx_mm: 10 }), { x: 0, y: 0 }, { x: 5, y: -7 });
    expect([f.dx_mm, f.dy_mm]).toEqual([15, -7]);
  });

  it("turns by the angle the pointer sweeps round the pivot", () => {
    const f = rotateBox(frame({ rotate_deg: 10 }), { x: 0, y: 0 }, { x: 100, y: 0 }, { x: 0, y: 100 });
    expect(f.rotate_deg).toBeCloseTo(100);
  });

  it("stretches along its own axes by how far the pointer went from the pivot", () => {
    const c = { x: 0, y: 0 };
    expect(stretchBox(frame(), "u", c, { x: 100, y: 0 }, { x: 150, y: 3 }).sx).toBeCloseTo(1.5);
    // Turned a quarter, 'u' points up the page.
    expect(stretchBox(frame({ rotate_deg: 90 }), "u", c, { x: 0, y: 100 }, { x: 40, y: 200 }).sx).toBeCloseTo(2);
    expect(stretchBox(frame(), "v", c, { x: 0, y: 100 }, { x: 0, y: 50 }).sy).toBeCloseTo(0.5);
  });

  it("never stretches across the pivot (that would mirror) or beyond what the server keeps", () => {
    const c = { x: 0, y: 0 };
    expect(stretchBox(frame({ sx: 2 }), "u", c, { x: 100, y: 0 }, { x: -50, y: 0 }).sx).toBe(2);
    expect(stretchBox(frame(), "u", c, { x: 100, y: 0 }, { x: 100000, y: 0 }).sx).toBe(5);
    expect(stretchBox(frame(), "u", c, { x: 100, y: 0 }, { x: 1, y: 0 }).sx).toBe(0.2);
  });

  it("puts its handles on the middle of its own sides and the turn handle on a corner", () => {
    const ring = rect(6000, 4000);
    const centre = ringMean(ring);
    const h = boxHandles(ring, centre, 0);
    close(h["u+"], { x: 6000, y: 2000 });
    close(h["v-"], { x: 3000, y: 0 });
    close(h.rotate, { x: 6000, y: 4000 });
    // Turned a quarter, 'u' points up the page and the box is 6000 tall: its u+ side is 3000 above the pivot.
    const turned = boxHandles(boxRing(ring, ring, frame({ rotate_deg: 90 })), centre, 90);
    close(turned["u+"], { x: 3000, y: 2000 + 3000 });
  });

  it("asks the server for the box and not for a rotation when there is one", () => {
    const s: BlueprintToolState = { ...newBlueprintTool(), blueprintId: "b", roomId: "r", shift: 2, frame: frame({ sx: 1.2 }) };
    const url = resolveUrl("p", s)!;
    expect(url).toContain("frame=");
    expect(url).not.toContain("shift=");
    expect(decodeURIComponent(url.split("frame=")[1]!)).toBe('{"dx_mm":0,"dy_mm":0,"rotate_deg":0,"sx":1.2,"sy":1}');
  });

  it("steps no rotation while a box is held, and a new room drops the box", () => {
    const s: BlueprintToolState = { ...newBlueprintTool(), blueprintId: "b", roomId: "r", frame: frame() };
    expect(stepRotation(s, [{ shift: 0 }, { shift: 1 }], 0, 1)).toBeNull();
    expect(pickRoom(s, "other").frame).toBeNull();
  });
});
