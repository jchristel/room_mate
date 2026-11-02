// The rooms grid: which columns exist, which rows survive the filters, and how
// both become a CSV.
//
// Pure, and the largest thing in the port that earns tests rather than a
// comparison by eye: a column that quietly disappears, a numeric sort that
// orders "10" before "9", or a CSV that exports the windowed slice instead of
// every filtered row are all failures a reader would take at face value.

import { levelsForPayload, type LevelSource } from "./levels.js";
import { sourceDisplayName } from "./properties.js";
import type { Room } from "../renderer/types.js";

/** One column: how to read it off a room, and which source it belongs to. */
export interface GridColumn {
  key: string;
  label: string;
  /** `"model"` or a reference source name. Decides the group header, the
   *  styling, and which toggle switches it off. */
  source: string;
  /** A reference column with no Revit property mapped, so QA never checks it.
   *  Marked rather than hidden — "not checked" is a fact about the column. */
  unmapped?: boolean;
  get: (room: Room) => string;
}

export interface GridPayload extends LevelSource {
  rooms?: readonly Room[];
  reference_labels?: Record<string, Record<string, { all_labels?: string[]; reconciliation?: Record<string, unknown> }>>;
}

/**
 * Every column the payload offers.
 *
 * Intrinsic columns first — the identity a reader scans by — then the rooms'
 * own property keys, then one group per reference source.
 *
 * A source's columns come from the response's own label set, NOT from a union
 * of the rooms' joined fields: a column that matched no room in scope would
 * otherwise be invisible, which is precisely the case the coverage report goes
 * out of its way to show as "not checked" rather than omit. The union fallback
 * fires only for a source with no vocabulary for this project — the unscoped
 * multi-project merge — and is an honest lesser capability there.
 */
export function buildColumns(payload: GridPayload | null, projectId: string | null, sources: readonly string[]): GridColumn[] {
  const cols: GridColumn[] = [];
  if (!payload) return cols;

  const levelName = new Map(levelsForPayload(payload).map((l) => [l.id, l.name]));
  cols.push({ key: "$level", label: "Level", source: "model", get: (r) => levelName.get(r.level_id ?? "") || r.level_id || "" });
  cols.push({ key: "$name", label: "Name", source: "model", get: (r) => r.name || "" });
  cols.push({ key: "$id", label: "Id", source: "model", get: (r) => r.id || "" });

  const propKeys = new Set<string>();
  for (const r of payload.rooms ?? []) for (const k of Object.keys(r.properties ?? {})) propKeys.add(k);
  for (const k of [...propKeys].sort()) {
    cols.push({ key: `p:${k}`, label: k, source: "model", get: (r) => r.properties?.[k]?.value ?? "" });
  }

  for (const name of sources) {
    const labels = payload.reference_labels?.[projectId ?? ""]?.[name];
    if (labels) {
      const mapped = labels.reconciliation ?? {};
      for (const label of labels.all_labels ?? []) {
        cols.push({
          key: `${name}:${label}`,
          label,
          source: name,
          unmapped: !(label in mapped),
          // An unmatched room simply has no key for this source — empty cells,
          // never an error: an unmatched link value is a signal, not a failure.
          get: (r) => fieldOf(r, name, label),
        });
      }
      continue;
    }
    const fieldKeys = new Set<string>();
    for (const r of payload.rooms ?? []) {
      const record = recordOf(r, name);
      if (record) for (const k of Object.keys(record.fields ?? {})) fieldKeys.add(k);
    }
    for (const label of [...fieldKeys].sort()) {
      cols.push({ key: `${name}:${label}`, label, source: name, get: (r) => fieldOf(r, name, label) });
    }
  }
  return cols;
}

function recordOf(room: Room, source: string): { fields?: Record<string, string> } | undefined {
  return (room as unknown as Record<string, { fields?: Record<string, string> } | undefined>)[source];
}

function fieldOf(room: Room, source: string, label: string): string {
  return recordOf(room, source)?.fields?.[label] ?? "";
}

/** The columns a reader has switched on. */
export function visibleColumns(
  columns: readonly GridColumn[],
  showModel: boolean,
  enabledSources: ReadonlySet<string>,
): GridColumn[] {
  return columns.filter((c) => (c.source === "model" ? showModel : enabledSources.has(c.source)));
}

export interface SortState {
  key: string | null;
  asc: boolean;
}

/**
 * Rooms after every active column filter, then the active sort.
 *
 * Sorting is NUMERIC when both sides parse, so "10" does not sort before "9" —
 * and falls back to a string compare otherwise, which is what keeps a column of
 * room numbers like "01.02" in a sensible order.
 */
export function computeRows(
  rooms: readonly Room[],
  columns: readonly GridColumn[],
  filters: ReadonlyMap<string, string>,
  sort: SortState,
): Room[] {
  const active = columns.filter((c) => filters.get(c.key));
  let rows = active.length
    ? rooms.filter((r) => active.every((c) => String(c.get(r)).toLowerCase().includes(filters.get(c.key)!)))
    : rooms.slice();

  const sortCol = columns.find((c) => c.key === sort.key);
  if (sortCol) {
    const dir = sort.asc ? 1 : -1;
    rows = rows.slice().sort((a, b) => {
      const x = sortCol.get(a);
      const y = sortCol.get(b);
      const nx = Number.parseFloat(x);
      const ny = Number.parseFloat(y);
      if (Number.isFinite(nx) && Number.isFinite(ny) && nx !== ny) return (nx - ny) * dir;
      return String(x).localeCompare(String(y)) * dir;
    });
  }
  return rows;
}

/** Column groups for the header: consecutive columns of one source. */
export function columnGroups(columns: readonly GridColumn[]): { source: string; label: string; span: number }[] {
  const groups: { source: string; label: string; span: number }[] = [];
  for (const c of columns) {
    const last = groups[groups.length - 1];
    if (last && last.source === c.source) last.span++;
    else groups.push({ source: c.source, label: c.source === "model" ? "Model" : sourceDisplayName(c.source), span: 1 });
  }
  return groups;
}

/** One CSV field, quoted when it has to be. */
export function csvEscape(value: string): string {
  const s = String(value ?? "");
  return /[",\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

/**
 * A CSV of what is on screen: the visible columns in order, and EVERY filtered
 * row — not the windowed slice, which is a rendering detail and would export a
 * few dozen rows of a few thousand.
 *
 * A reference column is qualified (`source.Label`) so two sources with a field
 * of the same name stay apart in the file.
 */
export function buildCsv(rows: readonly Room[], columns: readonly GridColumn[]): string {
  if (!columns.length) return "";
  const out = [columns.map((c) => (c.source === "model" ? c.label : `${c.source}.${c.label}`))];
  for (const room of rows) out.push(columns.map((c) => c.get(room)));
  return out.map((r) => r.map(csvEscape).join(",")).join("\r\n");
}
