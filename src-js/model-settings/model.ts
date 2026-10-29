// The RLS settings of one model, as the page edits them, and the small rules around them.
//
// The server holds this opaque (src/model_settings.rs); the plug-in writes it into the Revit
// model's extensible storage under the key `rls`. The names here are the contract with
// `room_m/rls_settings.py`: change one and the other in the same commit.

export const SIZES = ["a0", "a1", "a2", "a3"] as const;
export type Size = (typeof SIZES)[number];

export interface RlsSettings {
  /** Sheet size to a title block, "Family : Type". */
  sheet_sizes: Record<Size, string>;
  plan_view_type: string;
  elevation_view_type: string;
  /** A schedule whose FIRST filter is the room number. */
  schedule_base: string;
  room_tag: string;
  /** Must be a multi-category tag. */
  independent_tag: string;
  /** Templates: `{Room Parameter}` for a room's value, anything else is fixed text. */
  sheet_number: string;
  sheet_name: string;
  view_name: string;
  [unknown: string]: unknown;
}

/** What Revit offers to choose from; any list may be missing (that category could not be read). */
export interface Options {
  title_blocks?: string[];
  plan_view_types?: string[];
  elevation_view_types?: string[];
  schedules?: string[];
  room_tags?: string[];
  independent_tags?: string[];
  room_parameters?: string[];
}

export interface Draft {
  model_id: string;
  settings: Record<string, unknown>;
  options: Options;
  saved: boolean;
  created_at: string;
  saved_at?: string;
}

const text = (v: unknown): string => (typeof v === "string" ? v : "");

/** Settings with every field present. Keys this page does not know ride along untouched, so a
 *  setting a later version wrote survives this one saving. */
export function normalise(raw: Record<string, unknown>): RlsSettings {
  const sizes = (raw.sheet_sizes && typeof raw.sheet_sizes === "object" ? raw.sheet_sizes : {}) as Record<string, unknown>;
  return {
    ...raw,
    sheet_sizes: { a0: text(sizes.a0), a1: text(sizes.a1), a2: text(sizes.a2), a3: text(sizes.a3) },
    plan_view_type: text(raw.plan_view_type),
    elevation_view_type: text(raw.elevation_view_type),
    schedule_base: text(raw.schedule_base),
    room_tag: text(raw.room_tag),
    independent_tag: text(raw.independent_tag),
    sheet_number: text(raw.sheet_number),
    sheet_name: text(raw.sheet_name),
    view_name: text(raw.view_name),
  };
}

/** The `{token}` names a template uses, in order. */
export function tokensIn(template: string): string[] {
  return [...template.matchAll(/\{([^{}]*)\}/g)].map((m) => m[1] ?? "");
}

/** Tokens that are not a room parameter Revit offered (empty when Revit offered none, so a page
 *  that could not read the list does not call everything wrong). */
export function unknownTokens(template: string, known: string[] | undefined): string[] {
  if (!known || known.length === 0) return [];
  return tokensIn(template).filter((t) => !known.includes(t));
}

/** A template with `{token}` added at the end. */
export const withToken = (template: string, token: string): string => `${template}{${token}}`;

/** What stops the settings being used, in words; empty when they are complete. This is the same
 *  question the plug-in asks before it applies a task ("settings first, if incomplete: exit"). */
export function missing(s: RlsSettings): string[] {
  const out: string[] = [];
  for (const size of SIZES) if (!s.sheet_sizes[size]) out.push(`title block for ${size.toUpperCase()}`);
  if (!s.plan_view_type) out.push("plan view type");
  if (!s.elevation_view_type) out.push("elevation view type");
  if (!s.schedule_base) out.push("schedule base");
  if (!s.sheet_number.trim()) out.push("sheet number logic");
  if (!s.sheet_name.trim()) out.push("sheet name logic");
  if (!s.view_name.trim()) out.push("view name logic");
  return out;
}
