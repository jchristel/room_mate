// The wire shapes this page reads, hand-written.
//
// **Not generated, unlike the settings page's types**, and the difference is
// worth stating: `roommate-shared` exports the *settings* types through ts-rs,
// while `/projects/{id}/validation` and `/projects/{id}/comparison` are service
// response types that have never been exported. So these are a reader's copy,
// and every field is optional-tolerant where the server may omit it
// (`skip_serializing_if` is used throughout those structs).
//
// The consequence, stated rather than discovered later: a field renamed in Rust
// is NOT a type error here. It shows up as an empty section. Until those
// responses ride ts-rs too, a change to `service::validation` or
// `service::comparison` means re-reading this file.

export interface ProjectRow {
  id: string;
  name: string;
}

/** The whole settings object, held as read and sent back — see `saveBody`. */
export type Settings = Record<string, unknown> & {
  comparison_key?: string | null;
  comparison_properties?: string[];
};

export interface SettingsResponse {
  settings: Settings;
}

export interface Milestone {
  name: string;
  date: string;
}

export interface MilestonesResponse {
  milestones: Milestone[];
}

// ---------- comparison ----------

export interface DuplicateKeyValue {
  value: string;
  room_ids: string[];
}

export interface PropertyDifference {
  property: string;
  baseline_value: string;
  other_value: string;
}

export interface MissingProperty {
  property: string;
  baseline_value: string;
}

export interface ChangedRoom {
  room_id: string;
  key: string;
  differences: PropertyDifference[];
  missing_properties: MissingProperty[];
  /** Absent on an older server, which is why every read of it tolerates undefined. */
  unjoined_sources?: string[];
}

export interface MilestoneComparison {
  milestone: string;
  rooms_added: string[];
  rooms_removed: string[];
  changed_rooms: ChangedRoom[];
  duplicate_key_values: DuplicateKeyValue[];
}

export interface ComparisonResponse {
  comparison_key_configured: boolean;
  comparison_key: string;
  baseline: string;
  compared_properties: string[];
  baseline_duplicate_key_values: DuplicateKeyValue[];
  comparisons: MilestoneComparison[];
}

// ---------- validation ----------

export interface ErrorRoom {
  number?: string;
  name?: string;
  link_value?: string;
}

export interface PropertyMismatch {
  room_id: string;
  field: string;
  room_value: string;
  reference_value: string;
}

export interface FieldFinding {
  room_id: string;
  field: string;
}

export interface SourceValidation {
  link_property?: string;
  rooms_missing_link_value: string[];
  duplicate_link_values: DuplicateKeyValue[];
  rooms_unmatched: string[];
  reference_unmatched?: string[];
  reference_duplicate_ids?: string[];
  reference_blank_id_rows?: number;
  property_mismatches: PropertyMismatch[];
  fields_absent_in_revit: FieldFinding[];
  fields_empty_in_revit: FieldFinding[];
  error_rooms?: Record<string, ErrorRoom>;
}

export interface PhaseReport {
  disagree: boolean;
  by_model?: Record<string, string | null>;
}

export interface UnresolvedRoomReference {
  model_id: string;
  opening_id: string;
  side: string;
  room_id: string;
}

export interface OpeningWithoutRoom {
  model_id: string;
  opening_id: string;
}

export interface PendingRoomReference {
  model_id: string;
  opening_id: string;
}

export interface OpeningReport {
  total: number;
  without_room_reference: OpeningWithoutRoom[];
  unresolved_room: UnresolvedRoomReference[];
  pending_rooms: PendingRoomReference[];
}

export interface SpaceRef {
  model_id: string;
  space_id: string;
  name: string;
  key?: string;
}

export interface RoomKeyRef {
  model_id: string;
  room_id: string;
  name: string;
  key?: string;
}

export interface AmbiguousKey {
  key: string;
  count: number;
}

export interface SpaceModelMatch {
  model_id: string;
  total: number;
  matched: number;
  unmatched: SpaceRef[];
  ambiguous_keys?: AmbiguousKey[];
}

export interface SpaceReport {
  models_not_audited: string[];
  models_without_spaces: string[];
  total_spaces: number;
  comparison_key?: string;
  room_key?: string;
  room_models: string[];
  total_rooms: number;
  by_model: SpaceModelMatch[];
  rooms_without_space: RoomKeyRef[];
  ambiguous_room_keys: AmbiguousKey[];
}

export interface ValidationResponse {
  sources: Record<string, SourceValidation>;
  total_rooms: number;
  phases: PhaseReport;
  openings: Record<string, OpeningReport>;
  spaces?: SpaceReport;
}
