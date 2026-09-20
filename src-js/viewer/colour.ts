// Colour plans: what colour a room draws in when a project declares a plan.
//
// **All client-side by design.** The server stores `colour_plans` verbatim and
// computes nothing, because every mode here is a presentation reshuffle of data
// the browser already holds — the same rule that keeps CSV export and area
// tabulation off the server.
//
// Pure, and separated into a CONTEXT and a per-room lookup for a reason that is
// about cost: two of the three modes need a property of the whole level before
// any one room can be coloured (the largest absolute difference, which hues are
// taken), so resolving that once per paint keeps it out of the per-room path.
//
// The palette itself is NOT here. `SCHEMES` and `qualitative` live in
// `static/common.js`, a classic script the adjacency graph also loads, and two
// views that disagree about what colour a department is are worse than either
// being arbitrary. This module reads them through `palette.ts`.

import { hexToRgb, qualitative, rgbToHex, schemeStops } from "./palette.js";
import type { Room } from "../renderer/types.js";

/** Fill for a room the active plan cannot colour — a missing or unparseable
 *  property, a ratio by zero, a value in a gap between bands, an undefined
 *  tier. A muted neutral, deliberately distinct from the theme fill so "not
 *  coloured by this plan" reads as a state rather than a bug. */
export const NO_DATA_COLOUR = "#c9c2b0";
/** Match mode is binary and needs no palette: two fixed, colour-blind-safe
 *  stops (ColorBrewer RdYlGn extremes). */
export const MATCH_COLOUR = "#1a9850";
export const MISMATCH_COLOUR = "#d73027";
/** A date after `near_date` is "future" — a fixed blue, separate from the
 *  green→red near/far ramp, because RdYlGn has no blue of its own. */
export const FUTURE_COLOUR = "#2c7fb8";

export interface Band {
  lo?: number | null;
  hi?: number | null;
  colour: string;
}

export type ColourMode =
  | {
      kind: "propertycompare";
      property_a: string;
      property_b: string;
      op?: "diff" | "ratio";
      colouring:
        | { style: "match"; tolerance: number }
        | { style: "diverging"; scheme: string }
        | { style: "bands"; bands: Band[] };
    }
  | { kind: "hierarchy"; tiers?: string[]; scheme: string }
  | { kind: "daterange"; property: string; format?: string | null; near_date: string; scheme: string };

export interface ColourPlan {
  name: string;
  active?: boolean;
  mode: ColourMode;
}

export interface ColourContext {
  maxAbs: number;
  parentTier?: string | undefined;
  childTier?: string | undefined;
  parentHue?: Map<string, string>;
  childColour?: Map<string, string>;
  nearMs?: number | null;
  maxPast?: number;
}

/** A room property, or a joined reference field as `<source>.<field>`.
 *
 *  The namespace split follows the server's rule: a prefix binds as a source
 *  only when it names one this payload actually carries, so a Revit property
 *  with a dot in its name keeps resolving as itself. Absent and blank collapse
 *  to the same null — a plan cannot act on an empty string any more than on a
 *  missing key. */
export function roomValue(room: Room, name: string, knownSources: readonly string[]): string | null {
  const dot = name.indexOf(".");
  if (dot > 0) {
    const source = name.slice(0, dot);
    if (knownSources.includes(source)) {
      const record = (room as unknown as Record<string, { fields?: Record<string, string> }>)[source];
      const value = record?.fields?.[name.slice(dot + 1).trim()];
      return value == null || value === "" ? null : value;
    }
  }
  const p = room.properties?.[name];
  return p?.value != null && p.value !== "" ? p.value : null;
}

function numericProp(room: Room, name: string, sources: readonly string[]): number | null {
  const raw = roomValue(room, name, sources);
  if (raw == null) return null;
  const n = Number.parseFloat(raw);
  return Number.isFinite(n) ? n : null;
}

/** A hex lightened toward white by `t`. How a hierarchy plan tints one
 *  parent's children so they read as siblings rather than as separate groups. */
export function lighten(hex: string, t: number): string {
  const [r, g, b] = hexToRgb(hex);
  return rgbToHex(r + (255 - r) * t, g + (255 - g) * t, b + (255 - b) * t);
}

