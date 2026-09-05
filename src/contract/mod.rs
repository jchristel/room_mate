//! The JSON contract shared with the Revit extractor, plus the one lookup that
//! reads across its two property tiers.
//!
//! This module is the *shape of the data* and nothing else — no I/O, no state,
//! no handlers. It's the load-bearing type layer both `drofus` and `classify`
//! depend on, which is why `lookup_property` lives here rather than in either
//! consumer: it inspects `Room`'s property tiers, so it belongs next to them,
//! and keeping it here means the two consumers depend on the contract, not on
//! each other.
//!
//! Every type here must match the Revit extractor's serializer. Ids and
//! `ElementId` values ride as strings on the wire (width-safe across the
//! IronPython/CLR seam); numeric `ElementId`s are parsed to `i64` only here,
//! server-side, where the width is safe. See STRATEGY.md "Expand the room
//! properties contract".
//!
//! **This file holds the shared envelope, the geometry primitives, the property
//! machinery, and rooms; [`doors`] holds doors.** The split happened when the
//! door types arrived, which is exactly the trigger CODING-CONVENTIONS.md's
//! measured-module note named for it. Rooms have deliberately *not* been moved
//! out alongside doors: nothing motivates that yet, and moving a file's worth of
//! code with no reason to is how a split stops being reviewable. What is shared
//! stays here and is imported by `doors`, so neither entity carries a private
//! copy of the envelope — see STRATEGY-ENTITIES.md, "What makes something a
//! primary entity", for the list of what generalizes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::settings::BuiltinPropertyDef;

pub mod doors;
pub mod ffe;
pub mod items;
pub mod openings;
pub mod windows;

pub use doors::{DoorModelEnvelope, DoorPayload, DoorStreamEnvelope, DoorsUpload, StreamDoor, SUPPORTED_DOOR_SCHEMA};
pub use ffe::{FfeModelEnvelope, FfePayload, FfeStreamEnvelope, FfeUpload, StreamItem, SUPPORTED_FFE_SCHEMA};
pub use items::{Item, ItemEnvelope};
pub use openings::{Opening, OpeningEnvelope};
pub use windows::{
    StreamWindow, WindowModelEnvelope, WindowPayload, WindowStreamEnvelope, WindowsUpload, SUPPORTED_WINDOW_SCHEMA,
};

/// The six facts every entity's stored snapshot answers, whatever it holds.
///
/// **The seam that lets one scoping pipeline serve four entities.** Rooms,
/// doors, windows and FF&E all decompose into per-model snapshots carrying the
/// same identity block, the same phase, the same placement and the same level
/// list -- and the read side scopes, pins, hashes and phase-reports all four
/// identically. Only the element list differs, so only the element list is left
/// to the sub-trait: [`OpeningEnvelope`](openings::OpeningEnvelope) adds
/// `openings()`, [`ItemEnvelope`](items::ItemEnvelope) adds `items()`.
///
/// This exists because FF&E is the first entity whose *record* is not shared.
/// Windows could reuse the whole opening assembly by reusing `Opening`; an
/// `Item` is a different type, so the choice was between a second copy of the
/// scoping pipeline and a trait that names what the pipeline actually touches.
/// It touches these six methods and nothing else, which is why they are the
/// trait.
///
/// Kept deliberately narrow, on the terms `OpeningEnvelope` already stated:
/// every method is a field read, and the trait exists to hide *which struct* the
/// field came from rather than to grow behaviour. Anything an entity does
/// differently belongs at the call site with its kind beside it, not behind
/// another method here.
pub trait SnapshotEnvelope {
    fn project(&self) -> &Project;
    fn model(&self) -> &Model;
    fn taken_at(&self) -> &str;
    fn phase(&self) -> Option<&str>;
    fn model_to_shared(&self) -> Option<&ModelToShared>;
    fn levels(&self) -> &[Level];
}

/// A 2D point in Revit model space. Units are decimal feet, Y points UP.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Point2D {
    pub x: f64,
    pub y: f64,
}

/// A single closed loop of points. A room has one outer loop and zero or more
/// inner loops (holes, e.g. a column or shaft punched through the room).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Loop {
    pub points: Vec<Point2D>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Level {
    pub id: String,
    pub name: String,
    pub elevation: f64,
}

/// One custom property: the raw string value plus an optional storage-type
/// hint from Revit. Paired in one struct (not two parallel maps) so value and
/// type can't drift and an absent type degrades to "treat as string".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomValue {
    /// Raw value, always a string. Revit hands most params back as strings;
    /// any typing is deferred and done server-side, lazily.
    pub value: String,

    /// Revit's declared StorageType, forwarded as guidance only:
    /// "String" | "Integer" | "Double" | "ElementId". Optional — absent means
    /// "treat as string". This is a HINT: declared type and parseable content
    /// can disagree (a String param holding "12.5", an empty Double), so any
    /// coercion keyed off it must fall back to `value` on failure.
    ///
    /// Set by the Python extractor's DataProperty.storage_type field
    /// (str(p.StorageType) on the Revit parameter).
    #[serde(default)]
    pub storage_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub level_id: String,
    pub loops: Vec<Loop>,

    /// Raw properties as extracted, keyed by the *source's own* property name
    /// (e.g. Revit's `p.Definition.Name`). No builtin/custom split at the wire
    /// or storage level — that split isn't a type distinction anymore, it's a
    /// settings-driven, per-source *lookup* concern (see `lookup_property`),
    /// because no single fixed schema is guaranteed once a second source (e.g.
    /// IFC) can produce rooms alongside Revit. `#[serde(default)]` so a room
    /// with no properties still deserializes rather than failing.
    #[serde(default)]
    pub properties: BTreeMap<String, CustomValue>,
}

/// The human-meaningful container a model belongs to ("the hospital job").
/// Identity (`id`) is separated from display metadata (`name`) so a rename in
/// Revit never forks the stored record — storage keys on `id`, never `name`.
/// See STRATEGY.md "Identity".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Stable, globally-unique key. Addressable/comparable across projects
    /// without collision — a GUID-like value, NOT "project 1".
    pub id: String,
    /// Mutable display label. Never used as a storage key.
    pub name: String,
}

/// A single Revit file. One project routinely has several (architectural,
/// structural, linked consultant models), each POSTing independently — so
/// `model` is the level that stops those overwriting each other.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    /// The Revit model GUID: stable across renames, unique per file. Preferred
    /// over file name (which would fork the record on rename). This is the key
    /// the in-memory store buckets snapshots under.
    pub id: String,
    /// Mutable display label. Never used as a storage key.
    pub name: String,
    /// Which producer created this data ("revit" today). Selects which
    /// `BuiltinPropertyDef.by_source` entry resolves a canonical property name
    /// to *this* model's raw property name — the disambiguator a second source
    /// (e.g. IFC) would need, since the same canonical concept can live under a
    /// different raw name per source. A plain string, not a closed enum: adding
    /// a source is a settings-file change, not a Rust code change.
    pub source: String,
}

/// One timestamped push of one model. Its own contract level so "this floor as
/// it was last Tuesday" / "what changed since last push" become possible later
/// without restructuring — even though we only keep the latest for now.
///
/// Together with `schema_version` / `project` / `model` this forms the shared
/// **upload envelope**: the identity every upload type carries, rooms being
/// the first. Any future upload (FFE, etc.) associates back to room data by
/// exactly two keys — this snapshot id and the room id — so it must ride the
/// same envelope, resolved through the same `ensure_taken_at` /
/// `validate_snapshot_id` pair below rather than reimplementing either.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    /// The snapshot id: an RFC3339 date-time expressed in UTC. When the model
    /// was *read* (sourced from the export's own timestamp), not when the
    /// server received it — except when a producer leaves it blank/omitted, in
    /// which case the server mints one at ingest (`ensure_taken_at`) and
    /// returns it in the ingest response. Blank never survives past the ingest
    /// trust boundary; storage and read code always see a concrete id.
    #[serde(default)]
    pub taken_at: String,
}

