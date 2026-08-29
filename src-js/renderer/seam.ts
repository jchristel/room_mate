// The renderer seam: everything a zone asks of whatever draws its plan.
//
// There is ONE implementation now (../gl/renderer.ts). The interface earned its
// place before that was true: it existed so the SVG renderer and the WebGL one
// could be swapped under a running page and compared on the same data in the
// same session, which is what kept the WebGL change from being a thousand-line
// diff nobody could read.
//
// It is kept rather than inlined because it is the contract that says what a
// plan renderer OWES the page — paint, pan, highlight, mark, pick, dispose —
// independently of how any of it is drawn. That was worth writing down while
// two implementations disagreed about the how, and it is still worth having
// written down now that the surviving one is several files of shaders and
// buffers whose entry points would otherwise be guesswork.
//
// Note what is NOT here any more: `cull()`. Viewport culling was an SVG
// necessity — the DOM charges per element per frame — and the WebGL renderer
// has nothing to cull, because its frame is four draw calls regardless of room
// count. It went with the SVG path rather than surviving as a no-op nobody
// could delete.
//
// Two rules about what belongs here, both from HANDOVER-webgl-renderer.md's
// Decision 1 (hybrid, only the bulk layer moves):
//
//   - Things there are THOUSANDS of are behind the seam: fills, holes,
//     outlines, labels, the cull and the pick.
//   - Things there are DOZENS of are not: the areas overlay stays SVG and stays
//     entirely outside this interface. `renderAreasOverlay` and `areaAtNode` are
//     untouched by the whole exercise.
//
// `setSelection` and `setHover` sit inside the seam even though each concerns
// exactly ONE room, and that is deliberate: today they toggle a CSS class on a
// node, and after P4 the GL renderer draws them into a thin SVG overlay
// instead. Same call site, two implementations — which is the entire point of
// having a seam.

import type { Door, Rect, Room } from "./types.js";

/** Search state, applied WITHOUT a re-render. Preserving that property is an
 *  explicit obligation: a search can match thousands of rooms, and a keystroke
 *  must not re-upload a level. */
export interface HighlightState {
  searchActive: boolean;
  matchRoomIds: ReadonlySet<string> | null;
}

/** Everything needed to draw a level. Mirrors the appearance context plus the
 *  toggles that are page state rather than per-room state. */
export interface PaintRequest {
  colourFor?: ((room: Room) => string | null) | undefined;
  errorRoomIds?: ReadonlySet<string> | null | undefined;
  showErrors?: boolean | undefined;
  matchRoomIds?: ReadonlySet<string> | null | undefined;
  searchActive?: boolean | undefined;
  showLabels?: boolean | undefined;
  /**
   * The doors to draw on this level, already scoped by the caller.
   *
   * Carried in the paint REQUEST rather than as a second positional argument
   * to `paint`, so every existing call site keeps working unchanged. That is
   * not only convenience: `static/index.html` is thousands of lines of inline
   * JavaScript that TypeScript cannot check, so a signature change there is a
   * change no compiler would verify.
   */
  doors?: readonly Door[] | undefined;
  /** Whether to draw them. Absent means yes when `doors` is non-empty — the
   *  toggle is the page's state, and a renderer given doors and no instruction
   *  should draw them. */
  showDoors?: boolean | undefined;
}

/**
 * What the pick found. A discriminated union rather than `Room | Door`, because
 * the two share `id` and `loops` and would otherwise be told apart by
 * duck-typing on a field that both may carry.
 */
export type Pick = { kind: "room"; room: Room } | { kind: "door"; door: Door };

export interface PlanRenderer {
  /** Draw a level, framed to `fitted`. Replaces whatever was drawn before. */
  paint(rooms: readonly Room[], fitted: Rect, opts?: PaintRequest): void;

  /** Pan/zoom. Must NOT rebuild geometry — this is the per-frame path. */
  setView(view: Rect): void;

  /** Search match/dim, as a state change over already-drawn rooms. */
  applyHighlight(state: HighlightState): void;

  /** The one selected room, or `null`. Never drawn into an export. */
  setSelection(roomId: string | null): void;

  /** The one selected door, or `null`. Kept separate from `setSelection`
   *  rather than merged into a tagged argument, so the existing room call
   *  sites in `static/index.html` are untouched by doors existing. */
  setDoorSelection(doorId: string | null): void;

  /** The one hovered room, or `null`. */
  setHover(roomId: string | null): void;

  /** Areas mode ghosts the rooms beneath the footprint overlay. A cross-layer
   *  rule: the overlay is SVG, the rooms may be GL, and it is easy to miss and
   *  visibly wrong when missed. */
  setAreasActive(on: boolean): void;

  /**
   * A viewport point in world coordinates, or `null` when the plan has no size.
   *
   * The page owns pan and zoom — it holds the view rect and hands it back
   * through `setView` — but it cannot convert a pointer position itself,
   * because the rect it holds is not the rect on screen: aspect correction sits
   * between them, and only the renderer knows the canvas. A caller that does
   * the division itself gets one axis right and the other wrong by whatever
   * letterboxing added, which is a pan that drifts on one axis only.
   */
  toWorld(clientX: number, clientY: number): { x: number; y: number } | null;

  /** The room under a viewport point, or `null` for empty space. */
  roomAt(clientX: number, clientY: number): Room | null;

  /**
   * The room OR door under a viewport point, or `null`.
   *
   * Doors win where both are under the cursor, which is most places a door is:
   * a door glyph is drawn over the room it serves, it is much smaller, and
   * clicking a thing you can see should select that thing. `roomAt` is kept
   * beside this — unchanged, still room-only — because the two existing call
   * sites want exactly that, and widening their return type would have made
   * every one of them handle a case it has no use for.
   */
  pickAt(clientX: number, clientY: number): Pick | null;

  /** Release everything held. For the GL renderer this frees a WebGL context,
   *  which browsers cap (commonly ~16) and silently kill the oldest of past the
   *  limit — a blanked zone with no error anyone can act on. */
  dispose(): void;
}
