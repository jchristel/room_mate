// The appearance DECISION, in one place.
//
// Why this module exists at all: `paintLevel` used to be shared verbatim
// between the live renderer and the SVG export, and its doc comment said why —
// so the two "can't drift". A WebGL renderer cannot serialize to `.svg`, so
// that sharing has to be replaced by something, and the something must not be
// "two painters that happen to agree" (HANDOVER-webgl-renderer.md, Decision 3).
//
// So: one pure function resolves what a room LOOKS like, and two emitters
// consume it — the SVG painter (export) and, from P3, the GL renderer. What
// must not drift is the decision, and after this there is only one copy of it.

import type { AppearanceContext, Room, RoomAppearance } from "./types.js";

/**
 * Resolve one room's appearance. Pure: no DOM, no GL, no globals.
 *
 * The precedence encoded here is the whole point, so it is worth stating:
 *
 *   - **fill** — an active colour plan WINS over the error highlight, which
 *     wins over the default `--fill`. The plan's colour is a resolved literal,
 *     which is what lets it survive serialization into a saved `.svg`
 *     unchanged, and what makes it directly usable as a per-vertex colour.
 *   - **error** — reported only while `showErrors` is on. That flag follows the
 *     QA panel's expansion, so a level can have errors and correctly show none.
 *   - **match** / **dim** — both are search state and only exist while a search
 *     is active. They compose with fill rather than replacing it: `match` is a
 *     stroke and `dim` is opacity, so a colour-plan fill survives both.
 */
export function resolveRoomAppearance(room: Room, ctx: AppearanceContext): RoomAppearance {
  const isMatch = !!(ctx.searchActive && ctx.matchRoomIds && ctx.matchRoomIds.has(room.id));
  return {
    fill: ctx.colourFor ? ctx.colourFor(room) : null,
    error: !!(ctx.showErrors && ctx.errorRoomIds && ctx.errorRoomIds.has(room.id)),
    match: isMatch,
    // Every non-match dims while a search is running, so the matches stand out.
    dim: !!ctx.searchActive && !isMatch,
  };
}

/**
 * The `class` attribute for a room's outer polygon.
 *
 * The ORDER of these tokens is not cosmetic. `paintLevel`'s output is compared
 * byte-for-byte against a golden file, and the class string is serialized into
 * every exported `.svg`, so reordering them would show up as a diff on every
 * room in every export. Kept as it has always been emitted:
 * `room` -> `error` -> `match` -> `dim`.
 */
export function roomClassName(a: RoomAppearance): string {
  let cls = "room";
  if (a.error) cls += " error";
  if (a.match) cls += " match";
  if (a.dim) cls += " dim";
  return cls;
}

/** A void's class. Holes carry `dim` but never `error` or `match` — those are
 *  statements about a ROOM, and a hole is a subtraction from one. */
export function holeClassName(a: RoomAppearance): string {
  return a.dim ? "hole dim" : "hole";
}