/// Resolve a possibly-blank snapshot id at the ingest trust boundary: a
/// blank/whitespace `taken_at` (or one from an omitted `snapshot` object,
/// which defaults to empty) is replaced with "now" in UTC, at the same
/// microsecond precision the Revit producer stamps. Returns whether an id was
/// generated so the ingest response can say so. Every upload type resolves
/// its snapshot id through this one function.
pub fn ensure_taken_at(snapshot: &mut Snapshot) -> bool {
    if snapshot.taken_at.trim().is_empty() {
        snapshot.taken_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
        return true;
    }
    false
}

/// Whether a (non-blank) snapshot id is acceptable: it must parse as RFC3339
/// AND be expressed in UTC (`Z` or `+00:00`). One rule covers everything the
/// id must guarantee: it's a real date-time (the contract's definition of a
/// snapshot id), it keeps the store's lexical-max-is-newest ordering sound (a
/// non-UTC offset would sort wrongly against UTC neighbours), and it can't
/// smuggle a path escape (no RFC3339 string contains `/`, `\`, or `..`) —
/// which is why ingest needs no separate filename-safety check for it.
pub fn validate_snapshot_id(taken_at: &str) -> Result<(), String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(taken_at)
        .map_err(|e| format!("snapshot taken_at {taken_at:?} is not an RFC3339 date-time: {e}"))?;
    if parsed.offset().local_minus_utc() != 0 {
        return Err(format!(
            "snapshot taken_at {taken_at:?} must be expressed in UTC (\"Z\" or \"+00:00\"), not a local offset"
        ));
    }
    Ok(())
}

/// Normalize a phase name off the wire: trim surrounding whitespace, and treat
/// an all-whitespace name as absent. Every ingest path runs a pushed phase
/// through this before comparing or storing it, so a stored name and a compared
/// name can never disagree about their own edges.
///
/// Whitespace is export noise and absorbing it is uncontroversial. Collapsing
/// blank to `None` rather than keeping `Some("")` means there is exactly one
/// representation of "no phase", so the one-line absence check in
/// `phases_agree` cannot be fooled by a producer that sends `""`.
pub fn normalize_phase(phase: Option<&str>) -> Option<String> {
    phase.map(str::trim).filter(|p| !p.is_empty()).map(str::to_string)
}

/// Whether two phase names refer to the same Revit phase. `true` is
/// "compatible"; a `false` is what ingest turns into a rejected or quarantined
/// push (see PLAN-phasing.md "D5"/"D6").
///
/// Two rules, each load-bearing:
///
/// 1. **Either side absent ⇒ compatible.** An absent side constrains nothing:
///    a model stored before this field existed is *unphased*, and the first
///    phased push to it is what sets its phase rather than something to reject.
///    The ingest handler — not this function — decides whether an absent
///    *pushed* phase is legal at all; here it simply cannot disagree.
/// 2. **Trimmed and case-insensitive.** Trimming is `normalize_phase`'s job and
///    is repeated here so a caller that skipped it still gets the right answer.
///    Case is folded because a phase differing only in case is the same phase
///    typed twice, and quarantining a correct export over letter-case would be
///    a bad trade. `to_lowercase` rather than `eq_ignore_ascii_case`: phase
///    names are user-authored in the modeller's own language, and the ASCII
///    comparison would silently stop folding at the first non-ASCII character.
///
/// The name is the whole identity — there is no id to compare, and deliberately
/// so (see `RoomPayload::phase`).
pub fn phases_agree(pushed: Option<&str>, stored: Option<&str>) -> bool {
    match (pushed, stored) {
        (Some(a), Some(b)) => a.trim().to_lowercase() == b.trim().to_lowercase(),
        // Rule 1: an absent side constrains nothing.
        _ => true,
    }
}

/// The affine transform mapping a model's room points from Revit model space
/// into the project's SHARED coordinate system. One per model, not per room:
/// it's a model-level `ProjectLocation` fact (the *same* relationship on every
/// room), so it rides the envelope rather than each polygon.
///
/// It exists for two independent reasons: (a) it puts every room in a model into
/// one common frame, which cross-model comparison needs regardless of any map
/// (STRATEGY-SERVER "common coordinate frame"); (b) when the project is
/// survey-registered, shared space IS grid space in the declared CRS, which is
/// what later makes a map underlay placeable. It carries NO unit conversion —
/// this is a rigid-body placement (rotation + translation), not a scale, so
/// `|det|` of its linear part is ≈ 1 (a useful ingest sanity check).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ModelToShared {
    /// 2D affine as `[a, b, c, d, e, f]`: `shared_x = a*x + c*y + e`,
    /// `shared_y = b*x + d*y + f`. The linear part `[[a, c], [b, d]]` is a pure
    /// rotation from Revit's shared-coordinate `ProjectLocation` (no scale or
    /// shear), so `|det| = |a*d - c*b| ≈ 1`.
    pub matrix: [f64; 6],
}

impl ModelToShared {
    /// Determinant of the linear part `[[a, c], [b, d]]` = `a*d - c*b`. A pure
    /// rotation gives `|det| ≈ 1`; a value that has drifted means a scaled or
    /// sheared matrix that would silently distort placement.
    pub fn determinant(&self) -> f64 {
        let [a, b, c, d, _e, _f] = self.matrix;
        a * d - c * b
    }

    /// Whether the transform is a rigid-body placement (pure rotation), i.e.
    /// `|det| ≈ 1` within `tol`. Used at ingest to *warn* (not reject) — a
    /// non-rigid transform is advisory-suspect, not a broken contract.
    pub fn is_rigid(&self, tol: f64) -> bool {
        (self.determinant().abs() - 1.0).abs() <= tol
    }
}

/// Where a model's room boundaries sit relative to their walls — Revit's
/// `SpatialElementBoundaryLocation`, forwarded verbatim.
///
/// This is a **model fact, not a project policy**: Revit already knows it, and
/// asking a human to re-assert it in TOML duplicates an authoritative value and
/// invites getting it wrong. It rides the envelope per *model* rather than per
/// project because a project legitimately mixes both — each linked model
/// carries its own document setting.
///
/// It exists because `service::areas` otherwise has to *guess* which regime it
/// is looking at, and sizes its morphological close for the worst case. Every
/// footprint artifact chased so far — bevelled corners, 45° chamfers, the
/// million-foot spike, sibling overlaps — is downstream of that guess. Declaring
/// the regime does not merely improve the tolerance: on a centreline model the
/// close radius collapses to zero and the entire artifact class cannot arise.
/// The two regimes and what each implies are in `service::areas`' module
/// header; what the resulting number may be *called* is
/// STRATEGY-AREA-CALCULATION.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomBoundary {
    /// Neighbouring rooms tile edge-to-edge: their shared boundaries are
    /// coincident and the gap between them is zero up to float noise. Nothing
    /// needs bridging, and the walls are already inside the room polygons.
    Centreline,
    /// Rooms float inside their walls, so neighbours across a partition are
    /// separated by roughly its thickness. The gap is real and positive, and
    /// bridging it needs a declared thickness ceiling (`[areas]`
    /// `max_wall_thickness`).
    FinishFace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomPayload {
    pub schema_version: u32,

    /// v4 identity envelope. Tells the server *which* thing this snapshot is a
    /// version of, so two models POSTed to the same server no longer overwrite
    /// each other. The `(project, model)` pair locates the storage slot; the
    /// snapshot times it. `snapshot` is `#[serde(default)]`: omitting it (or
    /// its `taken_at`) asks the server to mint the id — see `ensure_taken_at`.
    pub project: Project,
    pub model: Model,
    #[serde(default)]
    pub snapshot: Snapshot,

    /// The Revit phase this push was filtered to, e.g. `"New Construction"`.
    ///
    /// **The first *authored* field on the envelope.** `model_to_shared` and
    /// `room_boundary` are both facts read off the document; this one is a
    /// choice the user makes at export time, because a document has many phases
    /// and only the user knows which is being pushed. Worth writing down: a
    /// future reader who assumes it can be read from the document, the way the
    /// other two envelope fields are, will "fix" this into something that
    /// cannot work.
    ///
    /// **A bare name, not an `{id, name}` pair.** Every document owns its own
    /// phases with its own `ElementId`s, so "New Construction" in the
    /// architectural model and in the structural model are different ids — the
    /// same problem `Level.id` has, and the reason `service::rooms` dedups
    /// levels across linked models. An id is therefore not comparable across
    /// models, would be carried for display only, and (in the sample export) may
    /// not even be an `ElementId` — the value `3` is low enough to be an index
    /// into `doc.Phases`. A field nothing reads and might be wrong is one that
    /// drifts, so there isn't one.
    ///
    /// **`Option` here does NOT mean optional on the wire.** A v6 push must
    /// carry a phase; ingest rejects one that doesn't. The `Option` exists
    /// because this same struct is what every stored snapshot deserializes back
    /// into, and every snapshot written before this field existed has no phase —
    /// making it required would stop the server hydrating its own store. So the
    /// type stays permissive and the ingest handler is strict. See
    /// PLAN-phasing.md "D2".
    ///
    /// The phase a *read* reports is always the one on the snapshot it loaded,
    /// never the lineage's current phase from the manifest — an old unfiltered
    /// snapshot must keep saying it was unphased, because it was.
    ///
    /// The server never decides what "exists in this phase" means: that is a
    /// range test over the document's phase ordering, and the extractor runs it
    /// because only the live document has the ordering.
    #[serde(default)]
    pub phase: Option<String>,

    /// Optional model→shared placement transform for this model (see
    /// `ModelToShared`). Absent on an un-placed model, which still renders fine
    /// via auto-fit exactly as before — `#[serde(default)]` keeps every
    /// pre-georeference payload valid and unchanged in meaning (no schema bump).
    #[serde(default)]
    pub model_to_shared: Option<ModelToShared>,

    /// Which boundary regime this model was drawn to (see `RoomBoundary`).
    /// Absent on any payload from an extractor predating the field — the Revit
    /// one now sends it, but a stored snapshot from before it did, or a future
    /// producer that cannot read the setting, still won't carry one —
    /// `#[serde(default)]` keeps every such payload valid and unchanged in
    /// meaning, so no schema bump, exactly as
    /// `model_to_shared` did. An absent value falls back to the project's
    /// `[areas] boundary_location`, and failing that to the conservative
    /// finish-face reading (a close still runs), which is today's behaviour.
    #[serde(default)]
    pub room_boundary: Option<RoomBoundary>,

    pub levels: Vec<Level>,
    pub rooms: Vec<Room>,
}

