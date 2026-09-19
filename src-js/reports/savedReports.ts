// Saved reports, as the page handles them: the id rule, and the two
// translations between a saved document and the form.
//
// Pure, because both translations are places a field can quietly go missing —
// a saved report that loses its measures on load looks like a report somebody
// built wrong.

import type { SavedReport } from "./generated/SavedReport.js";
import type { FormState } from "./reportRequest.js";

/**
 * A name turned into a file-name-safe id.
 *
 * **Derived once, at save, and then never again.** Renaming a report keeps its
 * id, so a link to it survives — which is the whole reason the id is not just
 * the name. A name that reduces to nothing (all punctuation) falls back to a
 * timestamp rather than an empty id the server would refuse.
 */
export function idFor(name: string): string {
  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 60);
  return slug || `report-${Date.now()}`;
}

/** The form, plus what only a saved report carries. */
export function toSaved(
  id: string,
  name: string,
  form: FormState,
  milestone: string,
  filter: unknown,
): SavedReport {
  return {
    id,
    name,
    entity: form.entityId,
    by_room: form.byRoom,
    columns: [...form.columns],
    room_columns: [...form.roomColumns],
    measures: [...form.measures],
    shape: form.shape,
    include_rooms_without: form.includeRoomsWithout,
    include_unattributed: form.includeUnattributed,
    building: null,
    milestone: milestone || null,
    limit: form.limit,
    filter: filter ?? null,
  };
}

/**
 * A saved report back into the form.
 *
 * Defaults are the server's, not this file's guesses: an older document written
 * before a field existed reads as the same default the server would apply, so
 * loading it and running it agree.
 */
export function toForm(report: SavedReport): FormState {
  return {
    entityId: report.entity,
    byRoom: report.by_room,
    columns: [...report.columns],
    roomColumns: [...report.room_columns],
    measures: [...report.measures],
    shape: report.shape === "per_room" ? "per_room" : "per_match",
    includeRoomsWithout: report.include_rooms_without,
    includeUnattributed: report.include_unattributed,
    limit: report.limit ?? 500,
  };
}

/** Which report type in the dropdown a saved report belongs to. */
export const typeIdFor = (report: { entity: string; by_room: boolean }): string =>
  `${report.by_room ? "by_room" : "sched"}.${report.entity}`;
