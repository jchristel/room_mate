//! In-memory `SnapshotStore` — the volatile, latest-only impl kept for
//! `[storage]`-less dev/test configs. See the module doc in `mod.rs` for the
//! trait contract; the one thing that differs here is history: there is none
//! (latest-only, by design), so replacement is the normal upsert.

use std::collections::BTreeMap;
use std::sync::Mutex;

use anyhow::Result;

use super::SnapshotStore;
use crate::contract::RoomPayload;
use crate::state::ModelKey;

/// (project id, source name) -> (taken_at, bytes) — factored out purely to
/// keep `MemStore`'s field declaration under clippy's type-complexity limit.
type DrofusByProjectSource = BTreeMap<(String, String), (String, Vec<u8>)>;

/// In-memory store: the pre-persistence behaviour, kept for tests and for a
/// `[storage]`-less config. Latest-only per model (no history) — history is a
/// disk affordance, not worth reproducing in the volatile store.
#[derive(Default)]
pub struct MemStore {
    latest: Mutex<BTreeMap<ModelKey, RoomPayload>>,
    /// Latest uploaded CSV per (project id, source name): `(taken_at,
    /// bytes)`. Latest-only like `latest` — history is a disk affordance.
    drofus: Mutex<DrofusByProjectSource>,
    /// Each lineage's declared phase — `FsStore` keeps this in `project.toml`,
    /// and this store has no manifest, so it gets its own map.
    ///
    /// Held separately rather than derived from `latest`'s payload, even though
    /// immutability means the two currently agree: the phase is a fact about
    /// the *lineage* and the payload's is a fact about one push, and collapsing
    /// them would quietly break the moment `promote_pending` makes them differ
    /// for an instant.
    phases: Mutex<BTreeMap<ModelKey, String>>,
    /// The one quarantined push per model, invisible to every read path.
    pending: Mutex<BTreeMap<ModelKey, RoomPayload>>,
}

impl MemStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SnapshotStore for MemStore {
    fn put(&self, payload: &RoomPayload) -> Result<()> {
        let key = ModelKey::from_payload(payload);
        // Record the lineage's phase on the first phased push and never
        // overwrite it — same immutability backstop `FsStore.put` applies to
        // the manifest entry. `or_insert` is exactly that rule.
        if let Some(phase) = payload.phase.clone() {
            self.phases.lock().unwrap().entry(key.clone()).or_insert(phase);
        }
        self.latest.lock().unwrap().insert(key, payload.clone());
        Ok(())
    }

    fn get_latest(&self, key: &ModelKey) -> Result<Option<RoomPayload>> {
        Ok(self.latest.lock().unwrap().get(key).cloned())
    }

    fn list_models(&self) -> Result<Vec<ModelKey>> {
        Ok(self.latest.lock().unwrap().keys().cloned().collect())
    }

    fn all_latest(&self) -> Result<Vec<(ModelKey, RoomPayload)>> {
        Ok(self.latest.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect())
    }

    fn list_snapshot_ids(&self, key: &ModelKey) -> Result<Vec<String>> {
        // Latest-only store, so "all snapshot ids" is at most the one current
        // id — honest about the fact that MemStore keeps no history.
        Ok(self
            .latest
            .lock()
            .unwrap()
            .get(key)
            .map(|p| vec![p.snapshot.taken_at.clone()])
            .unwrap_or_default())
    }

    fn get_snapshot(&self, key: &ModelKey, taken_at: &str) -> Result<Option<RoomPayload>> {
        // Latest-only store: an id can only be answered when it IS the
        // current latest; anything older is genuinely gone.
        Ok(self.latest.lock().unwrap().get(key).filter(|p| p.snapshot.taken_at == taken_at).cloned())
    }

    fn get_phase(&self, key: &ModelKey) -> Result<Option<String>> {
        Ok(self.phases.lock().unwrap().get(key).cloned())
    }

    fn put_pending(&self, key: &ModelKey, payload: &RoomPayload) -> Result<()> {
        // One slot per model: replacement is the rule here, same as on disk.
        self.pending.lock().unwrap().insert(key.clone(), payload.clone());
        Ok(())
    }

    fn get_pending(&self, key: &ModelKey) -> Result<Option<RoomPayload>> {
        Ok(self.pending.lock().unwrap().get(key).cloned())
    }

    fn promote_pending(&self, key: &ModelKey) -> Result<Option<RoomPayload>> {
        let Some(payload) = self.pending.lock().unwrap().remove(key) else {
            return Ok(None);
        };
        // Re-phase explicitly: `put` only fills an *absent* phase, so the
        // deliberate overwrite has to happen here, exactly as in `FsStore`.
        if let Some(phase) = payload.phase.clone() {
            self.phases.lock().unwrap().insert(key.clone(), phase);
        } else {
            self.phases.lock().unwrap().remove(key);
        }
        self.latest.lock().unwrap().insert(key.clone(), payload.clone());
        Ok(Some(payload))
    }

