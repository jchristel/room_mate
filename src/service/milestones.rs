//! Milestone listings for the viewer's dropdown: which named dated pins a
//! project defines. Read from the project's *settings bundle* (the registry),
//! not storage — milestones are settings-resident (see
//! `settings::Milestone`), so this list hot-updates the moment a settings
//! save lands, no push required.

use std::collections::BTreeMap;

use serde::Serialize;

use super::ServiceError;
use crate::state::AppState;

/// One milestone as the picker sees it. `attached_models` is a *count* of
/// model pins, not the pin map itself — the dropdown only labels options, and
/// the settings UI (which edits the pins) reads the full map through the
/// settings API instead. `reference_snapshots` is the exception: it surfaces
/// the milestone's reference-source pins verbatim (source name → the pinned
/// `taken_at`), which is what lets a consumer (notably the MCP
/// `list_milestones` tool) see *whether and what* each source pins without a
/// second `get_project_settings` call. A source absent from the map is one the
/// milestone joins at its current data (no pin); an empty map is skipped
/// entirely on the wire.
#[derive(Serialize)]
pub struct MilestoneSummary {
    pub name: String,
    pub date: String,
    pub attached_models: usize,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub reference_snapshots: BTreeMap<String, String>,
}

#[derive(Serialize)]
pub struct MilestonesResponse {
    pub milestones: Vec<MilestoneSummary>,
}

/// Every milestone one project defines, newest date first (the order a
/// picker wants under a "Latest" default). Unknown or unregistered project →
/// empty list, not an error — same soft-success discipline as
/// `list_buildings`.
pub fn list_milestones(state: &AppState, project_id: &str) -> Result<MilestonesResponse, ServiceError> {
    let registry = state.settings();
    let Some(bundle) = registry.settings_for(project_id) else {
        return Ok(MilestonesResponse { milestones: vec![] });
    };

    let mut milestones: Vec<MilestoneSummary> = bundle
        .milestones
        .iter()
        .map(|m| MilestoneSummary {
            name: m.name.clone(),
            date: m.date.clone(),
            attached_models: m.attachments.len(),
            reference_snapshots: m.reference_snapshots.clone(),
        })
        .collect();
    // Both accepted date shapes (`YYYY-MM-DD`, RFC3339) start with the
    // lexically-sortable date part, so string order == chronological order.
    milestones.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.name.cmp(&b.name)));
    Ok(MilestonesResponse { milestones })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Milestone;
    use crate::state::ProjectSettings;
    use crate::storage::MemStore;
    use std::collections::BTreeMap;

    fn make_milestone(name: &str, date: &str) -> Milestone {
        Milestone {
            name: name.to_string(),
            date: date.to_string(),
            reference_snapshots: BTreeMap::new(),
            attachments: BTreeMap::from([("m1".to_string(), "2026-01-01T00:00:00Z".to_string())]),
            door_attachments: BTreeMap::new(),
            window_attachments: Default::default(),
        }
    }

    fn make_bundle(milestones: Vec<Milestone>) -> ProjectSettings {
        ProjectSettings {
            reference: BTreeMap::new(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec!["$name".to_string()],
            milestones,
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            windows: Default::default(),
            hierarchy_exclusions: vec![],
        }
    }

    /// Milestones list newest date first, each carrying its pin count and
    /// every reference-source pin it sets (empty map when it sets none).
    /// Two sources pinned at once is the case the single `drofus_snapshot`
    /// scalar could not express at all.
    #[test]
    fn test_list_milestones_newest_first() {
        let mut pinned = make_milestone("Design Freeze", "2026-06-30");
        pinned
            .reference_snapshots
            .insert("drofus".to_string(), "2026-06-29T17:00:00Z".to_string());
        pinned.reference_snapshots.insert("ffe".to_string(), "2026-06-28T09:00:00Z".to_string());
        let bundle = make_bundle(vec![make_milestone("Concept", "2026-03-01"), pinned]);
        let registry = std::collections::HashMap::from([("p1".to_string(), bundle)]);
        let state = AppState::new(Box::new(MemStore::new()), registry, None);

        let result = list_milestones(&state, "p1").unwrap();

        assert_eq!(result.milestones.len(), 2);
        assert_eq!(result.milestones[0].name, "Design Freeze");
        assert_eq!(result.milestones[1].name, "Concept");
        assert_eq!(result.milestones[0].attached_models, 1);
        assert_eq!(
            result.milestones[0].reference_snapshots,
            BTreeMap::from([
                ("drofus".to_string(), "2026-06-29T17:00:00Z".to_string()),
                ("ffe".to_string(), "2026-06-28T09:00:00Z".to_string()),
            ]),
            "every source's pin, not just one"
        );
        assert!(result.milestones[1].reference_snapshots.is_empty(), "no pins → empty map");
    }

    /// An unknown/unregistered project answers an empty list, not an error.
    #[test]
    fn test_list_milestones_unknown_project_is_empty() {
        let registry = std::collections::HashMap::from([("p1".to_string(), make_bundle(vec![]))]);
        let state = AppState::new(Box::new(MemStore::new()), registry, None);

        assert!(list_milestones(&state, "p1").unwrap().milestones.is_empty());
        assert!(list_milestones(&state, "nonexistent").unwrap().milestones.is_empty());
    }
}
