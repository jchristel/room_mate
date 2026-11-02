//! The settings API's wire shapes: what `/api/settings/projects` lists, what a
//! project read returns, and what a save answers with.
//!
//! **Moved out of `roommate::settings_api` for the reason the settings types
//! were**, and the module name mirrors the server's so `roommate::settings_api`
//! re-exports all three without a caller noticing. They derive `Deserialize` as
//! well, which the server never needs: reading them back is the business of
//! whatever consumes this crate.
//!
//! What did NOT move is the handlers and the core functions — those read
//! directories and hot-swap a registry, which is the server's business.

use serde::{Deserialize, Serialize};

use crate::settings::Settings;

/// One project-settings file as the UI's list sees it. A file that fails to
/// parse still gets a row (with `error` set) rather than breaking the whole
/// list — the settings UI is exactly the tool you'd reach for to notice a
/// rotten file, so it must stay usable when one exists.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct ProjectFileSummary {
    /// File name within the projects dir (not a full path).
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// The project's display name, absent when the file sets none — consumers
    /// fall back to `project_id`. Carried on the summary so the pyRevit push
    /// picker can both label a project and send `project.name` from one call
    /// (see room_mate's `fetch_projects`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub is_default: bool,
    /// Every reference source this file declares, by name — the list form of
    /// what `drofus_configured: bool` used to answer for one hardcoded source.
    /// A bool could only ever say "is dRofus there", which was already the
    /// wrong question once a project could configure several.
    pub reference_sources: Vec<String>,
    /// The FF&E type parameter whose value keys a tag rule (`Settings::tag_type_code`). On the
    /// summary because the pyRevit push picker reads it from this one call, and a push that
    /// names no parameter cannot key its item tags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_type_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Wire shape of one project's settings: the parsed `Settings` plus which
/// file it lives in.
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct ProjectSettingsResponse {
    pub file: String,
    pub settings: Settings,
    /// Settings that name a property the project's data no longer has (see
    /// `SettingFinding`). Empty is the ordinary state, and also what a project with
    /// nothing stored yet answers: absent data is not a finding.
    #[serde(default)]
    pub findings: Vec<SettingFinding>,
}

/// One setting that names a property nothing in the project's latest data carries.
///
/// **A finding, never an error**: the project still loads and draws, and the name just
/// resolves to nothing -- exactly the "an unresolvable name contributes nothing" rule
/// the settings follow everywhere. What the finding adds is that somebody is told,
/// where they edit it, before a reader finds the colour plan grey or the tag blank.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct SettingFinding {
    /// The settings section: `room_label`, `colour_plans`, `hover` or `tags`.
    pub section: String,
    /// Which setting inside it: `room_label`, `hover.ffe`, `tags.ceilings`,
    /// `colour_plans.<plan name>`.
    pub setting: String,
    /// The property name that was not found.
    pub property: String,
    /// The entity whose data was searched.
    pub entity: String,
    pub message: String,
}

/// Result of one reference CSV upload, echoed to the uploader. Carries the
/// resolved snapshot id (minted when the request supplied none — same
/// contract as rooms ingest) and the parsed CSV's headline facts so the
/// settings UI can refresh its label dropdowns without a second call.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct ReferenceUploadResult {
    pub accepted: bool,
    /// False when a snapshot with this `taken_at` already existed — the upload
    /// was skipped (never overwritten), same duplicate rule as rooms ingest.
    pub stored: bool,
    /// Records actually stored — **the deduplicated count**, which is why the
    /// two fields below matter: a 200-row CSV with five repeated ids reports
    /// 195 here, and without them nothing would say where the other five went.
    pub record_count: usize,
    /// Ids the CSV repeated; only the last row of each survived. Reported at
    /// upload because that is the moment the operator is looking at the file.
    pub duplicate_ids: Vec<String>,
    /// Rows whose id cell was empty, skipped by the loader.
    pub blank_id_rows: usize,
    pub link_property: String,
    pub labels: Vec<String>,
    pub snapshot_taken_at: String,
    pub snapshot_id_generated: bool,
}

/// Save response: the settings as installed, plus the hot-reload confirmation
/// the UI shows ("saved & applied live").
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct SaveResponse {
    pub applied: bool,
    pub settings: Settings,
}
