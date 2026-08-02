// CSS colour -> GL floats.
//
// The plan's palette lives in the page's stylesheet as custom properties
// (`--ink`, `--fill`, `--paper`, `--accent`, `--error`, `--rule`) and follows
// the light/dark theme. SVG got that for free by naming the variables; GL needs
// literal numbers, so they are read once per paint and converted here.
//
// Read at PAINT time, never cached across paints: the theme can change under a
// running page, and a cached palette would leave the plan in the old theme while
// every other surface moved.

export type Rgba = readonly [number, number, number, number];

const CACHE = new Map<string, Rgba>();

/**
 * Parse any colour the browser will hand back. Deliberately delegates to the
 * browser rather than pattern-matching hex: `getComputedStyle` returns
 * `rgb(...)`/`rgba(...)`, but a custom property read with `getPropertyValue`
 * comes back as AUTHORED — which in tokens.css is hex, and elsewhere could be
 * any CSS colour syntax. One canvas 2D context normalises all of it.
 */
export function parseColour(css: string): Rgba {
  const key = css.trim();
  const hit = CACHE.get(key);
  if (hit) return hit;

  let out: Rgba = [0, 0, 0, 1];
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(key);
  if (m) {
    const h = m[1]!;
    const full = h.length === 3 ? h[0]! + h[0]! + h[1]! + h[1]! + h[2]! + h[2]! : h;
    const n = parseInt(full, 16);
    out = [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255, 1];
  } else {
    const rgb = /^rgba?\(([^)]+)\)$/i.exec(key);
    if (rgb) {
      const parts = rgb[1]!.split(/[,\s/]+/).filter(Boolean).map(Number);
      out = [
        (parts[0] ?? 0) / 255,
        (parts[1] ?? 0) / 255,
        (parts[2] ?? 0) / 255,
        parts[3] === undefined ? 1 : parts[3],
      ];
    }
  }
  CACHE.set(key, out);
  return out;
}

export interface PlanPalette {
  ink: Rgba;
  fill: Rgba;
  fillHover: Rgba;
  paper: Rgba;
  accent: Rgba;
  error: Rgba;
  rule: Rgba;
}

/** Resolve the plan palette from the document's custom properties. */
export function readPalette(root: HTMLElement): PlanPalette {
  const cs = getComputedStyle(root);
  const v = (name: string, fallback: string) => parseColour(cs.getPropertyValue(name).trim() || fallback);
  return {
    ink: v("--ink", "#222222"),
    fill: v("--fill", "#dddddd"),
    fillHover: v("--fill-hover", "#cccccc"),
    paper: v("--paper", "#ffffff"),
    accent: v("--accent", "#c8102e"),
    error: v("--error", "#ffb3b3"),
    rule: v("--rule", "#cccccc"),
  };
}

/** Multiply a colour's alpha — how `.dim` (opacity 0.15) is expressed when
 *  opacity is not a thing a vertex has. */
export function withAlpha(c: Rgba, a: number): Rgba {
  return [c[0], c[1], c[2], c[3] * a];
}
