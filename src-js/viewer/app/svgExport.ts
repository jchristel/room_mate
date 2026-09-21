// Exporting levels as standalone SVG files.
//
// **Reuses `paintLevel`**, the renderer bundle's own SVG painter, and draws what
// the ZONE draws: its rooms and labels toggles, its element layers, its spaces
// model, its colour plan and the project's `[appearance]`. An export that
// ignored the visibility menu would be a different drawing from the one the
// reader chose, and nothing in the file would say so. What it leaves out is
// deliberate, and it is everything that is a VIEW rather than the drawing:
// selection, search highlight, hover and the areas overlay. The export carries
// no selection plumbing for the same reason -- a data attribute stamped for a
// hit test would leak into every exported file.
//
// **The element layers are fetched per level, here, not taken from the polls.**
// Each poll holds only the storeys some zone is SHOWING (`StoreyScope`), so an
// export of any other level would come out with no doors and no way to tell.
// Levels go one at a time so a large project holds one storey's worth at once --
// RHH's `/ffe` is 23.6 MB for one storey and 133 MB for all of them -- and the
// same `storeyElements` join the screen uses decides what each level keeps.
//
// The file is framed to the LEVEL's own fitted bounds, never the zone's current
// pan and zoom: an export is the drawing, not the screenshot.

import { buildColourContext, colourForRoom, type ColourPlan } from "../colour.js";
import { elementUrl, matchSuffix, type ElementEntity } from "../elementUrls.js";
import { levelsForPayload, roomsOnLevel } from "../levels.js";
import type { Scope } from "../scope.js";
import { LAYERS, storeyElements, type ElementPayload, type StoreyMatch } from "./layers.js";
import { fittedBounds } from "./planRenderer.js";
import type { RoomsPayload } from "./api.js";
import type { PlanAppearance } from "../../renderer/seam.js";
import type { OverlayLayers } from "../../renderer/svg/overlays.js";
import type { PaintOptions } from "../../renderer/svg/paint.js";
import type { ExportPalette } from "../../renderer/svg/style.js";
import type { Level, Rect, Room } from "../../renderer/types.js";

const SVG_NS = "http://www.w3.org/2000/svg";
const EXPORT_WIDTH_PX = 1600;

/** What the export uses from the renderer bundle. Typed here rather than in
 *  `planRenderer.ts` because the export is its only consumer. */
interface ExportBundle {
  paintLevel(svg: SVGElement, rooms: readonly Room[], fitted: Rect, opts: PaintOptions): void;
  exportStyle(palette: ExportPalette, appearance?: PlanAppearance): string;
}

const bundle = (): ExportBundle => (globalThis as unknown as { PlanRenderer: ExportBundle }).PlanRenderer;

/** The theme, resolved to literal colours: a custom property means nothing in a
 *  file opened outside this page. */
function palette(): ExportPalette {
  const cs = getComputedStyle(document.documentElement);
  const v = (name: string) => cs.getPropertyValue(name).trim();
  return { ink: v("--ink"), fill: v("--fill"), paper: v("--paper"), accent: v("--accent"), error: v("--error"), rule: v("--rule") };
}

export interface ExportOptions {
  plan: ColourPlan | null;
  errorRoomIds: ReadonlySet<string>;
  showErrors: boolean;
  showLabels: boolean;
  /** The zone's rooms toggle. Off draws the overlays alone, framed as the full
   *  level would be. */
  showRooms: boolean;
  /** The zone's element layers. */
  layers: Readonly<Record<ElementEntity, boolean>>;
  /** The zone's services model, `""` for every model -- applied as the screen
   *  applies it, after the fetch. */
  spacesModel: string;
  appearance: PlanAppearance;
}

/** One level's element layers, and how each one's storey match resolved. */
export interface LevelOverlays {
  layers: OverlayLayers;
  /** Per layer that is ON: its match, or `"failed"` when the read did. */
  matches: Partial<Record<ElementEntity, StoreyMatch | "failed">>;
}

/**
 * The file's own statement of what it contains.
 *
 * On screen a layer that matched its storey by elevation, or showed every
 * level, says so on its toggle; a file that dropped that would put a guess on
 * paper as a measurement. A failed read is named for the same reason -- a
 * doors layer that is empty because the read failed and one that is empty
 * because the storey has none must not look alike.
 */
function describe(level: string, opts: ExportOptions, overlays: LevelOverlays): string {
  const drawn: string[] = [];
  if (opts.showRooms) drawn.push("Rooms");
  if (opts.showRooms && opts.showLabels) drawn.push("Labels");
  for (const layer of LAYERS) {
    const match = overlays.matches[layer.entity];
    if (!match) continue;
    let name = match === "failed" ? `${layer.label} (read failed)` : layer.label + matchSuffix(match);
    if (layer.entity === "spaces" && opts.spacesModel) name += ` [${opts.spacesModel}]`;
    drawn.push(name);
  }
  return `Level ${level}. Layers: ${drawn.join(", ") || "none"}.`;
}

/** One level as a standalone SVG document, or null when it has no rooms to
 *  frame. The frame is always the ROOMS' fit, even with rooms hidden, so every
 *  file for a level lines up whatever was switched on. */
