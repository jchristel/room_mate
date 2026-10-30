// Pure helpers for the library page: wording and filtering, with no DOM.

import type { RoomSummary, RuleSummary, Sample, SheetSummary, Suggestion } from "./types.js";

const trim = (s: string): string => s.trim();

/** "BED-01 tagged by FTag / Std" -- what a rule is about: the item's type code. */
export function ruleLabel(r: RuleSummary): string {
  const k = r.key;
  // A room, space or area has no type code: the rule is about the category.
  const tagged = k.type_code || k.tagged_category || "element";
  return `${tagged} tagged by ${k.tag_family} / ${k.tag_type}`;
}

/** Radians as whole-ish degrees, for people. */
export const degrees = (rad: number): number => Math.round(((rad * 180) / Math.PI) * 10) / 10;

/** Paper millimetres, one decimal. */
export const mm = (v: number): string => (Math.round(v * 10) / 10).toString();

/** Rules whose key contains every word of `text` (case-insensitive), in a stable order. */
export function filterRules(rules: readonly RuleSummary[], text: string): RuleSummary[] {
  const words = trim(text).toLowerCase().split(/\s+/).filter(Boolean);
  const hay = (r: RuleSummary) => [r.key.tagged_category, r.key.type_code, r.key.tag_category, r.key.tag_family, r.key.tag_type, r.key.viewport_type, r.tag_kind].join(" ").toLowerCase();
  return rules
    .filter((r) => {
      const h = hay(r);
      return words.every((w) => h.includes(w));
    })
    .sort((a, b) => ruleLabel(a).localeCompare(ruleLabel(b)) || a.key.viewport_type.localeCompare(b.key.viewport_type));
}

/** Where a sample was read: the sheet, the tag and the element, as Revit numbered them. */
export const sampleSource = (s: Sample): string => `sheet ${s.source.sheet.sheet_number || s.source.sheet.revit_sheet_id}, tag ${s.source.tag_id}, element ${s.source.element_id}`;

/** "4.0 x 3.9 m, 15.9 m2, 38 items" */
export function summaryText(s: RoomSummary | undefined): string {
  if (!s) return "no room figures";
  return `${(s.x_mm / 1000).toFixed(1)} x ${(s.y_mm / 1000).toFixed(1)} m, ${s.area_m2.toFixed(1)} m2, ${s.items} item${s.items === 1 ? "" : "s"}`;
}

/** "841 x 594 mm", or "not measured". */
export function sizeText(size: SheetSummary["size_mm"]): string {
  return size ? `${Math.round(size.width_mm)} x ${Math.round(size.height_mm)} mm` : "not measured";
}

/** A sheet a person should look at: a push waiting for a decision. */
export const sheetNeedsAttention = (s: Pick<SheetSummary, "pending">): boolean => s.pending > 0;

export function sheetReasons(s: Pick<SheetSummary, "pending">): string[] {
  const out: string[] = [];
  if (s.pending > 0) out.push(`${s.pending} renumbered push${s.pending === 1 ? "" : "es"} to accept or reject`);
  return out;
}

/** How far a suggestion is, as the three differences its distance is the mean of: each part is an
 *  absolute log-ratio, so e^part - 1 is how much bigger (or, as a ratio, smaller) the figure is. */
export function suggestionText(s: Suggestion): string {
  const pct = (part: number) => `${Math.round((Math.exp(part) - 1) * 100)}%`;
  return `area ${pct(s.parts.area)} off, proportions ${pct(s.parts.proportion)} off, items ${pct(s.parts.items)} off`;
}

export const shortTime = (iso: string): string => iso.replace("T", " ").slice(0, 16);
