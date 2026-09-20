// Exporting a level as a standalone SVG file.
//
// **Reuses `paintLevel`**, the renderer bundle's own SVG painter, so the
// exported geometry is identical to what the plan draws — the two cannot
// disagree, because there is one computation. That sharing is also why the
// export carries no selection plumbing: `paintLevel` is shared verbatim, and a
// data attribute stamped for a hit test would leak into every exported file.
//
// The file is framed to the LEVEL's own fitted bounds, never the zone's current
// pan and zoom: an export is the drawing, not the screenshot.

import { buildColourContext, colourForRoom, type ColourPlan } from "../colour.js";
import { levelsForPayload, roomsOnLevel } from "../levels.js";
import { fittedBounds } from "./planRenderer.js";
import type { RoomsPayload } from "./api.js";
import type { Room } from "../../renderer/types.js";

const SVG_NS = "http://www.w3.org/2000/svg";
const EXPORT_WIDTH_PX = 1600;

/** `paintLevel`, from the renderer bundle. Typed here rather than in
 *  `planRenderer.ts` because the export is its only consumer. */
type PaintLevel = (
  svg: SVGElement,
  rooms: readonly Room[],
  fitted: { x: number; y: number; w: number; h: number },
  opts: {
    colourFor?: (room: Room) => string;
    errorRoomIds?: ReadonlySet<string> | null;
    showErrors?: boolean;
    showLabels?: boolean;
  },
) => void;

/** The palette, resolved to literal colours: a custom property means nothing
 *  in a file opened outside this page. */
function styleBlock(): string {
  const cs = getComputedStyle(document.documentElement);
  const v = (name: string) => cs.getPropertyValue(name).trim();
  return `
    .grid line { stroke: ${v("--rule")}; stroke-width: 0.5; vector-effect: non-scaling-stroke; }
    .room { fill: ${v("--fill")}; stroke: ${v("--ink")}; stroke-width: 1.5; stroke-linejoin: round; vector-effect: non-scaling-stroke; }
    .room.error { fill: ${v("--error")}; }
    .hole { fill: ${v("--paper")}; stroke: ${v("--ink")}; stroke-width: 1; stroke-dasharray: 4 3; vector-effect: non-scaling-stroke; }
    .label { fill: ${v("--ink")}; text-anchor: middle; dominant-baseline: middle; }
    .label .tag { fill: ${v("--accent")}; letter-spacing: 0.05em; }
  `;
}

export interface ExportOptions {
  plan: ColourPlan | null;
  errorRoomIds: ReadonlySet<string>;
  showErrors: boolean;
  showLabels: boolean;
}

/** One level as a standalone SVG document, or null when it has nothing to
 *  draw. */
export function buildLevelSvg(payload: RoomsPayload, levelId: string, opts: ExportOptions): string | null {
  const rooms = roomsOnLevel(payload, levelId);
  const fitted = fittedBounds(rooms);
  if (!fitted) return null;

  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("xmlns", SVG_NS);
  svg.setAttribute("viewBox", `${fitted.x} ${fitted.y} ${fitted.w} ${fitted.h}`);
  svg.setAttribute("width", String(EXPORT_WIDTH_PX));
  svg.setAttribute("height", String(Math.max(1, Math.round((EXPORT_WIDTH_PX * fitted.h) / fitted.w))));

  const style = document.createElementNS(SVG_NS, "style");
  style.textContent = styleBlock();
  svg.appendChild(style);

  // An opaque paper background covering the viewBox, so the file is not
  // transparent when pasted onto a white or dark surface. First child, so it
  // is painted first and everything else sits on it.
  const bg = document.createElementNS(SVG_NS, "rect");
  bg.setAttribute("x", String(fitted.x));
  bg.setAttribute("y", String(fitted.y));
  bg.setAttribute("width", String(fitted.w));
  bg.setAttribute("height", String(fitted.h));
  bg.setAttribute("fill", getComputedStyle(document.documentElement).getPropertyValue("--paper").trim());
  svg.appendChild(bg);

  const paintLevel = (globalThis as unknown as { PlanRenderer: { paintLevel: PaintLevel } }).PlanRenderer.paintLevel;
  const ctx = opts.plan ? buildColourContext(rooms, opts.plan) : null;
  paintLevel(svg, rooms, fitted, {
    ...(opts.plan && ctx ? { colourFor: (room: Room) => colourForRoom(room, opts.plan!, ctx) } : {}),
    errorRoomIds: opts.errorRoomIds,
    showErrors: opts.showErrors,
    showLabels: opts.showLabels,
  });

  return `<?xml version="1.0" encoding="UTF-8"?>\n${new XMLSerializer().serializeToString(svg)}`;
}

/** A filesystem-safe, self-describing name. */
export function svgFilename(
  projectId: string | null,
  levelName: string,
  milestone: string | null,
  withErrors: boolean,
): string {
  const safe = (s: string | null, fallback: string) =>
    (s ? String(s) : "").replace(/[^a-z0-9._-]+/gi, "-").replace(/^-+|-+$/g, "") || fallback;
  const parts = ["roomplan", safe(projectId, "project"), safe(levelName, "level")];
  if (milestone) parts.push(safe(milestone, "milestone"));
  if (withErrors) parts.push("errors");
  return `${parts.join("_")}.svg`;
}

/**
 * Every level of the payload, one file each.
 *
 * One click, N downloads: browsers show a one-time "allow multiple downloads"
 * prompt and throttle rapid clicks, so the files are emitted with a small
 * stagger.
 */
export async function exportLevels(
  payload: RoomsPayload,
  projectId: string | null,
  milestone: string | null,
  opts: ExportOptions,
): Promise<void> {
  for (const level of levelsForPayload(payload)) {
    const text = buildLevelSvg(payload, level.id, opts);
    if (!text) continue; // a level with no drawable rooms
    download(text, svgFilename(projectId, level.name, milestone, opts.showErrors));
    await new Promise((r) => setTimeout(r, 150));
  }
}

function download(text: string, filename: string): void {
  const blob = new Blob([text], { type: "image/svg+xml;charset=utf-8" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = filename;
  a.click();
  URL.revokeObjectURL(a.href);
}
