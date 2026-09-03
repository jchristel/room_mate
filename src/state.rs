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

use crate::contract::{DoorPayload, RoomPayload};
use crate::reference::ReferenceData;
use crate::settings::{
    BuiltinPropertyDef, HierarchyExclusion, HierarchyTier, Milestone, ReferenceEntity, ReferenceFieldConfig, TestData,
};
use crate::storage::{ModelIndexRow, SnapshotKind, SnapshotMeta, SnapshotStore, SnapshotWriter};

/// The store-side description of a rooms push, derived from the payload.
///
/// This function *is* the typed layer `SnapshotStore` gave up: the store takes
/// bytes and needs five facts alongside them, and these are where those five
/// facts come from for rooms. Kept next to `parse_rooms` so the two halves of
/// the round-trip are read together.
fn rooms_meta<'a>(key: &'a ModelKey, payload: &'a RoomPayload) -> SnapshotMeta<'a> {
    SnapshotMeta {
        kind: SnapshotKind::Rooms,
        key,
        project_name: &payload.project.name,
        model_name: &payload.model.name,
        taken_at: &payload.snapshot.taken_at,
        phase: payload.phase.as_deref(),
    }
}

/// Parse stored bytes back into a `RoomPayload`, naming the model in the error.
///
/// The store used to name the *file path* here, which it can no longer do. The
/// model key is the better identifier anyway: it is what a reader would search
/// the settings and the manifest for, and it stays meaningful for a store that
/// has no paths at all.
fn parse_rooms(key: &ModelKey, bytes: &[u8]) -> anyhow::Result<RoomPayload> {
    serde_json::from_slice(bytes)
        .with_context(|| format!("malformed rooms snapshot for {}/{}", key.project_id, key.model_id))
}

/// `rooms_meta`'s counterpart for doors. Deliberately a second small function
/// rather than a trait over the two payloads: they share five of six fields
/// today, but the moment a third entity's meta differs at all, a trait would
/// have to grow an escape hatch — and there is nothing here to reuse but six
/// field reads.
fn doors_meta<'a>(key: &'a ModelKey, payload: &'a DoorPayload) -> SnapshotMeta<'a> {
    SnapshotMeta {
        kind: SnapshotKind::Doors,
        key,
        project_name: &payload.project.name,
        model_name: &payload.model.name,
        taken_at: &payload.snapshot.taken_at,
        phase: payload.phase.as_deref(),
    }
}

fn parse_doors(key: &ModelKey, bytes: &[u8]) -> anyhow::Result<DoorPayload> {
    parse_opening(SnapshotKind::Doors, key, bytes)
}

/// Parse one stored opening snapshot, naming the kind in the failure.
///
/// The kind is carried purely for the message. It is worth the parameter: the
/// three entities' snapshots are structurally similar enough that "malformed
/// snapshot for p/m" would leave a reader guessing which file to open, and this
/// is the error that fires at boot when a store has been hand-edited.
fn parse_opening<P: serde::de::DeserializeOwned>(
    kind: SnapshotKind,
    key: &ModelKey,
    bytes: &[u8],
) -> anyhow::Result<P> {
    serde_json::from_slice(bytes)
        .with_context(|| format!("malformed {} snapshot for {}/{}", kind.label(), key.project_id, key.model_id))
}

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

/// One reference source's resolved runtime state for a project: its loaded
/// data (`None` when configured but not yet uploaded — the `Upload`-sourced,
/// no-CSV-yet state) and its declared per-column type/QA fields, carried
/// together so a request-time consumer (comparison, validation) never needs a
/// second lookup back into `Settings` for the field configs that go with the
/// data it just resolved.
#[derive(Clone)]
pub struct ProjectReferenceSource {
    /// Which primary entity this source joins onto — carried through from
    /// `ReferenceSourceConfig::entity` so every read path can filter to its own
    /// entity without reaching back into the raw settings.
    pub entity: ReferenceEntity,
    pub data: Option<ReferenceData>,
    pub fields: Vec<ReferenceFieldConfig>,
}

/// One project's classification/join inputs — everything that used to be a
/// flat field on `AppState`, bundled so it can be registered per project
/// instead of applied globally.
#[derive(Clone)]
pub struct ProjectSettings {
    /// Resolved reference sources for this project, keyed by source name —
    /// the same name a `/rooms` filter or `comparison_key` writes before the
    /// dot (`drofus.NetArea`). Loaded once at startup from
    /// `Settings.sources.reference`. Joined onto rooms at response assembly —
    /// a stored snapshot is never mutated by the join.
    ///
    /// **Every source in this map is live on every read path**: the join and
    /// filter in `service::rooms`, the QA report in `service::validation`
    /// (one section per source), `service::comparison`, the graph nodes in
    /// `service::adjacency`, and each milestone's per-source snapshot pin. No
    /// source is privileged — "drofus" is just the name most projects happen
    /// to configure.
    pub reference: BTreeMap<String, ProjectReferenceSource>,

