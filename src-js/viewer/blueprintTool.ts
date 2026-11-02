// The blueprint tool's rules, with no store and no DOM (the reach tool's shape).
//
// **Two ways to lay a blueprint into a room.** BEST FIT: the server finds the rotation that fits the
// room's shape (Rotate steps through the candidates). BOX: rooms are not perfectly identical, so a
// person lays the blueprint's own outline over the room by hand, moving, turning and stretching it
// until it matches (plan decision 6: the box is the room REFERENCE the recipes resolve against, and
// items keep their size). The box is `Frame` on the server (`assignments::Frame`), and `boxPoint`
// here is the same arithmetic, used only to DRAW the box while it is dragged.
//
// **Nothing here works out where an item goes.** The server's resolver does (`GET .../resolve`), in
// the project's plan frame, which is the frame this page draws in. This file builds the request,
// edits the box, and draws the answer's boxes. Millimetres throughout (the viewer's world is feet,
// so the overlay converts at its edge).

/** The box a person laid over a room (`assignments::Frame`): relative to the room's vertex mean. */
export interface BoxFrame {
  dx_mm: number;
  dy_mm: number;
  rotate_deg: number;
  sx: number;
  sy: number;
}

export const MIN_SCALE = 0.2;
export const MAX_SCALE = 5;

export interface BlueprintToolState {
  /** The box, when the person is laying it by hand; null is the best fit by shape. */
  frame: BoxFrame | null;
  /** The blueprint's coarse outline in mm: what the box is made of. Null until it is read. */
  outline: PlanXY[] | null;
  blueprintId: string | null;
  /** The saved revision to place; the blueprint's current one when null. */
  revision: number | null;
  /** The room the ghost is in. */
  roomId: string | null;
  /** The rotation (a cyclic shift of edges) chosen with Rotate; null is the best fit. */
  shift: number | null;
  result: BlueprintResult;
}

export type BlueprintResult =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "error"; message: string }
  | { state: "done"; body: ResolveBody };

export interface PlanXY {
  x: number;
  y: number;
}

/** The parts of the resolver's answer this page reads (`GET .../blueprints/{id}/resolve`). */
export interface ResolveBody {
  revision: number;
  room_name: string;
  fit: { kind: string; shift: number };
  outline?: { coarse: PlanXY[]; fine: PlanXY[] };
  placements: { item_id: string; type_name: string; class: string; centre: PlanXY; facing: PlanXY; size_mm: [number, number] }[];
  findings: { kind: string }[];
  candidates: { shift: number; door_gap_mm: number | null; cost: number }[];
}

export const newBlueprintTool = (): BlueprintToolState => ({
  frame: null,
  outline: null,
  blueprintId: null,
  revision: null,
  roomId: null,
  shift: null,
  result: { state: "idle" },
});

/** Choosing another blueprint forgets the rotation and the answer; the room stays, so the same room can be tried with each. */
export const chooseBlueprint = (s: BlueprintToolState, blueprintId: string | null): BlueprintToolState => ({
  ...s,
  blueprintId,
  revision: null,
  shift: null,
  frame: null,
  outline: null,
  result: { state: "idle" },
});

/** A click on a room moves the ghost there. The same room again leaves it alone (a rotation chosen
 *  there is kept); a different room starts at the best fit, because a rotation means nothing in
 *  another room. */
export function pickRoom(s: BlueprintToolState, roomId: string): BlueprintToolState {
  if (s.roomId === roomId) return s;
  return { ...s, roomId, shift: null, frame: null, result: { state: "idle" } };
}

/** The request, or null when there is not yet a blueprint and a room to ask about. */
export function resolveUrl(project: string, s: BlueprintToolState): string | null {
  if (!s.blueprintId || !s.roomId) return null;
  const q = new URLSearchParams({ room_id: s.roomId });
  if (s.revision !== null) q.set("revision", String(s.revision));
  if (s.frame) q.set("frame", JSON.stringify(s.frame));
  else if (s.shift !== null) q.set("shift", String(s.shift));
  return `/projects/${encodeURIComponent(project)}/blueprints/${encodeURIComponent(s.blueprintId)}/resolve?${q.toString()}`;
}

/** Step to the next (or previous) rotation that fits, wrapping round. `candidates` come best-first,
 *  so the first press from the best fit goes to the second best. Null when there is nothing to step
 *  through (the room's edge count differs, or a rotation was mapped by hand elsewhere). */
