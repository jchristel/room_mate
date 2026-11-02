// One validation response turned into report rows, per check.
//
// **Pure, and separate from the component, because this is the part with
// answers in it.** Which findings a check lists, what it refuses to call a
// finding, and how each is worded are all testable without a DOM — and the
// wording is load-bearing: these strings are what a reader takes to the
// modeller, and what the CSV carries.
//
// Every row is `severity` plus flat cells, so one table component renders all
// three checks and the CSV writer takes rows rather than a response.

import type { ValidationResponse } from "./types.js";

/** What a row is: a real finding, an expected state, or context. */
export type Severity = "finding" | "expected" | "warning";

export interface CheckRow {
  severity: Severity;
  cells: Record<string, string>;
  /** The option that gates this row, when one does. */
  gate?: string;
}

export interface CheckOption {
  id: string;
  label: string;
  /** Default. An option is on when it admits rows nobody should have to ask for. */
  on: boolean;
}

export interface CheckDef {
  id: string;
  label: string;
  desc: string;
  columns: string[];
  options: CheckOption[];
  /** What this check deliberately does not call a finding. */
  foot: string;
  rows: (data: ValidationResponse) => CheckRow[];
}

const roomLabel = (id: string, source: { error_rooms?: Record<string, { number?: string; name?: string }> }): string => {
  const room = source.error_rooms?.[id];
  if (!room) return id;
  return [room.number, room.name].filter(Boolean).join(" ") || id;
};

/** The reference-data check: what the viewer's QA band shows today. */
function referenceRows(data: ValidationResponse): CheckRow[] {
  const rows: CheckRow[] = [];

  // The phase disagreement first, and under a "Phase" source label rather than
  // a section of its own: it is per MODEL, not per room, so it fills none of
  // the room columns — but hiding it from whoever opened this expecting every
  // finding in one place is worse than a slightly odd row.
  if (data.phases?.disagree) {
    for (const [model, phase] of Object.entries(data.phases.by_model ?? {})) {
      rows.push({
        severity: "warning",
        gate: "phase",
        cells: {
          Source: "Phase",
          Room: model,
          "Link value": "",
          Finding:
            `model ${model} is on phase ${phase ?? "(none — rooms were never filtered)"}; ` +
            "this project's models disagree, so the merged plan spans more than one phase",
        },
      });
    }
  }

  for (const [name, report] of Object.entries(data.sources ?? {})) {
    const link = report.link_property ?? "";
    const push = (roomId: string, finding: string, severity: Severity = "finding") =>
      rows.push({
        severity,
        cells: {
          Source: name,
          Room: roomLabel(roomId, report),
          "Link value": report.error_rooms?.[roomId]?.link_value ?? "",
          Finding: finding,
        },
      });

    for (const id of report.rooms_missing_link_value ?? []) push(id, `missing link value (${link})`);
    for (const dup of report.duplicate_link_values ?? [])
      for (const id of dup.room_ids) push(id, `duplicate link value: ${dup.value}`);
    // Both directions, worded as the viewer words them: whoever reads this is
    // finding out which side is short.
    for (const id of report.rooms_unmatched ?? []) push(id, `room in the model with no match in ${name}`);
    for (const value of report.reference_unmatched ?? [])
      rows.push({
        severity: "finding",
        cells: { Source: name, Room: "", "Link value": value, Finding: `row in ${name} with no match in the model` },
      });
    for (const value of report.reference_duplicate_ids ?? [])
      rows.push({
        severity: "finding",
        cells: {
          Source: name,
          Room: "",
          "Link value": value,
          Finding: `${name} id used by more than one row (only the last kept)`,
        },
      });
    if (report.reference_blank_id_rows)
      rows.push({
        severity: "finding",
        cells: {
          Source: name,
          Room: "",
          "Link value": "",
          Finding: `${name} has ${report.reference_blank_id_rows} row(s) with no id (skipped at load)`,
        },
      });
    for (const m of report.property_mismatches ?? [])
      push(m.room_id, `property mismatch: ${m.field} (room="${m.room_value}" vs ${name}="${m.reference_value}")`);
    for (const m of report.fields_absent_in_revit ?? []) push(m.room_id, `field absent in Revit: ${m.field}`);
    for (const m of report.fields_empty_in_revit ?? []) push(m.room_id, `field empty in Revit: ${m.field}`);
  }

  return rows;
}

