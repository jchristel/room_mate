//! In-memory `SnapshotStore` — the volatile, latest-only impl kept for
//! `[storage]`-less dev/test configs. See the module doc in `mod.rs` for the
//! trait contract; the one thing that differs here is history: there is none
//! (latest-only, by design), so replacement is the normal upsert.

use std::collections::BTreeMap;
use std::sync::Mutex;

use anyhow::Result;

use super::{SnapshotKind, SnapshotMeta, SnapshotStore, SnapshotWriter};
use crate::state::ModelKey;

/// (project id, source name) -> (taken_at, bytes) — factored out purely to
/// keep `MemStore`'s field declaration under clippy's type-complexity limit.
type DrofusByProjectSource = BTreeMap<(String, String), (String, Vec<u8>)>;

/// (kind, model key) -> (taken_at, bytes), for the same reason.
type LatestByKindAndModel = BTreeMap<(SnapshotKind, ModelKey), (String, Vec<u8>)>;

/// In-memory store: the pre-persistence behaviour, kept for tests and for a
/// `[storage]`-less config. Latest-only per model (no history) — history is a
/// disk affordance, not worth reproducing in the volatile store.
///
/// It holds serialized bytes rather than payloads, because the trait's boundary
/// is bytes. That costs a serde round-trip where a clone used to do, which is
/// irrelevant at this store's scale and buys something worth having: the
/// volatile store now exercises the same serialization path as the persistent
/// one, so a payload that would not survive a round-trip fails in the tests
/// that use `MemStore` instead of only in production.
#[derive(Default)]
pub struct MemStore {
    /// Latest snapshot per (kind, model): `(taken_at, bytes)`. Keyed by kind so
    /// a model's rooms and doors are independent slots — pushing one never
    /// disturbs the other.
    latest: Mutex<LatestByKindAndModel>,
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
    /// for an instant. With bytes in `latest` it could not be derived anyway.
    phases: Mutex<BTreeMap<ModelKey, String>>,
    /// The one quarantined push per model, invisible to every read path:
    /// `(taken_at, bytes)`. Not keyed by kind — quarantine is rooms-only, see
    /// `SnapshotStore::put_pending_raw`.
    pending: Mutex<BTreeMap<ModelKey, (String, Vec<u8>)>>,
}

impl MemStore {
    pub fn new() -> Self {
        Self::default()
    }
}

/// A streamed snapshot for the in-memory store: accumulate, then `put_raw` on
/// commit.
///
/// **It buffers, and that is the honest implementation rather than a shortcut.**
/// `put_streaming` exists to keep a large payload off the heap on its way to
/// *disk*; `MemStore`'s destination is the heap, so there is nothing for
/// streaming to save and pretending otherwise would be theatre. What it does buy
/// is that both stores answer the same call, so the ingest routes have one code
/// path and the tests that run against `MemStore` exercise the real one.
///
/// Dropping without committing discards the buffer, matching `FsSnapshotWriter`
/// — which is the behaviour the ingest routes actually depend on.
struct MemSnapshotWriter<'a> {
    store: &'a MemStore,
    buffer: Vec<u8>,
    kind: SnapshotKind,
    key: ModelKey,
    project_name: String,
    model_name: String,
    taken_at: String,
    phase: Option<String>,
}

impl SnapshotWriter for MemSnapshotWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    fn commit(self: Box<Self>) -> Result<()> {
        self.store.put_raw(
            &SnapshotMeta {
                kind: self.kind,
                key: &self.key,
                project_name: &self.project_name,
                model_name: &self.model_name,
                taken_at: &self.taken_at,
                phase: self.phase.as_deref(),
            },
            &self.buffer,
        )
    }
}

impl SnapshotStore for MemStore {
    fn put_raw(&self, meta: &SnapshotMeta<'_>, json: &[u8]) -> Result<()> {
        // Record the lineage's phase on the first phased push and never
        // overwrite it — same immutability backstop `FsStore::put_raw` applies
        // to the manifest entry. `or_insert` is exactly that rule.
        if let Some(phase) = meta.phase {
            self.phases.lock().unwrap().entry(meta.key.clone()).or_insert(phase.to_string());
        }
        self.latest
            .lock()
            .unwrap()
            .insert((meta.kind, meta.key.clone()), (meta.taken_at.to_string(), json.to_vec()));
        Ok(())
    }

    fn put_streaming<'a>(&'a self, meta: &SnapshotMeta<'_>) -> Result<Box<dyn SnapshotWriter + 'a>> {
        Ok(Box::new(MemSnapshotWriter {
            store: self,
            buffer: Vec::new(),
            kind: meta.kind,
            key: meta.key.clone(),
            project_name: meta.project_name.to_string(),
            model_name: meta.model_name.to_string(),
            taken_at: meta.taken_at.to_string(),
            phase: meta.phase.map(str::to_string),
        }))
    }

