// The reach tool's rules, with no store and no DOM.
//
// A reach is some rooms the reader clicked, a walk length, and the server's grid of
// walking distances from the NEAREST of those rooms (`POST /projects/{id}/reach`).
// What lives here is the part that is a RULE rather than plumbing: what a click does
// to the list of start points, how the typed numbers become the request, and how an
// answer becomes the layer one zone draws. Pure for the reason `route.ts` is.

import type { ReachLayer } from "../renderer/seam.js";
import type { PlanPoint } from "./route.js";
import type { Scope } from "./scope.js";

export type ReachMethod = "exact" | "grid";

/** One start point: a room, and where in it when a plan click set that. */
export interface ReachSource {
  roomId: string;
  at: PlanPoint | null;
}

/** One storey's grid, as the server sends it (`service::reach::ReachLevel`). */
export interface ReachLevelBody {
  level_id: string;
  name: string;
  origin: PlanPoint;
  spacing_ft: number;
  cols: number;
  rows: number;
  distance_mm: number[];
}

/** The parts of the server's answer the page reads. */
export interface ReachBody {
  limit_mm: number;
  method: ReachMethod;
  levels: ReachLevelBody[];
  summary: {
    cells: number;
    covered: number;
    fraction: number;
    area_sqm: number;
    covered_sqm: number;
    rooms: number;
    rooms_full: number;
    rooms_partial: number;
    rooms_none: number;
  };
  note?: string;
}

export type ReachResult =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "error"; message: string }
  | { state: "done"; body: ReachBody };

export interface ReachState {
  sources: readonly ReachSource[];
  /** The walk in metres, as typed, so a half-typed number is not lost. */
  limit: string;
  /** The grid spacing in metres, as typed. */
  spacing: string;
  method: ReachMethod;
  ignoreCuts: boolean;
  /** How opaque the heat map is, in percent. */
  opacity: number;
  result: ReachResult;
}

/** The server's own bounds on the request, in mm, so a bad number is said here and
 *  not as a 400. They are repeated, not shared: the server is the authority. */
const MAX_LIMIT_MM = 1_000_000;
const MIN_SPACING_MM = 100;
const MAX_SPACING_MM = 10_000;

/** The sources one request may name, as the server allows. */
export const MAX_SOURCES = 500;

export function newReach(): ReachState {
  return { sources: [], limit: "30", spacing: "1", method: "exact", ignoreCuts: false, opacity: 55, result: { state: "idle" } };
}

/**
 * What a room click does: a room not yet a start point becomes one, and a room that
 * already is one stops being one. The answer on screen is stale either way, so the
 * result goes back to idle and the bar offers to run it again.
 */
export function pickSource(state: ReachState, roomId: string, at: PlanPoint | null = null): ReachState {
  const has = state.sources.some((s) => s.roomId === roomId);
  if (!has && state.sources.length >= MAX_SOURCES) return state;
  const sources = has ? state.sources.filter((s) => s.roomId !== roomId) : [...state.sources, { roomId, at }];
  return { ...state, sources, result: { state: "idle" } };
}

/** A typed length in metres as millimetres: a number, `"bad"` when it is not a
 *  positive one, or `null` when blank. */
export function parseMetres(text: string): number | "bad" | null {
  const t = text.trim();
  if (t === "") return null;
  const n = Number(t);
  return Number.isFinite(n) && n > 0 ? Math.round(n * 1000) : "bad";
}

/** Why the request cannot be made yet, in the reader's words, or `null` when it can. */
export function problem(state: ReachState): string | null {
  if (state.sources.length === 0) return "Click a room to start from.";
  const limit = parseMetres(state.limit);
  if (limit === null || limit === "bad" || limit > MAX_LIMIT_MM) return `Walk must be a length in metres, up to ${MAX_LIMIT_MM / 1000}.`;
  const spacing = parseMetres(state.spacing);
  if (spacing === null || spacing === "bad" || spacing < MIN_SPACING_MM || spacing > MAX_SPACING_MM) {
    return `Grid must be between ${MIN_SPACING_MM / 1000} and ${MAX_SPACING_MM / 1000} metres.`;
  }
  return null;
}

/** The URL and body of the request, or `null` when it cannot be made. */
export function reachRequest(scope: Scope, state: ReachState): { url: string; body: string } | null {
  if (!scope.projectId || problem(state) !== null) return null;
  const body = {
    sources: state.sources.map((s) => ({
      room_id: s.roomId,
      ...(s.at ? { x: s.at.x, y: s.at.y } : {}),
    })),
    limit_mm: parseMetres(state.limit),
    spacing_mm: parseMetres(state.spacing),
    method: state.method,
    ignore_cuts: state.ignoreCuts,
    ...(scope.building ? { building: scope.building } : {}),
    ...(scope.milestone ? { milestone: scope.milestone } : {}),
  };
  return { url: `/projects/${encodeURIComponent(scope.projectId)}/reach`, body: JSON.stringify(body) };
}

/**
 * The layer one zone draws, from the answer: the grid of the zone's storey, at the
 * reader's opacity. `null` when there is no answer or the walk never reached this
 * storey, so a zone on another level draws nothing rather than a wrong grid.
 */
export function layerFor(result: ReachResult, levelId: string | null, opacityPct: number): ReachLayer | null {
  if (result.state !== "done" || levelId === null) return null;
  const level = result.body.levels.find((l) => l.level_id === levelId);
  if (!level) return null;
  return {
    origin: level.origin,
    spacing_ft: level.spacing_ft,
    cols: level.cols,
    rows: level.rows,
    distance_mm: level.distance_mm,
    limit_mm: result.body.limit_mm,
    alpha: Math.min(Math.max(opacityPct, 5), 100) / 100,
  };
}

/** One line of what the answer says. */
export function describeReach(result: ReachResult): string {
  switch (result.state) {
    case "idle":
      return "";
    case "loading":
      return "Measuring…";
    case "error":
      return result.message;
    case "done": {
      const s = result.body.summary;
      const pct = Math.round(s.fraction * 100);
      return `${pct}% covered (${Math.round(s.covered_sqm)} of ${Math.round(s.area_sqm)} m²) · rooms: ${s.rooms_full} full, ${s.rooms_partial} partial, ${s.rooms_none} none`;
    }
  }
}

/** The scope a stored answer was made for, so leaving it forgets the answer. */
export function scopeKey(scope: Scope): string {
  return `${scope.projectId}|${scope.building}|${scope.milestone}`;
}
