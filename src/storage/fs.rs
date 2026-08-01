//! Filesystem-backed `SnapshotStore` — the persistent, history-keeping impl.
//! See the module doc in `mod.rs` for the on-disk layout and the
//! manifest-is-index / snapshots-are-record discipline this file implements.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{ProjectManifest, SnapshotKind, SnapshotMeta, SnapshotStore, PENDING_DIR, PENDING_FILE, REFERENCE_DIR};
use crate::state::ModelKey;

/// Filesystem-backed store rooted at a configured directory.
///
/// Stateless beyond the root path: every call recomputes paths and touches disk,
/// so the on-disk tree is the single source of truth (no in-memory cache to keep
/// in sync). Fine at single-user scale; a caching layer is a later optimisation
/// if disk reads on `/rooms` ever bite.
pub struct FsStore {
    root: PathBuf,
}

/// Write `contents` to `path` so a reader never observes a partial file.
///
/// **Why this is not `fs::write`.** Every artifact this store writes is
/// re-read later and parsed strictly: a snapshot as JSON, a CSV through the
/// reference loader, and `project.toml` as TOML at *every* read and every
/// boot. `fs::write` truncates first and then fills, so a process killed
/// mid-write — or a full disk — leaves a syntactically broken file where a
/// valid one used to be.
///
/// For `project.toml` that is the worst case in the store, and the reason this
/// exists. `read_manifest` returns `Err("malformed manifest")` rather than a
/// default, and that error propagates through `list_models` -> `all_latest`
/// into every `/rooms` request and the next startup of both binaries. A
/// project would be bricked while **every snapshot file beside it stayed
/// perfectly intact** — and `list_snapshot_ids` already knows how to rebuild
/// the index from the directory (filesystem wins on disagreement). The
/// recovery path was there; only the write was not safe enough to reach it.
///
/// Write to a sibling temp file, flush and `sync_all` so the bytes are on the
/// device before anything points at them, then `rename` over the target —
/// atomic on POSIX, and on Windows for a same-directory replace. A crash
/// leaves either the old complete file or the new complete file. The temp file
/// is a sibling, not in the system temp dir, so the rename never crosses a
/// filesystem boundary (which would silently degrade to copy-then-delete).
///
/// The temp name carries a process id and a counter rather than being a plain
/// `.tmp` sibling. Two pushes for *different models of one project* both
/// rewrite that project's `project.toml`, and axum serves them concurrently —
/// with a shared temp name they would interleave inside the temp file and one
/// would rename a mixture of both into place. Unique names make the two writes
/// independent; the rename is still last-one-wins, which is the intended
/// upsert semantics, but neither file is ever malformed.
fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    let stem = path.file_name().and_then(|n| n.to_str()).unwrap_or("tmp");
    let tmp =
        path.with_file_name(format!(".{stem}.{}.{}.tmp", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));

    // Scoped so the handle is closed before the rename — Windows refuses to
    // rename a file that still has an open handle.
    let write = (|| -> Result<()> {
        let mut file =
            fs::File::create(&tmp).with_context(|| format!("could not create temp file: {}", tmp.display()))?;
        file.write_all(contents)
            .with_context(|| format!("could not write temp file: {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("could not flush temp file to disk: {}", tmp.display()))
    })();

    // Never leave a partial temp behind on failure: the names are unique, so
    // an abandoned one would accumulate rather than be reused.
    if let Err(e) = write {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(anyhow::Error::from(e).context(format!(
            "could not replace {} with {}",
            path.display(),
            tmp.display()
        )));
    }
    Ok(())
}

impl FsStore {
    /// Bind to a root dir, creating it if absent. Fail fast on an unwritable
    /// root — same startup-loud contract as the rest of config.
    pub fn new(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root).with_context(|| format!("could not create storage root: {}", root.display()))?;
        Ok(Self { root })
    }

    fn project_dir(&self, project_id: &str) -> PathBuf {
        self.root.join(project_id)
    }

    fn manifest_path(&self, project_id: &str) -> PathBuf {
        self.project_dir(project_id).join("project.toml")
    }

    fn model_dir(&self, project_id: &str, model_id: &str) -> PathBuf {
        self.project_dir(project_id).join(model_id)
    }

    /// Where one kind's snapshots live for a model: the model dir itself for
    /// rooms, a subdirectory of it for every other kind. The asymmetry is
    /// deliberate and explained in the module doc; `dir_component` is the only
    /// place it is decided, so both the read and write paths agree by
    /// construction.
    fn kind_dir(&self, kind: SnapshotKind, key: &ModelKey) -> PathBuf {
        let dir = self.model_dir(&key.project_id, &key.model_id);
        match kind.dir_component() {
            Some(sub) => dir.join(sub),
            None => dir,
        }
    }

    fn reference_dir(&self, project_id: &str, source: &str) -> PathBuf {
        self.project_dir(project_id).join(REFERENCE_DIR).join(source)
    }

    /// The one quarantined push for a model: `<model>/pending/snapshot.json`.
    /// Inside a subdirectory so the `.json`-extension model-dir scans can't see
    /// it — see `PENDING_DIR`.
    fn pending_file(&self, key: &ModelKey) -> PathBuf {
        self.model_dir(&key.project_id, &key.model_id).join(PENDING_DIR).join(PENDING_FILE)
    }

    /// Reference-source CSV filename from a snapshot id — same `:`
    /// sanitisation as `snapshot_filename` (so lexical-max-is-newest holds
    /// for `.csv` files exactly as for `.json`), different extension.
    fn drofus_filename(taken_at: &str) -> String {
        format!("{}.csv", taken_at.replace(':', "-"))
    }

    /// Read a project's manifest, or a default (empty) one if it doesn't exist
    /// yet — an absent manifest just means "first push for this project".
    fn read_manifest(&self, project_id: &str) -> Result<ProjectManifest> {
        let path = self.manifest_path(project_id);
        if !path.exists() {
            return Ok(ProjectManifest::default());
        }
        let raw = fs::read_to_string(&path).with_context(|| format!("could not read manifest: {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("malformed manifest: {}", path.display()))
    }

    fn write_manifest(&self, project_id: &str, manifest: &ProjectManifest) -> Result<()> {
        let path = self.manifest_path(project_id);
        let toml = toml::to_string_pretty(manifest).context("could not serialise manifest")?;
        // Atomic: a torn manifest fails every subsequent read AND the next
        // boot, even though the snapshots beside it are fine. See `write_atomic`.
        write_atomic(&path, toml.as_bytes()).with_context(|| format!("could not write manifest: {}", path.display()))
    }

    /// Snapshot filename from the payload's timestamp. The `taken_at` is an
    /// ISO-8601 string; `:` is illegal on some filesystems, so sanitise it to a
    /// safe, still-sortable form before using it as a filename.
    fn snapshot_filename(taken_at: &str) -> String {
        format!("{}.json", taken_at.replace(':', "-"))
    }

    /// The most recent snapshot file in a model dir, by lexical name order.
    /// Timestamp filenames sort chronologically, so lexical-max = newest.
    fn latest_snapshot_file(dir: &Path) -> Result<Option<PathBuf>> {
        if !dir.exists() {
            return Ok(None);
        }
        let mut newest: Option<PathBuf> = None;
        for entry in fs::read_dir(dir).with_context(|| format!("could not read model dir: {}", dir.display()))? {
            let path = entry?.path();
            // Only snapshot files count; skips anything non-`.json` in the dir.
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                // Keep the lexically-largest path. `is_none_or` seeds the first
                // match (None → take it), then compares subsequent paths.
                if newest.as_ref().is_none_or(|n| path > *n) {
                    newest = Some(path);
                }
            }
        }
        Ok(newest)
    }

    /// Best-effort reverse of `snapshot_filename` for a snapshot file the
    /// manifest doesn't index (a pre-`snapshots`-field store, or a manifest
    /// that lost an entry): restore the `:` separators the sanitiser
    /// replaced. Only the positions that are unambiguously time separators in
    /// an RFC3339 id are restored — the two inside the time-of-day and the
    /// one in a `+hh-mm` offset tail; anything unrecognisable stays as the
    /// raw stem. Warning-path fallback only, never the primary index.
    fn id_from_file_stem(stem: &str) -> String {
        let mut bytes = stem.as_bytes().to_vec();
        // "YYYY-MM-DDTHH-MM-SS…": bytes 13 and 16 are sanitised colons.
        if bytes.len() >= 19 && bytes[10] == b'T' && bytes[13] == b'-' && bytes[16] == b'-' {
            bytes[13] = b':';
            bytes[16] = b':';
        }
        // A "+hh-mm" numeric-offset tail (e.g. "+00-00"): its '-' was a ':'.
        if let Some(plus) = bytes.iter().rposition(|&b| b == b'+')
            && plus + 5 == bytes.len() - 1
            && bytes[plus + 3] == b'-'
        {
            bytes[plus + 3] = b':';
        }
        String::from_utf8(bytes).unwrap_or_else(|_| stem.to_string())
    }

    /// Every `.json` filename directly inside `dir`, or an empty list when the
    /// directory doesn't exist. Shared by the two scans that need it so
    /// "a snapshot file is a `.json` file, and subdirectories are not snapshots"
    /// is stated once — that rule is what keeps `doors/`, `pending/` and
    /// `reference/` invisible to the room scans.
    fn snapshot_filenames(dir: &Path) -> Result<Vec<String>> {
        let mut out = Vec::new();
        if !dir.exists() {
            return Ok(out);
        }
        for entry in fs::read_dir(dir).with_context(|| format!("could not read snapshot dir: {}", dir.display()))? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json")
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                out.push(name.to_string());
            }
        }
        Ok(out)
    }

    fn read_bytes(path: &Path) -> Result<Vec<u8>> {
        fs::read(path).with_context(|| format!("could not read snapshot: {}", path.display()))
    }
}