/** Openings naming a room their own model does not hold. */
function openingRows(data: ValidationResponse): CheckRow[] {
  const rows: CheckRow[] = [];
  for (const [entity, report] of Object.entries(data.openings ?? {})) {
    for (const u of report.unresolved_room ?? [])
      rows.push({
        severity: "finding",
        cells: {
          Element: `${entity} ${u.opening_id}`,
          Model: u.model_id,
          "Names room": u.room_id,
          Finding: `dangling ${u.side} reference: this model has no room with that id`,
        },
      });
    // Pending is not a fault: doors may be pushed before their rooms, and the
    // reference resolves the moment they land. Off by default for that reason.
    for (const p of report.pending_rooms ?? [])
      rows.push({
        severity: "expected",
        gate: "pending",
        cells: {
          Element: `${entity} ${p.opening_id}`,
          Model: p.model_id,
          "Names room": "",
          Finding: "pending: no rooms have been pushed for this model yet",
        },
      });
    for (const w of report.without_room_reference ?? [])
      rows.push({
        severity: "expected",
        gate: "homeless",
        cells: {
          Element: `${entity} ${w.opening_id}`,
          Model: w.model_id,
          "Names room": "",
          Finding: "names no room on either side — external, or in a model that holds none",
        },
      });
  }
  return rows;
}

/** Spaces against rooms, both directions, plus the keys that cannot be matched. */
function spaceRows(data: ValidationResponse): CheckRow[] {
  const report = data.spaces;
  if (!report) return [];
  const rows: CheckRow[] = [];

  for (const model of report.by_model ?? []) {
    for (const space of model.unmatched ?? [])
      rows.push({
        severity: "finding",
        cells: {
          Direction: "space",
          Key: space.key ?? "(no key value)",
          Name: space.name,
          Model: space.model_id,
          Finding: space.key ? "no room carries this key" : "this space carries no value for the configured key",
        },
      });
    for (const dup of model.ambiguous_keys ?? [])
      rows.push({
        severity: "warning",
        cells: {
          Direction: "key",
          Key: dup.key,
          Name: "",
          Model: model.model_id,
          Finding: `${dup.count} spaces share this key — the match cannot be made without guessing`,
        },
      });
  }

  // The room side is a full list, and of the same standing: a project that
  // expects 1:1 is as interested in a room with no space as the reverse.
  for (const room of report.rooms_without_space ?? [])
    rows.push({
      severity: "finding",
      gate: "rooms",
      cells: {
        Direction: "room",
        Key: room.key ?? "(no key value)",
        Name: room.name,
        Model: room.model_id,
        Finding: "no space matches this room",
      },
    });

  for (const dup of report.ambiguous_room_keys ?? [])
    rows.push({
      severity: "warning",
      cells: {
        Direction: "key",
        Key: dup.key,
        Name: "",
        Model: "(rooms)",
        Finding: `${dup.count} rooms share this key — the match cannot be made without guessing`,
      },
    });

  for (const model of report.models_without_spaces ?? [])
    rows.push({
      severity: "finding",
      cells: {
        Direction: "model",
        Key: "",
        Name: "",
        Model: model,
        Finding: "audited and holds no spaces at all",
      },
    });

  // Never audited is a different fact from audited-and-empty, and the one the
  // entity exists to tell apart. Informational: nobody has claimed otherwise.
  for (const model of report.models_not_audited ?? [])
    rows.push({
      severity: "expected",
      gate: "notAudited",
      cells: {
        Direction: "model",
        Key: "",
        Name: "",
        Model: model,
        Finding: "no spaces snapshot has ever been pushed for this model",
      },
    });

  return rows;
}

export const CHECKS: CheckDef[] = [
  {
    id: "reference",
    label: "Reference data check",
    desc: "Rooms and reference records that do not line up: missing keys, duplicates, and each side's unmatched rows.",
    columns: ["Source", "Room", "Link value", "Finding"],
    options: [{ id: "phase", label: "Include the phase disagreement", on: true }],
    foot: "This is what the viewer's QA band used to download. The band still shows the findings on screen.",
    rows: referenceRows,
  },
  {
    id: "openings",
    label: "Doors and windows with unresolved rooms",
    desc: "Openings naming a room their model does not hold, with pending references kept separate from dangling ones.",
    columns: ["Element", "Model", "Names room", "Finding"],
    options: [
      { id: "pending", label: "Include pending references (rooms not pushed yet)", on: false },
      { id: "homeless", label: "Include openings that name no room at all", on: false },
    ],
    foot:
      "Pending is not a fault: doors may be pushed before their rooms. Naming no room is ordinary too — every external door and facade-package window does.",
    rows: openingRows,
  },
  {
    id: "spaces",
    label: "Unmatched spaces and rooms",
    desc: "Spaces whose key matches no room, rooms no space matches, and keys too ambiguous to match at all.",
    columns: ["Direction", "Key", "Name", "Model", "Finding"],
    options: [
      { id: "rooms", label: "Also list rooms with no space", on: true },
      { id: "notAudited", label: "Include models never audited for spaces", on: false },
    ],
    foot: "Both directions are findings of the same standing: an unmatched room is as much a signal as an unmatched space.",
    rows: spaceRows,
  },
];

/** Rows a check shows under the options currently set. */
export function visibleRows(check: CheckDef, data: ValidationResponse, options: Record<string, boolean>): CheckRow[] {
  return check.rows(data).filter((row) => !row.gate || options[row.gate]);
}
