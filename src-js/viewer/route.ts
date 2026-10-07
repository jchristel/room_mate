// The route tool's rules, with no store and no DOM.
//
// A route is two rooms the reader clicked and the shortest door route between
// them, computed by the server (`/projects/{id}/connectivity`). What lives here
// is the part that is a RULE rather than plumbing: which rooms may be an
// endpoint, what a click does to the two slots, and how the server's polylines
// become a zone's slice. Pure and out of the components for the reason
// `panTo.ts` is: each is small, several zones apply it to themselves, and each
// is only wrong in ways that are hard to see on screen.

import type { Room } from "../renderer/types.js";
import type { Scope } from "./scope.js";

/** Where a route stands. `null` in the store is the tool being OFF. */
export interface RouteState {
  start: string | null;
  end: string | null;
  /** The routing method asked for, or `null` for the server's default. */
  method: string | null;
  /** Every method the server offers, learned from its first answer, so the picker
   *  is whatever the server supports and never a list kept here. */
  methods: readonly RouteMethod[];
  /** Room ids no door reaches, from the server's `isolated` list; `null` until
   *  the first read for this scope lands.
   *
   *  **A room in here cannot be an endpoint**, which is the rule this tool was
   *  asked to have: a route from a room nothing connects to can only ever answer
   *  "no route", so the pick is refused instead of producing that answer. When
   *  authored connections exist the server stops listing a connected room here
   *  and the rule follows without a change on this side. */
  unreachable: ReadonlySet<string> | null;
  /** What the last refused or ignored click was, for the panel to say. */
  notice: RouteNotice | null;
  result: RouteResult;
}

export type RouteNotice =
  | { kind: "no-door"; roomId: string }
  | { kind: "loading" };

export type RouteResult =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "error"; message: string }
  | { state: "done"; path: RoutePath };

/** One routing method, as the server lists it. */
export interface RouteMethod {
  id: string;
  name: string;
  summary: string;
  reference: string;
}

/** The part of the server's `path` the page reads. */
export interface RoutePath {
  found: boolean;
  reason: string | null;
  /** The method that produced it, and anything about how it was computed. */
  method?: string;
  note?: string | null;
  distance_ft: number;
  rooms: { model_id: string; room_id: string }[];
  steps: { kind: "door" | "zone" | "vertical"; door_id?: string; zone_id?: string; point: { x: number; y: number } }[];
  segments: { level_id: string; points: { x: number; y: number }[] }[];
}

export function newRoute(): RouteState {
  return { start: null, end: null, method: null, methods: [], unreachable: null, notice: null, result: { state: "idle" } };
}

/**
 * What a click on a room does to the route.
 *
 * - A room with no door route is refused with a notice and nothing changes.
 * - Before the first read has landed nothing can be judged, so the click is
 *   ignored with a notice rather than accepted and possibly wrong.
 * - Otherwise: the first free slot is filled, start before end. Clicking an
 *   endpoint that is already placed clears it, which is how one is undone
 *   without a control. With both placed, a click on another room replaces the
 *   END: the start is the one a reader fixed first and the end is the one they
 *   are still choosing.
 */
export function pickEndpoint(route: RouteState, roomId: string): RouteState {
  if (route.unreachable === null) return { ...route, notice: { kind: "loading" } };
  if (route.unreachable.has(roomId)) return { ...route, notice: { kind: "no-door", roomId } };
  const next = { ...route, notice: null };
  if (roomId === route.start) return { ...next, start: null, result: { state: "idle" } };
  if (roomId === route.end) return { ...next, end: null, result: { state: "idle" } };
  if (route.start === null) return { ...next, start: roomId };
  return { ...next, end: roomId };
}

/** The connectivity URL for a scope: the summary, and a route when both ends
 *  are placed. Rooms go by bare id, because the plan does not know a room's
 *  model; the server refuses an id two models share and says which. */
export function connectivityUrl(
  scope: Scope,
  from: string | null,
  to: string | null,
  method: string | null = null,
): string | null {
  if (!scope.projectId) return null;
  const q = new URLSearchParams();
  if (scope.building) q.set("building", scope.building);
  if (scope.milestone) q.set("milestone", scope.milestone);
  if (from && to) {
    q.set("from", from);
    q.set("to", to);
    if (method) q.set("method", method);
  }
  const tail = q.toString();
  return `/projects/${encodeURIComponent(scope.projectId)}/connectivity${tail ? `?${tail}` : ""}`;
}

/** The slice of a route one zone draws: the polylines on its level. A route
 *  across levels draws its first run in one zone and its last in another, which
 *  is exactly why the server groups points by level. */
export function segmentsOnLevel(path: RoutePath | null, levelId: string | null): RoutePath["segments"] {
  if (!path?.found || levelId === null) return [];
  return path.segments.filter((s) => s.level_id === levelId);
}

/** An SVG `points` string in the overlay's frame (world Y is flipped, like
 *  every drawn shape). */
export function polylinePoints(points: readonly { x: number; y: number }[]): string {
  return points.map((p) => `${p.x},${-p.y}`).join(" ");
}

/** A room's marker position: the mean of its outer ring. Only a mark, so a
 *  centroid outside an L-shaped room is acceptable here, unlike for geometry. */
export function markerPoint(room: Pick<Room, "loops">): { x: number; y: number } | null {
  const ring = room.loops?.[0]?.points;
  if (!ring?.length) return null;
  const n = ring.length;
  return { x: ring.reduce((s, p) => s + p.x, 0) / n, y: ring.reduce((s, p) => s + p.y, 0) / n };
}

/** The panel's one-line reading of a result. */
export function describeResult(result: RouteResult): string {
  switch (result.state) {
    case "idle":
      return "";
    case "loading":
      return "Finding route…";
    case "error":
      return result.message;
    case "done": {
      if (!result.path.found) return result.path.reason ?? "No route through doors.";
      const count = (kind: string) => result.path.steps.filter((s) => s.kind === kind).length;
      const doors = count("door");
      const zones = count("zone");
      const levels = count("vertical");
      // Said apart, because a door is the model's word and an open-zone hop is a
      // person's, and a reader weighing a route should be able to tell which.
      const parts = [`${doors} door${doors === 1 ? "" : "s"}`];
      if (zones > 0) parts.push(`${zones} open-zone hop${zones === 1 ? "" : "s"}`);
      if (levels > 0) parts.push(`${levels} level change${levels === 1 ? "" : "s"}`);
      return `${parts.join(", ")}, about ${Math.round(result.path.distance_ft)} ft`;
    }
  }
}
