//! Snapshot persistence, behind a trait so the backing store is swappable.
//!
//! The whole point of this module is the `SnapshotStore` trait: handlers and
//! `AppState` talk to *it*, never to the filesystem directly. Today the impl is
//! `FsStore` (a directory tree); tomorrow it could be a database — a new impl,
//! no change to callers. Same seam discipline as `ReferenceOrigin`.
//!
//! The two impls live in their own files behind this module — `fs` (`FsStore`)
//! and `mem` (`MemStore`) — re-exported here so `crate::storage::FsStore` /
//! `crate::storage::MemStore` stay the public paths regardless of the split.
//! The trait, the manifest types, and the reserved-dir constant stay here,
//! since both impls depend on them.
//!
//! On-disk layout (STRATEGY.md project → model → snapshot):
//!
//! ```text
//! <root>/
//!   <project-guid>/
//!     project.toml          authoritative: project name + known models
//!     reference/             reserved (never a model id): uploaded reference-source CSVs
//!       <source-name>/       one subdir per reference source (e.g. "drofus")
//!         <snapshot-ts>.csv  one file per upload — history kept, never overwritten
//!     <model-guid>/
//!       <snapshot-ts>.json  one file per rooms push — history kept, never overwritten
//!       doors/               reserved: one subdir per non-rooms entity kind
//!         <snapshot-ts>.json one file per doors push
//!       windows/             the same, for windows
//!         <snapshot-ts>.json one file per windows push
//! ```
//!
//! **The layout is deliberately asymmetric**: rooms sit directly in the model
//! dir and every other kind gets a subdirectory. Migrating the existing room
//! snapshots down into a `rooms/` subdir would buy symmetry and risk a store
//! nobody can read, against data already on disk. The subdirectory is additive
//! and safe because both model-dir scans filter on `extension == "json"`, so a
//! `doors/` or `windows/` directory is invisible to them — the same trick that makes
//! `REFERENCE_DIR` and `PENDING_DIR` safe (PLAN-generalisation.md R1).
//!
//! `project.toml` is **authoritative and two-way**: the server reads it to know
//! what exists and rewrites it on every push (upsert). A push for an unknown
//! project or model *creates* the structure rather than rejecting it — the store
//! grows from pushes.
//!
//! History is kept: every snapshot lands in its own timestamped file. Pruning is
//! a future UI concern (select-and-delete), not an ingest-time decision.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::state::ModelKey;

mod fs;
mod mem;

pub use fs::FsStore;
pub use mem::MemStore;

/// Which primary entity a snapshot call addresses.
///
/// **A closed enum, not an open string** — unlike `Model.source`, which is
/// deliberately open because adding a producer is a settings change. An entity
/// cannot exist without a Rust assembler to build its response, so the set
/// genuinely is code-bound. It also becomes a storage path component, and an
/// enum makes that path-safe by construction rather than by validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapshotKind {
    Rooms,
    Doors,
    Windows,
    Ffe,
}

impl SnapshotKind {
    /// Every kind, for the read paths that must sweep all of them.
    ///
    /// **This exists because an array literal is not exhaustive and the compiler
    /// will not say so.** `list_models` used to iterate
    /// `[SnapshotKind::Rooms, SnapshotKind::Doors]` written out by hand; adding
    /// `Windows` compiled cleanly and would have left every windows snapshot out
    /// of the model index — a silent omission, in the one structure whose job is
    /// to know what a model holds. Every `match` on this enum was caught by the
    /// compiler; the one list that was not is exactly where the bug landed.
    ///
    /// `position` below is what puts the guard back: it is an exhaustive match,
    /// so a new variant fails to compile there, and the test pairs the two so a
    /// variant left out of this array is caught rather than merely discouraged.
    pub const ALL: [SnapshotKind; 4] = [
        SnapshotKind::Rooms,
        SnapshotKind::Doors,
        SnapshotKind::Windows,
        SnapshotKind::Ffe,
    ];

