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

/** A door of the room, for orientation only: a blueprint never creates one. `edge` indexes the
 *  coarse outline; positions are mm along that edge. Absent on a revision captured before doors were. */
export interface BlueprintDoor {
  id: string;
  edge: number;
  along_mm: number;
  edge_len_mm: number;
  width_mm: number;
}

/** A room layout sheet a blueprint carries: layout only, no tags. Its number, name and id are for
 *  recognising it on a re-push and are never used on assignment. Opaque to this page. */
export interface BlueprintSheet {
  revit_sheet_id: string;
  document_title: string;
  sheet_number: string;
  sheet_name?: string;
  sheet: unknown;
}

export interface RevisionContent {
  outline: Outline;
  items: BlueprintItem[];
  doors?: BlueprintDoor[];
  /** Absent on a revision made before sheets were held. */
  sheets?: BlueprintSheet[];
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
  /** Room layout sheets in the current revision. */
  sheets: number;
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

// --- Placing a blueprint in another room (the resolver's answer) and the assignments kept ---

export type FitKind = "plain" | "mirror_image" | "no_fit" | "edge_count_differs" | "manual";

export interface Fit {
  kind: FitKind;
  shift: number;
  cost: number | null;
  mirror_cost: number | null;
}

export interface Candidate {
  shift: number;
  cost: number;
  /** Mean mm from each blueprint door to the nearest door of the room; null when either has none. */
  door_gap_mm: number | null;
}

export interface Placed {
  item_id: string;
  type_name: string;
  category: string;
  class: ItemClass;
  /** Centre in the project's plan frame, mm. */
  centre: Point;
  /** Unit vector of the facing. */
  facing: Point;
  size_mm: [number, number];
  edge: number;
  rule: string;
}

export type Finding =
  | { kind: "outside"; item: string }
  | { kind: "overlap"; a: string; b: string }
  | { kind: "follow_broken"; item: string; reason: string }
  | { kind: "corner_anchored"; item: string }
  | { kind: "mirror_image"; message: string }
  | { kind: "shape_mismatch"; message: string }
  | { kind: "doors_disagree"; message: string };

export interface ResolveResponse {
  blueprint_id: string;
  revision: number;
  room_name: string;
  fit: Fit;
  outline: Outline;
  placements: Placed[];
  findings: Finding[];
  excluded: number;
  candidates: Candidate[];
  /** The room's doors on its coarse outline. */
  doors: BlueprintDoor[];
  /** The blueprint's doors as the mapping lands them on the room. */
  mapped_doors: BlueprintDoor[];
}

/** How the blueprint's edges were put on the room's: neither field means the best fit. */
export interface Mapping {
  shift?: number;
  edge_map?: number[];
}

export interface AssignmentRow {
  id: string;
  blueprint_id: string;
  revision: number;
  room: { model_id: string; room_id: string };
  room_name: string;
  mapping: Mapping;
  note?: string | null;
  taken_at: string;
  blueprint_name: string | null;
  current_revision: number | null;
  newer_revision: boolean;
  blueprint_missing: boolean;
}

// --- Several rooms at once ---

export type Verdict = "clear" | "check" | "cannot";

/** How the blueprint does in one room at the best fit: the resolver's answer boiled down to a row. */
export interface RoomSolve {
  room_id: string;
  model_id?: string;
  room_name: string;
  verdict: Verdict;
  reason: string;
  fit?: FitKind;
  shift?: number;
  cost?: number | null;
  door_gap_mm?: number | null;
  placed: number;
  items: number;
  outside: number;
  overlap: number;
  doors_disagree: boolean;
}

export interface AssignOutcome {
  room_id: string;
  assigned: boolean;
  error?: string;
}
