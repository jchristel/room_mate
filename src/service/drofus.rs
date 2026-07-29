//! Reference-source upload read side: which uploaded CSV snapshots exist for
//! one (project, source) pair, and a parsed summary of one of them. Named
//! `drofus` still (like `crate::drofus`) because "drofus" is the only source
//! reachable over HTTP so far — see `Phase 4` in
//! `docs/HANDOVER-reference-sources.md` — though every function here already
//! takes `source` and works for any configured name.
//!
//! Same shape as `snapshots`: history comes from the store's per-source index
//! (`list_drofus_snapshot_ids`), an unregistered project is skipped on read
//! (soft empty, same policy as `list_project_snapshots`). The summary
//! deliberately reports headline facts — record count, link property, label
//! set — not the raw rows: its consumers (the settings UI's label dropdowns,
//! an MCP client asking "what shape is this data") want the shape, and the
//! full records are already joined onto `/rooms` where they belong.

use serde::Serialize;

use super::ServiceError;
use crate::drofus::load_reference_from_bytes;
use crate::state::AppState;

/// Every uploaded snapshot for one (project, source) pair, ascending —
/// `latest` duplicates the last element, same convenience as `ModelSnapshots`.
#[derive(Serialize)]
pub struct DrofusSnapshotList {
    pub project_id: String,
    pub snapshots: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
}

/// Parsed summary of one stored reference-source CSV.
#[derive(Serialize)]
pub struct DrofusSnapshotInfo {
    pub taken_at: String,
    pub record_count: usize,
    pub link_property: String,
    pub labels: Vec<String>,
}

/// Every uploaded snapshot id for one project's named source. Unknown/
/// unregistered projects (and a source with no uploads) answer an empty
/// list, not an error — soft-success, same as the other listings.
pub fn list_drofus_snapshots(
    state: &AppState,
    project_id: &str,
    source: &str,
) -> Result<DrofusSnapshotList, ServiceError> {
    if state.settings().settings_for(project_id).is_none() {
        return Ok(DrofusSnapshotList { project_id: project_id.to_string(), snapshots: vec![], latest: None });
    }
    let snapshots = state.list_drofus_snapshot_ids(project_id, source).map_err(ServiceError::Internal)?;
    let latest = snapshots.last().cloned();
    Ok(DrofusSnapshotList { project_id: project_id.to_string(), snapshots, latest })
}

/// A parsed summary of one stored CSV for one project's named source — the
/// given `taken_at`, or the latest when `None`. Answers `None` when the
/// project is unregistered, that source has no uploads, or the id names
/// nothing — the caller's "no such resource" signal (the HTTP adapter turns
/// it into 404, since this names one specific resource; same convention as
/// `latest_snapshot`).
pub fn get_drofus_snapshot(
    state: &AppState,
    project_id: &str,
    source: &str,
    taken_at: Option<&str>,
) -> Result<Option<DrofusSnapshotInfo>, ServiceError> {
    if state.settings().settings_for(project_id).is_none() {
        return Ok(None);
    }
    let resolved = match taken_at {
        Some(id) => state
            .get_drofus(project_id, source, id)
            .map_err(ServiceError::Internal)?
            .map(|bytes| (id.to_string(), bytes)),
        None => state.get_latest_drofus(project_id, source).map_err(ServiceError::Internal)?,
    };
    let Some((taken_at, bytes)) = resolved else {
        return Ok(None);
    };
    // A stored CSV was validated before storing, so a parse failure here is
    // genuinely internal (a hand-edited store), not caller fault.
    let data = load_reference_from_bytes(&bytes).map_err(ServiceError::Internal)?;
    Ok(Some(DrofusSnapshotInfo {
        taken_at,
        record_count: data.by_id.len(),
        link_property: data.link_property,
        labels: data.all_labels,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ProjectSettings;
    use crate::storage::MemStore;

    fn make_state() -> AppState {
        let bundle = ProjectSettings {
            reference: std::collections::BTreeMap::new(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec!["$name".to_string()],
            milestones: vec![],
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            hierarchy_exclusions: vec![],        };
        let registry = std::collections::HashMap::from([("p1".to_string(), bundle)]);
        AppState::new(Box::new(MemStore::new()), registry, None)
    }

    const CSV: &[u8] = b"DrofusRoomId,NetArea\nNumber,Area\n1,25.5\n2,30.0\n";

    /// Listing: soft-empty for unknown projects and for a registered project
    /// with no uploads; ids + latest once one exists.
    #[test]
    fn test_list_drofus_snapshots() {
        let state = make_state();

        let empty = list_drofus_snapshots(&state, "p1", "drofus").unwrap();
        assert!(empty.snapshots.is_empty());
        assert!(empty.latest.is_none());
        assert!(list_drofus_snapshots(&state, "ghost", "drofus").unwrap().snapshots.is_empty());

        state.put_drofus("p1", "drofus", "2026-01-01T10:00:00Z", CSV).unwrap();
        let listed = list_drofus_snapshots(&state, "p1", "drofus").unwrap();
        assert_eq!(listed.snapshots, vec!["2026-01-01T10:00:00Z".to_string()]);
        assert_eq!(listed.latest.as_deref(), Some("2026-01-01T10:00:00Z"));
    }

    /// Summary: latest resolution when no id is given, `None` for a missing
    /// id or an unregistered project.
    #[test]
    fn test_get_drofus_snapshot() {
        let state = make_state();
        assert!(get_drofus_snapshot(&state, "p1", "drofus", None).unwrap().is_none());

        state.put_drofus("p1", "drofus", "2026-01-01T10:00:00Z", CSV).unwrap();

        let info = get_drofus_snapshot(&state, "p1", "drofus", None).unwrap().unwrap();
        assert_eq!(info.taken_at, "2026-01-01T10:00:00Z");
        assert_eq!(info.record_count, 2);
        assert_eq!(info.link_property, "Number");
        assert_eq!(info.labels, vec!["NetArea".to_string()]);

        let by_id = get_drofus_snapshot(&state, "p1", "drofus", Some("2026-01-01T10:00:00Z")).unwrap().unwrap();
        assert_eq!(by_id.record_count, 2);

        assert!(get_drofus_snapshot(&state, "p1", "drofus", Some("2026-02-01T10:00:00Z")).unwrap().is_none());
        assert!(get_drofus_snapshot(&state, "ghost", "drofus", None).unwrap().is_none());
    }
}
