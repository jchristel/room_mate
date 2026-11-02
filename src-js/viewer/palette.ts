// The viewer's palette: the ColorBrewer stops and the functions over them.
//
// **One module for two views.** The plan's colour plans (`colour.ts`, the areas
// overlay and band) and the adjacency graph (`adjacency/canvas.ts`) both take
// their colours from here, and two views that disagree about what colour a
// department is are worse than either being arbitrary. Importing is what makes
// that true by construction rather than by care.
//
// These lived in `static/common.js` as classic-script globals, from when the
// viewer was a hand-written page and the graph a second script beside it, and
// reading them from a module was a bug for as long as it lasted. `SCHEMES` was
// a top-level `const`, which another script can name but which is NOT a
// property of `globalThis` — unlike the `function` declarations beside it. So
// `globalThis.SCHEMES` was undefined, every diverging and date-range colour
// plan sampled a one-stop black fallback, and hierarchy plans (which go through
// `qualitative`, a function) kept working and hid it. The test could not see
// it either: it assigned `globalThis.SCHEMES` itself. `colour.test.ts` now pins
// the maths against these real stops, and nothing is looked up by name.
//
// Literal hex stops, no d3/npm: six small tables do not justify a dependency.

/** A few ColorBrewer schemes. Sequential and diverging ones are sampled at
 *  t∈[0,1] by `colour.ts`'s `sampleScheme`; the categorical ones (Set2, Paired)
 *  are indexed by `qualitative`. */
export const SCHEMES: Readonly<Record<string, readonly string[]>> = {
  RdBu: ["#ca0020", "#f4a582", "#f7f7f7", "#92c5de", "#0571b0"],
  RdYlGn: ["#d7191c", "#fdae61", "#ffffbf", "#a6d96a", "#1a9641"],
  Greens: ["#edf8e9", "#bae4b3", "#74c476", "#31a354", "#006d2c"],
  Blues: ["#eff3ff", "#bdd7e7", "#6baed6", "#3182bd", "#08519c"],
  Set2: ["#66c2a5", "#fc8d62", "#8da0cb", "#e78ac3", "#a6d854", "#ffd92f", "#e5c494", "#b3b3b3"],
  Paired: [
    "#a6cee3",
    "#1f78b4",
    "#b2df8a",
    "#33a02c",
    "#fb9a99",
    "#e31a1c",
    "#fdbf6f",
    "#ff7f00",
    "#cab2d6",
    "#6a3d9a",
    "#ffff99",
    "#b15928",
  ],
};

/** A scheme's stops, falling back to RdBu for a name nobody declared: an
 *  unknown scheme (a settings file naming one this build lacks) is a safe
 *  default, never an exception. */
export function schemeStops(name: string): readonly string[] {
  return SCHEMES[name] ?? SCHEMES["RdBu"]!;
}

/** The k-th distinct hue of a categorical scheme, wrapping. Callers pass a
 *  stable index (a value's position in a sorted key list), so the same
 *  department gets the same colour on the plan and in the graph. An unknown
 *  scheme falls back to Set2, not RdBu — a categorical question should get
 *  categorical hues, not a diverging ramp read out of order. */
export function qualitative(scheme: string, k: number): string {
  const stops = SCHEMES[scheme] ?? SCHEMES["Set2"]!;
  return stops[k % stops.length]!;
}

export function hexToRgb(hex: string): [number, number, number] {
  const n = Number.parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

export function rgbToHex(r: number, g: number, b: number): string {
  const c = (v: number) =>
    Math.max(0, Math.min(255, Math.round(v)))
      .toString(16)
      .padStart(2, "0");
  return `#${c(r)}${c(g)}${c(b)}`;
}
