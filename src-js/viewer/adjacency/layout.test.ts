import { describe, expect, it } from "vitest";

import {
  ATTRACT,
  DAMPING,
  FOCUS_R,
  NODE_R,
  SETTLE_EPS,
  SPREAD,
  TAU,
  angleDelta,
  gapCCW,
  normAngle,
  reachable,
  ringRadius,
  seed,
  step,
  type Placed,
} from "./layout.js";
import { buildView, type AdjacencyPayload } from "./view.js";

const node = (id: string, x: number, y: number) => ({ room_id: id, name: id, level_id: "L1", centroid: { x, y } });
const edge = (a: string, b: string, shared_length = 1) => ({ a, b, shared_length });

// A focus with four neighbours at the compass points, and a chain beyond east.
const star: AdjacencyPayload = {
  nodes: [
    node("f", 0, 0),
    node("e", 1, 0),
    node("n", 0, 1),
    node("w", -1, 0),
    node("s", 0, -1),
    node("e2", 2, 0),
    node("e3", 3, 0),
  ],
  edges: [edge("f", "e"), edge("f", "n"), edge("f", "w"), edge("f", "s"), edge("e", "e2"), edge("e2", "e3")],
};
const view = buildView(star, null);

const settle = (placed: Placed[], maxFrames = 600): number => {
  const radius = (hop: number) => ringRadius(hop, 3, 400, 400);
  for (let i = 1; i <= maxFrames; i++) if (step(placed, view.neighbours, radius) < SETTLE_EPS) return i;
  return Infinity;
};

describe("the tuning constants", () => {
  it("are the ones the picture was tuned with", () => {
    // Each was chosen against a picture that went visibly wrong. Changing one
    // is allowed — but here, on purpose, after looking at the graph.
    expect({ ATTRACT, SPREAD, DAMPING, SETTLE_EPS, NODE_R, FOCUS_R }).toEqual({
      ATTRACT: 0.06,
      SPREAD: 0.35,
      DAMPING: 0.82,
      SETTLE_EPS: 0.0006,
      NODE_R: 7,
      FOCUS_R: 10,
    });
  });
});

describe("angles", () => {
  it("wrap the short way round", () => {
    expect(angleDelta(0.1, TAU - 0.1)).toBeCloseTo(-0.2);
    expect(angleDelta(TAU - 0.1, 0.1)).toBeCloseTo(0.2);
    expect(angleDelta(0, Math.PI)).toBeCloseTo(Math.PI);
  });

  it("measure a one-sided gap, which is not the short way", () => {
    expect(gapCCW(0.1, TAU - 0.1)).toBeCloseTo(TAU - 0.2);
    expect(gapCCW(TAU - 0.1, 0.1)).toBeCloseTo(0.2);
  });

  it("normalise into (-π, π]", () => {
    expect(normAngle(3 * Math.PI)).toBeCloseTo(Math.PI);
    expect(normAngle(-Math.PI)).toBeCloseTo(Math.PI);
  });
});

describe("reachable", () => {
  it("assigns hop counts, capped at the depth", () => {
    expect(Object.fromEntries(reachable(view, "f", 2))).toEqual({ f: 0, e: 1, n: 1, w: 1, s: 1, e2: 2 });
  });

  it("terminates at depth Infinity", () => {
    expect(reachable(view, "f", Infinity).get("e3")).toBe(3);
  });

  it("is empty for no focus, or a focus not in the view", () => {
    expect(reachable(view, null, 2).size).toBe(0);
    expect(reachable(view, "missing", 2).size).toBe(0);
  });
});

describe("seed", () => {
  it("puts ring 1 at even angles in plan order", () => {
    const { placed } = seed(view, "f", 1);
    const ring = placed.filter((p) => p.hop === 1);
    // Bearings from the focus sort s (-π/2), e (0), n (π/2), w (π).
    expect(ring.map((p) => p.node.id)).toEqual(["s", "e", "n", "w"]);
    ring.forEach((p, i) => expect(p.angle).toBeCloseTo(normAngle((i / 4) * TAU)));
  });

  it("resolves only the edges with both ends placed, and scales to them", () => {
    const l = seed(view, "f", 1);
    expect(l.edges).toHaveLength(4);
    expect(l.maxHop).toBe(1);
    expect(seed(view, "f", Infinity).edges).toHaveLength(6);
  });

  it("draws the focus larger", () => {
    const { placed } = seed(view, "f", 1);
    expect(placed.find((p) => p.hop === 0)!.r).toBe(FOCUS_R);
    expect(placed.find((p) => p.hop === 1)!.r).toBe(NODE_R);
  });
});

describe("step", () => {
  it("leaves ring 1 where it was seeded", () => {
    // The fold into one quarter: ring 1 was sprung toward the focus's stored
    // angle of 0. It must not be attracted to anything.
    const { placed } = seed(view, "f", Infinity);
    const before = placed.filter((p) => p.hop === 1).map((p) => p.angle);
    settle(placed);
    placed.filter((p) => p.hop === 1).forEach((p, i) => expect(p.angle).toBeCloseTo(before[i]!, 6));
  });

  it("pulls a deeper node toward its parent", () => {
    const { placed } = seed(view, "f", Infinity);
    const e = placed.find((p) => p.node.id === "e")!;
    const e2 = placed.find((p) => p.node.id === "e2")!;
    e2.angle = normAngle(e.angle + 1); // start it well away
    settle(placed);
    expect(Math.abs(angleDelta(e2.angle, e.angle))).toBeLessThan(0.05);
  });

  it("settles, and stops asking for frames", () => {
    // About a second at 60 fps on this graph (67 frames when written). The
    // bound is loose on purpose: what it pins is that the loop ENDS, since a
    // sim that never drops under SETTLE_EPS is a permanent repaint.
    expect(settle(seed(view, "f", Infinity).placed)).toBeLessThan(120);
  });
});

describe("ringRadius", () => {
  it("scales to the deepest ring present, inside a label margin", () => {
    expect(ringRadius(1, 1, 400, 300)).toBe(150 - 28);
    expect(ringRadius(1, 2, 400, 300)).toBe((150 - 28) / 2);
    expect(ringRadius(2, 2, 400, 300)).toBe(150 - 28);
  });
});