impl SnapshotStore for FsStore {
    fn put_raw(&self, meta: &SnapshotMeta<'_>, json: &[u8]) -> Result<()> {
        // Upsert: one path handles all three cases — unknown project, unknown
        // model under a known project, or a re-push of a known model. `create_dir_all`
        // and the manifest `entry(...).or_default()` are each idempotent, so no
        // branching on "does this exist yet" is needed.
        let SnapshotMeta { kind, key, project_name, model_name, taken_at, phase } = *meta;
        let project_id = &key.project_id;
        let model_id = &key.model_id;

        // 1. Ensure the kind's dir exists. `create_dir_all` also makes the model
        //    and project dirs when either is brand new — the unknown-project and
        //    unknown-model cases, and for a non-rooms kind the subdirectory too.
        let dir = self.kind_dir(kind, key);
        fs::create_dir_all(&dir).with_context(|| format!("could not create snapshot dir: {}", dir.display()))?;

        // 2. Upsert the authoritative manifest: refresh the project display name,
        //    insert this model if absent (`or_default` = the unknown-model case),
        //    update its name, and index this snapshot id in *this kind's* list
        //    (insert-if-absent, kept sorted so ascending == chronological for
        //    RFC3339-UTC ids). Rewritten every push so the manifest always
        //    mirrors what's on disk — which also backfills a pre-`snapshots`-field
        //    manifest one push at a time.
        let mut manifest = self.read_manifest(project_id)?;
        manifest.name = project_name.to_string();
        let entry = manifest.models.entry(model_id.clone()).or_default();
        entry.name = model_name.to_string();
        // Record the lineage's phase on the first phased push, and never
        // overwrite it afterwards. The ingest handler is the real gate — a
        // disagreeing rooms push is quarantined and a disagreeing doors push is
        // refused, both before ever reaching here — but expressing immutability
        // structurally means no future caller can re-phase a model by accident.
        // `promote_pending` is the one deliberate way past it.
        if entry.phase.is_none() {
            entry.phase = phase.map(str::to_string);
        }
        let index = entry.index_mut(kind);
        if !index.iter().any(|id| id == taken_at) {
            index.push(taken_at.to_string());
            index.sort();
        }
        self.write_manifest(project_id, &manifest)?;

        // 3. Write the snapshot under its own timestamped filename — never
        //    overwriting a prior one, so the dir accumulates full history.
        //    A same-`taken_at` re-push is skipped, not overwritten: the client
        //    stamps sub-second precision, so a collision means a genuinely
        //    re-sent payload, and even that must not silently destroy history.
        let file = dir.join(Self::snapshot_filename(taken_at));
        if file.exists() {
            tracing::warn!("snapshot already exists, skipping: {}", file.display());
            return Ok(());
        }
        write_atomic(&file, json).with_context(|| format!("could not write snapshot: {}", file.display()))?;

        tracing::info!("stored {} snapshot {}/{} @ {}", kind.label(), project_id, model_id, taken_at);
        Ok(())
    }