    /// This kind's index in [`ALL`](Self::ALL).
    ///
    /// Deliberately an exhaustive match rather than a derive: it is the
    /// compile-time half of the guard `ALL` describes. Adding a variant without
    /// extending `ALL` breaks this function, which is the reminder.
    ///
    /// `cfg(test)` because nothing in the running server needs a kind's ordinal
    /// — it exists only to be checked. That does not weaken the guard: both CI
    /// gates that matter here, `cargo test` and `cargo clippy --all-targets`,
    /// compile the test cfg, so a variant added without extending `ALL` still
    /// fails the build. Left in the normal cfg it would be dead code, and
    /// clippy runs with `-D warnings`.
    #[cfg(test)]
    const fn position(self) -> usize {
        match self {
            SnapshotKind::Rooms => 0,
            SnapshotKind::Doors => 1,
            SnapshotKind::Windows => 2,
            SnapshotKind::Ffe => 3,
        }
    }

    /// The subdirectory this kind's snapshots live in *within* a model dir, or
    /// `None` for the kind that lives directly in it. See the layout note in
    /// the module doc for why rooms are the asymmetric one.
    pub fn dir_component(&self) -> Option<&'static str> {
        match self {
            SnapshotKind::Rooms => None,
            SnapshotKind::Doors => Some("doors"),
            SnapshotKind::Windows => Some("windows"),
            SnapshotKind::Ffe => Some("ffe"),
        }
    }

    /// How this kind names itself in a log line or an error message.
    pub fn label(&self) -> &'static str {
        match self {
            SnapshotKind::Rooms => "rooms",
            SnapshotKind::Doors => "doors",
            SnapshotKind::Windows => "windows",
            SnapshotKind::Ffe => "ffe",
        }
    }
}

/// Everything a store needs to know about a snapshot it is being handed,
/// beyond the bytes themselves.
///
/// It exists because the trait takes **bytes, not a payload type** (see
/// `SnapshotStore`), and the store still has a manifest to maintain: it must
/// know where the snapshot goes, what to call the project and model in the
/// index, and what phase to record on a lineage that has none yet. Those five
/// facts are the entire typed surface the store lost, so they are passed back
/// explicitly rather than re-parsed out of the bytes.
///
/// Bundled rather than passed as six positional arguments for the same reason
/// `RoomScope` is a struct: the next entity adds a field here, not a re-read of
/// every call site.
pub struct SnapshotMeta<'a> {
    pub kind: SnapshotKind,
    pub key: &'a ModelKey,
    /// Project display name, refreshed into the manifest on every push.
    pub project_name: &'a str,
    /// Model display name, same.
    pub model_name: &'a str,
    /// The snapshot id — the filename, and the manifest index entry.
    pub taken_at: &'a str,
    /// The push's phase. Recorded on a lineage that has none; never overwrites
    /// one that does (see `ModelEntry::phase`).
    pub phase: Option<&'a str>,
}

/// Reserved subdirectory name inside a project dir for uploaded reference-
/// source CSVs, one further subdirectory per source name (e.g. `reference/
/// drofus/`). Never treated as a model dir — `list_models` skips it
/// explicitly. (Model ids are Revit GUIDs in practice, so a real collision is
/// implausible; the skip makes it impossible.)
pub const REFERENCE_DIR: &str = "reference";

/// Reserved subdirectory name inside a *model* dir holding the one quarantined
/// push waiting for a re-phase decision (`PENDING_FILE` inside it).
///
/// A directory rather than a `pending.json` file beside the real snapshots,
/// because both model-dir scans accept anything with a `.json` extension — a
/// quarantined file sitting there would be read as live history by
/// `latest_snapshot_file` and `list_snapshot_ids`. A directory has no
/// extension, so it is invisible to both with no change to either. Same
/// additivity trick that makes `REFERENCE_DIR` safe at the project level.
pub const PENDING_DIR: &str = "pending";

/// The single file inside a model's `PENDING_DIR`. A fixed name, not the
/// snapshot's `taken_at`: "at most one pending push per model" is then true by
/// construction — a second quarantined push overwrites the first rather than
/// relying on cleanup code to prune a directory that could otherwise
/// accumulate forever, given there is no delete route.
pub const PENDING_FILE: &str = "snapshot.json";

// ---------- project.toml ----------