/** A scheme sampled at `t` in [0, 1], linearly between its stops. */
export function sampleScheme(name: string, t: number): string {
  const stops = schemeStops(name);
  const clamped = Math.max(0, Math.min(1, t));
  if (stops.length === 1) return stops[0]!;
  const pos = clamped * (stops.length - 1);
  const i = Math.min(Math.floor(pos), stops.length - 2);
  const f = pos - i;
  const a = hexToRgb(stops[i]!);
  const b = hexToRgb(stops[i + 1]!);
  return rgbToHex(a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f);
}

/** One classification tier's key for a room, or null when the tier is absent
 *  or explicitly undefined — which is a reported state, not a missing value. */
export function classKey(room: Room, tierName: string): string | null {
  const tier = (room.classification ?? []).find((t) => t.tier === tierName);
  if (!tier || tier.undefined) return null;
  return tier.code ?? tier.name ?? null;
}

/** A date value, by an strftime pattern when the plan names one, else
 *  `Date.parse`. */
export function parseDate(value: string | null, format?: string | null): number | null {
  if (value == null || value === "") return null;
  if (format) return parseStrftime(value, format);
  const ms = Date.parse(value);
  return Number.isFinite(ms) ? ms : null;
}

/** strftime, limited to the specifiers the settings editor offers. An
 *  unsupported one returns null rather than guessing: a date guessed wrong
 *  colours a room confidently and wrongly. */
export function parseStrftime(value: string, pattern: string): number | null {
  const order: string[] = [];
  let regex = "^";
  for (let i = 0; i < pattern.length; i++) {
    if (pattern[i] !== "%") {
      regex += pattern[i]!.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      continue;
    }
    i++;
    let nopad = false;
    if (pattern[i] === "-") {
      nopad = true;
      i++;
    }
    const spec = pattern[i];
    const num = nopad ? "\\d{1,2}" : "\\d{2}";
    switch (spec) {
      case "Y":
        regex += "(\\d{4})";
        order.push("Y");
        break;
      case "y":
        regex += "(\\d{2})";
        order.push("y");
        break;
      case "m":
      case "d":
      case "H":
      case "I":
      case "M":
      case "S":
        regex += `(${num})`;
        order.push(spec);
        break;
      case "p":
        regex += "([AaPp][Mm])";
        order.push("p");
        break;
      case "z":
        regex += "(Z|[+-]\\d{2}:?\\d{2})";
        order.push("z");
        break;
      case "%":
        regex += "%";
        break;
      default:
        return null;
    }
  }
  const m = new RegExp(`${regex}$`).exec(String(value).trim());
  if (!m) return null;
  const f: Record<string, string> = {};
  order.forEach((k, idx) => {
    f[k] = m[idx + 1]!;
  });
  const year = f["Y"] != null ? +f["Y"] : f["y"] != null ? 2000 + +f["y"] : null;
  if (year == null) return null;
  let hour = f["H"] != null ? +f["H"] : f["I"] != null ? +f["I"] : 0;
  if (f["p"] && f["I"] != null) hour = (+f["I"] % 12) + (/pm/i.test(f["p"]) ? 12 : 0);
  let ms = Date.UTC(
    year,
    f["m"] != null ? +f["m"] - 1 : 0,
    f["d"] != null ? +f["d"] : 1,
    hour,
    f["M"] != null ? +f["M"] : 0,
    f["S"] != null ? +f["S"] : 0,
  );
  const zone = f["z"];
  if (zone && zone !== "Z") {
    const zm = /([+-])(\d{2}):?(\d{2})/.exec(zone);
    // Shift to UTC: a +10:00 reading is ten hours EARLIER in UTC.
    if (zm) ms += (zm[1] === "-" ? 1 : -1) * (+zm[2]! * 60 + +zm[3]!) * 60000;
  }
  return ms;
}

function propertyCompareNumber(room: Room, mode: ColourMode, sources: readonly string[]): number | null {
  if (mode.kind !== "propertycompare") return null;
  const a = numericProp(room, mode.property_a, sources);
  const b = numericProp(room, mode.property_b, sources);
  if (a == null || b == null) return null;
  if (mode.op === "ratio") return b === 0 ? null : a / b;
  return a - b;
}

/** What a plan needs to know about the WHOLE level before it can colour one
 *  room: the largest absolute difference, which hue each parent group takes,
 *  how far into the past the oldest date is. */