    fn get_latest_raw(&self, kind: SnapshotKind, key: &ModelKey) -> Result<Option<Vec<u8>>> {
        match Self::latest_snapshot_file(&self.kind_dir(kind, key))? {
            Some(path) => Ok(Some(Self::read_bytes(&path)?)),
            None => Ok(None),
        }
    }

    fn list_models(&self) -> Result<Vec<ModelKey>> {
        let mut out = Vec::new();
        if !self.root.exists() {
            return Ok(out);
        }
        for project in fs::read_dir(&self.root)? {
            let project_dir = project?.path();
            if !project_dir.is_dir() {
                continue;
            }
            // Dir name *is* the project GUID — display names live in the
            // manifest, identity lives in the path.
            let project_id = match project_dir.file_name().and_then(|n| n.to_str()) {
                Some(id) => id.to_string(),
                None => continue, // non-UTF-8 dir name: not one of ours, skip
            };

            // The manifest is the index: one key per `models` entry. But the
            // snapshots are the record, so a model dir the manifest doesn't
            // list is a manifest bug, not invisible data — warn (making the
            // drift noticeable) and include it anyway: filesystem truth wins.
            let manifest = self.read_manifest(&project_id)?;
            let mut model_ids: Vec<String> = manifest.models.keys().cloned().collect();
            for model in fs::read_dir(&project_dir)? {
                let model_dir = model?.path();
                if !model_dir.is_dir() {
                    continue; // skips project.toml (a file, not a model dir)
                }
                let model_id = match model_dir.file_name().and_then(|n| n.to_str()) {
                    Some(id) => id.to_string(),
                    None => continue,
                };
                if model_id == REFERENCE_DIR {
                    continue; // reserved reference-source upload dir, never a model
                }
                if !manifest.models.contains_key(&model_id) {
                    tracing::warn!(
                        "model dir {}/{} is missing from project.toml — including it anyway (filesystem wins)",
                        project_id,
                        model_id
                    );
                    model_ids.push(model_id);
                }
            }

            out.extend(
                model_ids
                    .into_iter()
                    .map(|model_id| ModelKey { project_id: project_id.clone(), model_id }),
            );
        }
        Ok(out)
    }

    fn all_latest_raw(&self, kind: SnapshotKind) -> Result<Vec<(ModelKey, Vec<u8>)>> {
        // The manifest-backed index supplies the keys, `get_latest_raw` reads
        // each key's newest snapshot — the manifest is the index, the snapshots
        // the record, exactly as the module doc claims. A manifest entry whose
        // dir holds no snapshots of this kind yet (or was deleted by hand)
        // simply yields nothing for that key.
        let mut out = Vec::new();
        for key in self.list_models()? {
            if let Some(bytes) = self.get_latest_raw(kind, &key)? {
                out.push((key, bytes));
            }
        }
        Ok(out)
    }

    fn list_snapshot_ids(&self, kind: SnapshotKind, key: &ModelKey) -> Result<Vec<String>> {
        // The manifest's per-kind index list; the directory is the record. Same
        // reconciliation stance as `list_models`: on disagreement the filesystem
        // wins — a file the manifest doesn't index is included (with a
        // best-effort id recovered from its name, since the sanitised filename
        // lost its `:`), and a manifest id with no file behind it is dropped.
        // Both are warned about, so drift is noticeable rather than silent.
        let indexed = self
            .read_manifest(&key.project_id)?
            .models
            .get(&key.model_id)
            .map(|m| m.index(kind).clone())
            .unwrap_or_default();

        let mut on_disk = Self::snapshot_filenames(&self.kind_dir(kind, key))?;

        let mut ids = Vec::new();
        for id in indexed {
            let filename = Self::snapshot_filename(&id);
            if let Some(pos) = on_disk.iter().position(|f| *f == filename) {
                on_disk.swap_remove(pos);
                ids.push(id);
            } else {
                tracing::warn!(
                    "manifest lists {} snapshot {:?} for {}/{} but no file exists — dropping it (filesystem wins)",
                    kind.label(),
                    id,
                    key.project_id,
                    key.model_id
                );
            }
        }
        for filename in on_disk {
            let stem = filename.strip_suffix(".json").unwrap_or(&filename);
            let id = Self::id_from_file_stem(stem);
            tracing::warn!(
                "{} snapshot file {}/{}/{} is missing from project.toml — including it as {:?} (filesystem wins)",
                kind.label(),
                key.project_id,
                key.model_id,
                filename,
                id
            );
            ids.push(id);
        }
        ids.sort();
        Ok(ids)
    }

