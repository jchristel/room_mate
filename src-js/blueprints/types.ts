// The wire shapes this page reads and writes, hand-written.
//
// **Not generated**, for the reason the reports page's types are not: these are the
// server's blueprint documents (`src/blueprints.rs`, `src/service/blueprint_model.rs`),
// which no ts-rs export covers. A field renamed in Rust is therefore NOT a type error
// here; it shows up as an empty inspector. A change to either file means re-reading this one.

export interface Point {
  x: number;
  y: number;
}

/** The room's outline at two tolerances. Placements index the COARSE outline's edges. */
export interface Outline {
  coarse: Point[];
  fine: Point[];
}

export type AlongMode = "fixed_from_start" | "fixed_from_end" | "percent";
export type OffMode = "fixed" | "percent";
export type ItemClass = "free" | "wall_device";

export interface Placement {
  edge: number;
  along_mm: number;
  along_mode: AlongMode;
  off_mm: number;
  off_mode: OffMode;
  edge_len_mm: number;
  depth_mm: number;
  facing_deg: number;
  corner_anchored?: boolean;
}

export interface Follow {
  item: string;
  gap_mm: number;
  inside: boolean;
}

export interface Property {
  name: string;
  value: string;
}

export interface BlueprintItem {
  id: string;
  family_name?: string | null;
  type_name: string;
  category: string;
  class: ItemClass;
  excluded: boolean;
  placement: Placement;
  /** Footprint in plan, mm: along the facing direction, then across it. */
  size_mm: [number, number];
  follows?: Follow | null;
  properties: Property[];
  type_properties: Property[];
}

export interface RevisionContent {
  outline: Outline;
  items: BlueprintItem[];
}

export interface Revision extends RevisionContent {
  number: number;
  created_at: string;
  note?: string | null;
}

export interface PendingRevision extends RevisionContent {
  pending_id: number;
  received_at: string;
  origin?: string | null;
  note?: string | null;
}

export interface RejectedRevision extends PendingRevision {
  rejected_at: string;
}

export interface SourceRoom {
  model_id: string;
  room_id: string;
  room_name: string;
}

export interface Blueprint {
  id: string;
  name: string;
  /** This version of the document: the `base` the next write must name. */
  taken_at: string;
  source?: SourceRoom | null;
  current_revision: number;
  revisions: Revision[];
  pending?: PendingRevision[];
  rejected?: RejectedRevision[];
}

export interface BlueprintSummary {
  id: string;
  name: string;
  taken_at: string;
  source: SourceRoom | null;
  current_revision: number;
  revisions: number;
  pending: number;
  items: number;
}

export interface CaptureResponse extends RevisionContent {
  room_name: string;
  /** The model the server resolved the room in. */
  model_id: string;
  skipped: { nested: number; no_footprint: number };
}

export interface ProjectRow {
  id: string;
  name: string;
}

export interface RoomRow {
  id: string;
  name: string;
  level_id: string;
}
