//! Snapshot history listings: which dated snapshots exist for a project's
//! models, and the latest id for one model — the id a follow-up upload (FFE
//! etc.) attaches its data to, alongside a room id.
//!
//! Same shape as `projects`: model identity/names come from `all_snapshots()`
//! (the payload envelope already carries them, no dedicated storage query),
//! history per model comes from the store's snapshot index
//! (`list_snapshot_ids`), and an unregistered project is skipped on read —
//! the same policy as `list_projects`, since its snapshots can never be
//! served anyway.

use serde::Serialize;

use super::ServiceError;
use crate::state::{AppState, ModelKey};

/// One model's snapshot history, ascending — `latest` duplicates the last
/// element so a caller after just "what do I attach to" never re-derives it.
#[derive(Serialize)]
pub struct ModelSnapshots {
    pub id: String,
    pub name: String,
    pub snapshots: Vec<String>,
    pub latest: String,
}

#[derive(Serialize)]
pub struct ProjectSnapshotsResponse {
    pub models: Vec<ModelSnapshots>,
}

/// The latest snapshot id for one model — `GET .../snapshots/latest`.
#[derive(Serialize)]
pub struct LatestSnapshot {
    pub taken_at: String,
    /// The phase this model's lineage is fixed to, or `null` for a model
    /// nothing phased has been pushed to.
    ///
    /// Here because this is already "the what-do-I-attach-this-follow-up-to
    /// call", and a follow-up upload needs *both* answers: the snapshot id, and
    /// the phase it must declare to be accepted rather than quarantined. One
    /// call, both halves — a sibling route would just make the client make two.
    ///
    /// The **lineage's** phase, read from the manifest index, not the latest
    /// snapshot's own. That is the distinction that matters for this caller:
    /// the question is "what will my next push be checked against", and the
    /// answer is the lineage. It also keeps this read cheap — answering from
    /// the index is why `snapshots/latest` never opens a snapshot file.
    pub phase: Option<String>,
}

/// Every stored snapshot id for one project, grouped per model. A project
/// with nothing stored (or unknown, or unregistered) answers an empty list,
/// not an error — same soft-success discipline as the other listings.
pub fn list_project_snapshots(state: &AppState, project_id: &str) -> Result<ProjectSnapshotsResponse, ServiceError> {
    let registry = state.settings();
    if registry.settings_for(project_id).is_none() {
        return Ok(ProjectSnapshotsResponse { models: vec![] });
    }

    let stored = state.all_snapshots().map_err(ServiceError::Internal)?;
    let mut models = Vec::new();
    for (key, payload) in &stored {
        if payload.project.id != project_id {
            continue;
        }
        let snapshots = state.list_snapshot_ids(key).map_err(ServiceError::Internal)?;
        // A model appears in `all_snapshots` only via a readable snapshot, so
        // an empty id list can't really happen — skip defensively if it does.
        let Some(latest) = snapshots.last().cloned() else {
            continue;
        };
        models.push(ModelSnapshots { id: key.model_id.clone(), name: payload.model.name.clone(), snapshots, latest });
    }
    models.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    Ok(ProjectSnapshotsResponse { models })
}

/// The latest snapshot id for one model, or `None` when the model is unknown,
/// has no snapshots, or its project is unregistered (skip-on-read, as above).
/// `None` is the caller's "no latest exists" signal — the HTTP adapter turns
/// it into 404, unlike the listing's soft empty success, because this call
/// names one specific resource.
pub fn latest_snapshot(
    state: &AppState,
    project_id: &str,
    model_id: &str,
) -> Result<Option<LatestSnapshot>, ServiceError> {
    if state.settings().settings_for(project_id).is_none() {
        return Ok(None);
    }
    let key = ModelKey { project_id: project_id.to_string(), model_id: model_id.to_string() };
    let ids = state.list_snapshot_ids(&key).map_err(ServiceError::Internal)?;
    let Some(taken_at) = ids.last().cloned() else {
        return Ok(None);
    };
    let phase = state.model_phase(&key).map_err(ServiceError::Internal)?;
    Ok(Some(LatestSnapshot { taken_at, phase }))
}

