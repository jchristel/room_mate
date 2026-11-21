//! Shared application state.
//!
//! State no longer owns the store's *mechanism* — it holds a
//! `Box<dyn SnapshotStore>` and delegates. Whether snapshots live on disk
//! (`FsStore`) or in memory (`MemStore`) is chosen once at startup from config;
//! nothing here or in the handlers changes when that choice changes. A database
//! backend later is a third impl, same seam.
//!
//! `ModelKey` lives here (not in `storage`) because it's the shared identity
//! both state and storage key on; keeping it here avoids a state↔storage import
//! cycle.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::Context;

use crate::contract::property_codec::{Decoder, Encoder, TRAILER_MARKER};
use crate::contract::{DoorPayload, RoomPayload, SpacePayload};
use crate::reference::ReferenceData;
use crate::settings::{
    BuiltinPropertyDef, HierarchyExclusion, HierarchyTier, Milestone, ReferenceEntity, ReferenceFieldConfig, TestData,
};
use crate::storage::{ModelIndexRow, SnapshotKind, SnapshotMeta, SnapshotStore, SnapshotWriter};

mod codec;
mod registry;
mod stream;
pub use codec::*;
pub use registry::*;
pub use stream::*;

/// Composite key identifying one storage bucket: a model within a project.
///
/// Keyed on the *ids* (immutable, machine-chosen — the Revit GUID and the
/// project's stable key), never the display names, so renaming in Revit can't
/// fork the record. Room ids are only unique *within* a model, which is exactly
/// why the model half of this key must exist — it disambiguates the same raw
/// room id appearing in two linked models.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModelKey {
    pub project_id: String,
    pub model_id: String,
}

impl ModelKey {
    /// Pull the key out of a payload's identity envelope. Centralised so every
    /// call site keys the same way — state and storage agree on "the key".
    pub fn from_payload(payload: &RoomPayload) -> Self {
        Self { project_id: payload.project.id.clone(), model_id: payload.model.id.clone() }
    }

    /// The same, off a doors payload. Both entities ride the same identity
    /// envelope, so both key identically — which is what makes a door's
    /// `from_room` resolvable against the rooms of the model it was pushed to.
    pub fn from_door_payload(payload: &DoorPayload) -> Self {
        Self { project_id: payload.project.id.clone(), model_id: payload.model.id.clone() }
    }
}

/// Names Windows reserves for DOS devices. Reserved **with any extension and
/// any case**, so `settings/projects/CON.toml` does not name a file — it names
/// the console — and `data/snapshots/NUL/` cannot be created at all.
///
/// The primary development machine here is Windows while CI runs ubuntu, which
/// is exactly the split that lets a Windows-only rule rot unnoticed: on Linux
/// these are perfectly ordinary directory names and every test passes.
const WINDOWS_RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Whether `s` is safe to use as a single filesystem path component —
/// `FsStore` builds paths from project/model ids verbatim, and the settings
/// API names project files `<project_id>.toml`. One predicate shared by
/// ingest validation and the settings API so the two can never disagree on
/// what a safe id is. Lives here next to `ModelKey` for the same reason it
/// does: it's identity policy both state and storage depend on.
///
/// **Rejects on every platform, including the Windows-specific rules.** The
/// store is meant to be portable — a project directory written on Windows and
/// read on Linux, or a repo cloned across both — so a name that is legal on
/// one and impossible on the other would make the tree unopenable after a
/// move. Applying the strictest union everywhere keeps ids meaning the same
/// thing wherever the store is opened, and costs only the handful of names
/// below, which nobody chooses deliberately.
///
/// The three Windows rules, beyond the reserved characters:
/// - **DOS device names** (`WINDOWS_RESERVED_NAMES`) — reserved regardless of
///   extension, so `CON.toml` is the console, not a settings file.
/// - **A trailing dot or space** — silently stripped by the Win32 layer, so
///   `"Ward "` and `"Ward"` become the same directory. Two ids that must stay
///   distinct would collide and one project would overwrite the other.
/// - Both checks are case-insensitive, since the reservation is.
pub fn is_path_safe_component(s: &str) -> bool {
    if s.trim().is_empty()
        || s == "."
        || s == ".."
        || s.contains(['/', '\\', '<', '>', ':', '"', '|', '?', '*'])
        || s.chars().any(|c| c.is_control())
    {
        return false;
    }
    // Trailing dot/space: Windows strips them, silently aliasing two ids.
    if s.ends_with('.') || s.ends_with(' ') {
        return false;
    }
    // DOS device names, extension and case irrelevant: "con", "CON.toml" and
    // "Com1.csv" all resolve to a device rather than a file.
    let stem = s.split('.').next().unwrap_or(s);
    !WINDOWS_RESERVED_NAMES.iter().any(|reserved| stem.eq_ignore_ascii_case(reserved))
}

