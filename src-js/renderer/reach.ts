// The reach heat map's geometry: which cells are drawn, and what colour each is.
//
// Pure and DOM-free, so the colour rule is testable without a GL context and the
// renderer only uploads what this returns. The data is the server's grid
// (`service::reach::ReachLevel`): `-1` no room, `-2` a room beyond the limit or
// unreached, else the walk in whole mm.
//
// **Only covered cells are drawn.** A `-2` cell is a room the walk does not
// reach, and painting it would hide the plan under a second colour for the
// answer "nothing here". The plan already shows the room; leaving it bare is
// what makes the covered part read as a stain on the layout rather than a
// replacement of it. A `-1` cell is no room and is never drawn.

import type { ReachLayer } from "./seam.js";

export type Rgb = readonly [number, number, number];

/** Near to far: blue, green, yellow, red. Four stops are enough to read a
 *  distance band off a plan and survive both themes; the alpha, not the hue, is
 *  what keeps the layout below visible. */
const RAMP: readonly (readonly [number, Rgb])[] = [
  [0, [0.0, 0.24, 0.78]],
  [0.33, [0.0, 0.67, 0.35]],
  [0.66, [0.94, 0.82, 0.0]],
  [1, [0.82, 0.12, 0.12]],
];

/** The default opacity: enough to read the colour, little enough that walls,
 *  doors and furniture show through. */
export const REACH_ALPHA = 0.55;

/** The colour of a walk of `mm` against a limit, `t` clamped to the ramp. */
export function reachColour(mm: number, limitMm: number): Rgb {
  const t = limitMm > 0 ? Math.min(Math.max(mm / limitMm, 0), 1) : 0;
  for (let i = 1; i < RAMP.length; i++) {
    const [t1, c1] = RAMP[i]!;
    if (t <= t1) {
      const [t0, c0] = RAMP[i - 1]!;
      const f = t1 === t0 ? 0 : (t - t0) / (t1 - t0);
      return [c0[0] + (c1[0] - c0[0]) * f, c0[1] + (c1[1] - c0[1]) * f, c0[2] + (c1[2] - c0[2]) * f];
    }
  }
  return RAMP[RAMP.length - 1]![1];
}

/** One drawn cell: its two triangles, in the renderer's flipped (y-down) space,
 *  and its colour. */
export interface ReachCell {
  triangles: number[];
  colour: readonly [number, number, number, number];
}

/**
 * The cells of a layer that are within the limit.
 *
 * A cell `(col, row)` spans `origin + (col, row) * spacing` to the next corner,
 * in the plan's own frame (feet, y up); the y is negated here, once, because the
 * renderer draws y-down and `flip` is the one place that sign changes.
 */
export function reachCells(layer: ReachLayer): ReachCell[] {
  const { origin, spacing_ft: s, cols, rows, distance_mm: d } = layer;
  const alpha = layer.alpha ?? REACH_ALPHA;
  const out: ReachCell[] = [];
  for (let row = 0; row < rows; row++) {
    for (let col = 0; col < cols; col++) {
      const mm = d[row * cols + col];
      if (mm === undefined || mm < 0) continue;
      const x0 = origin.x + col * s;
      const x1 = x0 + s;
      const y0 = -(origin.y + row * s);
      const y1 = -(origin.y + (row + 1) * s);
      const [r, g, b] = reachColour(mm, layer.limit_mm);
      out.push({
        triangles: [x0, y0, x1, y0, x1, y1, x0, y0, x1, y1, x0, y1],
        colour: [r, g, b, alpha],
      });
    }
  }
  return out;
}
