// The QA report, reduced to the three answers the page needs from it: which
// rooms are flagged, which GRID CELLS are flagged, and what the collapsed
// strip says.
//
// Pure, because each of the three is wrong in a way a reader cannot see. A
// room id that is not a room id would highlight an innocent room; a cell
// address built from the wrong key marks nothing and looks like a clean
// column; and a strip that under-reports makes a panel full of findings look
// like a pass.

/** One reference source's findings. A hand-written subset of
 *  `compute_validation`'s response — the fields this page addresses. */
export interface SourceReport {
  link_property?: string;
  link_property_absent_everywhere?: boolean;
  rooms_missing_link_value: string[];
  rooms_unmatched: string[];
  duplicate_link_values: { value: string; room_ids: string[] }[];
  property_mismatches: { room_id: string; field: string; revit?: string; reference?: string }[];
  fields_absent_in_revit: { room_id: string; field: string }[];
  fields_empty_in_revit: { room_id: string; field: string }[];
  /** `label -> revit_property`, which is where the two vocabularies meet. */
  field_coverage?: { label: string; checked?: boolean; revit_property?: string | null }[];
  reference_unmatched?: unknown[];
}

export interface ValidationReport {
  total_rooms?: number;
  sources?: Record<string, SourceReport>;
  discrepancies?: { total?: number };
  phases?: { disagree?: boolean; by_model?: Record<string, string | null> };
}

/**
 * Every room id one source flagged.
 *
 * `reference_unmatched` is deliberately absent: it holds the source's own LINK
 * VALUES, not room ids, and the finding is precisely that no room corresponds
 * to them. Mixing them in would put non-room ids into a set used to highlight
 * rooms — at best inert, at worst flagging an unrelated room whose id happens
 * to equal a link value.
 */
export function sourceErrorRoomIds(report: SourceReport): string[] {
  return [
    ...report.rooms_missing_link_value,
    ...report.duplicate_link_values.flatMap((d) => d.room_ids),
    ...report.rooms_unmatched,
    ...report.property_mismatches.map((m) => m.room_id),
    ...report.fields_absent_in_revit.map((m) => m.room_id),
    ...report.fields_empty_in_revit.map((m) => m.room_id),
  ];
}

/** The union across sources: a room is flagged on the plan if ANY source has
 *  something to say about it. WHICH source belongs in the band, not the plan. */
export function errorRoomIds(data: ValidationReport | null): Set<string> {
  const ids = new Set<string>();
  for (const report of Object.values(data?.sources ?? {})) for (const id of sourceErrorRoomIds(report)) ids.add(id);
  return ids;
}

export interface GridErrors {
  /** `${roomId}\0${columnKey}` — the finest address the report supports. */
  cells: Set<string>;
  /** Room-level findings, which have no column and mark the Id cell. */
  rooms: Set<string>;
}

/**
 * QA marks at grid addresses.
 *
 * Keyed by the grid's OWN column key, and two things force that. A finding's
 * `field` is the reference LABEL (`NetArea`) while the Revit column is keyed by
 * the PROPERTY it maps to (`p:Area`), so comparing the two only ever matched
 * where a label happened to equal its property name — `Department` marked
 * correctly and `NetArea` never did. And with two sources both declaring a
 * `NetArea`, the key has to name the source as well.
 *
 * A disagreement is between two cells, so BOTH are marked: the source's own
 * cell and the Revit cell it contradicts, resolved through that source's
 * `field_coverage`, which is where label → property already lives.
 */
export function gridErrors(data: ValidationReport | null): GridErrors {
  const cells = new Set<string>();
  const rooms = new Set<string>();
  for (const [name, report] of Object.entries(data?.sources ?? {})) {
    const revitProperty = new Map(
      (report.field_coverage ?? []).filter((c) => c.revit_property).map((c) => [c.label, c.revit_property!]),
    );
    const mark = (roomId: string, label: string) => {
      cells.add(`${roomId}\0${name}:${label}`);
      const prop = revitProperty.get(label);
      if (prop) cells.add(`${roomId}\0p:${prop}`);
    };
    for (const m of report.property_mismatches) mark(m.room_id, m.field);
    for (const m of report.fields_absent_in_revit) mark(m.room_id, m.field);
    for (const m of report.fields_empty_in_revit) mark(m.room_id, m.field);
    for (const id of report.rooms_missing_link_value) rooms.add(id);
    for (const id of report.rooms_unmatched) rooms.add(id);
    for (const dup of report.duplicate_link_values) for (const id of dup.room_ids) rooms.add(id);
  }
  return { cells, rooms };
}

/**
 * The collapsed strip's label — ONE definition, because it is written from two
 * places: a fresh report, and the expand/collapse toggle. They used to build it
 * separately, and adding the phase marker to only the first meant the marker
 * vanished the moment you expanded the panel — a strip reading "✓" over a body
 * full of findings.
 *
 * A phase disagreement is its OWN marker rather than folded into the count:
 * `discrepancies.total` means "rows that failed to reconcile", so adding one
 * for a project-level phase problem would make the number mean two things.
 */
export function stripLabel(data: ValidationReport | null): string {
  if (!data) return "QA";
  const marks: string[] = [];
  const issues = data.discrepancies?.total ?? 0;
  if (issues) marks.push(`⚠ ${issues}`);
  if (data.phases?.disagree) marks.push("⚠ mixed phases");
  return `QA · ${marks.length ? marks.join(" · ") : "✓"}`;
}

/** Whether the band has anything to show. A phase disagreement keeps it open
 *  even with no sources: it owes nothing to reference data, and a project that
 *  reconciles against nothing is exactly the one nobody would be looking at. */
export function hasFindings(data: ValidationReport | null): boolean {
  return !!data && (Object.keys(data.sources ?? {}).length > 0 || !!data.phases?.disagree);
}