/// The quarantined push waiting on one model: what it is, and what activating
/// it would change.
///
/// A model's phase is fixed by its first phased push, so the *only* way to move
/// it is to push the new phase (which quarantines) and then activate that push.
/// This type is what makes that second step possible for someone who wasn't at
/// the keyboard for the first: the `202` at push time is the only other place
/// the quarantine is ever announced, and it is gone as soon as the producer
/// closes.
#[derive(Debug, Serialize)]
pub struct PendingSnapshot {
    /// The quarantined push's snapshot id.
    pub taken_at: String,
    /// The phase it declares — what the model would move to if activated.
    pub phase: Option<String>,
    /// The phase the model is fixed to now — what activating replaces. Reported
    /// alongside `phase` so a caller can show both sides of the decision
    /// without a second request.
    pub current_phase: Option<String>,
    /// Rooms it carries, so a caller can sanity-check the size of the swap
    /// before making it.
    pub room_count: usize,
}

/// Build the report from a pending payload plus the phase it would replace.
fn describe_pending(payload: &crate::contract::RoomPayload, current_phase: Option<String>) -> PendingSnapshot {
    PendingSnapshot {
        taken_at: payload.snapshot.taken_at.clone(),
        phase: payload.phase.clone(),
        current_phase,
        room_count: payload.rooms.len(),
    }
}

/// The quarantined push waiting on one model, or `None` when there is none (or
/// the project is unregistered — skip-on-read, as everywhere here). `None`
/// becomes a 404: this names one specific resource, like `latest_snapshot`.
pub fn pending_snapshot(
    state: &AppState,
    project_id: &str,
    model_id: &str,
) -> Result<Option<PendingSnapshot>, ServiceError> {
    if state.settings().settings_for(project_id).is_none() {
        return Ok(None);
    }
    let key = ModelKey { project_id: project_id.to_string(), model_id: model_id.to_string() };
    let Some(payload) = state.pending_snapshot(&key).map_err(ServiceError::Internal)? else {
        return Ok(None);
    };
    let current = state.model_phase(&key).map_err(ServiceError::Internal)?;
    Ok(Some(describe_pending(&payload, current)))
}