    fn get_latest_raw(&self, kind: SnapshotKind, key: &ModelKey) -> Result<Option<Vec<u8>>> {
        Ok(self.latest.lock().unwrap().get(&(kind, key.clone())).map(|(_, bytes)| bytes.clone()))
    }

    fn list_models(&self) -> Result<Vec<ModelKey>> {
        // Every model holding a snapshot of ANY kind, deduped — a model with
        // both rooms and doors has two entries in `latest` and is one model.
        //
        // This used to filter to rooms, on the reasoning that a doors-only model
        // could not exist. It can now (see `SnapshotStore::list_models`), and
        // `FsStore` always counted it, so the filter was the thing that made the
        // two impls disagree rather than the thing that kept them aligned.
        Ok(self
            .latest
            .lock()
            .unwrap()
            .keys()
            .map(|(_, key)| key.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect())
    }

    fn all_latest_raw(&self, kind: SnapshotKind) -> Result<Vec<(ModelKey, Vec<u8>)>> {
        Ok(self
            .latest
            .lock()
            .unwrap()
            .iter()
            .filter(|((k, _), _)| *k == kind)
            .map(|((_, key), (_, bytes))| (key.clone(), bytes.clone()))
            .collect())
    }

    fn list_snapshot_ids(&self, kind: SnapshotKind, key: &ModelKey) -> Result<Vec<String>> {
        // Latest-only store, so "all snapshot ids" is at most the one current
        // id — honest about the fact that MemStore keeps no history.
        Ok(self
            .latest
            .lock()
            .unwrap()
            .get(&(kind, key.clone()))
            .map(|(id, _)| vec![id.clone()])
            .unwrap_or_default())
    }

    fn get_snapshot_raw(&self, kind: SnapshotKind, key: &ModelKey, taken_at: &str) -> Result<Option<Vec<u8>>> {
        // Latest-only store: an id can only be answered when it IS the
        // current latest; anything older is genuinely gone.
        Ok(self
            .latest
            .lock()
            .unwrap()
            .get(&(kind, key.clone()))
            .filter(|(id, _)| id == taken_at)
            .map(|(_, bytes)| bytes.clone()))
    }

    fn get_phase(&self, key: &ModelKey) -> Result<Option<String>> {
        Ok(self.phases.lock().unwrap().get(key).cloned())
    }

    fn put_pending_raw(&self, key: &ModelKey, taken_at: &str, _phase: Option<&str>, json: &[u8]) -> Result<()> {
        // One slot per model: replacement is the rule here, same as on disk.
        // The phase is not stored — it rides the bytes, and `promote_pending`
        // is told which phase to apply by the caller that parsed them.
        self.pending.lock().unwrap().insert(key.clone(), (taken_at.to_string(), json.to_vec()));
        Ok(())
    }

    fn get_pending_raw(&self, key: &ModelKey) -> Result<Option<Vec<u8>>> {
        Ok(self.pending.lock().unwrap().get(key).map(|(_, bytes)| bytes.clone()))
    }

    fn promote_pending(&self, meta: &SnapshotMeta<'_>) -> Result<bool> {
        let key = meta.key;
        let Some((taken_at, json)) = self.pending.lock().unwrap().remove(key) else {
            return Ok(false);
        };
        // Re-phase explicitly: `put_raw` only fills an *absent* phase, so the
        // deliberate overwrite has to happen here, exactly as in `FsStore`.
        match meta.phase {
            Some(phase) => self.phases.lock().unwrap().insert(key.clone(), phase.to_string()),
            None => self.phases.lock().unwrap().remove(key),
        };
        self.latest.lock().unwrap().insert((SnapshotKind::Rooms, key.clone()), (taken_at, json));
        Ok(true)
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
    use crate::contract::{Model, Project, RoomPayload, Snapshot, SUPPORTED_SCHEMA};

    /// The typed façade `AppState` puts over this byte-level trait. Duplicated
    /// from `fs`'s test module rather than hoisted (house rule), and for the
    /// same reason it exists there: these tests are about phase immutability and
    /// quarantine, which are stated in payloads, not in `Vec<u8>`.
    trait TypedStore {
        fn put(&self, payload: &RoomPayload) -> Result<()>;
        fn get_latest(&self, key: &ModelKey) -> Result<Option<RoomPayload>>;
        fn put_pending(&self, key: &ModelKey, payload: &RoomPayload) -> Result<()>;
        fn get_pending(&self, key: &ModelKey) -> Result<Option<RoomPayload>>;
        fn promote(&self, key: &ModelKey) -> Result<Option<RoomPayload>>;
    }

    fn meta<'a>(key: &'a ModelKey, payload: &'a RoomPayload) -> SnapshotMeta<'a> {
        SnapshotMeta {
            kind: SnapshotKind::Rooms,
            key,
            project_name: &payload.project.name,
            model_name: &payload.model.name,
            taken_at: &payload.snapshot.taken_at,
            phase: payload.phase.as_deref(),
        }
    }

    impl<S: SnapshotStore + ?Sized> TypedStore for S {
        fn put(&self, payload: &RoomPayload) -> Result<()> {
            let key = ModelKey::from_payload(payload);
            self.put_raw(&meta(&key, payload), &serde_json::to_vec(payload)?)
        }

        fn get_latest(&self, key: &ModelKey) -> Result<Option<RoomPayload>> {
            self.get_latest_raw(SnapshotKind::Rooms, key)?
                .map(|b| Ok(serde_json::from_slice(&b)?))
                .transpose()
        }

        fn put_pending(&self, key: &ModelKey, payload: &RoomPayload) -> Result<()> {
            self.put_pending_raw(
                key,
                &payload.snapshot.taken_at,
                payload.phase.as_deref(),
                &serde_json::to_vec(payload)?,
            )
        }

        fn get_pending(&self, key: &ModelKey) -> Result<Option<RoomPayload>> {
            self.get_pending_raw(key)?.map(|b| Ok(serde_json::from_slice(&b)?)).transpose()
        }

        fn promote(&self, key: &ModelKey) -> Result<Option<RoomPayload>> {
            let Some(payload) = self.get_pending(key)? else {
                return Ok(None);
            };
            Ok(self.promote_pending(&meta(key, &payload))?.then_some(payload))
        }
    }

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
        assert_eq!(
            store.list_snapshot_ids(SnapshotKind::Rooms, &key).unwrap(),
            vec!["2026-01-02T10:00:00Z".to_string()]
        );
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

        let promoted = store.promote(&key).unwrap().expect("something was pending");
        assert_eq!(promoted.snapshot.taken_at, "2026-06-01T10:00:00Z");
        assert_eq!(store.get_phase(&key).unwrap().as_deref(), Some("Demolition"), "promotion re-phases");
        assert!(store.get_pending(&key).unwrap().is_none());
        assert!(store.promote(&key).unwrap().is_none(), "nothing left to promote");
    }

    /// Kind separation is trait behaviour, so the volatile store has to honour
    /// it identically to `FsStore`: a doors push never disturbs the rooms slot,
    /// and `list_models` stays "models with rooms" rather than counting the
    /// model twice.
    #[test]
    fn test_mem_store_keeps_kinds_in_separate_slots() {
        let store = MemStore::new();
        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };

        store.put(&payload("p", "m", "2026-01-01T10:00:00Z")).unwrap();
        let doors = SnapshotMeta {
            kind: SnapshotKind::Doors,
            key: &key,
            project_name: "P",
            model_name: "M",
            taken_at: "2026-06-01T10:00:00Z",
            phase: None,
        };
        store.put_raw(&doors, b"{\"doors\":[]}").unwrap();

        assert_eq!(
            store.get_latest(&key).unwrap().expect("rooms").snapshot.taken_at,
            "2026-01-01T10:00:00Z",
            "a later doors push is not the latest rooms snapshot"
        );
        assert_eq!(
            store.get_latest_raw(SnapshotKind::Doors, &key).unwrap().as_deref(),
            Some(&b"{\"doors\":[]}"[..])
        );
        assert_eq!(store.list_models().unwrap(), vec![key.clone()], "one model, two kinds, listed once");
        assert_eq!(store.all_latest_raw(SnapshotKind::Doors).unwrap().len(), 1);
    }

    /// **A doors-only model is listed**, matching `FsStore`. It became a real
    /// state when the rooms-first ingest gate was removed, and a store that
    /// hid it here would answer `all_latest_raw` differently from the one on
    /// disk — a divergence only visible in production.
    #[test]
    fn test_mem_store_lists_a_model_that_has_only_doors() {
        let store = MemStore::new();
        let key = ModelKey { project_id: "p".into(), model_id: "doors-only".into() };
        store
            .put_raw(
                &SnapshotMeta {
                    kind: SnapshotKind::Doors,
                    key: &key,
                    project_name: "P",
                    model_name: "M",
                    taken_at: "2026-01-01T10:00:00Z",
                    phase: Some("New Construction"),
                },
                b"{\"doors\":[]}",
            )
            .unwrap();

        assert_eq!(store.list_models().unwrap(), vec![key.clone()]);
        assert!(
            store.get_latest_raw(SnapshotKind::Rooms, &key).unwrap().is_none(),
            "and it still has no rooms"
        );
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