    /// Classification tiers loaded from this project's settings. Resolved
    /// per-room inside `/rooms` assembly; not cached (see classify_room).
    pub hierarchy: Vec<HierarchyTier>,

    /// Canonical → per-source raw property name mappings loaded from this
    /// project's settings. Passed to `lookup_property` alongside each room's
    /// source so dRofus join and classification resolve names consistently
    /// regardless of which producer the room came from.
    pub builtin_properties: Vec<BuiltinPropertyDef>,

    /// Ordered property names shown on a room's label in the viewer. Resolved
    /// per-room inside `/rooms` assembly, same as `hierarchy`.
    pub room_label: Vec<String>,

    /// User-defined milestones (named dates with explicit snapshot pins)
    /// loaded from this project's settings. Read by the milestones listing
    /// and by `assemble_rooms`' milestone filter.
    pub milestones: Vec<Milestone>,

    /// The user-chosen room property that identifies "the same room" across
    /// milestones, or `None` when unset (see `Settings::comparison_key`). Read
    /// by `service::comparison`; its own concept, independent of the dRofus
    /// `link_property`.
    pub comparison_key: Option<String>,

    /// Ordered room property names compared across milestones (see
    /// `Settings::comparison_properties`). Read by `service::comparison`.
    pub comparison_properties: Vec<String>,

    /// This project's area-measurement policy (see `settings::AreaPolicy`):
    /// the declared standard, the wall-thickness ceiling, and the boundary-
    /// regime fallback for models whose extractor predates the envelope field.
    /// Server-used like `hierarchy_exclusions`, not client-only like
    /// `colour_plans`, so it belongs in the resolved bundle — `service::areas`
    /// sizes its wall zone from it and `service::adjacency` takes its default
    /// gap tolerance from the same value.
    pub areas: crate::settings::AreaPolicy,

    /// This project's door policy (see `settings::OpeningPolicy`): the comparison
    /// key and comparable property set for doors. Server-used like `areas`, so
    /// it belongs in the resolved bundle rather than being re-read from
    /// `Settings` at request time.
    pub doors: crate::settings::OpeningPolicy,

    /// The same, for windows. A second field rather than a map keyed on the
    /// entity: the two are read from different call sites and a map would put
    /// an `unwrap_or_default` at each, which is how a project silently gets the
    /// wrong policy.
    pub windows: crate::settings::OpeningPolicy,

    /// Footprint exclusions for the hierarchy-areas feature. Unlike
    /// `colour_plans` (client-only, never in this bundle), exclusions are used by
    /// the SERVER when it computes footprints in `service::areas`, so they belong
    /// here in the resolved bundle alongside `hierarchy` — resolved via
    /// `settings_for` like every other classification input.
    pub hierarchy_exclusions: Vec<HierarchyExclusion>,
}

/// One immutable snapshot of every project's settings. Swapped wholesale
/// behind `AppState`'s lock when the settings UI saves (see `settings_api`) —
/// a request takes ONE snapshot up front and works off it, so a mid-request
/// swap can never produce a torn read (half old hierarchy, half new dRofus).
pub struct SettingsRegistry {
    /// Per-project settings bundles, keyed by project id. Storage stays one
    /// tree keyed by `(project_id, model_id)` independently of this registry
    /// — a project can have stored snapshots with no registered settings (see
    /// `settings_for`'s fallback/skip semantics at each call site).
    pub by_project: HashMap<String, ProjectSettings>,

    /// Explicit fallback bundle for a project with no dedicated settings
    /// file, if the operator configured one (one project file marked
    /// `is_default = true`). When absent, an unregistered project is skipped
    /// on read and rejected on ingest rather than silently falling back to
    /// any bundle.
    pub default: Option<ProjectSettings>,
}

impl SettingsRegistry {
    /// Resolve the settings bundle for one project: its own registered
    /// settings if present, else the explicit default bundle if one is
    /// configured, else `None` (unregistered, no fallback).
    pub fn settings_for(&self, project_id: &str) -> Option<&ProjectSettings> {
        self.by_project.get(project_id).or(self.default.as_ref())
    }

