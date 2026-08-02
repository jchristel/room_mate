// Room labels, as bitmap text.
//
// `BitmapText`, not `Text`: `Text` rasterises a texture per string, which at
// 5,046 rooms is 5,046 textures. BitmapText shares one glyph atlas, which is
// what the POC measured as CHEAPER than Canvas2D's `fillText` — and this repo's
// own earlier SVG finding said the same thing from the other direction
// ("Labels are NOT the bottleneck"). The budget does not need defending here.
//
// SIZING IS RE-DERIVED, NOT PORTED. `addLabel` computes a font size in world
// units and hands it to SVG, which scales text with the viewBox. A bitmap font
// is rasterised at one size and scaled, so the same arithmetic would give the
// right proportions at the wrong resolution. What is preserved is the RULE —
// the label fits the room's own bbox, with the level's base font as a ceiling —
// expressed as a scale factor against a fixed atlas size.
//
// The three-state `label` rule comes along unchanged, because it is a data rule
// rather than a rendering one: absent falls back to name then id;
// present-but-empty renders nothing at all.

import { BitmapText, Container } from "pixi.js";
import { centroid, loopBox } from "../geometry.js";
import type { Room } from "../types.js";
import type { Rgba } from "./colour.js";

/** The atlas is generated once at this size and every label scales from it.
 *  Large enough that zooming in stays crisp, small enough that the atlas is not
 *  enormous. */
const ATLAS_PX = 48;
/** Mono glyph aspect ratio — the same 0.6 `addLabel` uses. */
const GLYPH_ASPECT = 0.6;

function rgbaToHex(c: Rgba): number {
  return (Math.round(c[0] * 255) << 16) | (Math.round(c[1] * 255) << 8) | Math.round(c[2] * 255);
}

export interface LabelStyle {
  ink: Rgba;
  accent: Rgba;
  fontFamily: string;
}

/**
 * One room's label: a container holding the primary line plus any accent lines,
 * positioned at the room's centroid in flipped world space.
 *
 * Carried no bounds. It used to, for a viewport cull that has since been
 * deleted (see `GlPlanRenderer.cull`) — and those bounds were themselves wrong,
 * being the room's bbox re-centred on its CENTROID, which is not the same point
 * for any room that is not symmetric. Nothing reads them now, so they are gone
 * rather than left as a trap for whoever adds level-of-detail later.
 */
export interface RoomLabel {
  room: Room;
  node: Container;
}

/**
 * Build the label objects for a level. Returns an empty array when labels are
 * off — omitting the objects entirely rather than hiding them, which is the same
 * choice `paintLevel` makes for the export.
 */
export function buildLabels(
  rooms: readonly Room[],
  fitted: { w: number; h: number },
  style: LabelStyle,
): { container: Container; labels: RoomLabel[] } {
  const container = new Container();
  const labels: RoomLabel[] = [];
  const baseFont = Math.max(fitted.w, fitted.h) * 0.02;

  for (const room of rooms) {
    const outer = room.loops?.[0];
    if (!outer) continue;

    const fields =
      room.label !== undefined ? room.label : [room.name || room.id].filter(Boolean);
    if (fields.length === 0) continue;

    // Same constraint as addLabel: the level's base font is a ceiling, the
    // room's own bbox is the limit.
    const box = loopBox(outer);
    const longest = fields.reduce((n, f) => Math.max(n, String(f).length), 1);
    const widthLimited = (box.w * 0.9) / longest / GLYPH_ASPECT;
    const heightLimited = (box.h * 0.8) / (1 + 0.7 * (fields.length - 1));
    const worldFont = Math.min(baseFont, widthLimited, heightLimited);
    if (!(worldFont > 0)) continue;

    const c = centroid(outer);
    const node = new Container();
    node.position.set(c.x, c.y);
    // World units per atlas pixel. The atlas is fixed; the scene scales.
    const scale = worldFont / ATLAS_PX;
    node.scale.set(scale, scale);

    // Vertically centred as a block, matching SVG's dominant-baseline:middle
    // over a stack of tspans.
    const lineHeights = fields.map((_, i) => (i === 0 ? ATLAS_PX : ATLAS_PX * 0.7) * 1.2);
    const total = lineHeights.reduce((s, h) => s + h, 0);
    let y = -total / 2;

    fields.forEach((text, i) => {
      const primary = i === 0;
      const t = new BitmapText({
        text: String(text),
        style: {
          fontFamily: style.fontFamily,
          fontSize: primary ? ATLAS_PX : ATLAS_PX * 0.7,
          fill: rgbaToHex(primary ? style.ink : style.accent),
        },
      });
      t.anchor.set(0.5, 0);
      t.position.set(0, y);
      y += lineHeights[i]!;
      node.addChild(t);
    });

    container.addChild(node);
    labels.push({ room, node });
  }

  return { container, labels };
}
