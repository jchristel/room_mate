//! A saved report: what someone asked for, kept so they need not ask again.
//!
//! **A document, not a setting**, and the test is what removing it does. A
//! setting changes what every read *means* — `room_attribution` re-owns every
//! door, the space key re-matches every space. Delete a saved report and every
//! other answer in the system is identical. So these live as one JSON file per
//! report beside the project settings (`roommate::reports_api`), never as a
//! block inside them, which keeps four costs off the settings file: a malformed
//! report cannot fail the boot-validated settings, a report save does not ride
//! the whole-file read-modify-write that once emptied every milestone's pins,
//! the settings page's exhaustiveness rule gains no exemption, and a filter tree
//! lands in a format that has trees.
//!
//! It lives in this crate for the reason the settings types do: the page reads
//! it, so its TypeScript is generated here rather than hand-copied.

use serde::{Deserialize, Serialize};

/// One saved report.
///
/// **The fields are the request the page would have sent**, plus a name and an
/// id. That is deliberate: a saved report is a request somebody kept, so
/// running one is loading this and posting it, with no second shape in between
/// to drift.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/reports/generated/")]
pub struct SavedReport {
    /// Stable id, and the file name. Path-safe, checked on save.
    pub id: String,
    /// What a human calls it. Renaming changes this and not the id, so a link
    /// to a report survives being renamed.
    pub name: String,

    /// `rooms` | `doors` | `windows` | `ceilings` | `floors` | `spaces` | `ffe`.
    pub entity: String,
    #[serde(default)]
    pub by_room: bool,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub room_columns: Vec<String>,
    #[serde(default)]
    pub measures: Vec<String>,
    /// `per_match` or `per_room`.
    #[serde(default)]
    pub shape: Option<String>,
    #[serde(default)]
    pub include_rooms_without: bool,
    #[serde(default = "default_true")]
    pub include_unattributed: bool,
    #[serde(default)]
    pub building: Option<String>,
    #[serde(default)]
    pub milestone: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,

    /// The filter tree, in the shape `service::reports::FilterWire` reads.
    ///
    /// **Held as opaque JSON here**, which is a decision rather than laziness:
    /// the tree is recursive and its vocabulary — the operators, the sides — is
    /// the server's to define and is documented where it is parsed. Mirroring
    /// it as a second recursive type in two languages would be two places to
    /// forget an operator. The page has its own editing model
    /// (`src-js/reports/filter.ts`) and the server validates on the way in, so
    /// a filter this file cannot describe is still a filter neither end can
    /// misread.
    #[serde(default)]
    #[ts(type = "unknown")]
    pub filter: Option<serde_json::Value>,
}

fn default_true() -> bool {
    true
}

/// One row of the saved-report list: enough to render the rail without reading
/// every document.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/reports/generated/")]
pub struct SavedReportSummary {
    pub id: String,
    pub name: String,
    pub entity: String,
    pub by_room: bool,
}

impl From<&SavedReport> for SavedReportSummary {
    fn from(report: &SavedReport) -> Self {
        Self {
            id: report.id.clone(),
            name: report.name.clone(),
            entity: report.entity.clone(),
            by_room: report.by_room,
        }
    }
}