/// The authoritative per-project manifest, one `project.toml` per project dir.
/// Lists the project's display name and every model seen under it. Rewritten on
/// each push so it always reflects the models actually on disk.
///
/// It intentionally duplicates the `name` the snapshot envelope also carries:
/// the manifest is the *index* (readable without opening any snapshot), the
/// envelope is the per-push record. On conflict the latest push wins and updates
/// the manifest.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectManifest {
    /// Project display name (mutable; the GUID dir name is the stable identity).
    pub name: String,
    /// Known models under this project, keyed by model GUID.
    #[serde(default)]
    pub models: BTreeMap<String, ModelEntry>,
    /// Snapshot ids (raw `taken_at` values) of uploaded reference-source
    /// CSVs, ascending, keyed by source name (e.g. "drofus"). Project-scoped,
    /// not model-scoped: a reference source is data joined onto every
    /// model's rooms, so it hangs off the manifest directly rather than a
    /// `ModelEntry`. Same index role (and same `default` back-compat rule) as
    /// `ModelEntry::snapshots`.
    #[serde(default)]
    pub reference_snapshots: BTreeMap<String, Vec<String>>,
}

/// One model in the store's index: its key, plus the display names the manifest
/// holds for it and its project.
///
/// Deliberately **not** a `ModelEntry` — that is the on-disk shape, private to
/// `FsStore` and carrying a phase and two snapshot-id lists no index consumer
/// wants. This is the store-agnostic answer, and `MemStore` (which has no
/// manifest at all) fills it from the `SnapshotMeta` of the pushes it has seen.
#[derive(Debug, Clone)]
pub struct ModelIndexRow {
    pub key: ModelKey,
    /// Project display name. Mutable — the latest push wins, same as the
    /// manifest field it comes from.
    pub project_name: String,
    /// Model display name, under the same mutability rule.
    pub model_name: String,
    /// The newest stored snapshot id (`taken_at`) per kind, absent for a kind
    /// this model holds nothing of.
    ///
    /// **Reconciled, not read straight off the manifest.** These ids are what a
    /// content cursor is built from (`rooms::scope_cursor`), and a cursor that
    /// named a snapshot no read path would serve is worse than no cursor: it
    /// would answer "nothing changed" about content that was never there. So
    /// this goes through `list_snapshot_ids`, which drops a manifest id with no
    /// file behind it — filesystem wins, exactly as everywhere else in the
    /// store.
    pub latest: BTreeMap<SnapshotKind, String>,
}

/// One model's entry in a `ProjectManifest`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelEntry {
    /// Model display name (mutable; the GUID dir name is the stable identity).
    pub name: String,
    /// The Revit phase this model's lineage is declared to be in, set by its
    /// first phased push and immutable thereafter — changing it takes a
    /// quarantined push and an explicit promotion (`promote_pending`).
    ///
    /// **The enforcement key, not a record of history.** It says what future
    /// pushes must match; it says nothing about what any individual snapshot
    /// contains. A snapshot written before phases existed reports itself as
    /// unphased forever, because its own file has no phase — and that stays
    /// true after a later push phases the lineage. Reads therefore take the
    /// phase from the snapshot they loaded, never from here. (PLAN-phasing.md
    /// "D8".)
    ///
    /// Living in the manifest is what lets `snapshots/latest` answer "what
    /// phase is this model in" without opening a snapshot file — the same
    /// index-not-record role `snapshots` below plays, and the reason that read
    /// can stay cheap.
    ///
    /// A scalar declared before `snapshots` per the TOML ordering rule in
    /// CODING-CONVENTIONS.md. `default` keeps every manifest written before
    /// this field existed parseable, as an unphased lineage — which is exactly
    /// what it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Snapshot ids (raw `taken_at` values) stored for this model, ascending.
    /// The manifest's index role extended to snapshots: listing a model's
    /// history reads this, never the (possibly >100 MB) snapshot JSONs.
    /// `default` keeps manifests written before this field existed parseable —
    /// their history is recovered from the directory instead (filesystem wins,
    /// see `list_snapshot_ids`).
    #[serde(default)]
    pub snapshots: Vec<String>,

    /// The same index, for this model's **doors** snapshots. A second list
    /// rather than a `BTreeMap<SnapshotKind, Vec<String>>`: keying a TOML table
    /// on an enum would put the kind names in the file format, so adding a kind
    /// would be a manifest migration instead of a new field with a `default`.
    ///
    /// `default` keeps every manifest written before doors existed parseable,
    /// as a model with no doors — which is exactly what it is.
    #[serde(default)]
    pub doors: Vec<String>,

    /// The same index again, for **windows**. A third flat list rather than the
    /// map the second one already argued against: keying a TOML table on the
    /// kind would put its names in the file format, so this — a new field with
    /// a `default` — is what "adding a kind" is supposed to cost.
    ///
    /// That it cost exactly that is the point. `default` keeps every manifest
    /// written before windows existed parseable, as a model with no windows,
    /// which is what such a model is.
    #[serde(default)]
    pub windows: Vec<String>,

    /// The same index a fourth time, for **FF&E**.
    ///
    /// A fourth flat list, and the *second* time the argument above has been
    /// paid rather than re-made -- which is the evidence it was right. Adding a
    /// kind costs one field with a `default` and two match arms the compiler
    /// insists on; it does not cost a manifest migration, and no manifest
    /// already on disk needs rewriting.
    ///
    /// Note this is the first kind whose entity is not an opening, and the
    /// manifest did not notice. The index is about *where snapshots live*, not
    /// about what is in them.
    #[serde(default)]
    pub ffe: Vec<String>,
}