/// Make the quarantined push live and re-phase the model to its phase. Returns
/// what was activated (with `current_phase` naming the phase it *replaced*), or
/// `None` when nothing was pending — a 404 rather than a 500, since asking to
/// activate a quarantine that isn't there is a caller mistake, not a fault.
///
/// This is the only mutation in this module, and the only route in the server
/// that can change a model's phase. It exists because the alternative — making
/// a disagreeing push a hard error — would leave a model pushed under the wrong
/// phase permanently wrong, there being no delete route.
pub fn activate_pending_snapshot(
    state: &AppState,
    project_id: &str,
    model_id: &str,
) -> Result<Option<PendingSnapshot>, ServiceError> {
    if state.settings().settings_for(project_id).is_none() {
        return Ok(None);
    }
    let key = ModelKey { project_id: project_id.to_string(), model_id: model_id.to_string() };
    // Read the outgoing phase *before* promoting, so the result can report what
    // was replaced rather than echoing the new value back as if nothing moved.
    let replaced = state.model_phase(&key).map_err(ServiceError::Internal)?;
    let Some(payload) = state.promote_pending_snapshot(&key).map_err(ServiceError::Internal)? else {
        return Ok(None);
    };
    tracing::info!(
        "activated pending snapshot for {}/{}: phase {:?} replaces {:?}",
        project_id,
        model_id,
        payload.phase,
        replaced
    );
    Ok(Some(describe_pending(&payload, replaced)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{Model, Project, RoomPayload, Snapshot, SUPPORTED_SCHEMA};
    use crate::state::ProjectSettings;
    use crate::storage::MemStore;

    fn make_payload(project_id: &str, model_id: &str, model_name: &str, ts: &str) -> RoomPayload {
        RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: project_id.to_string(), name: "P".to_string() },
            model: Model {
                id: model_id.to_string(),
                name: model_name.to_string(),
                source: "revit".to_string(),
            },
            snapshot: Snapshot { taken_at: ts.to_string() },
            phase: None,
            model_to_shared: None,
            room_boundary: None,
            levels: vec![],
            rooms: vec![],
        }
    }

    fn make_bundle() -> ProjectSettings {
        ProjectSettings {
            reference: std::collections::BTreeMap::new(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec!["$name".to_string()],
            milestones: vec![],
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            hierarchy_exclusions: vec![],
        }
    }

    fn make_state() -> AppState {
        let registry = std::collections::HashMap::from([("p1".to_string(), make_bundle())]);
        AppState::new(Box::new(MemStore::new()), registry, None)
    }

    fn phased(project_id: &str, model_id: &str, ts: &str, phase: &str) -> RoomPayload {
        RoomPayload { phase: Some(phase.to_string()), ..make_payload(project_id, model_id, "Arch", ts) }
    }

    /// An `FsStore`-backed state: the re-phase flow moves a file on disk, and
    /// `MemStore` can't prove that happened. Same "history matters -> FsStore"
    /// rule the storage tests follow.
    fn fs_state(dir: &std::path::Path) -> AppState {
        let registry = std::collections::HashMap::from([("p1".to_string(), make_bundle())]);
        AppState::new(Box::new(crate::storage::FsStore::new(dir.to_path_buf()).unwrap()), registry, None)
    }

    /// `snapshots/latest` answers both halves of what a follow-up push needs:
    /// the id to attach to, and the phase it must declare to be accepted rather
    /// than quarantined. The *lineage's* phase, since that is what the next push
    /// will be checked against.
    #[test]
    fn test_latest_snapshot_carries_the_lineage_phase() {
        let state = make_state();
        assert!(latest_snapshot(&state, "p1", "m1").unwrap().is_none(), "nothing pushed yet");

        state.set_snapshot(make_payload("p1", "m1", "Arch", "2026-01-01T00:00:00Z")).unwrap();
        let latest = latest_snapshot(&state, "p1", "m1").unwrap().expect("a snapshot exists");
        assert_eq!(latest.phase, None, "an unphased lineage reports null");

        state
            .set_snapshot(phased("p1", "m2", "2026-01-01T00:00:00Z", "New Construction"))
            .unwrap();
        let latest = latest_snapshot(&state, "p1", "m2").unwrap().expect("a snapshot exists");
        assert_eq!(latest.taken_at, "2026-01-01T00:00:00Z");
        assert_eq!(latest.phase.as_deref(), Some("New Construction"));
    }

    /// The read half: a quarantined push reports both sides of the decision --
    /// what the model would become and what it is now -- so a caller can show
    /// the choice without a second request.
    #[test]
    fn test_pending_snapshot_reports_both_phases() {
        let dir = std::env::temp_dir().join(format!("roommate-svc-pending-{}", std::process::id()));
        let state = fs_state(&dir);
        let key = ModelKey { project_id: "p1".into(), model_id: "m1".into() };

        state
            .set_snapshot(phased("p1", "m1", "2026-01-01T00:00:00Z", "New Construction"))
            .unwrap();
        assert!(pending_snapshot(&state, "p1", "m1").unwrap().is_none(), "nothing pending yet");

        state
            .set_pending_snapshot(&key, &phased("p1", "m1", "2026-06-01T00:00:00Z", "Existing"))
            .unwrap();

        let pending = pending_snapshot(&state, "p1", "m1").unwrap().expect("a push is waiting");
        assert_eq!(pending.taken_at, "2026-06-01T00:00:00Z");
        assert_eq!(pending.phase.as_deref(), Some("Existing"), "what it would become");
        assert_eq!(pending.current_phase.as_deref(), Some("New Construction"), "what it is now");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Activation makes the push live and re-phases the model. The result names
    /// the phase it *replaced*, not the new one echoed twice -- the interesting
    /// fact is what changed.
    #[test]
    fn test_activate_pending_snapshot_rephases_the_model() {
        let dir = std::env::temp_dir().join(format!("roommate-svc-activate-{}", std::process::id()));
        let state = fs_state(&dir);
        let key = ModelKey { project_id: "p1".into(), model_id: "m1".into() };

        state
            .set_snapshot(phased("p1", "m1", "2026-01-01T00:00:00Z", "New Construction"))
            .unwrap();
        state
            .set_pending_snapshot(&key, &phased("p1", "m1", "2026-06-01T00:00:00Z", "Existing"))
            .unwrap();

        let activated = activate_pending_snapshot(&state, "p1", "m1").unwrap().expect("something was pending");
        assert_eq!(activated.taken_at, "2026-06-01T00:00:00Z");
        assert_eq!(activated.phase.as_deref(), Some("Existing"));
        assert_eq!(activated.current_phase.as_deref(), Some("New Construction"), "names what it replaced");

        assert_eq!(state.model_phase(&key).unwrap().as_deref(), Some("Existing"), "the model moved");
        assert_eq!(latest_snapshot(&state, "p1", "m1").unwrap().unwrap().taken_at, "2026-06-01T00:00:00Z");
        assert!(pending_snapshot(&state, "p1", "m1").unwrap().is_none(), "the quarantine is cleared");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Activating nothing is a `None` -- the handler's 404 -- not an error.
    /// Also covers the unregistered-project case, which is skip-on-read here
    /// exactly as it is for every other call in this module.
    #[test]
    fn test_activate_with_nothing_pending_is_none() {
        let dir = std::env::temp_dir().join(format!("roommate-svc-activate-none-{}", std::process::id()));
        let state = fs_state(&dir);

        state
            .set_snapshot(phased("p1", "m1", "2026-01-01T00:00:00Z", "New Construction"))
            .unwrap();
        assert!(activate_pending_snapshot(&state, "p1", "m1").unwrap().is_none());
        assert!(activate_pending_snapshot(&state, "unregistered", "m1").unwrap().is_none());
        assert!(pending_snapshot(&state, "unregistered", "m1").unwrap().is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Two models under one project each get their own group, sorted by
    /// name, with `latest` mirroring the last (only, for MemStore) id.
    #[test]
    fn test_list_project_snapshots_groups_per_model() {
        let state = make_state();
        state.set_snapshot(make_payload("p1", "m2", "Struct", "2026-01-02T00:00:00Z")).unwrap();
        state.set_snapshot(make_payload("p1", "m1", "Arch", "2026-01-01T00:00:00Z")).unwrap();

        let result = list_project_snapshots(&state, "p1").unwrap();

        assert_eq!(result.models.len(), 2);
        assert_eq!(result.models[0].name, "Arch");
        assert_eq!(result.models[0].latest, "2026-01-01T00:00:00Z");
        assert_eq!(result.models[0].snapshots, vec!["2026-01-01T00:00:00Z".to_string()]);
        assert_eq!(result.models[1].name, "Struct");
    }

    /// An unknown or unregistered project answers an empty list, not an
    /// error — and never leaks another project's models.
    #[test]
    fn test_list_project_snapshots_unknown_and_unregistered_are_empty() {
        let state = make_state();
        state.set_snapshot(make_payload("p1", "m1", "Arch", "2026-01-01T00:00:00Z")).unwrap();
        state.set_snapshot(make_payload("ghost", "mg", "Ghost", "2026-01-01T00:00:00Z")).unwrap();

        assert!(list_project_snapshots(&state, "nonexistent").unwrap().models.is_empty());
        assert!(list_project_snapshots(&state, "ghost").unwrap().models.is_empty());
    }

    /// `latest_snapshot` answers the one id for a known model and `None` for
    /// an unknown model or an unregistered project.
    #[test]
    fn test_latest_snapshot() {
        let state = make_state();
        state.set_snapshot(make_payload("p1", "m1", "Arch", "2026-01-01T00:00:00Z")).unwrap();
        state.set_snapshot(make_payload("ghost", "mg", "Ghost", "2026-01-01T00:00:00Z")).unwrap();

        let latest = latest_snapshot(&state, "p1", "m1").unwrap().unwrap();
        assert_eq!(latest.taken_at, "2026-01-01T00:00:00Z");

        assert!(latest_snapshot(&state, "p1", "unknown-model").unwrap().is_none());
        assert!(latest_snapshot(&state, "ghost", "mg").unwrap().is_none());
    }
}
