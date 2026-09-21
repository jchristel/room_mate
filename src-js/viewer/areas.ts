// Hierarchy areas: the dissolved footprints the server computes, the net room
// areas the browser sums, and the difference between them.
//
// **The difference is the point, not a rounding error.** A dissolved footprint
// is always at least the summed net of its rooms; the gap is the enclosed wall
// bands plus filled room voids (columns), while an open courtyard is excluded
// from the footprint altogether. That is why both numbers are shown side by
// side rather than one being chosen — see STRATEGY-AREA-CALCULATION.md before
// quoting either to anyone.
//
// Pure, because the identity that joins a room to a group — level plus
// classification prefix — is the part that silently produces wrong totals when
// it drifts. The adjacency graph imports `pathKey` and `tierLabel` from here to
// name its area nodes, so a footprint and a graph node are the same string. It
// used to keep a copy in `common.js`, which joined with "/" where this joins
// with ">": equal at tier 0, where there is no separator, and matching nothing
// below it.

import type { ClassificationTier, Room } from "../renderer/types.js";

export interface AreaGroup {
  level_id: string;
  path: ClassificationTier[];
  area: number;
  /** Whether the group counts toward the tiers above it. A group that does not
   *  is shown faded and flagged rather than omitted. */
  counted_upward?: boolean;
  polygons: { exterior: [number, number][]; holes?: [number, number][][] }[];
}

export interface AreasData {
  levels?: { id: string; name: string }[];
  groups: AreaGroup[];
}

/** Identity of one classification prefix. The same tuple the server matches on
 *  and the graph aggregates by — a copy that drifted would silently match
 *  nothing, which looks like a group with no rooms rather than a bug. */
export function tierSig(t: ClassificationTier): string {
  return `${t.code == null ? "" : t.code}|${t.name == null ? "" : t.name}|${t.undefined ? "U" : ""}`;
}

/** The path prefix down to `depth`, as one key. */
export function pathKey(path: readonly ClassificationTier[], depth: number): string {
  return path
    .slice(0, depth + 1)
    .map(tierSig)
    .join(">");
}

/** A group's identity: level plus its own full path. What a footprint is
 *  stamped with, so a click on one resolves to the group it names. */
export function areaKey(group: AreaGroup): string {
  return `${group.level_id}|${pathKey(group.path, group.path.length - 1)}`;
}

/** How a tier reads on screen. An `undefined` tier is NAMED, because the areas
 *  service treats the undefined bucket as a real group. */
export function tierLabel(t: ClassificationTier | undefined): string {
  if (!t) return "";
  if (t.undefined) return "(undefined)";
  return t.name || t.code || "";
}

/** The tier names in depth order, learned from the groups themselves. */
export function tierNames(data: AreasData | null): string[] {
  const names: string[] = [];
  for (const g of data?.groups ?? []) g.path.forEach((t, i) => (names[i] ??= t.tier));
  return names;
}

/** Net area of one room: outer loop minus its holes, by the shoelace formula. */
export function roomNetArea(room: Room): number {
  const loops = room.loops ?? [];
  if (!loops.length) return 0;
  const ring = (points: { x: number; y: number }[]): number => {
    let a = 0;
    for (let i = 0; i < points.length; i++) {
      const p = points[i]!;
      const q = points[(i + 1) % points.length]!;
      a += p.x * q.y - q.x * p.y;
    }
    return Math.abs(a) / 2;
  };
  let area = ring(loops[0]!.points);
  for (let i = 1; i < loops.length; i++) area -= ring(loops[i]!.points);
  return Math.max(0, area);
}

/** Summed net room area per `level|depth|prefix`, over every room and every
 *  depth — so a group at any tier can look its own number up directly. */