/// One model's block on a multi-model upload — the facts that are true of *one
/// Revit document* and cannot be shared across a push.
///
/// **This is the type that makes a combined push possible at all.** A run now
/// exports several models at once and sends them as one bucket, so identity can
/// no longer sit on the envelope: `levels` are keyed by per-document
/// `ElementId`s, `model_to_shared` places one document, and `room_boundary` is
/// one document's Area and Volume Computations setting. Merging any of them
/// across models would be merging things that only look alike.
///
/// What stays on the envelope above is what a *run* genuinely shares: the
/// project it targets, the one phase the user picked, and the moment it was
/// read.
///
/// `model` is flattened, so a block reads `{"id", "name", "source", "levels",
/// ...}` — the same keys the single-model envelope carried, one level down.
#[derive(Debug, Clone, Deserialize)]
pub struct RoomModelEnvelope {
    #[serde(flatten)]
    pub model: Model,
    /// Model→shared placement transform for *this* document (see
    /// `ModelToShared`), on the same optional terms `RoomPayload` states.
    #[serde(default)]
    pub model_to_shared: Option<ModelToShared>,
    /// This document's boundary regime, on the same optional terms
    /// `RoomPayload` states.
    #[serde(default)]
    pub room_boundary: Option<RoomBoundary>,
    pub levels: Vec<Level>,
}

impl RoomModelEnvelope {
    /// Rebuild the single-model `RoomPayload` this block plus the run's shared
    /// envelope describes — the **decomposition** every rooms ingest performs.
    ///
    /// A push carries N models; storage keeps N snapshots, one per model, keyed
    /// exactly as they always were. So the bucket is a *transport* shape and
    /// never a storage one: `ModelKey`, milestone pins, per-model QA scoping and
    /// comparison all keep working on a type this function did not change.
    ///
    /// Shared by the buffered and streaming routes so the two cannot drift on
    /// what a decomposed snapshot contains — the same discipline that keeps
    /// `validate_ingest` in one place.
    pub fn into_payload(
        self,
        schema_version: u32,
        project: Project,
        snapshot: Snapshot,
        phase: Option<String>,
        rooms: Vec<Room>,
    ) -> RoomPayload {
        RoomPayload {
            schema_version,
            project,
            model: self.model,
            snapshot,
            phase,
            model_to_shared: self.model_to_shared,
            room_boundary: self.room_boundary,
            levels: self.levels,
            rooms,
        }
    }
}

/// The first NDJSON line of a streamed push (`POST /rooms/stream`): the run's
/// shared identity plus one `RoomModelEnvelope` per model, with every room
/// arriving on a following line as a `StreamRoom`.
///
/// Kept as its own type (rather than making `rooms` optional somewhere) so the
/// envelope deserializes on its own with no rooms present, and so `RoomPayload`
/// keeps `rooms` guaranteed for every other consumer.
#[derive(Debug, Clone, Deserialize)]
pub struct StreamEnvelope {
    pub schema_version: u32,
    pub project: Project,
    #[serde(default)]
    pub snapshot: Snapshot,
    /// The push's phase — **one per run, not one per model**. `choose_phase`
    /// offers only names common to every selected document and each document
    /// resolves that name against its own phase table, so a run is scoped to one
    /// phase by construction and a per-model phase could only ever disagree with
    /// itself.
    #[serde(default)]
    pub phase: Option<String>,
    /// Every model this push carries, in no particular order. Ingest refuses an
    /// empty list and a duplicate id — see `handlers::validate_models`.
    pub models: Vec<RoomModelEnvelope>,
}

/// One room line of a streamed push: the room, plus the id of the model it
/// belongs to.
///
/// **The model id is on the element, not inferred from position.** A stream
/// could have grouped rooms by model and switched on a marker line, but then a
/// dropped or reordered line would silently file rooms under the wrong model —
/// and a room id is unique only within a model, so the result would resolve
/// against real-looking rooms rather than failing. Naming the model per line
/// costs a few bytes gzip removes anyway and makes that failure impossible.
///
/// Flattened, so a line is the room object it always was with one extra key.
#[derive(Debug, Clone, Deserialize)]
pub struct StreamRoom {
    pub model_id: String,
    #[serde(flatten)]
    pub room: Room,
}

/// The buffered multi-model rooms upload (`POST /rooms`) — the same shape the
/// stream sends, with each model's rooms inline rather than on their own lines.
///
/// Retained for fixture generation and small manual pushes, exactly as before;
/// the live Revit path streams. Both decompose through
/// `RoomModelEnvelope::into_payload`.
#[derive(Debug, Clone, Deserialize)]
pub struct RoomsUpload {
    pub schema_version: u32,
    pub project: Project,
    #[serde(default)]
    pub snapshot: Snapshot,
    #[serde(default)]
    pub phase: Option<String>,
    pub models: Vec<RoomModelUpload>,
}

/// One model's block on a buffered upload: its `RoomModelEnvelope` plus its
/// rooms.
#[derive(Debug, Clone, Deserialize)]
pub struct RoomModelUpload {
    #[serde(flatten)]
    pub envelope: RoomModelEnvelope,
    pub rooms: Vec<Room>,
}