/// Shared application state: the snapshot store plus the swappable settings
/// registry (resolved at startup, replaceable at runtime by the settings UI).
pub struct AppState {
    /// The snapshot store, behind the trait so the backend is swappable.
    store: Box<dyn SnapshotStore>,

    /// The current settings registry. `RwLock<Arc<..>>` so reads are one
    /// cheap Arc clone and a save swaps the whole registry atomically —
    /// in-flight requests keep the snapshot they started with.
    registry: RwLock<Arc<SettingsRegistry>>,

    /// The `--project-settings` directory the registry was loaded from.
    /// `None` when the state wasn't built from files (unit tests) — the
    /// settings API reports "not file-backed" in that case.
    projects_dir: Option<PathBuf>,
}

impl AppState {
    pub fn new(
        store: Box<dyn SnapshotStore>,
        project_settings: HashMap<String, ProjectSettings>,
        default_settings: Option<ProjectSettings>,
    ) -> Self {
        Self {
            store,
            registry: RwLock::new(Arc::new(SettingsRegistry::from_bundles(project_settings, default_settings))),
            projects_dir: None,
        }
    }

    /// Build state around an already-constructed registry — the file-backed
    /// path, where `bootstrap` has the settings files' digest and `new`'s
    /// bundles-only registry would carry a revision that never moved when a
    /// policy changed.
    pub fn with_registry(store: Box<dyn SnapshotStore>, registry: SettingsRegistry) -> Self {
        Self { store, registry: RwLock::new(Arc::new(registry)), projects_dir: None }
    }

    /// Record which directory the registry came from — chained by `bootstrap`
    /// right after `new`, so the settings API knows where to read/write files.
    pub fn with_projects_dir(mut self, dir: PathBuf) -> Self {
        self.projects_dir = Some(dir);
        self
    }

    pub fn projects_dir(&self) -> Option<&PathBuf> {
        self.projects_dir.as_ref()
    }

    /// The current settings registry snapshot. Take it ONCE at the top of a
    /// request and resolve every bundle off that one `Arc` — a save that
    /// lands mid-request then simply applies from the next request on.
    pub fn settings(&self) -> Arc<SettingsRegistry> {
        self.registry.read().unwrap().clone()
    }

    /// Replace the whole registry — the hot-reload half of a settings save.
    /// Only called after the new registry loaded and validated completely, so
    /// the running server can never observe a half-updated state.
    pub fn swap_registry(&self, new: SettingsRegistry) {
        *self.registry.write().unwrap() = Arc::new(new);
    }

    /// Store a pushed payload whole. Upsert semantics live in the store impl;
    /// state just forwards.
    ///
    /// **The ingest routes do not use this** — they stream, via
    /// `open_room_snapshot`. What is left are the callers that genuinely have a
    /// complete payload in hand and nothing to stream: the startup seed and the
    /// tests. Kept rather than folded into the streaming path, because
    /// "serialize this whole thing" is what those callers actually mean.
    pub fn set_snapshot(&self, payload: RoomPayload) -> anyhow::Result<()> {
        let key = ModelKey::from_payload(&payload);
        let json = encode_snapshot(&payload).context("could not serialise snapshot")?;
        self.store.put_raw(&rooms_meta(&key, &payload), &json)
    }

