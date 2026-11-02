// How each layer LOOKS, for both painters.
//
// **Two emitters, one set of numbers.** The screen is WebGL (./gl/renderer.ts)
// and the export is SVG (./svg/), and since the export learned to draw the
// overlay layers each of them is drawn twice. The geometry was already shared --
// the glyph builders are GL-free -- so what could still drift is everything
// here: which dash tells a ceiling from a floor, how see-through a footprint is,
// and which theme colour a layer falls back to. A ceiling exported with the
// floor's dot would be a wrong drawing that looks entirely deliberate.
//
// Plain data and one pure function, so both bundles can import it without
// either pulling in the other's runtime.

/** Outline stroke width in CSS pixels, for rooms and every ring overlay. */
export const W_OUTLINE = 1.5;

/** `stroke-dasharray: 4 3` on `.hole`. */
export const HOLE_DASH: readonly [number, number] = [4, 3];
/** The ceiling outline's dash. Longer than `HOLE_DASH` on purpose: a hole and a
 *  ceiling can sit on the same plan, and two dashes a pixel apart in period read
 *  as one pattern drawn badly rather than as two different things. */
export const CEILING_DASH: readonly [number, number] = [6, 4];
/** The floor outline's dot. Told apart from the ceiling's dash and the hole's
 *  by DUTY CYCLE rather than period -- 20% ink against 60% and 57% -- so a floor
 *  ring reads as dotted wherever it lies on either of them. */
export const FLOOR_DASH: readonly [number, number] = [2, 8];

/** An opening's or item's footprint fill, as an alpha over its fill colour.
 *  Light enough that a room's label still reads through a door drawn over it,
 *  dark enough to register as a solid object rather than a smudge. The alpha
 *  rides an override rather than being replaced by it: the transparency is
 *  structural, not a shade anyone chose. */
export const FOOTPRINT_ALPHA = 0.25;

/** Which theme colour each overlay falls back to when the project sets none.
 *  Spaces default to the ACCENT where everything else defaults to ink: a space
 *  is a services boundary held up against the architecture, while a ceiling or
 *  floor IS the architecture and is told apart by its dash instead. */
export const LAYER_THEME_COLOUR = {
  doors: "ink",
  windows: "ink",
  ffe: "ink",
  spaces: "accent",
  ceilings: "ink",
  floors: "ink",
} as const;

/**
 * The project's colour override when it is one a painter can use, else `null`.
 *
 * The pattern test is not belt-and-braces. These strings come from a project's
 * TOML, which a person may edit by hand, and a typo must fall back to the theme
 * rather than black out a whole layer -- the "signal, not error" reading of a
 * bad value. One test for both painters, so a colour the screen ignores is
 * never one the export draws.
 */
export function usableColour(css: string | null | undefined): string | null {
  if (typeof css !== "string") return null;
  const key = css.trim();
  if (!key) return null;
  return /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(key) || /^rgba?\([^)]+\)$/i.test(key) ? key : null;
}
