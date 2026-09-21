// The shared palette, read from `static/common.js`.
//
// **One reader for two views.** The plan's colour plans and the adjacency
// graph (`adjacency/canvas.ts`) both take their colours from here, and two
// views that disagree about what colour a department is are worse than either
// being arbitrary. The stops are still `common.js` globals only because they
// predate the build; nothing outside the viewer reads them any more, so moving
// them in here is a deletion waiting to happen, not a sharing constraint.
//
// Accessors rather than re-exports of the globals, so the colour maths can be
// tested: a test defines these four names on `globalThis` and gets a known
// palette, which is also how it pins the maths rather than the stops.

interface PaletteGlobals {
  SCHEMES?: Record<string, string[]>;
  qualitative?: (scheme: string, k: number) => string;
  hexToRgb?: (hex: string) => [number, number, number];
  rgbToHex?: (r: number, g: number, b: number) => string;
}

const globals = globalThis as unknown as PaletteGlobals;

/** A scheme's stops, falling back to RdBu for a name nobody declared — the old
 *  page's rule: an unknown scheme is a safe default, never an exception. */
export function schemeStops(name: string): string[] {
  const schemes = globals.SCHEMES ?? {};
  return schemes[name] ?? schemes["RdBu"] ?? ["#000000"];
}

/** The k-th distinct hue of a categorical scheme, wrapping. */
export function qualitative(scheme: string, k: number): string {
  if (globals.qualitative) return globals.qualitative(scheme, k);
  const stops = schemeStops(scheme);
  return stops[k % stops.length]!;
}

export function hexToRgb(hex: string): [number, number, number] {
  if (globals.hexToRgb) return globals.hexToRgb(hex);
  const n = Number.parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

export function rgbToHex(r: number, g: number, b: number): string {
  if (globals.rgbToHex) return globals.rgbToHex(r, g, b);
  const c = (v: number) =>
    Math.max(0, Math.min(255, Math.round(v)))
      .toString(16)
      .padStart(2, "0");
  return `#${c(r)}${c(g)}${c(b)}`;
}