/// Schema version this server accepts. Now v5: the fixed, typed `builtin`
/// struct is gone — `Room.properties` is one flat, source-native map, and
/// "which properties are builtin" moved from a Rust type to a settings-driven,
/// per-source name mapping (see `BuiltinPropertyDef` / `lookup_property`).
/// A v4 producer (split builtin/custom) 422s loud rather than silently
/// misparsing. No transition window — update the extractor and the server
/// together.
///
/// Still 5 after `snapshot.taken_at` became omittable: that change is a pure
/// relaxation — every payload that was valid v5 before is still valid and
/// means the same thing — and bumps are reserved for changes that would make
/// an existing producer's payload misparse or change meaning.
///
/// Still 5 after the optional `model_to_shared` envelope field was added:
/// same reasoning — it defaults to
/// `None`, so a pre-georeference payload stays valid and means exactly what it
/// did (an un-placed model, rendered via auto-fit).
///
/// Still 5 after the optional `room_boundary` envelope field
/// (see `RoomBoundary`) joined it, on the same
/// `model_to_shared` precedent: absent it defaults to `None`, and a payload
/// that omits it means exactly what it did before — a model whose regime the
/// server infers from project policy rather than reads.
///
/// **Now 6: `phase` is required at ingest.** The three additions above all held
/// the line at 5 by the same test — every payload that was valid before is
/// still valid and means what it meant — and `phase` would have passed that
/// test too, had it stayed optional. It did not: the extractor now always
/// resolves a phase, so a push arriving without one is a producer predating
/// phase support, whose rooms were never filtered by the phase range test and
/// are therefore unfiltered mixed-phase content. Rejecting it changes a
/// previously-valid payload's meaning from "a legal unphased push" to "an
/// error", which is exactly what a bump is for.
///
/// The bump is also the more useful failure: a stale producer is told its schema
/// is unsupported — which names the real problem, its extractor is old — rather
/// than that it forgot a field. No transition window, same as v4 → v5: update
/// the extractor and the server together.
///
/// This does not touch stored data. The version is checked at ingest only;
/// snapshots already on disk deserialize without a version check, and
/// `RoomPayload::phase` being `Option` is what keeps them readable. See
/// PLAN-phasing.md "D2".
///
/// **Now 7: one push carries many models.** The envelope's single `model` block
/// became a `models` list (`RoomModelEnvelope`), and each room names the model
/// it belongs to. A v6 payload no longer parses — the strongest form of the
/// bump test, and the right one: a run selects several documents, and sending
/// them one request at a time is what made a doors push depend on the order its
/// siblings arrived in.
///
/// The permissive-type / strict-handler split is unchanged and is what keeps
/// this cheap. `RoomPayload` — the **stored** type — is untouched: ingest
/// decomposes a push into one payload per model, so every snapshot on disk,
/// every milestone pin and every per-model read keeps working on the shape it
/// always had. Only the wire moved.
pub const SUPPORTED_SCHEMA: u32 = 7;

/// Resolve a *canonical* property name (e.g. "Area") to the source-specific
/// raw property name a room's `properties` map actually keys on, via
/// `builtin_defs`. Shared by `lookup_property` and `property_presence` so the
/// two can never disagree on what a canonical name resolves to.
///
/// When no `BuiltinPropertyDef` names `canonical_name`, or none of its
/// `by_source` entries match `source`, `canonical_name` is used verbatim as
/// the raw property name — this is what makes project/shared params (which
/// were never in the builtin set to begin with) work unchanged, and what lets
/// hierarchy/dRofus configs reference a raw name directly when no canonical
/// mapping is configured.
fn resolve_raw_name<'a>(canonical_name: &'a str, source: &str, builtin_defs: &'a [BuiltinPropertyDef]) -> &'a str {
    builtin_defs
        .iter()
        .find(|d| d.canonical == canonical_name)
        .and_then(|d| d.by_source.get(source))
        .map(String::as_str)
        .unwrap_or(canonical_name)
}

/// The three states an entity property can be in — distinguished because they
/// mean different things for data-quality reporting: `Absent` means the
/// property was never extracted from Revit for this entity at all (a mapping
/// typo or a parameter the extractor never wired up — a setup problem worth
/// flagging loudly), while `Empty` means the property exists but nobody has
/// filled in a value yet (an ordinary per-entity gap).
///
/// The distinction has to survive tiering (see `PropertyTiers`): "absent from
/// every tier" and "present but blank in every tier" stay different findings,
/// which is why `property_presence` accumulates rather than returning on the
/// first tier that carries the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyPresence {
    /// No property of the resolved raw name exists on this entity at all.
    Absent,
    /// The property exists but its value is an empty string.
    Empty,
    /// The property exists with a non-empty value.
    Present(String),
}

/// An entity's property maps in precedence order, highest first.
///
/// **This exists because a door has two tiers and a room has one.** A door
/// carries its own instance properties *and* its family type's, shared across
/// every instance of that type, and the two are different claims — "this leaf
/// is 820 wide" versus "every door of this type is 820 wide"
/// (see `contract::doors`). Flattening them into one map at the
/// contract level would lose that distinction permanently, so the tiers stay
/// separate on the type and this trait is how a *lookup* walks them.
///
/// Rejected: taking `&BTreeMap<String, CustomValue>` directly. It is the cheap
/// fix — these functions only ever needed the map — but a flat map cannot
/// express tier order, so doors would have needed their own parallel lookup and
/// the precedence rule would have lived in two places
/// (PLAN-generalisation.md R2).
pub trait PropertyTiers {
    /// This entity's property maps, **highest precedence first**.
    fn tiers(&self) -> Vec<&BTreeMap<String, CustomValue>>;
}

/// A room is single-tier: it has no type-level properties, so there is nothing
/// for a lookup to fall through to. Every pre-doors caller therefore keeps
/// exactly the behaviour it had.
impl PropertyTiers for Room {
    fn tiers(&self) -> Vec<&BTreeMap<String, CustomValue>> {
        vec![&self.properties]
    }
}

/// Look up an entity property by its *canonical* name, resolving it to the
/// source-specific raw property name first (see `resolve_raw_name`), then
/// reporting which of the three `PropertyPresence` states it's in across the
/// entity's tiers. Used where the absent/empty distinction matters
/// (data-quality reporting); most callers just want `lookup_property`'s
/// collapsed `Option<String>`.
///
/// **A tier wins only when it is `Present`.** Walking the tiers and taking the
/// first that merely *carries* the name would be plain shadowing — the
/// conventional Revit reading — and it is wrong on real data: in the sample
/// door export, `Door Leaf Thickness` is a blank instance parameter on 22 of 26
/// doors while the family type states `40.0`, so shadowing would hide the only
/// real value behind an empty one. A blank instance parameter is an unfilled
/// field, not an assertion that the type's value does not apply.
///
/// **A name in both tiers is not a finding.** The alternative reading — treat a
/// collision as a data-quality signal — was rejected against the same data:
/// `Workset` and `Edited by` collide on *every* door because Revit carries them
/// on instances and types alike, so the check would fire on 26 of 26 doors and
/// mean nothing. The tier separation is preserved by the two maps staying
/// two maps on the wire, not by making an overlap an error.
pub fn property_presence(
    entity: &impl PropertyTiers,
    canonical_name: &str,
    source: &str,
    builtin_defs: &[BuiltinPropertyDef],
) -> PropertyPresence {
    let raw_name = resolve_raw_name(canonical_name, source, builtin_defs);
    // `Empty` is remembered rather than returned, so a lower tier still gets
    // the chance to supply a real value — and so "blank everywhere" is still
    // reported as `Empty` rather than decaying to `Absent`.
    let mut seen_empty = false;
    for tier in entity.tiers() {
        match tier.get(raw_name) {
            None => continue,
            Some(v) if v.value.is_empty() => seen_empty = true,
            Some(v) => return PropertyPresence::Present(v.value.clone()),
        }
    }
    if seen_empty {
        PropertyPresence::Empty
    } else {
        PropertyPresence::Absent
    }
}

/// Look up an entity property by its *canonical* name (e.g. "Area"), resolving
/// it to the source-specific raw property name via `builtin_defs` before
/// reading the entity's property tiers. Used by both the dRofus join and the
/// classifier so the lookup strategy is consistent and lives in one place.
///
/// Returns `None` when the resolved property is absent or holds an empty
/// value — i.e. collapses `PropertyPresence::Absent`/`Empty` together. A thin
/// wrapper over `property_presence` so the two can never drift apart.
pub fn lookup_property(
    entity: &impl PropertyTiers,
    canonical_name: &str,
    source: &str,
    builtin_defs: &[BuiltinPropertyDef],
) -> Option<String> {
    match property_presence(entity, canonical_name, source, builtin_defs) {
        PropertyPresence::Present(v) => Some(v),
        PropertyPresence::Absent | PropertyPresence::Empty => None,
    }
}