export function buildColourContext(
  rooms: readonly Room[],
  plan: ColourPlan | null,
  sources: readonly string[] = [],
): ColourContext {
  const ctx: ColourContext = { maxAbs: 0 };
  if (!plan) return ctx;
  const mode = plan.mode;

  if (mode.kind === "propertycompare" && mode.colouring.style === "diverging") {
    for (const room of rooms) {
      const n = propertyCompareNumber(room, mode, sources);
      if (n != null) ctx.maxAbs = Math.max(ctx.maxAbs, Math.abs(n));
    }
    return ctx;
  }

  if (mode.kind === "hierarchy") {
    const parentTier = mode.tiers?.[0];
    const childTier = mode.tiers?.[1];
    const parents = new Set<string>();
    const childrenByParent = new Map<string, Set<string>>();
    for (const room of rooms) {
      const pk = parentTier ? classKey(room, parentTier) : null;
      if (pk == null) continue;
      parents.add(pk);
      if (!childTier) continue;
      const ck = classKey(room, childTier);
      if (ck == null) continue;
      if (!childrenByParent.has(pk)) childrenByParent.set(pk, new Set());
      childrenByParent.get(pk)!.add(ck);
    }
    ctx.parentHue = new Map();
    // SORTED, so a department keeps its colour across a repaint — and takes the
    // same one in the adjacency graph, which sorts the same keys the same way.
    [...parents].sort().forEach((pk, i) => ctx.parentHue!.set(pk, qualitative(mode.scheme, i)));
    ctx.childColour = new Map();
    for (const [pk, set] of childrenByParent) {
      const base = ctx.parentHue.get(pk)!;
      const list = [...set].sort();
      list.forEach((ck, j) =>
        ctx.childColour!.set(`${pk}\0${ck}`, lighten(base, list.length > 1 ? (j / (list.length - 1)) * 0.55 : 0)),
      );
    }
    ctx.parentTier = parentTier;
    ctx.childTier = childTier;
    return ctx;
  }

  if (mode.kind === "daterange") {
    ctx.nearMs = parseDate(mode.near_date, null);
    ctx.maxPast = 0;
    for (const room of rooms) {
      const d = parseDate(roomValue(room, mode.property, sources), mode.format);
      if (d == null || ctx.nearMs == null) continue;
      const delta = d - ctx.nearMs;
      // Past and at only: the future is a flat blue, so it must not stretch
      // the ramp the rest of the rooms are measured against.
      if (delta <= 0) ctx.maxPast = Math.max(ctx.maxPast, -delta);
    }
  }
  return ctx;
}

/** One room's fill under a plan. */
export function colourForRoom(
  room: Room,
  plan: ColourPlan,
  ctx: ColourContext,
  sources: readonly string[] = [],
): string {
  const mode = plan.mode;

  if (mode.kind === "hierarchy") {
    const pk = ctx.parentTier ? classKey(room, ctx.parentTier) : null;
    if (pk == null) return NO_DATA_COLOUR;
    if (ctx.childTier) {
      const ck = classKey(room, ctx.childTier);
      const tinted = ck != null ? ctx.childColour?.get(`${pk}\0${ck}`) : undefined;
      if (tinted) return tinted;
    }
    return ctx.parentHue?.get(pk) ?? NO_DATA_COLOUR;
  }

  if (mode.kind === "daterange") {
    const d = parseDate(roomValue(room, mode.property, sources), mode.format);
    if (d == null || ctx.nearMs == null) return NO_DATA_COLOUR;
    const delta = d - ctx.nearMs;
    if (delta > 0) return FUTURE_COLOUR;
    const t = ctx.maxPast && ctx.maxPast > 0 ? 1 - Math.abs(delta) / ctx.maxPast : 1;
    return sampleScheme(mode.scheme, t);
  }

  if (mode.kind !== "propertycompare") return NO_DATA_COLOUR;
  const number = propertyCompareNumber(room, mode, sources);
  if (number == null) return NO_DATA_COLOUR;

  const c = mode.colouring;
  if (c.style === "match") return Math.abs(number) <= c.tolerance ? MATCH_COLOUR : MISMATCH_COLOUR;
  if (c.style === "diverging") {
    const extent = ctx.maxAbs || 1;
    return sampleScheme(c.scheme, 0.5 + number / (2 * extent));
  }
  if (c.style === "bands") {
    // Ordered first-match scan over half-open [lo, hi). The simplicity is
    // earned by the server's validation: bands arrive sorted and disjoint.
    for (const band of c.bands) {
      const aboveLo = band.lo == null || number >= band.lo;
      const belowHi = band.hi == null || number < band.hi;
      if (aboveLo && belowHi) return band.colour;
    }
  }
  return NO_DATA_COLOUR;
}
