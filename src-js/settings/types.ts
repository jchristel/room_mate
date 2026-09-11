// What this page reads off the wire, in one import point.
//
// **The settings tree is GENERATED from the Rust types** — ts-rs, in
// `crates/roommate-shared` — into `./generated/`, committed, written by
// `cargo test` and checked by CI (rust.yml). That is the open item
// STRATEGY-BROWSER.md recorded against the framework decision, and this page is
// what fired its stated signal: it edits nearly every field of `Settings`, so a
// hand-written copy would be a second definition of a versioned contract, kept
// in step by nobody.
//
// Everything re-exported below therefore carries the Rust doc comments with it,
// which is why the editor can show a field's rationale without this file
// repeating it. Components import from here rather than from `./generated/`, so
// the generator's one-file-per-type layout stays an implementation detail.
// Generated imports are extensionless, matching what ts-rs emits.

export type { Settings } from "./generated/Settings";
export type { Sources } from "./generated/Sources";
export type { ReferenceSourceConfig } from "./generated/ReferenceSourceConfig";
export type { ReferenceEntity } from "./generated/ReferenceEntity";
export type { ReferenceFieldConfig } from "./generated/ReferenceFieldConfig";
export type { FieldType } from "./generated/FieldType";
export type { CompareMode } from "./generated/CompareMode";
export type { Milestone } from "./generated/Milestone";
export type { HierarchyTier } from "./generated/HierarchyTier";
export type { HierarchyExclusion } from "./generated/HierarchyExclusion";
export type { BuiltinPropertyDef } from "./generated/BuiltinPropertyDef";
export type { ColourPlan } from "./generated/ColourPlan";
export type { ColourMode } from "./generated/ColourMode";
export type { Colouring } from "./generated/Colouring";
export type { CompareOp } from "./generated/CompareOp";
export type { Band } from "./generated/Band";
export type { AreaPolicy } from "./generated/AreaPolicy";
export type { MeasurementStandard } from "./generated/MeasurementStandard";
export type { RoomBoundary } from "./generated/RoomBoundary";
export type { OpeningPolicy } from "./generated/OpeningPolicy";
export type { RoomAttribution } from "./generated/RoomAttribution";
export type { RoomResolution } from "./generated/RoomResolution";
export type { FfePolicy } from "./generated/FfePolicy";
export type { NestedComponents } from "./generated/NestedComponents";
export type { SpacePolicy } from "./generated/SpacePolicy";
export type { SpaceFieldConfig } from "./generated/SpaceFieldConfig";
export type { ProjectFileSummary } from "./generated/ProjectFileSummary";
export type { ProjectSettingsResponse } from "./generated/ProjectSettingsResponse";
export type { SaveResponse } from "./generated/SaveResponse";
export type { ReferenceUploadResult } from "./generated/ReferenceUploadResult";

// ---------------------------------------------------------------------------
// Hand-written, deliberately: the stored-snapshot reads.
//
// These are `service::` read shapes rather than part of the settings contract —
// what the STORE holds, not what a project file says — and each is three or four
// fields the page uses to fill a dropdown. Generating them would mean annotating
// server types in a crate the browser has no other business with, which is a
// heavier seam than the drift it would prevent. The rule this follows is the
// one in STRATEGY-BROWSER.md: generate when a page needs *most* of a type.
// ---------------------------------------------------------------------------

/** One model's stored snapshots, from `GET /projects/{id}/snapshots`. */
export interface ModelSnapshots {
  id: string;
  name: string;
  /** Ascending, so the last entry is the latest. */
  snapshots: string[];
  latest: string;
}

export interface ProjectSnapshotsResponse {
  models: ModelSnapshots[];
}

/** One reference source's stored uploads, from
 *  `GET /projects/{id}/reference/{source}/snapshots`. */
export interface ReferenceSnapshotList {
  project_id: string;
  /** Ascending; `latest` is absent when the source has no upload yet. */
  snapshots: string[];
  latest?: string;
}

/** The latest stored upload's headline facts, from
 *  `…/reference/{source}/latest`. A 404 means "no upload yet", which is a
 *  normal state rather than an error. */
export interface ReferenceSnapshotInfo {
  taken_at: string;
  record_count: number;
  link_property: string;
  /** Row 1 of the uploaded CSV — the column vocabulary a field declaration
   *  and a `<source>.<label>` property name are picked from. */
  labels: string[];
}