    fn get_snapshot_raw(&self, kind: SnapshotKind, key: &ModelKey, taken_at: &str) -> Result<Option<Vec<u8>>> {
        let path = self.kind_dir(kind, key).join(Self::snapshot_filename(taken_at));
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(Self::read_bytes(&path)?))
    }

    fn get_phase(&self, key: &ModelKey) -> Result<Option<String>> {
        Ok(self
            .read_manifest(&key.project_id)?
            .models
            .get(&key.model_id)
            .and_then(|m| m.phase.clone()))
    }

    fn put_pending_raw(&self, key: &ModelKey, taken_at: &str, phase: Option<&str>, json: &[u8]) -> Result<()> {
        let file = self.pending_file(key);
        let dir = file.parent().expect("pending file always has a parent dir");
        fs::create_dir_all(dir).with_context(|| format!("could not create pending dir: {}", dir.display()))?;
        // Overwrite, unlike `put_raw`: there is exactly one pending slot per
        // model and the newest quarantined push is the only one worth promoting.
        write_atomic(&file, json).with_context(|| format!("could not write pending snapshot: {}", file.display()))?;
        tracing::info!("quarantined push {}/{} @ {} (phase {:?})", key.project_id, key.model_id, taken_at, phase);
        Ok(())
    }

    fn get_pending_raw(&self, key: &ModelKey) -> Result<Option<Vec<u8>>> {
        let file = self.pending_file(key);
        if !file.exists() {
            return Ok(None);
        }
        Ok(Some(Self::read_bytes(&file)?))
    }

    fn promote_pending(&self, meta: &SnapshotMeta<'_>) -> Result<bool> {
        let key = meta.key;
        let Some(json) = self.get_pending_raw(key)? else {
            return Ok(false);
        };

        // Store it as a normal snapshot first, reusing `put_raw` rather than
        // reimplementing the write + index. `put_raw` leaves the existing phase
        // alone (it only fills an absent one), which is why the re-phase below
        // is a separate, explicit step rather than a side effect.
        self.put_raw(meta, &json)?;

        let mut manifest = self.read_manifest(&key.project_id)?;
        if let Some(entry) = manifest.models.get_mut(&key.model_id) {
            entry.phase = meta.phase.map(str::to_string);
        }
        self.write_manifest(&key.project_id, &manifest)?;

        // Clear the quarantine only after the snapshot and manifest are both
        // committed: a failure above leaves the pending push intact and
        // retryable, where removing it first could lose it entirely.
        let file = self.pending_file(key);
        fs::remove_file(&file).with_context(|| format!("could not clear pending snapshot: {}", file.display()))?;

        tracing::info!(
            "promoted pending push {}/{} @ {}; lineage phase is now {:?}",
            key.project_id,
            key.model_id,
            meta.taken_at,
            meta.phase
        );
        Ok(true)
    }

    fn put_reference(&self, project_id: &str, source: &str, taken_at: &str, csv: &[u8]) -> Result<bool> {
        // Same upsert shape as `put`: ensure the dir, index the id in the
        // manifest, then write the file — skipping (never overwriting) a
        // duplicate `taken_at`.
        let dir = self.reference_dir(project_id, source);
        fs::create_dir_all(&dir)
            .with_context(|| format!("could not create reference-source dir: {}", dir.display()))?;

        let mut manifest = self.read_manifest(project_id)?;
        let snapshots = manifest.reference_snapshots.entry(source.to_string()).or_default();
        if !snapshots.iter().any(|id| id == taken_at) {
            snapshots.push(taken_at.to_string());
            snapshots.sort();
        }
        self.write_manifest(project_id, &manifest)?;

        let file = dir.join(Self::drofus_filename(taken_at));
        if file.exists() {
            tracing::warn!("reference-source snapshot already exists, skipping: {}", file.display());
            return Ok(false);
        }
        write_atomic(&file, csv)
            .with_context(|| format!("could not write reference-source snapshot: {}", file.display()))?;

        tracing::info!("stored reference-source snapshot {}/{} @ {}", project_id, source, taken_at);
        Ok(true)
    }

    fn list_reference_snapshot_ids(&self, project_id: &str, source: &str) -> Result<Vec<String>> {
        // Same manifest-vs-directory reconciliation as `list_snapshot_ids`:
        // the manifest is the index, the files are the record, filesystem
        // wins on disagreement, both directions warned.
        let indexed = self
            .read_manifest(project_id)?
            .reference_snapshots
            .get(source)
            .cloned()
            .unwrap_or_default();

        let dir = self.reference_dir(project_id, source);
        let mut on_disk: Vec<String> = Vec::new();
        if dir.exists() {
            for entry in
                fs::read_dir(&dir).with_context(|| format!("could not read reference-source dir: {}", dir.display()))?
            {
                let path = entry?.path();
                if path.extension().and_then(|e| e.to_str()) == Some("csv")
                    && let Some(name) = path.file_name().and_then(|n| n.to_str())
                {
                    on_disk.push(name.to_string());
                }
            }
        }

        let mut ids = Vec::new();
        for id in indexed {
            let filename = Self::drofus_filename(&id);
            if let Some(pos) = on_disk.iter().position(|f| *f == filename) {
                on_disk.swap_remove(pos);
                ids.push(id);
            } else {
                tracing::warn!(
                    "manifest lists reference-source snapshot {:?} for {}/{} but no file exists — dropping it (filesystem wins)",
                    id, project_id, source
                );
            }
        }
        for filename in on_disk {
            let stem = filename.strip_suffix(".csv").unwrap_or(&filename);
            let id = Self::id_from_file_stem(stem);
            tracing::warn!(
                "reference-source file {}/{}/{}/{} is missing from project.toml — including it as {:?} (filesystem wins)",
                project_id, REFERENCE_DIR, source, filename, id
            );
            ids.push(id);
        }
        ids.sort();
        Ok(ids)
    }

    fn get_reference(&self, project_id: &str, source: &str, taken_at: &str) -> Result<Option<Vec<u8>>> {
        let path = self.reference_dir(project_id, source).join(Self::drofus_filename(taken_at));
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(fs::read(&path).with_context(|| {
            format!("could not read reference-source snapshot: {}", path.display())
        })?))
    }

    fn get_latest_reference(&self, project_id: &str, source: &str) -> Result<Option<(String, Vec<u8>)>> {
        // Latest = last of the reconciled ascending list (RFC3339-UTC ids, so
        // lexical max is newest). Going through the reconciliation instead of
        // a raw directory scan means an un-indexed file still wins its way in
        // and a phantom manifest id can't name a file that isn't there.
        let Some(id) = self.list_reference_snapshot_ids(project_id, source)?.pop() else {
            return Ok(None);
        };
        match self.get_reference(project_id, source, &id)? {
            Some(bytes) => Ok(Some((id, bytes))),
            None => Ok(None), // racing delete; treat as no data
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{Model, Project, RoomPayload, Snapshot, SUPPORTED_SCHEMA};
    use crate::storage::{MemStore, ModelEntry};
    use std::collections::BTreeMap;

    /// The typed façade `AppState` puts over this byte-level trait, duplicated
    /// into the test module (house rule: a shared helper is duplicated per
    /// module, not hoisted).
    ///
    /// It exists because what these tests are *about* — history, indexing,
    /// quarantine, phase immutability — is stated in payloads. Rewriting every
    /// assertion in `Vec<u8>` would bury the behaviour under serde noise, and
    /// the serde half is exercised by `state.rs`'s own callers regardless.
    trait TypedStore {
        fn put(&self, payload: &RoomPayload) -> Result<()>;
        fn get_latest(&self, key: &ModelKey) -> Result<Option<RoomPayload>>;
        fn all_latest(&self) -> Result<Vec<(ModelKey, RoomPayload)>>;
        fn get_snapshot(&self, key: &ModelKey, taken_at: &str) -> Result<Option<RoomPayload>>;
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

        fn all_latest(&self) -> Result<Vec<(ModelKey, RoomPayload)>> {
            self.all_latest_raw(SnapshotKind::Rooms)?
                .into_iter()
                .map(|(k, b)| Ok((k, serde_json::from_slice(&b)?)))
                .collect()
        }

        fn get_snapshot(&self, key: &ModelKey, taken_at: &str) -> Result<Option<RoomPayload>> {
            self.get_snapshot_raw(SnapshotKind::Rooms, key, taken_at)?
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

    fn phased(project: &str, model: &str, ts: &str, phase: &str) -> RoomPayload {
        RoomPayload { phase: Some(phase.into()), ..payload(project, model, ts) }
    }

    /// The lineage's phase is set by the first phased push and never moved by a
    /// later one — the immutability backstop `put` applies regardless of what
    /// the ingest handler decided. A push that disagrees should never reach
    /// `put` at all; if one does, it must not silently re-phase the model.
    #[test]
    fn test_put_records_lineage_phase_once_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("roommate-phase-set-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();
        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };

        // An unphased push leaves the lineage unphased.
        store.put(&payload("p", "m", "2026-01-01T10:00:00Z")).unwrap();
        assert_eq!(store.get_phase(&key).unwrap(), None);

        // The first phased push sets it.
        store.put(&phased("p", "m", "2026-01-02T10:00:00Z", "New Construction")).unwrap();
        assert_eq!(store.get_phase(&key).unwrap().as_deref(), Some("New Construction"));

        // A later disagreeing one does not move it.
        store.put(&phased("p", "m", "2026-01-03T10:00:00Z", "Existing")).unwrap();
        assert_eq!(
            store.get_phase(&key).unwrap().as_deref(),
            Some("New Construction"),
            "a lineage's phase is immutable once set"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A quarantined push is stored but inert: no read path can see it, and it
    /// cannot be mistaken for history. This is the property the whole
    /// `pending/` subdirectory exists to guarantee — a `.json` file sitting
    /// beside the real snapshots would be picked up by both model-dir scans.
    #[test]
    fn test_pending_push_is_invisible_to_every_read_path() {
        let dir = std::env::temp_dir().join(format!("roommate-pending-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();
        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };

        store.put(&phased("p", "m", "2026-01-01T10:00:00Z", "New Construction")).unwrap();
        store.put_pending(&key, &phased("p", "m", "2026-06-01T10:00:00Z", "Existing")).unwrap();

        let latest = store.get_latest(&key).unwrap().expect("the live snapshot");
        assert_eq!(latest.snapshot.taken_at, "2026-01-01T10:00:00Z", "the pending push is not the latest");
        assert_eq!(
            store.list_snapshot_ids(SnapshotKind::Rooms, &key).unwrap(),
            vec!["2026-01-01T10:00:00Z".to_string()]
        );
        assert_eq!(store.all_latest().unwrap().len(), 1);
        assert!(store.get_snapshot(&key, "2026-06-01T10:00:00Z").unwrap().is_none());
        // The lineage's phase is untouched by a quarantined push.
        assert_eq!(store.get_phase(&key).unwrap().as_deref(), Some("New Construction"));
        // But it is retrievable, which is what makes promotion possible.
        assert_eq!(store.get_pending(&key).unwrap().expect("pending").phase.as_deref(), Some("Existing"));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// One pending slot per model: a second quarantined push replaces the
    /// first. With no delete route, an accumulating backlog would be
    /// unclearable, and only the newest is ever worth promoting.
    #[test]
    fn test_second_pending_push_replaces_the_first() {
        let dir = std::env::temp_dir().join(format!("roommate-pending-2-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();
        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };

        store.put_pending(&key, &phased("p", "m", "2026-06-01T10:00:00Z", "Existing")).unwrap();
        store.put_pending(&key, &phased("p", "m", "2026-07-01T10:00:00Z", "Demolition")).unwrap();

        let pending = store.get_pending(&key).unwrap().expect("pending");
        assert_eq!(pending.snapshot.taken_at, "2026-07-01T10:00:00Z");
        assert_eq!(pending.phase.as_deref(), Some("Demolition"));

        let pending_dir = dir.join("p").join("m").join(PENDING_DIR);
        assert_eq!(std::fs::read_dir(&pending_dir).unwrap().count(), 1, "exactly one pending file, always");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Promotion is the one deliberate way a lineage re-phases: the quarantined
    /// push becomes a normal snapshot, the manifest's phase moves to its phase,
    /// and the quarantine clears. History is not rewritten — the earlier
    /// snapshot still reports the phase it was pushed under.
    #[test]
    fn test_promote_pending_rephases_the_lineage_without_rewriting_history() {
        let dir = std::env::temp_dir().join(format!("roommate-promote-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();
        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };

        store.put(&phased("p", "m", "2026-01-01T10:00:00Z", "New Construction")).unwrap();
        store.put_pending(&key, &phased("p", "m", "2026-06-01T10:00:00Z", "Existing")).unwrap();

        let promoted = store.promote(&key).unwrap().expect("something was pending");
        assert_eq!(promoted.snapshot.taken_at, "2026-06-01T10:00:00Z");

        assert_eq!(store.get_phase(&key).unwrap().as_deref(), Some("Existing"), "the lineage re-phased");
        assert_eq!(store.get_latest(&key).unwrap().unwrap().snapshot.taken_at, "2026-06-01T10:00:00Z");
        assert_eq!(
            store.list_snapshot_ids(SnapshotKind::Rooms, &key).unwrap().len(),
            2,
            "both snapshots are history now"
        );
        assert!(store.get_pending(&key).unwrap().is_none(), "the quarantine is cleared");

        // The pre-existing snapshot keeps its own phase: the manifest is the
        // enforcement key, not a retroactive relabelling of what was stored.
        let old = store.get_snapshot(&key, "2026-01-01T10:00:00Z").unwrap().expect("still there");
        assert_eq!(old.phase.as_deref(), Some("New Construction"));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Promoting with nothing pending is a `None`, not an error — the endpoint
    /// turns that into a 404 rather than a 500.
    #[test]
    fn test_promote_pending_with_nothing_pending_is_none() {
        let dir = std::env::temp_dir().join(format!("roommate-promote-none-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();
        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };

        store.put(&phased("p", "m", "2026-01-01T10:00:00Z", "New Construction")).unwrap();
        assert!(store.promote(&key).unwrap().is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A doors push described to the store. The bytes stay opaque in these
    /// tests — there is no `Door` type yet, and the store's job is precisely to
    /// keep two kinds' bytes apart without knowing what either means.
    fn doors_meta<'a>(key: &'a ModelKey, taken_at: &'a str) -> SnapshotMeta<'a> {
        SnapshotMeta {
            kind: SnapshotKind::Doors,
            key,
            project_name: "P",
            model_name: "M",
            taken_at,
            phase: Some("New Construction"),
        }
    }

    /// **The additivity claim R1 rests on.** Both model-dir scans filter on
    /// `extension == "json"`, so a `doors/` subdirectory must be completely
    /// invisible to the rooms read paths — otherwise a doors push would show up
    /// as room history and, worse, could be returned as the latest *room*
    /// snapshot. This is the same property `pending/` and `reference/` rely on,
    /// asserted for the kind that is about to start using it.
    #[test]
    fn test_doors_snapshots_are_invisible_to_the_rooms_read_paths() {
        let dir = std::env::temp_dir().join(format!("roommate-doors-invisible-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();
        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };

        store.put(&payload("p", "m", "2026-01-01T10:00:00Z")).unwrap();
        // A doors push with a *later* id: if the scans leaked, this is the one
        // that would wrongly win as "latest rooms".
        store.put_raw(&doors_meta(&key, "2026-06-01T10:00:00Z"), b"{\"doors\":[]}").unwrap();

        assert_eq!(
            store.get_latest(&key).unwrap().expect("rooms").snapshot.taken_at,
            "2026-01-01T10:00:00Z",
            "a later doors push must not become the latest rooms snapshot"
        );
        assert_eq!(
            store.list_snapshot_ids(SnapshotKind::Rooms, &key).unwrap(),
            vec!["2026-01-01T10:00:00Z".to_string()],
            "doors ids are not rooms history"
        );
        assert!(store.get_snapshot(&key, "2026-06-01T10:00:00Z").unwrap().is_none());
        assert_eq!(store.all_latest().unwrap().len(), 1);

        // The doors dir really is where it is claimed to be, so the invisibility
        // above is the extension filter doing its job rather than the push
        // having silently gone nowhere.
        assert!(dir.join("p").join("m").join("doors").is_dir());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Rooms and doors are independent slots for one model: each kind reads back
    /// its own bytes, and the model itself is still listed exactly once — a
    /// model is one model however many kinds it carries.
    #[test]
    fn test_rooms_and_doors_are_independent_slots() {
        let dir = std::env::temp_dir().join(format!("roommate-doors-slots-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();
        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };

        store.put(&payload("p", "m", "2026-01-01T10:00:00Z")).unwrap();
        store.put_raw(&doors_meta(&key, "2026-01-01T10:00:00Z"), b"{\"doors\":[1]}").unwrap();

        assert_eq!(
            store.get_latest_raw(SnapshotKind::Doors, &key).unwrap().as_deref(),
            Some(&b"{\"doors\":[1]}"[..])
        );
        assert_eq!(store.list_snapshot_ids(SnapshotKind::Doors, &key).unwrap().len(), 1);
        assert_eq!(store.list_models().unwrap().len(), 1, "one model, two kinds");

        // The manifest indexes each kind in its own list.
        let manifest = store.read_manifest("p").unwrap();
        let entry = &manifest.models["m"];
        assert_eq!(entry.snapshots, vec!["2026-01-01T10:00:00Z".to_string()]);
        assert_eq!(entry.doors, vec!["2026-01-01T10:00:00Z".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The history and duplicate-skip rules are trait behaviour, not a rooms
    /// special case: doors accumulate history under their own ids, and a
    /// re-sent `taken_at` is skipped rather than overwriting what is there.
    #[test]
    fn test_doors_snapshots_keep_history_and_skip_duplicates() {
        let dir = std::env::temp_dir().join(format!("roommate-doors-history-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();
        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };

        store.put_raw(&doors_meta(&key, "2026-01-01T10:00:00Z"), b"first").unwrap();
        store.put_raw(&doors_meta(&key, "2026-01-02T10:00:00Z"), b"second").unwrap();
        store.put_raw(&doors_meta(&key, "2026-01-02T10:00:00Z"), b"resent").unwrap();

        assert_eq!(
            store.list_snapshot_ids(SnapshotKind::Doors, &key).unwrap(),
            vec!["2026-01-01T10:00:00Z".to_string(), "2026-01-02T10:00:00Z".to_string()],
        );
        assert_eq!(
            store.get_latest_raw(SnapshotKind::Doors, &key).unwrap().as_deref(),
            Some(&b"second"[..]),
            "a duplicate id is skipped, never overwritten"
        );
        assert_eq!(
            store
                .get_snapshot_raw(SnapshotKind::Doors, &key, "2026-01-01T10:00:00Z")
                .unwrap()
                .as_deref(),
            Some(&b"first"[..]),
            "history is kept"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A doors push to an unphased lineage sets the lineage's phase, and a later
    /// one cannot move it — the immutability backstop is a property of the
    /// lineage, not of the kind that happened to establish it.
    #[test]
    fn test_doors_push_obeys_lineage_phase_immutability() {
        let dir = std::env::temp_dir().join(format!("roommate-doors-phase-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();
        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };

        store.put(&payload("p", "m", "2026-01-01T10:00:00Z")).unwrap();
        assert_eq!(store.get_phase(&key).unwrap(), None, "the rooms push declared no phase");

        store.put_raw(&doors_meta(&key, "2026-02-01T10:00:00Z"), b"doors").unwrap();
        assert_eq!(store.get_phase(&key).unwrap().as_deref(), Some("New Construction"));

        let mut later = doors_meta(&key, "2026-03-01T10:00:00Z");
        later.phase = Some("Existing");
        store.put_raw(&later, b"doors").unwrap();
        assert_eq!(
            store.get_phase(&key).unwrap().as_deref(),
            Some("New Construction"),
            "immutable once set, whichever kind set it"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Two models under one project don't overwrite; each keeps its own latest.
    #[test]
    fn test_fs_store_keeps_models_separate() {
        let dir = std::env::temp_dir().join(format!("roommate-test-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();

        store.put(&payload("proj1", "modelA", "2026-01-01T10:00:00Z")).unwrap();
        store.put(&payload("proj1", "modelB", "2026-01-01T11:00:00Z")).unwrap();

        let all = store.all_latest().unwrap();
        assert_eq!(all.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A newer snapshot is returned as latest, older one still on disk (history).
    #[test]
    fn test_fs_store_latest_wins_history_kept() {
        let dir = std::env::temp_dir().join(format!("roommate-hist-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();

        store.put(&payload("p", "m", "2026-01-01T10:00:00Z")).unwrap();
        store.put(&payload("p", "m", "2026-01-02T10:00:00Z")).unwrap();

        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };
        let latest = store.get_latest(&key).unwrap().unwrap();
        assert_eq!(latest.snapshot.taken_at, "2026-01-02T10:00:00Z");

        // Both snapshot files present — history not overwritten.
        let files = std::fs::read_dir(dir.join("p").join("m")).unwrap().count();
        assert_eq!(files, 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The manifest is the read index: a model dir present on disk but
    /// missing from `project.toml` must still appear in `all_latest`
    /// (filesystem truth wins over a buggy manifest), alongside the
    /// manifest-listed model.
    #[test]
    fn test_fs_store_filesystem_wins_over_manifest() {
        let dir = std::env::temp_dir().join(format!("roommate-manifest-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();

        store.put(&payload("proj1", "modelA", "2026-01-01T10:00:00Z")).unwrap();
        store.put(&payload("proj1", "modelB", "2026-01-01T11:00:00Z")).unwrap();

        // Sabotage the manifest: drop modelB from it, as if a push had
        // crashed between snapshot write and manifest write.
        let manifest_path = dir.join("proj1").join("project.toml");
        let manifest = ProjectManifest {
            name: "P".to_string(),
            models: BTreeMap::from([(
                "modelA".to_string(),
                ModelEntry {
                    name: "M".to_string(),
                    phase: None,
                    snapshots: vec!["2026-01-01T10:00:00Z".to_string()],
                    doors: vec![],
                },
            )]),
            reference_snapshots: BTreeMap::new(),
        };
        std::fs::write(&manifest_path, toml::to_string_pretty(&manifest).unwrap()).unwrap();

        let mut keys: Vec<String> = store.list_models().unwrap().into_iter().map(|k| k.model_id).collect();
        keys.sort();
        assert_eq!(keys, vec!["modelA".to_string(), "modelB".to_string()]);

        let all = store.all_latest().unwrap();
        assert_eq!(all.len(), 2, "the un-manifested model still appears");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Each push indexes its snapshot id in the manifest, and
    /// `list_snapshot_ids` reads them back ascending regardless of push
    /// order — never opening a snapshot JSON.
    #[test]
    fn test_fs_store_lists_snapshot_ids_ascending() {
        let dir = std::env::temp_dir().join(format!("roommate-snap-ids-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();

        store.put(&payload("p", "m", "2026-01-02T10:00:00Z")).unwrap();
        store.put(&payload("p", "m", "2026-01-01T10:00:00Z")).unwrap();

        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };
        assert_eq!(
            store.list_snapshot_ids(SnapshotKind::Rooms, &key).unwrap(),
            vec!["2026-01-01T10:00:00Z".to_string(), "2026-01-02T10:00:00Z".to_string()]
        );

        // Unknown model: empty, not an error.
        let unknown = ModelKey { project_id: "p".into(), model_id: "nope".into() };
        assert!(store.list_snapshot_ids(SnapshotKind::Rooms, &unknown).unwrap().is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Reconciliation, filesystem wins both ways: a snapshot file the
    /// manifest doesn't index (a store written before the `snapshots` field
    /// existed) is included with its id recovered from the sanitised
    /// filename, and a manifest id with no file behind it is dropped.
    #[test]
    fn test_fs_store_snapshot_ids_filesystem_wins_over_manifest() {
        let dir = std::env::temp_dir().join(format!("roommate-snap-rec-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();

        store.put(&payload("p", "m", "2026-01-01T10:00:00Z")).unwrap();
        store.put(&payload("p", "m", "2026-01-02T10:00:00Z")).unwrap();

        // Sabotage the manifest: drop the first id (as if written pre-field)
        // and add a phantom id whose file doesn't exist.
        let manifest_path = dir.join("p").join("project.toml");
        let manifest = ProjectManifest {
            name: "P".to_string(),
            models: BTreeMap::from([(
                "m".to_string(),
                ModelEntry {
                    name: "M".to_string(),
                    phase: None,
                    snapshots: vec!["2026-01-02T10:00:00Z".to_string(), "2026-01-03T10:00:00Z".to_string()],
                    doors: vec![],
                },
            )]),
            reference_snapshots: BTreeMap::new(),
        };
        std::fs::write(&manifest_path, toml::to_string_pretty(&manifest).unwrap()).unwrap();

        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };
        assert_eq!(
            store.list_snapshot_ids(SnapshotKind::Rooms, &key).unwrap(),
            vec!["2026-01-01T10:00:00Z".to_string(), "2026-01-02T10:00:00Z".to_string()],
            "un-indexed file recovered (with its ':' restored), phantom id dropped"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `get_snapshot` answers a specific historic id (not just the latest),
    /// and `None` for an id that was never stored.
    #[test]
    fn test_fs_store_get_snapshot_by_id() {
        let dir = std::env::temp_dir().join(format!("roommate-get-snap-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();

        store.put(&payload("p", "m", "2026-01-01T10:00:00Z")).unwrap();
        store.put(&payload("p", "m", "2026-01-02T10:00:00Z")).unwrap();

        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };
        let old = store.get_snapshot(&key, "2026-01-01T10:00:00Z").unwrap().unwrap();
        assert_eq!(old.snapshot.taken_at, "2026-01-01T10:00:00Z");
        assert!(store.get_snapshot(&key, "2026-03-01T10:00:00Z").unwrap().is_none());

        // MemStore can only answer for its current latest.
        let mem = MemStore::new();
        mem.put(&payload("p", "m", "2026-01-01T10:00:00Z")).unwrap();
        mem.put(&payload("p", "m", "2026-01-02T10:00:00Z")).unwrap();
        assert!(mem.get_snapshot(&key, "2026-01-02T10:00:00Z").unwrap().is_some());
        assert!(mem.get_snapshot(&key, "2026-01-01T10:00:00Z").unwrap().is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// dRofus uploads: put/list/get/latest round-trip, ascending ids,
    /// duplicate `taken_at` skipped with the original bytes preserved.
    #[test]
    fn test_fs_store_drofus_round_trip() {
        let dir = std::env::temp_dir().join(format!("roommate-drofus-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();

        assert!(store.get_latest_reference("p", "drofus").unwrap().is_none());
        assert!(store.list_reference_snapshot_ids("p", "drofus").unwrap().is_empty());

        assert!(store.put_reference("p", "drofus", "2026-01-02T10:00:00Z", b"csv-two").unwrap());
        assert!(store.put_reference("p", "drofus", "2026-01-01T10:00:00Z", b"csv-one").unwrap());

        assert_eq!(
            store.list_reference_snapshot_ids("p", "drofus").unwrap(),
            vec!["2026-01-01T10:00:00Z".to_string(), "2026-01-02T10:00:00Z".to_string()]
        );
        assert_eq!(store.get_reference("p", "drofus", "2026-01-01T10:00:00Z").unwrap().unwrap(), b"csv-one");
        assert!(store.get_reference("p", "drofus", "2026-03-01T10:00:00Z").unwrap().is_none());

        // Latest is the lexical max — the older backfill did not displace it.
        let (id, bytes) = store.get_latest_reference("p", "drofus").unwrap().unwrap();
        assert_eq!(id, "2026-01-02T10:00:00Z");
        assert_eq!(bytes, b"csv-two");

        // Duplicate taken_at: skipped (false), original bytes preserved.
        assert!(!store.put_reference("p", "drofus", "2026-01-02T10:00:00Z", b"CHANGED").unwrap());
        assert_eq!(store.get_reference("p", "drofus", "2026-01-02T10:00:00Z").unwrap().unwrap(), b"csv-two");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The reserved `drofus/` dir must never surface as a phantom model in
    /// `list_models` — the single most likely silent regression of adding a
    /// non-model subdirectory to the project dir.
    #[test]
    fn test_fs_store_drofus_dir_is_not_a_model() {
        let dir = std::env::temp_dir().join(format!("roommate-drofus-dir-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();

        store.put(&payload("p", "m", "2026-01-01T10:00:00Z")).unwrap();
        store.put_reference("p", "drofus", "2026-01-01T11:00:00Z", b"csv").unwrap();

        let keys = store.list_models().unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].model_id, "m");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// dRofus reconciliation, filesystem wins both ways: an un-indexed file
    /// is included with its id recovered from the sanitised filename, a
    /// manifest id with no file behind it is dropped.
    #[test]
    fn test_fs_store_drofus_ids_filesystem_wins_over_manifest() {
        let dir = std::env::temp_dir().join(format!("roommate-drofus-rec-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();

        store.put_reference("p", "drofus", "2026-01-01T10:00:00Z", b"one").unwrap();
        store.put_reference("p", "drofus", "2026-01-02T10:00:00Z", b"two").unwrap();

        // Sabotage the manifest: drop the first id, add a phantom one.
        let manifest_path = dir.join("p").join("project.toml");
        let manifest = ProjectManifest {
            name: String::new(),
            models: BTreeMap::new(),
            reference_snapshots: BTreeMap::from([(
                "drofus".to_string(),
                vec!["2026-01-02T10:00:00Z".to_string(), "2026-01-03T10:00:00Z".to_string()],
            )]),
        };
        std::fs::write(&manifest_path, toml::to_string_pretty(&manifest).unwrap()).unwrap();

        assert_eq!(
            store.list_reference_snapshot_ids("p", "drofus").unwrap(),
            vec!["2026-01-01T10:00:00Z".to_string(), "2026-01-02T10:00:00Z".to_string()],
            "un-indexed file recovered (with its ':' restored), phantom id dropped"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A re-push with an identical `taken_at` is skipped, not overwritten —
    /// history must never be silently destroyed by a duplicate timestamp.
    #[test]
    fn test_fs_store_duplicate_taken_at_does_not_overwrite() {
        let dir = std::env::temp_dir().join(format!("roommate-dup-ts-{}", std::process::id()));
        let store = FsStore::new(dir.clone()).unwrap();

        let first = payload("p", "m", "2026-01-01T10:00:00Z");
        store.put(&first).unwrap();

        // Same taken_at, different content — must NOT replace the original.
        let mut second = payload("p", "m", "2026-01-01T10:00:00Z");
        second.project.name = "CHANGED".to_string();
        store.put(&second).unwrap();

        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };
        let latest = store.get_latest(&key).unwrap().unwrap();
        assert_eq!(latest.project.name, "P", "the original snapshot survives a duplicate-timestamp re-push");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `write_atomic` replaces content in full and leaves nothing behind. The
    /// no-leftovers half matters: the temp names are unique per call, so a
    /// stray one would accumulate forever rather than being reused.
    #[test]
    fn test_write_atomic_replaces_and_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("roommate-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("project.toml");

        write_atomic(&target, b"first").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "first");

        // A shorter payload over a longer one: a non-atomic writer that only
        // truncated would be the obvious way to leave trailing garbage.
        write_atomic(&target, b"second, which is longer").unwrap();
        write_atomic(&target, b"third").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "third");

        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
            .filter(|n| n != "project.toml")
            .collect();
        assert!(leftovers.is_empty(), "temp files left behind: {leftovers:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A failed write must not clobber the file already in place — the whole
    /// point. An unwritable temp (its parent directory does not exist) errors
    /// out with the previous content untouched.
    #[test]
    fn test_failed_write_leaves_the_previous_file_intact() {
        let dir = std::env::temp_dir().join(format!("roommate-atomic-fail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("project.toml");
        write_atomic(&target, b"good").unwrap();

        let unwritable = dir.join("does").join("not").join("exist").join("project.toml");
        assert!(write_atomic(&unwritable, b"doomed").is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "good", "untouched by an unrelated failure");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The manifest goes through `write_atomic`, and a rewritten one still
    /// parses — the round trip that a torn write would break, and that
    /// `read_manifest` turns into a hard error on every later read.
    #[test]
    fn test_manifest_rewrite_round_trips() {
        let dir = std::env::temp_dir().join(format!("roommate-atomic-manifest-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let store = FsStore::new(dir.clone()).unwrap();

        store.put(&payload("p1", "m1", "2026-01-01T10:00:00Z")).unwrap();
        store.put(&payload("p1", "m2", "2026-01-02T10:00:00Z")).unwrap();
        store.put(&payload("p1", "m1", "2026-01-03T10:00:00Z")).unwrap();

        let manifest = store.read_manifest("p1").unwrap();
        assert_eq!(manifest.models.len(), 2);
        assert_eq!(manifest.models["m1"].snapshots.len(), 2, "both m1 snapshots indexed");

        let files: Vec<_> = std::fs::read_dir(dir.join("p1"))
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(files.is_empty(), "temp files left in the project dir: {files:?}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
