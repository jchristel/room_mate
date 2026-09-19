// The report definition, and the sentences a reader needs to trust it.
//
// Pure, and separate from the component, because this is the part with answers
// in it: what the server is asked for, and what the row counts mean once it
// answers. Both are testable without a DOM, and both are easy to get subtly
// wrong — a cap that lies about the total, or a count that silently includes
// the rows nobody matched.

export interface ReportBody {
  entity: string;
  by_room: boolean;
  columns: string[];
  room_columns: string[];
  measures: string[];
  shape: "per_match" | "per_room";
  include_rooms_without: boolean;
  include_unattributed: boolean;
  building?: string;
  milestone?: string;
  limit?: number;
}

export interface FormState {
  entityId: string;
  byRoom: boolean;
  columns: string[];
  roomColumns: string[];
  measures: string[];
  shape: "per_match" | "per_room";
  includeRoomsWithout: boolean;
  includeUnattributed: boolean;
  limit: number;
}

/**
 * The form as the server reads it.
 *
 * A schedule sends no room columns and no measures even when the form is
 * holding some: they are the by-room half of the same form, and sending them
 * would ask for columns the server would have to decide to ignore.
 */
export function toBody(form: FormState): ReportBody {
  return {
    entity: form.entityId,
    by_room: form.byRoom,
    columns: [...form.columns],
    room_columns: form.byRoom ? [...form.roomColumns] : [],
    measures: form.byRoom ? [...form.measures] : [],
    shape: form.byRoom ? form.shape : "per_match",
    include_rooms_without: form.byRoom && form.includeRoomsWithout,
    include_unattributed: form.byRoom ? form.includeUnattributed : true,
    limit: form.limit,
  };
}

export interface ReportSummary {
  /** What the preview is showing, against what it counted. */
  shown: string;
  /** The unmatched rows, when there are any — never hidden, never assumed. */
  unmatched?: string;
}

/**
 * "First 500 of 39,412", and the unmatched note beside it.
 *
 * The cap is the honest half of asking for fewer rows: a preview that showed
 * 500 and said nothing would read as the whole answer, and the number that
 * matters for a take-off is the one the cap hid.
 */
export function summarise(
  rows: number,
  total: number,
  unmatched: number,
  entity: { one: string; many: string },
): ReportSummary {
  const noun = (n: number) => (n === 1 ? "row" : "rows");
  const shown =
    rows < total ? `first ${rows.toLocaleString()} of ${total.toLocaleString()} ${noun(total)}` : `${total.toLocaleString()} ${noun(total)}`;
  if (!unmatched) return { shown };
  return {
    shown,
    unmatched: `${unmatched.toLocaleString()} of them matched nothing — an unattributed ${entity.one}, or a room with no ${entity.many}. They are part of the answer, not an error.`,
  };
}
