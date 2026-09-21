// The exported file's stylesheet.
//
// **Here, beside the painter, rather than in the page that triggers the
// export**, because it is the one place the look of every layer is spelled out
// for SVG, and every number in it must be the screen's: the dashes, the
// footprint alpha and each layer's theme fallback come from `../style.ts`, which
// the GL renderer reads too. A copy in the viewer would be a third definition.
//
// The palette arrives RESOLVED to literal colours: a custom property means
// nothing in a file opened outside this page.

import type { PlanAppearance } from "../seam.js";
import { CEILING_DASH, FLOOR_DASH, FOOTPRINT_ALPHA, HOLE_DASH, LAYER_THEME_COLOUR, usableColour, W_OUTLINE } from "../style.js";

/** The theme, as literal colours read from the page. */
export interface ExportPalette {
  ink: string;
  fill: string;
  paper: string;
  accent: string;
  error: string;
  rule: string;
}

const ring = "fill: none; stroke-linejoin: round; vector-effect: non-scaling-stroke;";

/** The `<style>` text for an exported level. The project's `[appearance]`
 *  overrides apply as they do on screen -- an override the screen would ignore
 *  as unusable is ignored here too, by the same test. */
export function exportStyle(pal: ExportPalette, appearance: PlanAppearance = {}): string {
  const or = (css: string | null | undefined, fallback: string) => usableColour(css) ?? fallback;
  const theme = (layer: keyof typeof LAYER_THEME_COLOUR) => pal[LAYER_THEME_COLOUR[layer]];
  const roomLine = or(appearance.rooms?.line, pal.ink);
  const roomFill = or(appearance.rooms?.fill, pal.fill);
  const glyph = (layer: "doors" | "windows" | "ffe", cls: string) => {
    const line = or(appearance[layer]?.line, theme(layer));
    const fill = or(appearance[layer]?.fill, theme(layer));
    return `
    .${cls}-rect { fill: ${fill}; fill-opacity: ${FOOTPRINT_ALPHA}; }
    .${cls}-mark { fill: ${line}; }`;
  };
  const outline = (layer: "spaces" | "ceilings" | "floors", cls: string, dash?: readonly [number, number]) =>
    `
    .${cls} { ${ring} stroke: ${or(appearance[layer]?.line, theme(layer))}; stroke-width: ${W_OUTLINE};${dash ? ` stroke-dasharray: ${dash[0]} ${dash[1]};` : ""} }`;
  return `
    .grid line { stroke: ${pal.rule}; stroke-width: 0.5; vector-effect: non-scaling-stroke; }
    .room { fill: ${roomFill}; stroke: ${roomLine}; stroke-width: ${W_OUTLINE}; stroke-linejoin: round; vector-effect: non-scaling-stroke; }
    .room.error { fill: ${pal.error}; }
    .hole { fill: ${pal.paper}; stroke: ${roomLine}; stroke-width: 1; stroke-dasharray: ${HOLE_DASH[0]} ${HOLE_DASH[1]}; vector-effect: non-scaling-stroke; }
    .label { fill: ${pal.ink}; text-anchor: middle; dominant-baseline: middle; }
    .label .tag { fill: ${pal.accent}; letter-spacing: 0.05em; }${outline("floors", "floor", FLOOR_DASH)}${outline(
      "ceilings",
      "ceiling",
      CEILING_DASH,
    )}${outline("spaces", "space")}${glyph("doors", "door")}${glyph("windows", "window")}${glyph("ffe", "ffe")}
  `;
}