    /// Begin streaming one model's rooms snapshot: the envelope is written now,
    /// each room as it arrives, and nothing is visible until `commit`.
    ///
    /// `envelope` is the payload with an **empty** `rooms` list — the array is
    /// written from the pushed rooms instead. Taking the real payload type
    /// rather than a purpose-built header struct is what stops the two drifting:
    /// a field added to `RoomPayload` is carried here automatically, where a
    /// mirror struct would have to be remembered.
    pub fn open_room_snapshot(&self, envelope: &RoomPayload) -> anyhow::Result<StreamingSnapshot<'_>> {
        let key = ModelKey::from_payload(envelope);
        StreamingSnapshot::open(self.store.as_ref(), &rooms_meta(&key, envelope), SnapshotKind::Rooms, envelope)
    }

    /// `open_room_snapshot`'s doors counterpart. `envelope` carries an empty
    /// `doors` list, for the same reason.
    /// Open a streamed snapshot for one opening kind.
    ///
    /// Generic over the envelope, and the kind is what makes it work rather than
    /// a coincidence: `StreamingSnapshot::open` strips the element array by
    /// `kind.label()` before writing the header, and each envelope names its
    /// list after its own kind (`doors`, `windows`). So the one function writes
    /// the right header for either, and a kind/envelope mismatch would strip the
    /// wrong key -- which is why the pairing is made once, in the handler that
    /// knows both.
    pub fn open_opening_snapshot<P: crate::contract::SnapshotEnvelope + serde::Serialize>(
        &self,
        kind: SnapshotKind,
        envelope: &P,
    ) -> anyhow::Result<StreamingSnapshot<'_>> {
        let key = ModelKey { project_id: envelope.project().id.clone(), model_id: envelope.model().id.clone() };
        let meta = SnapshotMeta {
            kind,
            key: &key,
            project_name: &envelope.project().name,
            model_name: &envelope.model().name,
            taken_at: envelope.taken_at(),
            phase: envelope.phase(),
            model_to_shared: envelope.model_to_shared().copied(),
        };
        StreamingSnapshot::open(self.store.as_ref(), &meta, kind, envelope)
    }

    /// The store's index — every model, with display names, and **no snapshot
    /// opened**. See `SnapshotStore::model_index` for why this exists.
    pub fn model_index(&self) -> anyhow::Result<Vec<ModelIndexRow>> {
        self.store.model_index()
    }

    /// Whether any model anywhere holds a snapshot of this kind — the
    /// "nothing has ever been pushed to this server" question behind both
    /// assemblers' 204.
    ///
    /// **Store-wide on purpose, and answered from the index.** It used to fall
    /// out of `all_snapshots().is_empty()`, which was fine while that read the
    /// whole store and became wrong the moment it took a scope: a scoped read
    /// of an unknown project is empty, and answering 204 there would collapse
    /// the distinction both assemblers document — "nothing pushed" (204) versus
    /// "the question has an empty answer" (200 with empty vecs).
    pub fn has_any_snapshot(&self, kind: SnapshotKind) -> anyhow::Result<bool> {
        Ok(self.store.model_index()?.iter().any(|row| row.latest.contains_key(&kind)))
    }

    /// Every model's latest snapshot, for the `/rooms` merge, narrowed to one
    /// project — `None` for the genuinely unscoped read.
    ///
    /// **Pass the scope you have.** Parsing is what a read costs here, and a
    /// caller that filters the result afterwards has already paid for every
    /// project it is about to discard. See `SnapshotStore::all_latest_raw`.
    pub fn all_snapshots(&self, project: Option<&str>) -> anyhow::Result<Vec<(ModelKey, RoomPayload)>> {
        self.store
            .all_latest_raw(SnapshotKind::Rooms, project)
            .and_then(|raw| raw.into_iter().map(|(key, bytes)| parse_rooms(&key, &bytes).map(|p| (key, p))).collect())
    }

    /// Store a pushed doors payload whole — `set_snapshot`'s counterpart, and
    /// the same thin serde layer over the byte-level store. Like it, the ingest
    /// route streams instead; this is for callers holding a complete payload.
    pub fn set_door_snapshot(&self, payload: DoorPayload) -> anyhow::Result<()> {
        let key = ModelKey::from_door_payload(&payload);
        let json = encode_snapshot(&payload).context("could not serialise doors snapshot")?;
        self.store.put_raw(&doors_meta(&key, &payload), &json)
    }

    /// The same, for any non-rooms entity, taking its kind.
    ///
    /// Generic where `set_door_snapshot` is not, because by the fourth entity
    /// the pattern is clear: the payload supplies the six identity facts through
    /// `SnapshotEnvelope`, the caller supplies the kind, and nothing else varied
    /// between the per-entity copies. `set_door_snapshot` stays as it is rather
    /// than being rewritten through this — 28 call sites for no behaviour change
    /// is a diff that hides the ones that matter.
    ///
    /// Same thin serde layer over the byte-level store, on the same terms: the
    /// ingest routes stream instead, and this is for a caller holding a complete
    /// payload — fixtures, generators and tests.
    pub fn set_element_snapshot<P: crate::contract::SnapshotEnvelope + serde::Serialize>(
        &self,
        kind: SnapshotKind,
        payload: &P,
    ) -> anyhow::Result<()> {
        let key = ModelKey { project_id: payload.project().id.clone(), model_id: payload.model().id.clone() };
        let json =
            encode_snapshot(payload).with_context(|| format!("could not serialise {} snapshot", kind.label()))?;
        let meta = SnapshotMeta {
            kind,
            key: &key,
            project_name: &payload.project().name,
            model_name: &payload.model().name,
            taken_at: payload.taken_at(),
            phase: payload.phase(),
            model_to_shared: payload.model_to_shared().copied(),
        };
        self.store.put_raw(&meta, &json)
    }

    /// Every model's latest snapshot of one opening kind, parsed into that
    /// kind's envelope. A model that has not pushed this kind contributes
    /// nothing — the normal state, not an error.
    ///
    /// Generic over the envelope rather than one function per entity: the store
    /// already takes the kind and hands back bytes, so the only thing that
    /// varied between `all_door_snapshots` and a windows twin was the type the
    /// bytes were parsed into. `P` supplies that and the caller supplies the
    /// kind — which does mean the two must agree, and nothing here can check it.
    /// That pairing is made once per entity in `service::openings::OpeningKind`
    /// and nowhere else, which is what keeps a mismatch impossible to write by
    /// accident rather than merely unlikely.
    pub fn all_opening_snapshots<P: serde::de::DeserializeOwned>(
        &self,
        kind: SnapshotKind,
        project: Option<&str>,
    ) -> anyhow::Result<Vec<(ModelKey, P)>> {
        self.store.all_latest_raw(kind, project).and_then(|raw| {
            raw.into_iter()
                .map(|(key, bytes)| parse_opening(kind, &key, &bytes).map(|p| (key, p)))
                .collect()
        })
    }

    /// One specific stored snapshot of one opening kind, by id — the milestone
    /// read path. The generic twin of `all_opening_snapshots`.
    pub fn get_opening_snapshot<P: serde::de::DeserializeOwned>(
        &self,
        kind: SnapshotKind,
        key: &ModelKey,
        taken_at: &str,
    ) -> anyhow::Result<Option<P>> {
        self.store
            .get_snapshot_raw(kind, key, taken_at)?
            .map(|bytes| parse_opening(kind, key, &bytes))
            .transpose()
    }

    /// One model's doors snapshot ids, ascending.
    pub fn list_door_snapshot_ids(&self, key: &ModelKey) -> anyhow::Result<Vec<String>> {
        self.list_opening_snapshot_ids(SnapshotKind::Doors, key)
    }

    /// One model's snapshot ids for any opening kind, ascending. The store
    /// already takes the kind, so this is a pass-through -- it exists so callers
    /// name the kind rather than reaching for a per-entity method that would
    /// have to be added for each one.
    pub fn list_opening_snapshot_ids(&self, kind: SnapshotKind, key: &ModelKey) -> anyhow::Result<Vec<String>> {
        self.store.list_snapshot_ids(kind, key)
    }

    /// One specific stored doors snapshot by id — the milestone read path.
    pub fn get_door_snapshot(&self, key: &ModelKey, taken_at: &str) -> anyhow::Result<Option<DoorPayload>> {
        self.store
            .get_snapshot_raw(SnapshotKind::Doors, key, taken_at)?
            .map(|bytes| parse_doors(key, &bytes))
            .transpose()
    }

    /// The phase one model's lineage is declared to be in — see
    /// `SnapshotStore::get_phase`. Read at ingest to decide whether a push
    /// agrees with the model it is joining.
    pub fn model_phase(&self, key: &ModelKey) -> anyhow::Result<Option<String>> {
        self.store.get_phase(key)
    }

    /// Quarantine a push whose phase disagrees with the lineage's — see
    /// `SnapshotStore::put_pending_raw`. Stored but not live, awaiting an
    /// explicit promotion.
    ///
    /// **Generic over the payload, exactly as `open_opening_snapshot` is**, and
    /// for the same reason: this is the serde-at-the-boundary layer, so it
    /// serialises whatever it is handed and the store still takes bytes plus a
    /// kind. A `set_pending_spaces_snapshot` beside a rooms one would be the
    /// parallel method set R1 exists to prevent.
    pub fn set_pending_snapshot<P: crate::contract::SnapshotEnvelope + serde::Serialize>(
        &self,
        key: &ModelKey,
        kind: SnapshotKind,
        payload: &P,
    ) -> anyhow::Result<()> {
        let json = encode_snapshot(payload).context("could not serialise pending snapshot")?;
        self.store.put_pending_raw(key, kind, payload.taken_at(), payload.phase(), &json)
    }

    /// The quarantined rooms push waiting on one model, if any — see
    /// `SnapshotStore::get_pending_raw`.
    ///
    /// Rooms-typed on the way *out* while the way in is generic, and the
    /// asymmetry is real rather than an oversight: writing needs only bytes, and
    /// reading needs to know what to parse them into. `pending_spaces_snapshot`
    /// is its sibling; a caller always knows which kind it is asking about,
    /// because the route says so.
    pub fn pending_snapshot(&self, key: &ModelKey) -> anyhow::Result<Option<RoomPayload>> {
        self.store
            .get_pending_raw(key, SnapshotKind::Rooms)?
            .map(|bytes| parse_rooms(key, &bytes))
            .transpose()
    }

    /// The quarantined **spaces** push waiting on one model, if any.
    pub fn pending_spaces_snapshot(&self, key: &ModelKey) -> anyhow::Result<Option<SpacePayload>> {
        self.store
            .get_pending_raw(key, SnapshotKind::Spaces)?
            .map(|bytes| parse_spaces(key, &bytes))
            .transpose()
    }

    /// Make a quarantined push live, re-phasing the lineage — see
    /// `SnapshotStore::promote_pending`. The one deliberate way a model's phase
    /// changes after it is first set.
    ///
    /// The parse happens *here* rather than in the store because promoting is
    /// defined in terms of the pending payload's own phase, and the store sees
    /// only bytes. Reading it first also means a corrupt quarantined file fails
    /// before anything is written, leaving it intact and retryable.
    pub fn promote_pending_snapshot(&self, key: &ModelKey) -> anyhow::Result<Option<RoomPayload>> {
        let Some(payload) = self.pending_snapshot(key)? else {
            return Ok(None);
        };
        if self.store.promote_pending(&rooms_meta(key, &payload))? {
            Ok(Some(payload))
        } else {
            // The quarantine was cleared between the read above and the
            // promotion — report it as "nothing was pending", the same answer
            // the caller would have got had it arrived a moment later.
            Ok(None)
        }
    }

    /// `promote_pending_snapshot`'s spaces counterpart, on identical terms.
    ///
    /// **A spaces lineage is re-phased by promoting a spaces push**, which is
    /// the whole point of D11: a services model usually holds no rooms, so
    /// "re-phase it with a rooms push first" is not an escape hatch it has.
    pub fn promote_pending_spaces_snapshot(&self, key: &ModelKey) -> anyhow::Result<Option<SpacePayload>> {
        let Some(payload) = self.pending_spaces_snapshot(key)? else {
            return Ok(None);
        };
        let meta = SnapshotMeta {
            kind: SnapshotKind::Spaces,
            key,
            project_name: &payload.project.name,
            model_name: &payload.model.name,
            taken_at: &payload.snapshot.taken_at,
            phase: payload.phase.as_deref(),
            model_to_shared: payload.model_to_shared,
        };
        if self.store.promote_pending(&meta)? {
            Ok(Some(payload))
        } else {
            Ok(None)
        }
    }

    /// What a level-scoped push carries over from this model's latest snapshot of `kind`: every
    /// element NOT on `replaced_levels` (Revit level ids, as the elements name them). `None` when the
    /// model has no snapshot of that kind yet, in which case the push is simply what it sent.
    pub fn carried_elements(
        &self,
        kind: SnapshotKind,
        key: &ModelKey,
        replaced_levels: &[String],
    ) -> anyhow::Result<Option<Carried>> {
        use crate::contract::{CeilingPayload, FfePayload, FloorPayload, WindowPayload};
        let Some(bytes) = self.store.get_latest_raw(kind, key)? else {
            return Ok(None);
        };
        let (b, m, l) = (bytes.as_slice(), key.model_id.as_str(), replaced_levels);
        Ok(Some(match kind {
            SnapshotKind::Rooms => carry!(b, RoomPayload, rooms, m, l, key),
            SnapshotKind::Doors => carry!(b, DoorPayload, doors, m, l, key),
            SnapshotKind::Windows => carry!(b, WindowPayload, windows, m, l, key),
            SnapshotKind::Ffe => carry!(b, FfePayload, ffe, m, l, key),
            SnapshotKind::Spaces => carry!(b, SpacePayload, spaces, m, l, key),
            SnapshotKind::Ceilings => carry!(b, CeilingPayload, ceilings, m, l, key),
            SnapshotKind::Floors => carry!(b, FloorPayload, floors, m, l, key),
        }))
    }

    /// One model's snapshot ids, ascending — see `SnapshotStore::list_snapshot_ids`.
    pub fn list_snapshot_ids(&self, key: &ModelKey) -> anyhow::Result<Vec<String>> {
        self.store.list_snapshot_ids(SnapshotKind::Rooms, key)
    }

    /// One specific stored snapshot by id — see `SnapshotStore::get_snapshot_raw`.
    pub fn get_snapshot(&self, key: &ModelKey, taken_at: &str) -> anyhow::Result<Option<RoomPayload>> {
        self.store
            .get_snapshot_raw(SnapshotKind::Rooms, key, taken_at)?
            .map(|bytes| parse_rooms(key, &bytes))
            .transpose()
    }

    /// Direct access to the store, for callers that need to pass it on
    /// (`load_project_bundle` hydrates `Upload`-sourced dRofus from it during
    /// a settings save's re-validation).
    pub fn store(&self) -> &dyn SnapshotStore {
        self.store.as_ref()
    }

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