export function netAreaIndex(rooms: readonly Room[]): Map<string, number> {
  const net = new Map<string, number>();
  for (const r of rooms) {
    const cls = r.classification;
    if (!cls) continue;
    for (let d = 0; d < cls.length; d++) {
      const key = `${r.level_id}|${d}|${pathKey(cls, d)}`;
      net.set(key, (net.get(key) ?? 0) + roomNetArea(r));
    }
  }
  return net;
}

export interface AreaRow {
  levelId: string;
  levelName: string;
  label: string;
  footprint: number;
  net: number;
  delta: number;
  countedUp: boolean;
}

/** The band's rows at one tier, grouped by level in server order — which is
 *  also the order the overlay colours by, so a swatch names the same group. */
export function bandRows(data: AreasData | null, rooms: readonly Room[], depth: number): Map<string, AreaRow[]> {
  const byLevel = new Map<string, AreaRow[]>();
  if (!data) return byLevel;
  const levelName = new Map((data.levels ?? []).map((l) => [l.id, l.name]));
  const net = netAreaIndex(rooms);
  for (const g of data.groups.filter((g) => g.path.length === depth + 1)) {
    const netA = net.get(`${g.level_id}|${depth}|${pathKey(g.path, depth)}`) ?? 0;
    const row: AreaRow = {
      levelId: g.level_id,
      levelName: levelName.get(g.level_id) ?? g.level_id,
      label: tierLabel(g.path[depth]),
      footprint: g.area,
      net: netA,
      delta: g.area - netA,
      countedUp: g.counted_upward !== false,
    };
    if (!byLevel.has(g.level_id)) byLevel.set(g.level_id, []);
    byLevel.get(g.level_id)!.push(row);
  }
  return byLevel;
}

/** The groups one zone's overlay draws: its level, at its tier. */
export function groupsForOverlay(data: AreasData | null, levelId: string | null, depth: number): AreaGroup[] {
  if (!data || !levelId) return [];
  return data.groups.filter((g) => g.level_id === levelId && g.path.length === depth + 1);
}

/** An SVG path for a footprint: its exterior plus every void, as closed
 *  subpaths, Y flipped like all drawn geometry. With `fill-rule: evenodd` a
 *  courtyard reads as a hole cut from the footprint — matching the area, which
 *  excludes it. A polygon per ring cannot express that. */
export function footprintPathD(poly: AreaGroup["polygons"][number]): string {
  const sub = (ring: [number, number][]) => `M${ring.map(([x, y]) => `${x},${-y}`).join("L")}Z`;
  let d = sub(poly.exterior);
  for (const hole of poly.holes ?? []) if (hole && hole.length >= 3) d += sub(hole);
  return d;
}

/** Every group across EVERY level and tier as CSV — not just the view on
 *  screen, which is a question the export should not have an opinion about. */
export function buildAreasCsv(data: AreasData | null, rooms: readonly Room[]): string {
  if (!data?.groups?.length) return "";
  const levelName = new Map((data.levels ?? []).map((l) => [l.id, l.name]));
  const tiers = tierNames(data);
  const net = netAreaIndex(rooms);
  // Trim float-noise tails, and no locale separators: a CSV is read by a
  // spreadsheet, not by a person.
  const num = (x: number) => Math.round(x * 1000) / 1000;
  const rows: (string | number)[][] = [["level", ...tiers, "footprint", "net", "delta", "counted_up"]];
  for (const g of data.groups) {
    const depth = g.path.length - 1;
    const netA = net.get(`${g.level_id}|${depth}|${pathKey(g.path, depth)}`) ?? 0;
    rows.push([
      levelName.get(g.level_id) ?? g.level_id,
      ...tiers.map((_, i) => (i < g.path.length ? tierLabel(g.path[i]) : "")),
      num(g.area),
      num(netA),
      num(g.area - netA),
      g.counted_upward !== false ? "yes" : "no",
    ]);
  }
  return rows.map((r) => r.map((v) => csvCell(String(v))).join(",")).join("\r\n");
}

function csvCell(s: string): string {
  return /[",\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}