export function buildLevelSvg(
  payload: RoomsPayload,
  level: Level,
  opts: ExportOptions,
  overlays: LevelOverlays = { layers: {}, matches: {} },
): string | null {
  const rooms = roomsOnLevel(payload, level.id);
  const fitted = fittedBounds(rooms);
  if (!fitted) return null;
  const pal = palette();
  const { paintLevel, exportStyle } = bundle();

  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("xmlns", SVG_NS);
  svg.setAttribute("viewBox", `${fitted.x} ${fitted.y} ${fitted.w} ${fitted.h}`);
  svg.setAttribute("width", String(EXPORT_WIDTH_PX));
  svg.setAttribute("height", String(Math.max(1, Math.round((EXPORT_WIDTH_PX * fitted.h) / fitted.w))));

  const desc = document.createElementNS(SVG_NS, "desc");
  desc.textContent = describe(level.name, opts, overlays);
  svg.appendChild(desc);

  const style = document.createElementNS(SVG_NS, "style");
  style.textContent = exportStyle(pal, opts.appearance);
  svg.appendChild(style);

  // An opaque paper background covering the viewBox, so the file is not
  // transparent when pasted onto a white or dark surface. Before the drawing,
  // so it is painted first and everything else sits on it.
  const bg = document.createElementNS(SVG_NS, "rect");
  bg.setAttribute("x", String(fitted.x));
  bg.setAttribute("y", String(fitted.y));
  bg.setAttribute("width", String(fitted.w));
  bg.setAttribute("height", String(fitted.h));
  bg.setAttribute("fill", pal.paper);
  svg.appendChild(bg);

  const ctx = opts.plan ? buildColourContext(rooms, opts.plan) : null;
  paintLevel(svg, rooms, fitted, {
    ...(opts.plan && ctx ? { colourFor: (room: Room) => colourForRoom(room, opts.plan!, ctx) } : {}),
    errorRoomIds: opts.errorRoomIds,
    showErrors: opts.showErrors,
    showLabels: opts.showLabels,
    showRooms: opts.showRooms,
    ...overlays.layers,
  });

  return `<?xml version="1.0" encoding="UTF-8"?>\n${new XMLSerializer().serializeToString(svg)}`;
}

/**
 * Fetch one level's element layers -- the ones the zone has ON, and only those.
 *
 * Never throws: a layer whose read fails is recorded as failed and the level is
 * still exported, the rule every element poll follows. A model with rooms and
 * no furniture is legitimate and the plan is still worth drawing.
 */
export async function fetchLevelOverlays(
  scope: Scope,
  payload: RoomsPayload,
  level: Level,
  opts: ExportOptions,
): Promise<LevelOverlays> {
  // A payload that declares no levels asks unscoped, as the polls do: its
  // levels are synthesised from room ids and mean nothing to the server.
  const storeys = payload.levels?.length ? [level] : [];
  const on = LAYERS.filter((l) => opts.layers[l.entity]);
  const results = await Promise.all(
    on.map(async (layer) => {
      const url = elementUrl(layer.entity, scope, storeys);
      try {
        const res = await fetch(url!, { cache: "no-store" });
        if (res.status === 204) return { entity: layer.entity, body: null };
        if (!res.ok) return { entity: layer.entity, body: "failed" as const };
        return { entity: layer.entity, body: (await res.json()) as ElementPayload };
      } catch {
        return { entity: layer.entity, body: "failed" as const };
      }
    }),
  );

  const overlays: LevelOverlays = { layers: {}, matches: {} };
  for (const { entity, body } of results) {
    if (body === "failed") {
      overlays.matches[entity] = "failed";
      continue;
    }
    const { kept, match } = storeyElements(entity, body, level.id);
    // A 204 is "this scope holds none", which is an exact answer, not a guess.
    overlays.matches[entity] = body === null ? "exact" : match;
    // The services-model filter is applied after the read, exactly as the
    // screen applies it.
    const list =
      entity === "spaces" && opts.spacesModel
        ? (kept as readonly { model_id?: string }[]).filter((s) => s.model_id === opts.spacesModel)
        : kept;
    (overlays.layers as Record<string, unknown>)[entity] = list;
  }
  return overlays;
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

/** What an export run did, for the menu to report. A silent skip reads as a
 *  broken picker, so both kinds of shortfall are named. */
export interface ExportResult {
  exported: number;
  /** Levels chosen that had no rooms to frame. */
  empty: string[];
  /** Levels exported with at least one layer whose read failed. */
  partial: string[];
}

/**
 * The chosen levels, one file each, lowest first.
 *
 * One click, N downloads: browsers show a one-time "allow multiple downloads"
 * prompt and throttle rapid clicks, so the files are emitted with a small
 * stagger -- which the per-level fetch now mostly provides by itself.
 */
export async function exportLevels(
  payload: RoomsPayload,
  scope: Scope,
  levelIds: ReadonlySet<string>,
  opts: ExportOptions,
  onProgress: (done: number, total: number) => void = () => {},
): Promise<ExportResult> {
  const chosen = levelsForPayload(payload).filter((l) => levelIds.has(l.id));
  const result: ExportResult = { exported: 0, empty: [], partial: [] };
  for (const [i, level] of chosen.entries()) {
    onProgress(i, chosen.length);
    // Checked before fetching: a level with no rooms has no frame, and its
    // doors are not worth a read that could only be thrown away.
    if (!fittedBounds(roomsOnLevel(payload, level.id))) {
      result.empty.push(level.name);
      continue;
    }
    const overlays = await fetchLevelOverlays(scope, payload, level, opts);
    const text = buildLevelSvg(payload, level, opts, overlays);
    if (!text) {
      result.empty.push(level.name);
      continue;
    }
    if (Object.values(overlays.matches).includes("failed")) result.partial.push(level.name);
    download(text, svgFilename(scope.projectId, level.name, scope.milestone, opts.showErrors));
    result.exported++;
    await new Promise((r) => setTimeout(r, 150));
  }
  onProgress(chosen.length, chosen.length);
  return result;
}

function download(text: string, filename: string): void {
  const blob = new Blob([text], { type: "image/svg+xml;charset=utf-8" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = filename;
  a.click();
  URL.revokeObjectURL(a.href);
}