    /// The union of every registered project's reference-source names —
    /// what a `/rooms` filter or `comparison_key` may write before the dot
    /// (`service::rooms::split_namespace`'s vocabulary). Computed fresh from
    /// the live registry rather than cached: a `/rooms` filter can span
    /// several projects unscoped, so its namespace vocabulary can't be
    /// resolved from any one project's config alone, and recomputing this
    /// small union (a handful of projects, a source or two each) per request
    /// is cheap — cheaper than a second piece of state to keep in sync across
    /// a settings hot-swap.
    pub fn known_reference_sources(&self) -> std::collections::BTreeSet<String> {
        self.by_project
            .values()
            .chain(self.default.iter())
            .flat_map(|p| p.reference.keys())
            .cloned()
            .collect()
    }
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
            registry: RwLock::new(Arc::new(SettingsRegistry {
                by_project: project_settings,
                default: default_settings,
            })),
            projects_dir: None,
        }
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
        let json = serde_json::to_vec_pretty(&payload).context("could not serialise snapshot")?;
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
    pub fn open_opening_snapshot<P: crate::contract::OpeningEnvelope + serde::Serialize>(
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
        let json = serde_json::to_vec_pretty(&payload).context("could not serialise doors snapshot")?;
        self.store.put_raw(&doors_meta(&key, &payload), &json)
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
        self.store.list_snapshot_ids(SnapshotKind::Doors, key)
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
    pub fn set_pending_snapshot(&self, key: &ModelKey, payload: &RoomPayload) -> anyhow::Result<()> {
        let json = serde_json::to_vec_pretty(payload).context("could not serialise pending snapshot")?;
        self.store
            .put_pending_raw(key, &payload.snapshot.taken_at, payload.phase.as_deref(), &json)
    }

    /// The quarantined push waiting on one model, if any — see
    /// `SnapshotStore::get_pending_raw`.
    pub fn pending_snapshot(&self, key: &ModelKey) -> anyhow::Result<Option<RoomPayload>> {
        self.store.get_pending_raw(key)?.map(|bytes| parse_rooms(key, &bytes)).transpose()
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

/// One snapshot being written element by element — the JSON half of
/// `SnapshotStore::put_streaming`.
///
/// **The format lives here, not in the store.** The store's boundary is bytes
/// (`CLAUDE.md`: "The store takes bytes plus a `SnapshotMeta`, never a payload
/// type"), and a store that parsed the payload in order to write it would be the
/// first crack in that rule. This is the layer that already owns serde, so it is
/// the one that knows a snapshot is a JSON object with an array of elements in
/// it.
///
/// The file it produces is **one element per line**:
///
/// ```text
/// {"schema_version":7,...,"rooms":[
/// {"id":"r1",...},
/// {"id":"r2",...}
/// ]}
/// ```
///
/// Not `to_vec_pretty`'s indented output, which the buffered path still writes.
/// The two differ only in whitespace and key order, and nothing re-reads a
/// snapshot by shape — serde does not care. What one-per-line buys is that it is
/// writable incrementally at all, while staying greppable and diffable in a way
/// a single long line would not be.
///
/// **Dropping without committing discards the whole snapshot**, and that is the
/// ordinary path rather than an error one: a rooms push is only known to be
/// empty once its rooms have been counted, which is after writing has started.
pub struct StreamingSnapshot<'a> {
    writer: Box<dyn SnapshotWriter + 'a>,
    /// Elements written so far — also what tells `push` whether it owes a comma.
    count: usize,
}

impl<'a> StreamingSnapshot<'a> {
    /// Open the store's writer and emit everything up to the opening bracket of
    /// the element array.
    ///
    /// `envelope` is the payload type itself, serialized and then stripped of
    /// its element array — the array key being `kind.label()`, which is already
    /// exactly the JSON field name for both entities. Reusing the payload type
    /// rather than mirroring it is what keeps a new envelope field from silently
    /// going missing on the streamed path only.
    fn open<T: serde::Serialize>(
        store: &'a dyn SnapshotStore,
        meta: &SnapshotMeta<'_>,
        kind: SnapshotKind,
        envelope: &T,
    ) -> anyhow::Result<Self> {
        let mut fields = match serde_json::to_value(envelope).context("could not serialise snapshot envelope")? {
            serde_json::Value::Object(map) => map,
            other => anyhow::bail!("snapshot envelope must serialise to a JSON object, got {other}"),
        };
        // Drop the element array the caller passed empty; it is written below,
        // one line at a time, from the elements that actually arrive.
        fields.remove(kind.label());

        let mut writer = store.put_streaming(meta)?;
        let mut header =
            serde_json::to_vec(&serde_json::Value::Object(fields)).context("could not serialise snapshot envelope")?;
        // `header` is a complete JSON object; splice the element array in before
        // its closing brace. An envelope with no fields at all is not reachable
        // (identity is validated at ingest), but an empty object would produce
        // `{,"rooms":[` — so the comma is conditional rather than assumed.
        let empty = header == b"{}";
        header.truncate(header.len() - 1);
        writer.write(&header)?;
        if !empty {
            writer.write(b",")?;
        }
        writer.write(format!("\"{}\":[\n", kind.label()).as_bytes())?;
        Ok(Self { writer, count: 0 })
    }

    /// Append one element. The separator is written *before* the element rather
    /// than after, because only this side knows whether a previous one exists —
    /// a trailing comma is the one thing that would make the file unparseable.
    pub fn push<T: serde::Serialize>(&mut self, element: &T) -> anyhow::Result<()> {
        if self.count > 0 {
            self.writer.write(b",\n")?;
        }
        let bytes = serde_json::to_vec(element).context("could not serialise snapshot element")?;
        self.writer.write(&bytes)?;
        self.count += 1;
        Ok(())
    }

    /// How many elements have been written — the count an ingest route reports,
    /// and the one it checks before deciding whether the push is empty.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Close the array and publish the snapshot.
    pub fn commit(mut self) -> anyhow::Result<()> {
        self.writer.write(b"\n]}\n")?;
        self.writer.commit()
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
}
