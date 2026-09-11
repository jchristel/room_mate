// The settings API's wire shapes, as far as this page touches them.
//
// **Hand-written, and a SUBSET** — the call src-js/renderer/types.ts makes, for
// the same reason: `Settings` is a carefully versioned Rust type, and a complete
// TypeScript copy would be a second definition free to drift. It is also the one
// cost the framework comparison charged React for: a Rust+WASM page could use
// `roommate_shared::settings::Settings` directly. The fields below are the ones
// the slice edits; every other section rides along in the index signature,
// untouched, and goes back as it came.
//
// Generating these from `roommate-shared` would remove the copy. That is an open
// item in STRATEGY-BROWSER.md, not worth doing while the subset is this small.

/** One row of `GET /api/settings/projects`. */
export interface ProjectFileSummary {
  file: string;
  project_id?: string;
  name?: string;
  is_default: boolean;
  reference_sources: string[];
  error?: string;
}

export interface Settings {
  project_id: string;
  /** Absent when the file sets none (the server skips a `None` name). `null` is
   *  what this page sends to CLEAR one — see `saveBody`. */
  name?: string | null;
  is_default: boolean;
  room_label: string[];
  /** Every section this page has no control for. Carried, never read. */
  [otherSection: string]: unknown;
}

export interface ProjectSettingsResponse {
  file: string;
  settings: Settings;
}

export interface SaveResponse {
  applied: boolean;
  settings: Settings;
}
