// One zone's hierarchy-footprint overlay: the dissolved polygons for its level
// at its tier, drawn over the ghosted rooms.
//
// **SVG, and deliberately outside the renderer seam.** There are DOZENS of
// footprints against thousands of rooms, so this is the half of the hybrid that
// stays in the DOM — `renderAreasOverlay` was untouched by the whole WebGL
// move, and this is that component. What does cross the seam is the ghosting:
// the rooms are the renderer's, so `setAreasActive` is a cross-layer call,
// easy to miss and visibly wrong when missed.

import { useEffect } from "react";

import { areaKey, footprintPathD, groupsForOverlay, tierLabel, type AreasData } from "../areas.js";
import { qualitative } from "../palette.js";
import { handleOf } from "./zoneRegistry.js";

/** The mono glyph aspect ratio, and the share of a footprint's bounding box a
 *  label may use. 0.7 rather than a room label's 0.9: a bbox OVERSTATES the
 *  usable interior of an L-shaped dissolved footprint. */
const LABEL_WIDTH_FACTOR = 0.7;
const GLYPH_ASPECT = 0.6;

export function AreasOverlay({
  zoneId,
  data,
  levelId,
  depth,
  active,
  selectedKey,
}: {
  zoneId: string;
  data: AreasData | null;
  levelId: string | null;
  depth: number;
  active: boolean;
  selectedKey: string | null;
}) {
  // Ghosting the rooms beneath the footprints is the renderer's job, since the
  // rooms are GL and this overlay is SVG.
  useEffect(() => {
    handleOf(zoneId)?.renderer.setAreasActive(active);
  }, [zoneId, active]);

  if (!active || !data) return null;
  const groups = groupsForOverlay(data, levelId, depth);
  if (!groups.length) return null;

  const fitted = handleOf(zoneId)?.fitted;
  const baseFont = fitted ? Math.max(fitted.w, fitted.h) * 0.022 : 1;

  return (
    <g className="areas-overlay">
      {groups.map((grp, i) => {
        const colour = qualitative("Set2", i);
        const key = areaKey(grp);
        const label = labelFor(grp, depth, baseFont);
        return (
          <g key={key}>
            {grp.polygons.map((poly, j) =>
              poly.exterior?.length >= 3 ? (
                <path
                  key={j}
                  d={footprintPathD(poly)}
                  fillRule="evenodd"
                  className={`area-poly${grp.counted_upward === false ? " faded" : ""}${
                    selectedKey === key ? " selected" : ""
                  }`}
                  data-area-key={key}
                  // The FILL stays inline — a footprint keeps its group colour
                  // selected or not, so the band's swatches still name it. The
                  // STROKE goes through a custom property instead, because an
                  // inline stroke outranks every stylesheet rule and
                  // `.area-poly.selected` could then never repaint the outline.
                  style={{ fill: colour, ["--area-colour" as string]: colour }}
                />
              ) : null,
            )}
            {label ? (
              <text className="area-label" x={label.x} y={-label.y} fontSize={label.size}>
                {label.text}
              </text>
            ) : null}
          </g>
        );
      })}
    </g>
  );
}

/** A group's label, on its largest island, or null when it would be too small
 *  to read — a sub-pixel speck is worse than nothing, and the band names every
 *  group anyway. (Unlike a ROOM label, where the same threshold was a bug: a
 *  suppressed room label had no other surface.) */
function labelFor(
  grp: ReturnType<typeof groupsForOverlay>[number],
  depth: number,
  baseFont: number,
): { x: number; y: number; size: number; text: string } | null {
  const exteriors = grp.polygons.map((p) => p.exterior).filter((r) => r && r.length >= 3);
  if (!exteriors.length) return null;
  const big = exteriors.reduce((a, b) => (ringArea(b) > ringArea(a) ? b : a), exteriors[0]!);
  const text = tierLabel(grp.path[depth]);
  if (!text) return null;

  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  for (const [x, y] of big) {
    x0 = Math.min(x0, x);
    x1 = Math.max(x1, x);
    y0 = Math.min(y0, y);
    y1 = Math.max(y1, y);
  }
  const widthLimited = ((x1 - x0) * LABEL_WIDTH_FACTOR) / Math.max(text.length, 1) / GLYPH_ASPECT;
  const size = Math.min(baseFont, widthLimited, (y1 - y0) * 0.8);
  if (size < baseFont * 0.25) return null;
  const c = centroid(big);
  return { x: c.x, y: c.y, size, text };
}

function ringArea(ring: readonly [number, number][]): number {
  let a = 0;
  for (let i = 0; i < ring.length; i++) {
    const [x1, y1] = ring[i]!;
    const [x2, y2] = ring[(i + 1) % ring.length]!;
    a += x1 * y2 - x2 * y1;
  }
  return Math.abs(a) / 2;
}

/** Area-weighted centroid, with a vertex average as the fallback — a
 *  degenerate ring would otherwise place a label at NaN. */
function centroid(ring: readonly [number, number][]): { x: number; y: number } {
  let a = 0;
  let cx = 0;
  let cy = 0;
  for (let i = 0; i < ring.length; i++) {
    const [x1, y1] = ring[i]!;
    const [x2, y2] = ring[(i + 1) % ring.length]!;
    const cross = x1 * y2 - x2 * y1;
    a += cross;
    cx += (x1 + x2) * cross;
    cy += (y1 + y2) * cross;
  }
  if (Math.abs(a) < 1e-9) {
    const n = ring.length || 1;
    return { x: ring.reduce((s, p) => s + p[0], 0) / n, y: ring.reduce((s, p) => s + p[1], 0) / n };
  }
  return { x: cx / (3 * a), y: cy / (3 * a) };
}