/// IEEE 754 zero has two bit patterns (`0.0` and `-0.0`) that compare equal
/// numerically but format differently (`"-0"` vs `"0"`) -- collapse to the
/// positive form before formatting so a genuine zero never spuriously
/// mismatches itself.
fn normalize_zero(v: f64) -> f64 {
    if v == 0.0 {
        0.0
    } else {
        v
    }
}

/// Count the digits after the decimal point in a raw numeric string -- the
/// "stated precision" of a value as authored. This has to run on the string,
/// not the parsed `f64`: reformatting a parsed float loses (`"1.50"` ->
/// `1.5`) or fabricates (binary rounding noise) digits that were never part
/// of what was actually written.
fn decimal_places(s: &str) -> usize {
    s.trim().split_once('.').map_or(0, |(_, frac)| frac.len())
}

/// Compare two raw numeric strings tolerant of float-precision drift: round
/// both to the *lesser* of their two stated decimal precisions, rather than
/// to a fixed epsilon. This is what lets dRofus's `"1.5"` agree with Revit's
/// `"1.49999935417"` (a unit-conversion rounding artifact) -- dRofus only
/// stated 1 decimal digit of precision, so disagreement past that digit
/// isn't a real mismatch, whereas two values that both state 6 digits of
/// precision and differ in the 6th are a genuine disagreement.
///
/// Returns `None` when either side doesn't parse as a number at all; callers
/// should fall back to exact string comparison in that case.
pub fn numeric_match(a: &str, b: &str) -> Option<bool> {
    let x: f64 = a.trim().parse().ok()?;
    let y: f64 = b.trim().parse().ok()?;
    let n = decimal_places(a).min(decimal_places(b));
    let x = normalize_zero(x);
    let y = normalize_zero(y);
    Some(format!("{:.*}", n, x) == format!("{:.*}", n, y))
}

/// How a date string parsed, which decides how two sides can be compared
/// (see `date_match`).
enum ParsedDate {
    /// The pattern carried an offset (`%z`-family): a real instant.
    Zoned(chrono::DateTime<chrono::FixedOffset>),
    /// No offset in the pattern: a wall-clock reading with no timezone.
    Naive(chrono::NaiveDateTime),
}

/// Parse one side's raw string with its declared strftime pattern. Tries the
/// offset-aware form first (a pattern without `%z` never matches it), then
/// datetime, then bare date (midnight) — so one declaration covers whichever
/// granularity the column actually holds.
fn parse_date_side(s: &str, fmt: &str) -> Option<ParsedDate> {
    use chrono::{DateTime, NaiveDate, NaiveDateTime};
    if let Ok(dt) = DateTime::parse_from_str(s, fmt) {
        return Some(ParsedDate::Zoned(dt));
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
        return Some(ParsedDate::Naive(dt));
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
        return Some(ParsedDate::Naive(d.and_hms_opt(0, 0, 0).expect("midnight is always valid")));
    }
    None
}

/// Typed comparison for a date-declared field: parse both sides with their
/// declared patterns and compare what they *denote*, so two renderings of the
/// same moment don't count as a difference. Same `None = fall back` contract
/// as `numeric_match`: if either side fails to parse, the caller drops to the
/// string path — the declaration is a hint, not truth (the same stance
/// `CustomValue.storage_type` takes).
///
/// Comparison rule when the two sides differ in offset-awareness: two zoned
/// sides compare as instants; a zoned side against a naive side compares the
/// zoned side's *local* wall-clock reading against the naive one (the naive
/// side has no timezone to convert with, and its writer most plausibly wrote
/// local time); two naive sides compare directly.
///
/// **Symmetric by construction** — a value and a pattern per side, with no
/// notion of which side is dRofus and which is Revit. That is what lets two
/// unrelated callers share it: `validation` compares dRofus *against* Revit
/// (two different patterns), while `service::comparison` compares one dRofus
/// snapshot against another (the same pattern twice). Contrast
/// `validation::field_values_agree`, which is deliberately asymmetric and is
/// **not** reusable for a same-source diff.
pub fn date_match(left: &str, right: &str, left_fmt: &str, right_fmt: &str) -> Option<bool> {
    let a = parse_date_side(left.trim(), left_fmt)?;
    let b = parse_date_side(right.trim(), right_fmt)?;
    Some(match (a, b) {
        (ParsedDate::Zoned(a), ParsedDate::Zoned(b)) => a == b,
        (ParsedDate::Zoned(z), ParsedDate::Naive(n)) | (ParsedDate::Naive(n), ParsedDate::Zoned(z)) => {
            z.naive_local() == n
        }
        (ParsedDate::Naive(a), ParsedDate::Naive(b)) => a == b,
    })
}

