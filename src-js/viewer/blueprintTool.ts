// The blueprint tool's rules, with no store and no DOM (the reach tool's shape).
//
// **What the plan's "drag, rotate and scale a box" became.** That sketch predates the resolver, and
// under the recipe model two of its three verbs have nothing to do: the ROOM sets the position and
// the scale (plan decision 6), so the ghost is always fitted and there is nothing to drag or
// stretch. What is a real choice is which room, and how the blueprint's edges are laid on that room's
// (a rotation). So: a click on a room moves the ghost there; Rotate steps through the rotations the
// server says fit, best by doors first; and the ghost is the resolver's answer drawn on the plan.
//
// **Nothing here works out where an item goes.** The server's resolver does (`GET .../resolve`), in
// the project's plan frame, which is the frame this page draws in. This file builds the request and
// draws the answer's boxes.

export interface BlueprintToolState {
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
  placements: { item_id: string; type_name: string; class: string; centre: PlanXY; facing: PlanXY; size_mm: [number, number] }[];
  findings: { kind: string }[];
  candidates: { shift: number; door_gap_mm: number | null; cost: number }[];
}

export const newBlueprintTool = (): BlueprintToolState => ({
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
  result: { state: "idle" },
});

/** A click on a room moves the ghost there. The same room again leaves it alone (a rotation chosen
 *  there is kept); a different room starts at the best fit, because a rotation means nothing in
 *  another room. */
export function pickRoom(s: BlueprintToolState, roomId: string): BlueprintToolState {
  if (s.roomId === roomId) return s;
  return { ...s, roomId, shift: null, result: { state: "idle" } };
}

/** The request, or null when there is not yet a blueprint and a room to ask about. */
export function resolveUrl(project: string, s: BlueprintToolState): string | null {
  if (!s.blueprintId || !s.roomId) return null;
  const q = new URLSearchParams({ room_id: s.roomId });
  if (s.revision !== null) q.set("revision", String(s.revision));
  if (s.shift !== null) q.set("shift", String(s.shift));
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
  if (candidates.length < 2) return null;
  const at = candidates.findIndex((c) => c.shift === current);
  const next = candidates[(Math.max(at, 0) + direction + candidates.length) % candidates.length];
  return next ? { ...s, shift: next.shift, result: { state: "idle" } } : null;
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
