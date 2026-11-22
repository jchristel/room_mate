//! Uploaded reference-source CSVs (`AppState`'s reference half): bytes in, bytes out, no parsing.

use super::*;

impl AppState {
    /// Store one uploaded reference-source CSV — see `SnapshotStore::put_reference`.
    pub fn put_reference(&self, project_id: &str, source: &str, taken_at: &str, csv: &[u8]) -> anyhow::Result<bool> {
        self.store.put_reference(project_id, source, taken_at, csv)
    }

    /// One project's snapshot ids for one reference source, ascending — see
    /// `SnapshotStore::list_reference_snapshot_ids`.
    pub fn list_reference_snapshot_ids(&self, project_id: &str, source: &str) -> anyhow::Result<Vec<String>> {
        self.store.list_reference_snapshot_ids(project_id, source)
    }

    /// One stored reference-source CSV by snapshot id — see
    /// `SnapshotStore::get_reference`.
    pub fn get_reference(&self, project_id: &str, source: &str, taken_at: &str) -> anyhow::Result<Option<Vec<u8>>> {
        self.store.get_reference(project_id, source, taken_at)
    }

    /// The newest stored CSV for one reference source, with its id — see
    /// `SnapshotStore::get_latest_reference`.
    pub fn get_latest_reference(&self, project_id: &str, source: &str) -> anyhow::Result<Option<(String, Vec<u8>)>> {
        self.store.get_latest_reference(project_id, source)
    }
}