impl ModelEntry {
    /// This entry's snapshot index for one kind. Paired with `index_mut` so the
    /// read and write paths cannot disagree about which list a kind uses.
    fn index(&self, kind: SnapshotKind) -> &Vec<String> {
        match kind {
            SnapshotKind::Rooms => &self.snapshots,
            SnapshotKind::Doors => &self.doors,
            SnapshotKind::Windows => &self.windows,
            SnapshotKind::Ffe => &self.ffe,
        }
    }

    fn index_mut(&mut self, kind: SnapshotKind) -> &mut Vec<String> {
        match kind {
            SnapshotKind::Rooms => &mut self.snapshots,
            SnapshotKind::Doors => &mut self.doors,
            SnapshotKind::Windows => &mut self.windows,
            SnapshotKind::Ffe => &mut self.ffe,
        }
    }
}

// ---------- the trait ----------

/// Abstract snapshot store. Callers depend only on this; the concrete backend
/// (filesystem now, a database later) is chosen once at startup.
///
/// **Bytes at this boundary, types above it.** No method here names a payload
/// type. `AppState` owns the thin serde layer that turns a `RoomPayload` into
/// bytes on the way in and back on the way out, so every caller outside
/// `storage/` and `state.rs` sees unchanged signatures.
///
/// This shape was forced, not chosen for elegance. `AppState` holds a
/// `Box<dyn SnapshotStore>`, so the trait must stay **object-safe** — a generic
/// `fn put<T: Serialize>` is out. Adding a `kind` parameter while the payload
/// type stayed `RoomPayload` would have accomplished nothing. The alternative,
/// a parallel `put_doors`/`get_latest_doors`/… set, is precisely what
/// the bytes-at-the-trait-boundary rule exists to prevent (`CLAUDE.md`): it
/// would be the third such set, and FFE would make it a fourth.
///
/// It is also not a new pattern here — `put_reference` has always been bytes in,
/// a discriminator alongside, and typed parsing above. R1 applied the store's
/// own existing shape to its other half.
///
/// `put_raw` is an **upsert**: it creates whatever project/model structure is
/// missing, then stores the snapshot. It never rejects an unknown id — a push
/// defines new structure. On a duplicate `taken_at`, a history-keeping store
/// (`FsStore`) **skips with a warning** rather than overwrite — a re-sent
/// payload must not silently destroy the record it duplicates. `MemStore`
/// keeps no history at all (latest-only, by design), so replacement *is* its
/// normal upsert and the skip rule doesn't apply.
pub trait SnapshotStore: Send + Sync {
    /// Persist one pushed snapshot's bytes, creating project/model structure as
    /// needed. Everything the store needs beyond the bytes rides `meta`.
    ///
    /// Takes the whole snapshot at once. `put_streaming` is the incremental
    /// counterpart, and is what an ingest route uses; this one stays for callers
    /// that genuinely have the bytes already — the dev-config seed and
    /// `promote_pending`.
    fn put_raw(&self, meta: &SnapshotMeta<'_>, json: &[u8]) -> Result<()>;

    /// Begin a snapshot that is written **incrementally**, and appears in one
    /// step when committed.
    ///
    /// The reason this exists: an ingest route used to accumulate every element
    /// of a push in memory and then serialize the assembled payload, so peak was
    /// roughly the payload twice. Neither buffer is inherent — the elements
    /// arrive one at a time on the wire and could go to disk one at a time —
    /// and the multi-model push raised the ceiling further, since a request now
    /// carries a whole run rather than one model.
    ///
    /// **The unit is still one snapshot file.** Nothing about the layout,
    /// `ModelKey`, milestone pins or history changes; only *when* the bytes are
    /// written. That is deliberate — element-addressed storage would flatten
    /// exactly the identity the rest of the codebase reads.
    ///
    /// **Bytes, and the caller frames them.** The writer appends whatever it is
    /// given and knows nothing about JSON, keeping the "bytes at this boundary,
    /// types above it" rule intact: `state::StreamingSnapshot` owns the framing
    /// because it is the layer that already owns serde. A store that parsed the
    /// payload to write it would be the first crack in that rule.
    ///
    /// **Nothing is visible until `commit`**, and an uncommitted writer must
    /// leave no trace when dropped. A half-written snapshot that a reader could
    /// find would be worse than no streaming at all: the store re-parses every
    /// snapshot at boot, so a truncated file is a model that silently lost
    /// elements. That is why the returned writer, not this call, is what
    /// publishes.
    fn put_streaming<'a>(&'a self, meta: &SnapshotMeta<'_>) -> Result<Box<dyn SnapshotWriter + 'a>>;

    /// Latest snapshot of one kind for a model, if any. (Latest = newest by
    /// snapshot key.)
    fn get_latest_raw(&self, kind: SnapshotKind, key: &ModelKey) -> Result<Option<Vec<u8>>>;

    /// Every model key the store knows about, **whatever kinds of snapshot it
    /// holds** — the index question. For `FsStore` this is answered by the
    /// `project.toml` manifests (the manifest is the index, snapshots are the
    /// record), reconciled against the directory tree.
    ///
    /// Keys are unique: a model holding both rooms and doors appears once.
    ///
    /// **It used to mean "models with rooms", and the reason it stopped is worth
    /// knowing.** R1 left open whether this should count a doors-only model, and
    /// answered no on the grounds that ingest refused a doors push to a model
    /// with no live rooms snapshot — so the case could not arise. That gate is
    /// gone (see `handlers::check_doors_ingest`): doors may now be pushed first,
    /// and a doors-only model is an ordinary state.
    ///
    /// So the case arises, and the two impls have to answer it the same way or
    /// `all_latest_raw` would find a model's doors on disk and not in memory.
    /// The kind-agnostic answer is also the one `FsStore` was always giving —
    /// its manifest gains an entry on *any* push — so the narrower contract was
    /// already only true of `MemStore`.
    fn list_models(&self) -> Result<Vec<ModelKey>>;

    /// Every model the store knows about, with the display names the manifest
    /// carries for it — the index question `list_models` answers, plus the two
    /// strings a picker needs.
    ///
    /// **It exists so `/projects` can stop parsing the store.** A project's id
    /// and name ride on every payload it stores, so listing projects used to be
    /// derived from `all_latest_raw` — which parses every model's newest
    /// snapshot to read two strings. That is fine at sample-project scale and
    /// absurd at RHH's: measured, a 213-byte `/projects` response cost 0.36 s of
    /// CPU, on a route the viewer polls every two seconds. The names were in the
    /// manifest the whole time; nothing but the missing accessor forced the
    /// parse.
    ///
    /// Its second consumer is the content cursor behind `ETag`/`If-None-Match`
    /// on `/rooms` and `/doors` (`rooms::scope_cursor`), which needs the same
    /// question answered one field further: *which* snapshot each model would
    /// contribute, without reading any of them.
    ///
    /// Names, unlike the ids, are **mutable and best-effort**: a model on disk
    /// that the manifest does not index still appears (filesystem wins, same
    /// rule as `list_models`), carrying its id in place of a name. A caller that
    /// needs a guaranteed name must read the payload — but no caller does, since
    /// a name nothing has ever pushed does not exist to be read.
    fn model_index(&self) -> Result<Vec<ModelIndexRow>>;

    /// Every model's latest snapshot of one kind, for the merge `/rooms` does.
    /// A model with no snapshot of that kind contributes nothing.
    ///
    /// `project` narrows the read to one project, and it is a **parameter
    /// rather than a caller-side filter** because filtering afterwards is what
    /// made every read cost the whole store. Each caller here is answering a
    /// question about one project and then discarding the rest — measured, a
    /// 257 KB `/doors?project=House A` response spent 1.3 s of CPU parsing RHH
    /// first. `None` still reads everything, for the genuinely unscoped
    /// `/rooms`.
    ///
    /// It takes the filter rather than a sibling `all_latest_raw_scoped`
    /// existing beside it: two methods answering one question in two scopes is
    /// the parallel-method-set shape the store's byte-level boundary was
    /// written to avoid, and the unscoped call is just this one with `None`.
    fn all_latest_raw(&self, kind: SnapshotKind, project: Option<&str>) -> Result<Vec<(ModelKey, Vec<u8>)>>;

    /// Every snapshot id (`taken_at`) stored for one model and kind,
    /// ascending — so the latest is the last element. Empty when the model is
    /// unknown or has no snapshots of that kind yet. A history-less store
    /// (`MemStore`) reports just its current latest.
    fn list_snapshot_ids(&self, kind: SnapshotKind, key: &ModelKey) -> Result<Vec<String>>;

    /// One specific stored snapshot by its id (`taken_at`), or `None` when no
    /// such snapshot exists — the milestone read path. A history-less store
    /// (`MemStore`) can only answer for its current latest.
    fn get_snapshot_raw(&self, kind: SnapshotKind, key: &ModelKey, taken_at: &str) -> Result<Option<Vec<u8>>>;

    /// The phase this model's lineage is declared to be in (`ModelEntry.phase`),
    /// or `None` for a lineage nothing phased has ever been pushed to.
    ///
    /// Answered from the index, never by opening a snapshot — that is the whole
    /// reason the phase is mirrored into the manifest.
    fn get_phase(&self, key: &ModelKey) -> Result<Option<String>>;

    /// Store a push whose phase disagrees with the lineage's, without making it
    /// live. **At most one per model**: a second quarantined push replaces the
    /// first, since only the newest could sensibly be promoted and there is no
    /// delete route to clear a backlog.
    ///
    /// A quarantined payload is invisible to every read path —
    /// `get_latest_raw`, `all_latest_raw`, `list_snapshot_ids`,
    /// `get_snapshot_raw` — and cannot be pinned by a milestone. It exists only
    /// to be promoted or overwritten.
    ///
    /// **Rooms-only, and takes no `SnapshotKind`.** Quarantine exists so a model
    /// can be re-phased, and promoting is what moves the lineage's phase. A
    /// doors push has nothing to re-phase towards: promoting it would move the
    /// lineage while every room snapshot stayed on the old phase, stranding the
    /// rooms a door's references resolve against. So a doors push whose phase
    /// disagrees is refused outright rather than quarantined, and there is no
    /// second pending slot to key by kind.
    fn put_pending_raw(&self, key: &ModelKey, taken_at: &str, phase: Option<&str>, json: &[u8]) -> Result<()>;

    /// The quarantined push waiting on this model, if any.
    fn get_pending_raw(&self, key: &ModelKey) -> Result<Option<Vec<u8>>>;

    /// Make the quarantined push live: store it as a normal snapshot, re-phase
    /// the lineage to `meta.phase`, and clear the quarantine. Returns whether
    /// anything was pending to promote.
    ///
    /// The caller supplies `meta` because it has already read and parsed the
    /// pending bytes to decide what promoting *means* — the store cannot, with
    /// bytes at its boundary. Two reads for one promotion is a fair price: it is
    /// a rare, deliberate act, and the alternative was a second phase field on
    /// disk to keep in step with the payload's own.
    ///
    /// This is the **only** way a lineage's phase changes after it is first set
    /// — the deliberate act that keeps `put_raw` free to treat the phase as
    /// immutable. Promoting does not rewrite history: earlier snapshots keep
    /// reporting whatever phase they were pushed under.
    fn promote_pending(&self, meta: &SnapshotMeta<'_>) -> Result<bool>;

    /// Store one uploaded CSV against a project's named reference source
    /// (e.g. "drofus"). Returns `false` when a snapshot with this `taken_at`
    /// already exists for that source — skipped with a warning, never
    /// overwritten, same duplicate rule as `put`. The caller is expected to
    /// have *validated the CSV before storing it*: a stored CSV is hydrated
    /// at every boot, so a bad one stored here fails the next startup loudly.
    fn put_reference(&self, project_id: &str, source: &str, taken_at: &str, csv: &[u8]) -> Result<bool>;

    /// Every snapshot id (`taken_at`) stored for one project's named
    /// reference source, ascending — latest is the last element. Empty when
    /// the project or source is unknown or has no uploads yet. A
    /// history-less store (`MemStore`) reports just its current latest.
    fn list_reference_snapshot_ids(&self, project_id: &str, source: &str) -> Result<Vec<String>>;

    /// One stored CSV for one reference source, by its snapshot id, or `None`.
    fn get_reference(&self, project_id: &str, source: &str, taken_at: &str) -> Result<Option<Vec<u8>>>;

    /// The newest stored CSV for one reference source, with its id — the
    /// bootstrap hydration read that turns an `Upload`-sourced project's
    /// stored data into its in-memory `ReferenceData`.
    fn get_latest_reference(&self, project_id: &str, source: &str) -> Result<Option<(String, Vec<u8>)>>;
}

/// One snapshot being written incrementally — the handle `put_streaming` hands
/// back.
///
/// **An append-only byte sink that publishes atomically, and nothing more.** It
/// does not know it is writing JSON, which is what keeps the store's boundary at
/// bytes; `state::StreamingSnapshot` wraps it and owns the format.
///
/// The two ways it can end are deliberately asymmetric:
///
/// - **`commit`** consumes it and makes the snapshot visible — indexed in the
///   manifest, findable by every read path, and complete.
/// - **`drop`** discards it, leaving no trace. That is the *normal* path for a
///   push that turns out to be refused: an empty rooms push is only recognisable
///   once its elements have been counted, which is after the writing has
///   started. Discarding is therefore an ordinary outcome, not an error path,
///   which is why it needs no method and cannot fail.
///
/// A store that made "dropped without commit" observable — a stray temp file, a
/// manifest entry, a half snapshot — would turn a correct refusal into corrupt
/// data. That is the one thing an implementation must get right.
pub trait SnapshotWriter: Send {
    /// Append bytes to the snapshot in progress.
    fn write(&mut self, bytes: &[u8]) -> Result<()>;

    /// Finish and publish. Consumes the writer, so a committed snapshot cannot
    /// be appended to afterwards.
    ///
    /// `Box<Self>` rather than `self`, because this trait is used as
    /// `Box<dyn SnapshotWriter>` and a by-value `self` would not be object-safe.
    fn commit(self: Box<Self>) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **`ALL` really is all of them**, paired with `position`'s exhaustive
    /// match so the pairing is what enforces it.
    ///
    /// Adding a variant breaks `position` at compile time; this then catches the
    /// two ways it could still go wrong afterwards — a variant added to the enum
    /// and to `position` but forgotten in `ALL`, or `ALL` written in an order
    /// that disagrees with `position`. Both would be silent, and both would
    /// surface as a kind quietly missing from `list_models`, which is where this
    /// went wrong once already.
    #[test]
    fn test_all_kinds_agree_with_their_positions() {
        assert_eq!(SnapshotKind::ALL.len(), 4, "extend ALL when adding a kind");
        for (index, kind) in SnapshotKind::ALL.iter().enumerate() {
            assert_eq!(kind.position(), index, "{} is in the wrong slot of ALL", kind.label());
        }
    }

    /// Each kind must own a distinct storage location and a distinct name.
    /// Rooms are the deliberate `None` — they live directly in the model dir —
    /// so the check is that no two kinds *collide*, not that all are `Some`.
    #[test]
    fn test_kinds_do_not_share_a_directory_or_a_label() {
        let dirs: Vec<_> = SnapshotKind::ALL.iter().map(|k| k.dir_component()).collect();
        let labels: Vec<_> = SnapshotKind::ALL.iter().map(|k| k.label()).collect();
        for i in 0..SnapshotKind::ALL.len() {
            for j in (i + 1)..SnapshotKind::ALL.len() {
                assert_ne!(dirs[i], dirs[j], "two kinds would share a directory");
                assert_ne!(labels[i], labels[j], "two kinds would share a label");
            }
        }
    }
}
