// The shapes the renderer reads off the wire, and the shapes it hands back.
//
// Deliberately a SUBSET of what `/rooms` returns. Typing the whole contract here
// would create a second, drifting copy of a carefully versioned server-side
// definition, which STRATEGY-BROWSER.md's framework discussion already names as
// a recurring cost. What is written down is exactly what the renderer touches.

/** A point in the payload's coordinate space: **Y-up**, as the model authored
 *  it. Everything downstream of `flip()` is Y-down instead; the two are not
 *  interchangeable and mixing them is the bug this comment exists to prevent. */
export interface Point2D {
  x: number;
  y: number;
}

export interface Loop {
  points: Point2D[];
}

export interface PropertyValue {
  value: string;
  storage_type: string | null;
}

export interface ClassificationTier {
  tier: string;
  name: string;
  undefined: boolean;
}

export interface Room {
  id: string;
  name?: string;
  level_id?: string;
  /** `loops[0]` is the outer ring; `loops[1..]` are voids. May be empty or
   *  absent — a door family with no 3D geometry arrives this way (see CLAUDE.md
   *  on `±1e30`), and such a room is skipped rather than treated as an error. */
  loops?: Loop[];
  /**
   * The server-resolved, ordered field list from `room_label`. Three states,
   * and they are NOT interchangeable:
   *   - absent      -> an older payload; fall back to name, then id.
   *   - `[]`        -> the configured properties did not resolve. Render NOTHING.
   *                    Falling back here would invent data the server withheld.
   *   - `[a, b, …]` -> `a` is the primary line, the rest stack smaller in accent.
   */
  label?: string[];
  properties?: Record<string, PropertyValue>;
  classification?: ClassificationTier[];
}

/**
 * One door, as `/doors` returns it — a SUBSET, on the same terms as `Room`.
 *
 * A door is not a room with different fields. The two that matter here:
 * `loops` may be legitimately EMPTY (a door family with no 3D geometry — 2 of
 * the 26 House A doors), and a door carries its own placement so it is still
 * drawable when that happens.
 */
export interface Door {
  id: string;
  level_id?: string;
  /** `loops[0]` is the footprint rectangle; the room convention verbatim.
   *  **Empty is a real state**, not an error — see `buildDoorGlyph`. */
  loops?: Loop[];
  /** The room the door opens from, or absent for an external door. */
  from_room?: string | null;
  to_room?: string | null;
  /** Where the door sits. Present even when `loops` is empty, which is the
   *  whole reason a geometry-less door can still be drawn. */
  insertion_point?: Point2D | null;
  /**
   * A unit vector along the direction the door **faces**, Y-up like every
   * other point here.
   *
   * **Not "toward `to_room`".** They usually coincide, but a door is
   * attributed to the room it *serves* and the modeller decides that — a
   * cupboard off a corridor swings into the corridor and belongs to the
   * cupboard. So an arrow drawn from this can legitimately point away from
   * `owner_rooms`, and that is correct data rather than something to
   * reconcile.
   */
  through_wall_normal?: Point2D | null;
  type_name?: string;
  /** Rooms this door is attributed to under the project's policy. **Empty
   *  means homeless** — a reported state, not a missing value. */
  owner_rooms?: string[];
  properties?: Record<string, PropertyValue>;
  type_properties?: Record<string, PropertyValue>;
}

/**
 * One window, as `/windows` returns it.
 *
 * **The same record as `Door`, and an alias rather than a copy**, because that
 * is what the measurement found: a full-depth key-path diff of real exports from
 * two documents showed zero structural differences between a window record and a
 * door record. The server shares one `Opening` type for exactly this reason, and
 * a second interface here would only be somewhere for the two to drift apart.
 *
 * **Named `WindowOpening`, not `Window`**, which is not a stylistic choice:
 * `Window` is the DOM global, and shadowing it in a file that also touches the
 * canvas is a class of bug nobody enjoys finding.
 *
 * What differs is the DATA, not the shape, and the glyph is where that shows up
 * — see `buildWindowGlyph`.
 */
export type WindowOpening = Door;

/** A rectangle in the **flipped** (Y-down) space — the same space as
 *  `zone.view` and the SVG viewBox. */
export interface Extent {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

/** A viewport/frame in the flipped space, in `viewBox` order. */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Size {
  w: number;
  h: number;
}

/**
 * One room's resolved appearance — the whole output of the appearance decision,
 * and the thing that must not drift between the SVG export and the GL renderer.
 *
 * Note what is NOT here: selection. `paintLevel` is shared with the SVG export
 * and an exported file must never carry a selection stroke, so selection is
 * re-applied after painting rather than resolved with everything else.
 */
export interface RoomAppearance {
  /** A literal colour from the active colour plan, or `null` to mean "leave the
   *  CSS `--fill` alone". `null` is not `""`: an empty string would emit an
   *  inline `fill:` that overrides the stylesheet with nothing. */
  fill: string | null;
  error: boolean;
  match: boolean;
  dim: boolean;
}

/** Everything the appearance decision needs that is not the room itself. */
export interface AppearanceContext {
  /**
   * Resolves the active colour plan to a literal colour, or `null` when no plan
   * is active. INJECTED rather than implemented here: the palette
   * (`qualitative`/`SCHEMES`) lives in `static/common.js`, which four pages load
   * as a classic script and which therefore cannot be imported. The precedence
   * *rule* is what matters and it lives in `resolveRoomAppearance`; which hex a
   * value maps to is a separate concern.
   */
  colourFor?: ((room: Room) => string | null) | undefined;
  errorRoomIds?: ReadonlySet<string> | null | undefined;
  showErrors?: boolean | undefined;
  matchRoomIds?: ReadonlySet<string> | null | undefined;
  searchActive?: boolean | undefined;
}