export function stepRotation(
  s: BlueprintToolState,
  candidates: readonly { shift: number }[],
  current: number,
  direction: 1 | -1,
): BlueprintToolState | null {
  if (s.frame || candidates.length < 2) return null;
  const at = candidates.findIndex((c) => c.shift === current);
  const next = candidates[(Math.max(at, 0) + direction + candidates.length) % candidates.length];
  return next ? { ...s, shift: next.shift, frame: null, result: { state: "idle" } } : null;
}

// ---- the box ----------------------------------------------------------------

const D2R = Math.PI / 180;

/** The mean of a ring's vertices: the reference the server uses for both the room and the blueprint. */
export function ringMean(ring: readonly PlanXY[]): PlanXY {
  const n = Math.max(ring.length, 1);
  return { x: ring.reduce((a, p) => a + p.x, 0) / n, y: ring.reduce((a, p) => a + p.y, 0) / n };
}

/** Where blueprint point `p` lands: the server's `Frame::place`. `c0` is the blueprint ring's mean
 *  and `centre` where it goes. */
export function boxPoint(p: PlanXY, c0: PlanXY, centre: PlanXY, f: BoxFrame): PlanXY {
  const x = (p.x - c0.x) * f.sx;
  const y = (p.y - c0.y) * f.sy;
  const c = Math.cos(f.rotate_deg * D2R);
  const s = Math.sin(f.rotate_deg * D2R);
  return { x: centre.x + x * c - y * s, y: centre.y + x * s + y * c };
}

/** Where the box's pivot sits: the room's vertex mean plus the box's offset. */
export const boxCentre = (room: readonly PlanXY[], f: BoxFrame): PlanXY => {
  const m = ringMean(room);
  return { x: m.x + f.dx_mm, y: m.y + f.dy_mm };
};

/** The blueprint's outline as laid over the room. */
export function boxRing(outline: readonly PlanXY[], room: readonly PlanXY[], f: BoxFrame): PlanXY[] {
  const c0 = ringMean(outline);
  const centre = boxCentre(room, f);
  return outline.map((p) => boxPoint(p, c0, centre, f));
}

const clampScale = (v: number): number => Math.min(MAX_SCALE, Math.max(MIN_SCALE, v));
const wrapDeg = (d: number): number => {
  const r = ((((d + 180) % 360) + 360) % 360) - 180;
  return r === -180 ? 180 : r;
};
const axes = (rotateDeg: number) => {
  const c = Math.cos(rotateDeg * D2R);
  const s = Math.sin(rotateDeg * D2R);
  return { u: { x: c, y: s }, v: { x: -s, y: c } };
};
const along = (p: PlanXY, o: PlanXY, a: PlanXY): number => (p.x - o.x) * a.x + (p.y - o.y) * a.y;

/** A starting box for a room: the blueprint turned the way the best fit turns it (`shift`, when the
 *  edge counts agree; otherwise not turned) and stretched to fill the room's extent along those axes.
 *  Only a first guess for the person to adjust. */
export function initialFrame(outline: readonly PlanXY[], room: readonly PlanXY[], shift: number | null): BoxFrame {
  const k = room.length;
  let rotate = 0;
  if (shift !== null && outline.length === k && k >= 3) {
    const bp0 = outline[0]!;
    const bp1 = outline[1]!;
    const r0 = room[shift % k]!;
    const r1 = room[(shift + 1) % k]!;
    rotate = wrapDeg(((Math.atan2(r1.y - r0.y, r1.x - r0.x) - Math.atan2(bp1.y - bp0.y, bp1.x - bp0.x)) / D2R));
  }
  const { u, v } = axes(rotate);
  const mean = ringMean(room);
  const ru = room.map((p) => along(p, mean, u));
  const rv = room.map((p) => along(p, mean, v));
  const c0 = ringMean(outline);
  const bx = outline.map((p) => p.x - c0.x);
  const by = outline.map((p) => p.y - c0.y);
  const span = (a: number[]) => Math.max(...a) - Math.min(...a);
  const mid = (a: number[]) => (Math.max(...a) + Math.min(...a)) / 2;
  const sx = clampScale(span(bx) > 0 ? span(ru) / span(bx) : 1);
  const sy = clampScale(span(by) > 0 ? span(rv) / span(by) : 1);
  // Put the blueprint's extent over the room's: the pivot goes where the room's extent centre is,
  // less the pivot-to-extent-centre offset after stretching and turning.
  const local = { x: mid(bx) * sx, y: mid(by) * sy };
  const c = Math.cos(rotate * D2R);
  const s = Math.sin(rotate * D2R);
  const want = { x: mean.x + u.x * mid(ru) + v.x * mid(rv), y: mean.y + u.y * mid(ru) + v.y * mid(rv) };
  const centre = { x: want.x - (local.x * c - local.y * s), y: want.y - (local.x * s + local.y * c) };
  return { dx_mm: centre.x - mean.x, dy_mm: centre.y - mean.y, rotate_deg: rotate, sx, sy };
}