pub type Shared = Arc<AppState>;

pub fn seed_if_test(state: &AppState, test_data: Option<&TestData>) -> anyhow::Result<()> {
    if let Some(test) = test_data {
        let raw = std::fs::read_to_string(&test.snapshot_path)
            .with_context(|| format!("could not read test snapshot: {}", test.snapshot_path.display()))?;
        // Parse into the same type a push is *stored* as — seed and push
        // converge on one representation and can never drift.
        //
        // The stored type, not the wire type: a push now arrives as a
        // `RoomsUpload` carrying many models and is decomposed into one
        // `RoomPayload` each. A seed is one model by construction, so it is
        // written in the shape it lands in rather than in the shape it would
        // have been sent in.
        let snapshot: RoomPayload = serde_json::from_str(&raw).context("failed to parse test snapshot JSON")?;
        state.set_snapshot(snapshot)?;
        tracing::info!("seeded snapshot from {}", test.snapshot_path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ordinary ids — the real shapes this sees: a Revit document GUID, a
    /// human project name with spaces, a dotted version. All must pass, or
    /// the guard is rejecting legitimate pushes.
    #[test]
    fn test_ordinary_ids_are_accepted() {
        for id in [
            "sample-project",
            "Riverside Hospital",
            "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
            "Building_BF_Framing_jan.r.christel",
            "130486",
            "v1.2.3",
            "CONTRACT", // starts with a reserved name but is not one
            "NULLABLE",
            "COM10", // only COM1-9 are devices
        ] {
            assert!(is_path_safe_component(id), "{id:?} is a legitimate id");
        }
    }

    /// **The traversal guard.** `FsStore` joins these ids straight into a path,
    /// so a separator or `..` would escape the storage root entirely.
    #[test]
    fn test_traversal_and_separators_are_rejected() {
        for id in [
            "", "   ", ".", "..", "../etc", "a/b", "a\\b", "a:b", "a|b", "a?b", "a*b", "a\"b", "a<b", "a>b",
        ] {
            assert!(!is_path_safe_component(id), "{id:?} must not become a path component");
        }
    }

    /// Control characters, including the NUL that would truncate a path at the
    /// syscall boundary.
    #[test]
    fn test_control_characters_are_rejected() {
        for id in ["a\0b", "a\nb", "a\tb", "a\rb"] {
            assert!(!is_path_safe_component(id), "{id:?} must be rejected");
        }
    }

    /// **Windows DOS device names, with any extension and any case.** The
    /// settings API names files `<project_id>.toml`, so a project id of `CON`
    /// produces `CON.toml` — which on Windows is the console, not a file. CI
    /// runs ubuntu, where these are ordinary names, so nothing else would
    /// catch this.
    #[test]
    fn test_windows_device_names_are_rejected() {
        for id in [
            "CON", "con", "Con", "NUL", "PRN", "AUX", "COM1", "com9", "LPT1", "lpt9", "CON.toml", "nul.csv",
        ] {
            assert!(!is_path_safe_component(id), "{id:?} is a DOS device name, not a file name");
        }
    }

    /// A trailing dot or space is silently stripped by Windows, so two ids
    /// that must stay distinct would resolve to one directory and one
    /// project's snapshots would land on top of another's.
    #[test]
    fn test_trailing_dot_or_space_is_rejected() {
        for id in ["Ward ", "Ward.", "project ", "project."] {
            assert!(!is_path_safe_component(id), "{id:?} aliases to its stripped form on Windows");
        }
        assert!(is_path_safe_component("Ward"), "the stripped form itself is fine");
    }

    // ---------- the stored property codec, through its framing ----------

    use crate::contract::property_codec::{map as property_map, shared_map};
    use crate::contract::{CustomValue, PropertyMap};

    /// The smallest two-tier element: enough to exercise both field codecs
    /// without dragging a whole contract record's fixture along.
    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Element {
        id: String,
        #[serde(with = "property_map")]
        properties: PropertyMap,
        #[serde(with = "shared_map")]
        type_properties: Arc<PropertyMap>,
    }

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Doc {
        schema_version: u32,
        elements: Vec<Element>,
    }

    fn props(pairs: &[(&str, &str)]) -> PropertyMap {
        pairs
            .iter()
            .map(|(k, v)| ((*k).into(), CustomValue { value: v.to_string(), storage_type: Some("String".into()) }))
            .collect()
    }

    fn element(id: &str, instance: &[(&str, &str)], type_properties: &[(&str, &str)]) -> Element {
        Element {
            id: id.into(),
            properties: props(instance),
            type_properties: Arc::new(props(type_properties)),
        }
    }

    /// What goes in comes out, and what repeated is stored once and shared once.
    ///
    /// The sharing is the point of the codec, so it is asserted directly rather
    /// than inferred from equality: two elements with equal type bags must hold
    /// the SAME allocation after a read, and a key used by both must be one
    /// `Arc<str>`. Equality alone would pass on the uncompressed layout.
    #[test]
    fn test_an_encoded_snapshot_round_trips_and_shares_what_repeats() {
        let doc = Doc {
            schema_version: 3,
            elements: vec![
                element("a", &[("Mark", "A-1")], &[("Width", "820"), ("Fire", "FD30")]),
                element("b", &[("Mark", "B-1")], &[("Width", "820"), ("Fire", "FD30")]),
            ],
        };
        let bytes = encode_snapshot(&doc).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert_eq!(text.matches("FD30").count(), 1, "one type bag stored, not two:\n{text}");
        assert_eq!(text.matches("\"Mark\"").count(), 1, "one key stored, not two:\n{text}");

        let read: Doc = decode_snapshot(&bytes).unwrap();
        assert_eq!(read, doc);
        assert!(Arc::ptr_eq(&read.elements[0].type_properties, &read.elements[1].type_properties));
        let key = |e: &Element| e.properties.keys().next().unwrap().clone();
        assert!(Arc::ptr_eq(&key(&read.elements[0]), &key(&read.elements[1])));
    }

    /// Type bags are stored by content, never by anything like a type id, so two
    /// instances that disagree both keep their own bag -- nothing is silently
    /// replaced by whichever arrived first.
    #[test]
    fn test_differing_type_bags_are_both_kept() {
        let doc = Doc {
            schema_version: 1,
            elements: vec![
                element("a", &[], &[("Width", "820")]),
                element("b", &[], &[("Width", "920")]),
            ],
        };
        let read: Doc = decode_snapshot(&encode_snapshot(&doc).unwrap()).unwrap();
        assert_eq!(read, doc);
        assert!(!Arc::ptr_eq(&read.elements[0].type_properties, &read.elements[1].type_properties));
    }

    /// Every snapshot written before the codec reads unchanged, in both layouts
    /// it was ever written in -- and still shares its keys, because the legacy
    /// path interns them.
    #[test]
    fn test_a_legacy_snapshot_still_reads() {
        let doc = Doc {
            schema_version: 1,
            elements: vec![
                element("a", &[("Mark", "A-1")], &[]),
                element("b", &[("Mark", "B-1")], &[]),
            ],
        };
        for bytes in [
            serde_json::to_vec_pretty(&doc).unwrap(),
            serde_json::to_vec(&doc).unwrap(),
        ] {
            let read: Doc = decode_snapshot(&bytes).unwrap();
            assert_eq!(read, doc);
            let key = |e: &Element| e.properties.keys().next().unwrap().clone();
            assert!(Arc::ptr_eq(&key(&read.elements[0]), &key(&read.elements[1])));
        }
    }

    /// A legacy snapshot is not mistaken for an encoded one because a property
    /// happens to share the marker's name -- the trailer is only ever looked for
    /// on the last line.
    #[test]
    fn test_the_marker_inside_a_legacy_snapshot_is_not_a_trailer() {
        let doc = Doc { schema_version: 1, elements: vec![element("a", &[("property_codec", "1")], &[])] };
        let bytes = serde_json::to_vec_pretty(&doc).unwrap();
        assert_eq!(decode_snapshot::<Doc>(&bytes).unwrap(), doc);
    }

    /// The streamed writer and the buffered one produce the same record: both
    /// end on the dictionary line and read back to the same payload.
    #[test]
    fn test_a_streamed_snapshot_reads_back_like_a_buffered_one() {
        use crate::storage::MemStore;

        let key = ModelKey { project_id: "p".into(), model_id: "m".into() };
        let meta = SnapshotMeta {
            kind: SnapshotKind::Doors,
            key: &key,
            project_name: "P",
            model_name: "M",
            taken_at: "2026-01-01T00:00:00Z",
            phase: None,
            model_to_shared: None,
        };
        let store = MemStore::new();
        let doc = Doc {
            schema_version: 2,
            elements: vec![
                element("a", &[("Mark", "A-1")], &[("Width", "820")]),
                element("b", &[], &[("Width", "820")]),
            ],
        };

        #[derive(serde::Serialize)]
        struct Envelope {
            schema_version: u32,
            doors: Vec<Element>,
        }
        let mut streaming =
            StreamingSnapshot::open(&store, &meta, SnapshotKind::Doors, &Envelope { schema_version: 2, doors: vec![] })
                .unwrap();
        for e in &doc.elements {
            streaming.push(e).unwrap();
        }
        streaming.commit().unwrap();

        let stored = store.get_latest_raw(SnapshotKind::Doors, &key).unwrap().unwrap();
        assert!(trailer_line(&stored).is_some(), "the dictionary is the last line");

        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct Stored {
            schema_version: u32,
            doors: Vec<Element>,
        }
        let read: Stored = decode_snapshot(&stored).unwrap();
        assert_eq!(read.schema_version, doc.schema_version);
        assert_eq!(read.doors, doc.elements);
        assert!(Arc::ptr_eq(&read.doors[0].type_properties, &read.doors[1].type_properties));
    }
}