    fn put_reference(&self, project_id: &str, source: &str, taken_at: &str, csv: &[u8]) -> Result<bool> {
        // Latest-only: replacement is the normal upsert (same stance as
        // `put`), so the duplicate-skip rule doesn't apply here.
        self.drofus
            .lock()
            .unwrap()
            .insert((project_id.to_string(), source.to_string()), (taken_at.to_string(), csv.to_vec()));
        Ok(true)
    }

    fn list_reference_snapshot_ids(&self, project_id: &str, source: &str) -> Result<Vec<String>> {
        Ok(self
            .drofus
            .lock()
            .unwrap()
            .get(&(project_id.to_string(), source.to_string()))
            .map(|(id, _)| vec![id.clone()])
            .unwrap_or_default())
    }

    fn get_reference(&self, project_id: &str, source: &str, taken_at: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .drofus
            .lock()
            .unwrap()
            .get(&(project_id.to_string(), source.to_string()))
            .filter(|(id, _)| id == taken_at)
            .map(|(_, bytes)| bytes.clone()))
    }

    fn get_latest_reference(&self, project_id: &str, source: &str) -> Result<Option<(String, Vec<u8>)>> {
        Ok(self.drofus.lock().unwrap().get(&(project_id.to_string(), source.to_string())).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{Model, Project, Snapshot, SUPPORTED_SCHEMA};

    fn payload(project: &str, model: &str, ts: &str) -> RoomPayload {
        RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: project.into(), name: "P".into() },
            model: Model { id: model.into(), name: "M".into(), source: "revit".into() },
            snapshot: Snapshot { taken_at: ts.into() },
            phase: None,
            model_to_shared: None,
            room_boundary: None,
            levels: vec![],
            rooms: vec![],
        }
    }

    /// MemStore keeps no history: its snapshot id list is just the current
    /// latest.
    #[test]
    fn test_mem_store_lists_only_latest_snapshot_id() {
        let store = MemStore::new();
        store.put(&payload("p", "m", "2026-01-01T10:00:00Z")).unwrap();
        store.put(&payload("p", "m", "2026-01-02T10:00:00Z")).unwrap();

        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };
        assert_eq!(store.list_snapshot_ids(&key).unwrap(), vec!["2026-01-02T10:00:00Z".to_string()]);
    }

    /// The phase rules are trait behaviour, not an `FsStore` detail, so the
    /// volatile store has to honour them identically: a lineage phases once and
    /// stays, a quarantined push is inert, and promotion is the one way past
    /// that. Asserted here so the two impls can't drift.
    #[test]
    fn test_mem_store_honours_phase_immutability_and_quarantine() {
        let phased = |ts: &str, phase: &str| RoomPayload { phase: Some(phase.into()), ..payload("p", "m", ts) };
        let store = MemStore::new();
        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };

        store.put(&phased("2026-01-01T10:00:00Z", "New Construction")).unwrap();
        store.put(&phased("2026-01-02T10:00:00Z", "Existing")).unwrap();
        assert_eq!(
            store.get_phase(&key).unwrap().as_deref(),
            Some("New Construction"),
            "immutable once set, exactly as on disk"
        );

        store.put_pending(&key, &phased("2026-06-01T10:00:00Z", "Demolition")).unwrap();
        assert_eq!(
            store.get_latest(&key).unwrap().unwrap().snapshot.taken_at,
            "2026-01-02T10:00:00Z",
            "a quarantined push is never the latest"
        );
        assert_eq!(store.get_phase(&key).unwrap().as_deref(), Some("New Construction"));

        let promoted = store.promote_pending(&key).unwrap().expect("something was pending");
        assert_eq!(promoted.snapshot.taken_at, "2026-06-01T10:00:00Z");
        assert_eq!(store.get_phase(&key).unwrap().as_deref(), Some("Demolition"), "promotion re-phases");
        assert!(store.get_pending(&key).unwrap().is_none());
        assert!(store.promote_pending(&key).unwrap().is_none(), "nothing left to promote");
    }

    /// MemStore dRofus: latest-only, replacement is the normal upsert.
    #[test]
    fn test_mem_store_drofus_latest_only() {
        let store = MemStore::new();
        assert!(store.get_latest_reference("p", "drofus").unwrap().is_none());

        store.put_reference("p", "drofus", "2026-01-01T10:00:00Z", b"one").unwrap();
        store.put_reference("p", "drofus", "2026-01-02T10:00:00Z", b"two").unwrap();

        assert_eq!(
            store.list_reference_snapshot_ids("p", "drofus").unwrap(),
            vec!["2026-01-02T10:00:00Z".to_string()]
        );
        let (id, bytes) = store.get_latest_reference("p", "drofus").unwrap().unwrap();
        assert_eq!(id, "2026-01-02T10:00:00Z");
        assert_eq!(bytes, b"two");
        assert!(store.get_reference("p", "drofus", "2026-01-01T10:00:00Z").unwrap().is_none());
        assert_eq!(store.get_reference("p", "drofus", "2026-01-02T10:00:00Z").unwrap().unwrap(), b"two");
    }
}