export type HandleKind = "move" | "rotate" | "u+" | "u-" | "v+" | "v-";

/** The grab points of a box: where each handle is, in mm. The four stretch handles sit at the middle
 *  of the box's sides in ITS OWN axes (so they turn with it), and the turn handle is at the box's
 *  top-right corner: a corner can overlap no side handle, however far the plan is zoomed out (a
 *  handle that stands off a side by a fixed distance in the world lands on that side's handle). */
export function boxHandles(ring: readonly PlanXY[], centre: PlanXY, rotateDeg: number): Record<HandleKind, PlanXY> {
  const { u, v } = axes(rotateDeg);
  const us = ring.map((p) => along(p, centre, u));
  const vs = ring.map((p) => along(p, centre, v));
  const [u0, u1, v0, v1] = [Math.min(...us), Math.max(...us), Math.min(...vs), Math.max(...vs)];
  const at = (a: number, b: number): PlanXY => ({ x: centre.x + u.x * a + v.x * b, y: centre.y + u.y * a + v.y * b });
  return {
    move: centre,
    rotate: at(u1, v1),
    "u+": at(u1, (v0 + v1) / 2),
    "u-": at(u0, (v0 + v1) / 2),
    "v+": at((u0 + u1) / 2, v1),
    "v-": at((u0 + u1) / 2, v0),
  };
}

/** Drag the whole box by the pointer's movement. */
export const moveBox = (f: BoxFrame, from: PlanXY, to: PlanXY): BoxFrame => ({ ...f, dx_mm: f.dx_mm + (to.x - from.x), dy_mm: f.dy_mm + (to.y - from.y) });

/** Turn the box about its pivot by the angle the pointer swept round it. */
export function rotateBox(f: BoxFrame, centre: PlanXY, from: PlanXY, to: PlanXY): BoxFrame {
  const a0 = Math.atan2(from.y - centre.y, from.x - centre.x);
  const a1 = Math.atan2(to.y - centre.y, to.x - centre.x);
  return { ...f, rotate_deg: wrapDeg(f.rotate_deg + (a1 - a0) / D2R) };
}

/** Stretch along one of the box's own axes by how far the pointer moved from the pivot, about the
 *  pivot. A press at the pivot, or a drag across it, changes nothing (that would mirror). */
export function stretchBox(f: BoxFrame, axis: "u" | "v", centre: PlanXY, from: PlanXY, to: PlanXY): BoxFrame {
  const a = axes(f.rotate_deg)[axis];
  const d0 = along(from, centre, a);
  const d1 = along(to, centre, a);
  if (Math.abs(d0) < 1e-6 || d0 * d1 <= 0) return f;
  const k = d1 / d0;
  return axis === "u" ? { ...f, sx: clampScale(f.sx * k) } : { ...f, sy: clampScale(f.sy * k) };
}

/** The four corners of an item's footprint, in plan: `size_mm` is along the facing, then across it. */
export function ghostCorners(p: { centre: PlanXY; facing: PlanXY; size_mm: [number, number] }): PlanXY[] {
  const len = Math.hypot(p.facing.x, p.facing.y) || 1;
  const f = { x: p.facing.x / len, y: p.facing.y / len };
  const left = { x: -f.y, y: f.x };
  const [along, across] = [p.size_mm[0] / 2, p.size_mm[1] / 2];
  return [
    [1, 1],
    [-1, 1],
    [-1, -1],
    [1, -1],
  ].map(([a, b]) => ({
    x: p.centre.x + a! * along * f.x + b! * across * left.x,
    y: p.centre.y + a! * along * f.y + b! * across * left.y,
  }));
}

/** One line for the bar. */
export function describeTool(s: BlueprintToolState, roomName: string | null): string {
  if (!s.blueprintId) return "choose a blueprint";
  if (!s.roomId) return "click a room to lay it in";
  switch (s.result.state) {
    case "idle":
    case "loading":
      return "placing…";
    case "error":
      return s.result.message;
    case "done": {
      const b = s.result.body;
      const how = b.fit.kind === "manual" ? "mapped by hand" : b.fit.kind === "plain" ? "fits by shape" : b.fit.kind === "edge_count_differs" ? "different number of edges: map it by hand on the blueprints page" : b.fit.kind.replace(/_/g, " ");
      const flagged = b.findings.filter((f) => f.kind === "outside" || f.kind === "overlap").length;
      return `${roomName ?? b.room_name}: ${b.placements.length} items, ${how}${flagged ? `, ${flagged} to look at` : ""}`;
    }
  }
}