/// Same rounding discipline as `numeric_match`, for a value that was never a
/// string to begin with (`Level.elevation` arrives as a parsed JSON number,
/// so any "stated precision" it once had is already gone by the time Rust
/// sees it). Approximated instead: format to a generous fixed precision, then
/// trim trailing zeros, so a value authored as a clean `0.0` collapses to 0
/// decimals while one carrying real float noise from a unit conversion keeps
/// a long non-zero tail. Falls back to exact equality on the vanishingly
/// unlikely chance both trimmed strings fail to parse.
pub fn elevation_match(a: f64, b: f64) -> bool {
    const PRECISION: usize = 9;
    let sa = format!("{:.*}", PRECISION, normalize_zero(a));
    let sb = format!("{:.*}", PRECISION, normalize_zero(b));
    numeric_match(sa.trim_end_matches('0'), sb.trim_end_matches('0')).unwrap_or(a == b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A v5 payload (identity envelope incl. `model.source`, plus a flat,
    /// source-native room properties map) survives a serde round-trip intact.
    #[test]
    fn test_v5_room_properties_round_trip() {
        let json = serde_json::json!({
            "schema_version": 6,
            "project":  { "id": "p1", "name": "Hospital Job" },
            "model":    { "id": "m-guid", "name": "ARCH", "source": "revit" },
            "snapshot": { "taken_at": "2026-05-09T11:13:34Z" },
            "levels": [{ "id": "lvl1", "name": "Level 1", "elevation": 0.0 }],
            "rooms": [{
                "id": "r1",
                "name": "Office",
                "level_id": "lvl1",
                "loops": [],
                "properties": {
                    "Number": { "value": "101", "storage_type": "String" },
                    "Area": { "value": "25.5", "storage_type": "Double" },
                    "Dept": { "value": "Finance", "storage_type": "String" }
                }
            }]
        });

        let payload: RoomPayload = serde_json::from_value(json).unwrap();
        let room = &payload.rooms[0];

        assert_eq!(payload.model.source, "revit");
        assert_eq!(room.properties["Number"].value, "101");
        assert_eq!(room.properties["Area"].value, "25.5");
        assert_eq!(room.properties["Dept"].storage_type, Some("String".to_string()));

        // Confirm round-trip: serialise and re-parse.
        let serialised = serde_json::to_string(&payload).unwrap();
        let reparsed: RoomPayload = serde_json::from_str(&serialised).unwrap();
        assert_eq!(reparsed.rooms[0].properties["Number"].value, "101");
    }

    /// A room JSON with no "properties" key deserialises to an empty map —
    /// proves the `#[serde(default)]` wiring is correct.
    #[test]
    fn test_room_deserialises_to_empty_properties() {
        let json = serde_json::json!({
            "id": "r1",
            "name": "Office",
            "level_id": "lvl1",
            "loops": []
            // no "properties" key
        });

        let room: Room = serde_json::from_value(json).unwrap();
        assert!(room.properties.is_empty());
    }

    /// `model_to_shared` round-trips on `RoomPayload`: present it deserializes
    /// into the affine and survives re-serialization; absent it defaults to
    /// `None` (the pre-georeference payload, unchanged in meaning).
    #[test]
    fn test_model_to_shared_round_trips_and_defaults_to_none() {
        let base = serde_json::json!({
            "schema_version": 6,
            "project":  { "id": "p1", "name": "Hospital Job" },
            "model":    { "id": "m-guid", "name": "ARCH", "source": "revit" },
            "snapshot": { "taken_at": "2026-05-09T11:13:34Z" },
            "levels": [],
            "rooms": []
        });

        // Absent → None.
        let without: RoomPayload = serde_json::from_value(base.clone()).unwrap();
        assert!(without.model_to_shared.is_none());

        // Present → the affine, and it round-trips.
        let mut with = base;
        with["model_to_shared"] = serde_json::json!({
            "matrix": [0.9704980833640151, -0.2411088347339701, 0.2411088347339701, 0.9704980833640151, 945737.6456106724, 20545096.538269494]
        });
        let payload: RoomPayload = serde_json::from_value(with).unwrap();
        let mts = payload.model_to_shared.expect("present");
        assert!((mts.matrix[4] - 945737.6456106724).abs() < 1e-6);

        // Survives a serialize→parse cycle (compared with tolerance: a JSON f64
        // round-trip can differ by an ULP between the `from_value` and
        // `from_str` paths, which is not what this test is about).
        let reparsed: RoomPayload = serde_json::from_str(&serde_json::to_string(&payload).unwrap()).unwrap();
        let back = reparsed.model_to_shared.expect("present after round-trip");
        for (a, b) in back.matrix.iter().zip(mts.matrix.iter()) {
            // Absolute 1e-6 (sub-micron in feet) absorbs the ULP-scale drift a
            // ~1e7 grid coordinate picks up crossing the JSON f64 boundary.
            assert!((a - b).abs() < 1e-6, "round-trip drifted: {a} vs {b}");
        }
    }

    /// `is_rigid` accepts the real geo_data.json rotation (a pure ~13.95° spin,
    /// |det| ≈ 1) and rejects a scaled matrix that would distort placement.
    #[test]
    fn test_model_to_shared_determinant_flags_non_rigid() {
        let rigid = ModelToShared {
            matrix: [
                0.9704980833640151,
                -0.2411088347339701,
                0.2411088347339701,
                0.9704980833640151,
                945737.6,
                20545096.5,
            ],
        };
        assert!((rigid.determinant() - 1.0).abs() < 1e-9);
        assert!(rigid.is_rigid(1e-6));

        // Identity is trivially rigid.
        assert!(ModelToShared { matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] }.is_rigid(1e-6));

        // A 2× scale on both axes: det = 4, not rigid.
        let scaled = ModelToShared { matrix: [2.0, 0.0, 0.0, 2.0, 0.0, 0.0] };
        assert!(!scaled.is_rigid(1e-6));
    }

    /// `room_boundary` follows `model_to_shared`'s contract exactly: absent it
    /// defaults to `None` (an extractor predating the field — still valid v5,
    /// unchanged in meaning), present it parses the snake_case wire spellings
    /// and survives a round-trip. Both variants are exercised because the
    /// centreline one is the case that collapses the close radius to zero.
    #[test]
    fn test_room_boundary_round_trips_and_defaults_to_none() {
        let base = serde_json::json!({
            "schema_version": 6,
            "project":  { "id": "p1", "name": "Hospital Job" },
            "model":    { "id": "m-guid", "name": "ARCH", "source": "revit" },
            "snapshot": { "taken_at": "2026-05-09T11:13:34Z" },
            "levels": [],
            "rooms": []
        });

        // Absent → None: every pre-declaration payload stays valid.
        let without: RoomPayload = serde_json::from_value(base.clone()).unwrap();
        assert!(without.room_boundary.is_none());

        for (wire, expected) in [
            ("centreline", RoomBoundary::Centreline),
            ("finish_face", RoomBoundary::FinishFace),
        ] {
            let mut with = base.clone();
            with["room_boundary"] = serde_json::json!(wire);
            let payload: RoomPayload = serde_json::from_value(with).unwrap();
            assert_eq!(payload.room_boundary, Some(expected));

            let reparsed: RoomPayload = serde_json::from_str(&serde_json::to_string(&payload).unwrap()).unwrap();
            assert_eq!(reparsed.room_boundary, Some(expected), "survives a round-trip");
        }
    }

    /// The streamed envelope carries `room_boundary` in lockstep with the
    /// buffered payload — the two ingest paths must store identical envelope
    /// facts, or which route a producer picked would change the geometry the
    /// areas service computes.
    #[test]
    fn test_stream_envelope_carries_room_boundary() {
        let mut json = serde_json::json!({
            "schema_version": 7,
            "project":  { "id": "p1", "name": "Hospital Job" },
            "models": [{ "id": "m-guid", "name": "ARCH", "source": "revit", "levels": [] }]
        });
        let envelope: StreamEnvelope = serde_json::from_value(json.clone()).unwrap();
        assert!(envelope.models[0].room_boundary.is_none());

        json["models"][0]["room_boundary"] = serde_json::json!("finish_face");
        let envelope: StreamEnvelope = serde_json::from_value(json).unwrap();
        assert_eq!(envelope.models[0].room_boundary, Some(RoomBoundary::FinishFace));
    }

    /// The regime is **per model**, not per push — a run legitimately mixes a
    /// centreline model with a finish-face one, and collapsing them onto the
    /// shared envelope would size one model's wall zone off the other's setting.
    #[test]
    fn test_each_model_declares_its_own_boundary() {
        let json = serde_json::json!({
            "schema_version": 7,
            "project":  { "id": "p1", "name": "Hospital Job" },
            "models": [
                { "id": "arch", "name": "ARCH", "source": "revit", "levels": [], "room_boundary": "centreline" },
                { "id": "struct", "name": "STR", "source": "revit", "levels": [], "room_boundary": "finish_face" },
            ]
        });
        let envelope: StreamEnvelope = serde_json::from_value(json).unwrap();
        assert_eq!(envelope.models[0].room_boundary, Some(RoomBoundary::Centreline));
        assert_eq!(envelope.models[1].room_boundary, Some(RoomBoundary::FinishFace));
    }

    /// `phase` rides the envelope as a bare name and survives a round-trip;
    /// absent it defaults to `None` — which on the *type* means "a snapshot
    /// stored before this field existed", not "a legal push". Ingest is what
    /// requires one (see `SUPPORTED_SCHEMA`), and that check is a handler
    /// concern, not this struct's.
    #[test]
    fn test_phase_round_trips_and_defaults_to_none() {
        let base = serde_json::json!({
            "schema_version": 6,
            "project":  { "id": "p1", "name": "Hospital Job" },
            "model":    { "id": "m-guid", "name": "ARCH", "source": "revit" },
            "snapshot": { "taken_at": "2026-05-09T11:13:34Z" },
            "levels": [],
            "rooms": []
        });

        // Absent → None: every snapshot already on disk stays readable.
        let without: RoomPayload = serde_json::from_value(base.clone()).unwrap();
        assert!(without.phase.is_none());

        let mut with = base;
        with["phase"] = serde_json::json!("New Construction");
        let payload: RoomPayload = serde_json::from_value(with).unwrap();
        assert_eq!(payload.phase.as_deref(), Some("New Construction"));

        let reparsed: RoomPayload = serde_json::from_str(&serde_json::to_string(&payload).unwrap()).unwrap();
        assert_eq!(reparsed.phase.as_deref(), Some("New Construction"), "survives a round-trip");
    }

    /// The streamed envelope carries `phase` in lockstep with the buffered
    /// payload — which ingest route a producer picked must never change what
    /// phase the stored snapshot claims. Same lockstep rule `room_boundary` has.
    #[test]
    fn test_stream_envelope_carries_phase() {
        let mut json = serde_json::json!({
            "schema_version": 7,
            "project":  { "id": "p1", "name": "Hospital Job" },
            "models": [{ "id": "m-guid", "name": "ARCH", "source": "revit", "levels": [] }]
        });
        let envelope: StreamEnvelope = serde_json::from_value(json.clone()).unwrap();
        assert!(envelope.phase.is_none());

        json["phase"] = serde_json::json!("New Construction");
        let envelope: StreamEnvelope = serde_json::from_value(json).unwrap();
        assert_eq!(envelope.phase.as_deref(), Some("New Construction"));
    }

    /// Whitespace is export noise, and an all-whitespace name is not a phase —
    /// collapsing it to `None` means there is exactly one representation of
    /// "no phase" for `phases_agree`'s absence check to recognise.
    #[test]
    fn test_normalize_phase_trims_and_collapses_blank_to_absent() {
        assert_eq!(normalize_phase(Some("  New Construction ")), Some("New Construction".to_string()));
        assert_eq!(normalize_phase(Some("")), None);
        assert_eq!(normalize_phase(Some("   ")), None);
        assert_eq!(normalize_phase(None), None);
    }

    /// The comparison the ingest check is built on: same phase typed with
    /// different whitespace or casing is the same phase. Asserted together so
    /// the two foldings read as one deliberate rule rather than two accidents.
    #[test]
    fn test_phases_agree_folds_whitespace_and_case() {
        assert!(phases_agree(Some("  New Construction "), Some("New Construction")));
        assert!(phases_agree(Some("NEW CONSTRUCTION"), Some("New Construction")));
        assert!(phases_agree(Some("new construction"), Some("New Construction")));
    }

    /// Case folding is Unicode-aware, not ASCII-only: phase names are authored
    /// in the modeller's own language, and `eq_ignore_ascii_case` would stop
    /// folding at the first non-ASCII character and call these two different
    /// phases.
    #[test]
    fn test_phases_agree_folds_non_ascii_case() {
        assert!(phases_agree(Some("ABBRÜCHE"), Some("Abbrüche")));
    }

    /// Two genuinely different phases must not be conflated — this is the
    /// disagreement that quarantines a push.
    #[test]
    fn test_phases_agree_rejects_different_names() {
        assert!(!phases_agree(Some("New Construction"), Some("Existing")));
    }

    /// An absent side constrains nothing, in either direction: an unphased
    /// model accepts the first phase it is told, which is what gives every
    /// model already on disk a migration path. Whether an absent *pushed*
    /// phase is legal at all is the ingest handler's call, not this function's.
    #[test]
    fn test_phases_agree_treats_absence_as_compatible() {
        assert!(phases_agree(Some("New Construction"), None));
        assert!(phases_agree(None, Some("New Construction")));
        assert!(phases_agree(None, None));
    }

    /// A `StreamEnvelope` (line 1 of a `/rooms/stream` push) deserializes with
    /// no `rooms` key present -- proves it doesn't accidentally require one.
    #[test]
    fn test_stream_envelope_deserializes_without_rooms() {
        let json = serde_json::json!({
            "schema_version": 7,
            "project":  { "id": "p1", "name": "Hospital Job" },
            "snapshot": { "taken_at": "2026-05-09T11:13:34Z" },
            "models": [{
                "id": "m-guid", "name": "ARCH", "source": "revit",
                "levels": [{ "id": "lvl1", "name": "Level 1", "elevation": 0.0 }]
            }]
        });

        let envelope: StreamEnvelope = serde_json::from_value(json).unwrap();
        assert_eq!(envelope.schema_version, SUPPORTED_SCHEMA);
        assert_eq!(envelope.project.id, "p1");
        assert_eq!(envelope.models[0].model.source, "revit");
        assert_eq!(envelope.models[0].levels.len(), 1);
        // No `model_to_shared` key present → defaults to None (in lockstep with
        // RoomPayload), so an un-placed streamed push stays valid.
        assert!(envelope.models[0].model_to_shared.is_none());
    }

    /// A room line names its model, and the room itself deserializes flat
    /// alongside that one extra key — so an element can never be filed under a
    /// model the push did not declare, and a reordered stream cannot silently
    /// misfile it.
    #[test]
    fn test_stream_room_carries_its_model_id() {
        let json = serde_json::json!({
            "model_id": "arch",
            "id": "r1", "name": "Ward 1", "level_id": "lvl1",
            "loops": [], "properties": {}
        });
        let line: StreamRoom = serde_json::from_value(json).unwrap();
        assert_eq!(line.model_id, "arch");
        assert_eq!(line.room.id, "r1");
        assert_eq!(line.room.name, "Ward 1");
    }

    /// A payload with no "snapshot" key at all still deserializes (the
    /// server-generates case) — `taken_at` arrives empty for `ensure_taken_at`
    /// to resolve.
    #[test]
    fn test_payload_deserializes_without_snapshot() {
        let stored = serde_json::json!({
            "schema_version": 7,
            "project":  { "id": "p1", "name": "Hospital Job" },
            "model":    { "id": "m-guid", "name": "ARCH", "source": "revit" },
            "levels": [],
            "rooms": []
        });
        let payload: RoomPayload = serde_json::from_value(stored).unwrap();
        assert_eq!(payload.snapshot.taken_at, "");

        // The same relaxation on the wire envelope, whose shape differs — one
        // `models` list rather than one `model` block.
        let wire = serde_json::json!({
            "schema_version": 7,
            "project":  { "id": "p1", "name": "Hospital Job" },
            "models": [{ "id": "m-guid", "name": "ARCH", "source": "revit", "levels": [] }]
        });
        let envelope: StreamEnvelope = serde_json::from_value(wire).unwrap();
        assert_eq!(envelope.snapshot.taken_at, "");
    }

    /// A blank/omitted taken_at is replaced with a generated UTC id that
    /// passes the contract's own validation; a supplied one is left alone.
    #[test]
    fn test_ensure_taken_at_generates_only_when_blank() {
        let mut blank = Snapshot { taken_at: "  ".to_string() };
        assert!(ensure_taken_at(&mut blank));
        assert!(
            validate_snapshot_id(&blank.taken_at).is_ok(),
            "generated id must be valid: {}",
            blank.taken_at
        );

        let mut supplied = Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() };
        assert!(!ensure_taken_at(&mut supplied));
        assert_eq!(supplied.taken_at, "2026-01-01T00:00:00Z");
    }

    /// The snapshot id rule: RFC3339, expressed in UTC. Non-dates (including
    /// anything path-shaped) and non-UTC offsets are rejected; "Z" and
    /// "+00:00" both count as UTC.
    #[test]
    fn test_validate_snapshot_id() {
        assert!(validate_snapshot_id("2026-01-01T00:00:00Z").is_ok());
        assert!(validate_snapshot_id("2026-01-01T00:00:00.123456Z").is_ok());
        assert!(validate_snapshot_id("2026-01-01T00:00:00+00:00").is_ok());

        assert!(validate_snapshot_id("2026-01-01T00:00:00+10:00").is_err());
        assert!(validate_snapshot_id("not-a-date").is_err());
        assert!(validate_snapshot_id("2026/01/01").is_err());
        assert!(validate_snapshot_id("..\\..\\evil").is_err());
        assert!(validate_snapshot_id("").is_err());
    }

    /// lookup_property resolves a canonical name to a source-specific raw
    /// property name before reading the room's map.
    #[test]
    fn test_lookup_property_resolves_via_source_mapping() {
        let mut properties = BTreeMap::new();
        properties.insert(
            "Fläche".to_string(),
            CustomValue { value: "25.5".to_string(), storage_type: Some("Double".to_string()) },
        );
        let room = Room {
            id: "r1".into(),
            name: "Office".into(),
            level_id: "lvl1".into(),
            loops: vec![],
            properties,
        };

        let defs = vec![BuiltinPropertyDef {
            canonical: "Area".to_string(),
            by_source: HashMap::from([("revit_de".to_string(), "Fläche".to_string())]),
        }];

        assert_eq!(lookup_property(&room, "Area", "revit_de", &defs), Some("25.5".to_string()));
        // A source with no configured mapping falls back to matching the
        // canonical name verbatim — and finds nothing here, correctly.
        assert_eq!(lookup_property(&room, "Area", "revit", &defs), None);
    }

    /// With no builtin_defs at all, lookup_property matches the raw property
    /// map directly by name — the same behaviour project/shared params always
    /// had, and what tests elsewhere (classify.rs) rely on.
    #[test]
    fn test_lookup_property_falls_through_with_no_defs() {
        let mut properties = BTreeMap::new();
        properties.insert("Dept".to_string(), CustomValue { value: "Finance".to_string(), storage_type: None });
        let room = Room {
            id: "r1".into(),
            name: "Office".into(),
            level_id: "lvl1".into(),
            loops: vec![],
            properties,
        };

        assert_eq!(lookup_property(&room, "Dept", "revit", &[]), Some("Finance".to_string()));
    }

    /// The reported bug: dRofus's `"1.5"` (1 stated decimal) agrees with
    /// Revit's `"1.49999935417"` (a unit-conversion rounding artifact) once
    /// both are rounded to the lesser of the two stated precisions.
    #[test]
    fn test_numeric_match_adaptive_precision() {
        assert_eq!(numeric_match("1.5", "1.49999935417"), Some(true));
    }

    /// Two values that both state 6 digits of precision and genuinely differ
    /// at that precision are a real mismatch, not noise to round away.
    #[test]
    fn test_numeric_match_genuine_disagreement_at_stated_precision() {
        assert_eq!(numeric_match("1.500001", "1.499999"), Some(false));
    }

    /// A value with no decimal point at all (0 stated decimals) forces
    /// whole-number comparison.
    #[test]
    fn test_numeric_match_integer_side_forces_whole_number_compare() {
        assert_eq!(numeric_match("150", "150.0000001"), Some(true));
        assert_eq!(numeric_match("150", "150.6"), Some(false));
    }

    /// Either side failing to parse as a number falls back to `None` so the
    /// caller knows to use exact string comparison instead.
    #[test]
    fn test_numeric_match_non_numeric_returns_none() {
        assert_eq!(numeric_match("Cardiology", "25.5"), None);
        assert_eq!(numeric_match("25.5", "Cardiology"), None);
    }

    /// `elevation_match` approximates stated precision from a bare `f64` by
    /// trimming trailing zeros off a fixed-precision format, rather than
    /// requiring a raw string.
    #[test]
    fn test_elevation_match_trims_float_noise() {
        // A "clean" 0.0 vs a value carrying float noise many decimals out.
        assert!(elevation_match(0.0, 0.000000000_1));
        assert!(elevation_match(12.0, 12.000000001));
        // 12.6, not 12.5 -- avoids depending on round-half-to-even tie-breaking.
        assert!(!elevation_match(12.0, 12.6));
    }

    /// Negative zero and positive zero must compare equal, not mismatch on
    /// their differing sign when formatted.
    #[test]
    fn test_elevation_match_negative_zero() {
        assert!(elevation_match(-0.0, 0.0));
    }

    /// `property_presence` distinguishes a property that was never extracted
    /// at all (`Absent` -- a mapping/setup problem) from one that exists but
    /// is blank (`Empty` -- an ordinary per-room gap), and reports a real
    /// value as `Present`.
    #[test]
    fn test_property_presence_distinguishes_absent_empty_present() {
        let mut properties = BTreeMap::new();
        properties.insert("Blank".to_string(), CustomValue { value: "".to_string(), storage_type: None });
        properties.insert("Filled".to_string(), CustomValue { value: "25.5".to_string(), storage_type: None });
        let room = Room {
            id: "r1".into(),
            name: "Office".into(),
            level_id: "lvl1".into(),
            loops: vec![],
            properties,
        };

        assert_eq!(property_presence(&room, "Missing", "revit", &[]), PropertyPresence::Absent);
        assert_eq!(property_presence(&room, "Blank", "revit", &[]), PropertyPresence::Empty);
        assert_eq!(
            property_presence(&room, "Filled", "revit", &[]),
            PropertyPresence::Present("25.5".to_string())
        );
    }

    /// A stand-in for the two-tier entity doors will be: an instance map that
    /// takes precedence over a type map. Defined here rather than waiting for
    /// `Opening` so the tier *rule* is pinned by the change that introduces it —
    /// the rule is a contract decision, and a decision with no test is one the
    /// next reader is free to re-derive differently.
    struct TwoTier {
        instance: BTreeMap<String, CustomValue>,
        type_properties: BTreeMap<String, CustomValue>,
    }

    impl PropertyTiers for TwoTier {
        fn tiers(&self) -> Vec<&BTreeMap<String, CustomValue>> {
            vec![&self.instance, &self.type_properties]
        }
    }

    fn two_tier(instance: &[(&str, &str)], type_properties: &[(&str, &str)]) -> TwoTier {
        let map = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), CustomValue { value: v.to_string(), storage_type: None }))
                .collect()
        };
        TwoTier { instance: map(instance), type_properties: map(type_properties) }
    }

    /// The precedence rule: a higher tier wins when it has a real value, and
    /// the lower tier is never consulted for a name the instance already
    /// answers.
    #[test]
    fn test_property_presence_prefers_the_higher_tier() {
        let door = two_tier(&[("Workset", "1258")], &[("Workset", "4411")]);
        assert_eq!(
            property_presence(&door, "Workset", "revit", &[]),
            PropertyPresence::Present("1258".to_string())
        );
    }

    /// **The case that decided the rule.** `Door Leaf Thickness` is a blank
    /// instance parameter on 22 of the 26 doors in the sample export while the
    /// family type states a real value — so plain shadowing (first tier that
    /// carries the name wins) would hide `40.0` behind an empty string on
    /// almost every door. A blank instance parameter is an unfilled field, not
    /// a claim that the type's value does not apply.
    #[test]
    fn test_blank_higher_tier_does_not_shadow_a_real_lower_tier_value() {
        let door = two_tier(&[("Door Leaf Thickness", "")], &[("Door Leaf Thickness", "40.0")]);
        assert_eq!(
            property_presence(&door, "Door Leaf Thickness", "revit", &[]),
            PropertyPresence::Present("40.0".to_string())
        );
    }

    /// The `Absent`/`Empty` distinction has to survive tiering, or the QA
    /// report loses the difference between "nobody wired this parameter up"
    /// and "nobody has filled it in". Blank in every tier is `Empty`; missing
    /// from every tier is `Absent` — and blank in one tier with the other
    /// silent is still `Empty`, not `Absent`.
    #[test]
    fn test_tiering_preserves_absent_versus_empty() {
        let blank_both = two_tier(&[("Finish", "")], &[("Finish", "")]);
        assert_eq!(property_presence(&blank_both, "Finish", "revit", &[]), PropertyPresence::Empty);

        let blank_instance_only = two_tier(&[("Finish", "")], &[]);
        assert_eq!(property_presence(&blank_instance_only, "Finish", "revit", &[]), PropertyPresence::Empty);

        let blank_type_only = two_tier(&[], &[("Finish", "")]);
        assert_eq!(property_presence(&blank_type_only, "Finish", "revit", &[]), PropertyPresence::Empty);

        let neither = two_tier(&[("Other", "x")], &[("Another", "y")]);
        assert_eq!(property_presence(&neither, "Finish", "revit", &[]), PropertyPresence::Absent);
    }

    /// Canonical-name resolution runs *before* the tier walk, so one resolved
    /// raw name is looked for in every tier — a canonical name must not mean
    /// one property on the instance and another on the type.
    #[test]
    fn test_canonical_resolution_applies_to_every_tier() {
        let door = two_tier(&[], &[("Fläche", "25.5")]);
        let defs = vec![BuiltinPropertyDef {
            canonical: "Area".to_string(),
            by_source: HashMap::from([("revit_de".to_string(), "Fläche".to_string())]),
        }];
        assert_eq!(lookup_property(&door, "Area", "revit_de", &defs), Some("25.5".to_string()));
    }

    /// A room is single-tier, which is what makes R2 a no-op for every
    /// pre-doors caller — the walk has exactly one map to look in.
    #[test]
    fn test_room_exposes_exactly_one_tier() {
        let room = Room {
            id: "r1".into(),
            name: "Office".into(),
            level_id: "lvl1".into(),
            loops: vec![],
            properties: BTreeMap::new(),
        };
        assert_eq!(room.tiers().len(), 1);
    }

    /// `lookup_property`'s existing collapsed behavior must survive the
    /// refactor onto `property_presence` unchanged: both `Absent` and `Empty`
    /// read as `None`.
    #[test]
    fn test_lookup_property_still_collapses_absent_and_empty_to_none() {
        let mut properties = BTreeMap::new();
        properties.insert("Blank".to_string(), CustomValue { value: "".to_string(), storage_type: None });
        let room = Room {
            id: "r1".into(),
            name: "Office".into(),
            level_id: "lvl1".into(),
            loops: vec![],
            properties,
        };

        assert_eq!(lookup_property(&room, "Missing", "revit", &[]), None);
        assert_eq!(lookup_property(&room, "Blank", "revit", &[]), None);
    }
}
