// The shapes the library page reads, as the server writes them (`src/sheets.rs`, `sheets_api.rs`,
// `service/sheet_suggest.rs`, `tag_rules.rs`, `tag_rules_api.rs`).

export interface ProjectRow {
  id: string;
  name: string;
}

export interface RoomRow {
  id: string;
  name: string;
  level_id: string;
}

export interface SheetRoom {
  model_id: string;
  room_id: string;
}

/** The room (or rooms) a sheet covers, measured in Revit when it was exported. Millimetres and square metres. */
export interface RoomSummary {
  x_mm: number;
  y_mm: number;
  area_m2: number;
  items: number;
}

export interface SheetSummary {
  id: string;
  taken_at: string;
  revit_sheet_id: string;
  document_title: string;
  sheet_number: string;
  sheet_name: string;
  /** A link to a room: only a sheet that came with a blueprint has one. */
  room: SheetRoom | null;
  room_summary?: RoomSummary;
  /** Paper size measured from the title block, mm. */
  size_mm?: { width_mm: number; height_mm: number };
  /** "number name" of the rooms said at export. */
  rooms?: string[];
  viewports: number;
  /** Pushes waiting for a decision (the sheet number changed). */
  pending: number;
}

export interface PendingSheet {
  pending_id: number;
  received_at: string;
  sheet_number: string;
  sheet_name: string;
  room?: SheetRoom;
  phase?: string;
}

export interface ViewportInfo {
  vp_type?: string;
  view?: { view_scale?: number; name?: string };
}

export interface SheetRecord extends Omit<SheetSummary, "viewports" | "pending"> {
  phase?: string;
  received_at: string;
  /** The sheet as duHast exports it: opaque here beyond its viewports. */
  sheet: { view_ports?: ViewportInfo[]; sheet_size?: string } & Record<string, unknown>;
  pending: PendingSheet[];
  rejected: (PendingSheet & { rejected_at: string })[];
}

export interface Suggestion {
  sheet_id: string;
  sheet_number: string;
  sheet_name: string;
  document_title: string;
  room?: SheetRoom;
  room_name: string;
  signature: { area_m2: number; proportion: number; items: number };
  parts: { area: number; proportion: number; items: number };
  /** The mean of the parts: smaller is closer. */
  distance: number;
}

export interface Suggestions {
  target: { model_id: string; room_id: string; room_name: string; signature: { area_m2: number; proportion: number; items: number } };
  suggestions: Suggestion[];
  unassigned: number;
  unresolved: { sheet_id: string; sheet_number: string; reason: string }[];
}

export interface Offset {
  x: number;
  y: number;
}

export interface RuleKey {
  tagged_category: string;
  tagged_family: string;
  tagged_type: string;
  tag_category: string;
  tag_family: string;
  tag_type: string;
  viewport_type: string;
}

export interface Placement {
  head: Offset;
  /** Radians, relative to the element's facing. */
  rotation: number;
  orientation: string;
  has_leader: boolean;
  leader_end_condition: string;
  elbow?: Offset;
  end?: Offset;
}

export interface RuleSummary {
  id: string;
  key: RuleKey;
  tag_kind: string;
  placement: Placement;
  sample_count: number;
  updated_at: string;
}

export interface Sample {
  source: { sheet: { document_title: string; revit_sheet_id: string; sheet_number: string }; tag_id: number; element_id: number };
  view_scale: number;
  head: Offset;
  rotation: number;
  orientation: string;
  has_leader: boolean;
  leader_end_condition: string;
  elbow?: Offset;
  end?: Offset;
}

export interface TagRule extends RuleSummary {
  samples: Sample[];
}
