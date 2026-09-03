//! HTTP handlers for `/rooms`, `/projects`, and validation: thin Axum
//! adapters over `service/`. Each handler extracts its own input form, calls
//! exactly one `service` function, and translates the result into HTTP --
//! `StatusCode`, `Query`, `Path`, `Json` never leak past this file.
//!
//! Ingest (`ingest_rooms` / `ingest_rooms_stream`) is the exception: it has no
//! derive logic worth sharing with the MCP server (which deliberately exposes
//! no ingest -- see `src/bin/mcp.rs`), so it stays here in full.

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncBufReadExt;
use tokio_util::io::StreamReader;

use std::collections::BTreeSet;

use crate::contract::{
    DoorModelEnvelope, DoorStreamEnvelope, DoorsUpload, ModelToShared, Opening, Project, Room, RoomBoundary,
    RoomModelEnvelope, RoomPayload, RoomsUpload, Snapshot, StreamDoor, StreamEnvelope, StreamRoom,
    SUPPORTED_DOOR_SCHEMA, SUPPORTED_SCHEMA,
};
use crate::service::adjacency;
use crate::service::areas;
use crate::service::comparison::{self, ComparisonResponse};
use crate::service::milestones::MilestonesResponse;
use crate::service::projects::{BuildingsResponse, ProjectSummary};
use crate::service::reference::{ReferenceSnapshotInfo, ReferenceSnapshotList};
use crate::service::snapshots::{LatestSnapshot, PendingSnapshot, ProjectSnapshotsResponse};
use crate::service::validation::ValidationResponse;
use crate::service::{
    doors, milestones, projects, reference, rooms, scope_cursor, snapshots, validation, ServiceError,
};
use crate::state::{ModelKey, Shared, StreamingSnapshot};
use crate::storage::SnapshotKind;

/// Reject a project/model id that can't safely become a filesystem path
/// component. `FsStore` builds paths as `root/<project_id>/<model_id>` straight
/// from these ids, and the client currently sends the Revit document *title*
/// as the model id — a title containing `/`, `\`, or `..` would be a path
/// traversal out of the storage root. Same startup-loud spirit as settings
/// validation, applied at the ingest trust boundary; shared by both ingest
/// handlers, and the predicate itself (`state::is_path_safe_component`) is
/// shared with the settings API so the two agree on what a safe id is.
fn validate_id(kind: &str, id: &str) -> Result<(), (StatusCode, String)> {
    if !crate::state::is_path_safe_component(id) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{kind} id {id:?} is empty or contains characters unsafe for storage paths"),
        ));
    }
    Ok(())
}

/// Every pre-flight check both ingest routes share, in one place so the
/// buffered and streaming paths can't drift on what gets rejected:
/// - a schema version this server doesn't speak;
/// - a project with no registered settings — rejected rather than lazily
///   accepted, pairing with `assemble_rooms`'s "skip on read" policy: a
///   project must be explicitly onboarded (a settings file registered under
///   its id, or an explicit `is_default` fallback) before it can push at all;
/// - identity ids unsafe as storage path components (`validate_id`);
/// - a `taken_at` that isn't an RFC3339 UTC date-time (`validate_taken_at`).
///
/// Callers resolve a blank `taken_at` (`contract::ensure_taken_at`) BEFORE
/// this runs, so the id checked here is always the one the store will key on.
///
/// Takes already-parsed identity fields (not a payload) so the streaming
/// route can run it from the envelope line alone, before reading any rooms —
/// and so the doors routes can share it despite carrying a different payload
/// type and their own, independently-versioned `supported_schema`.
///
/// **Project-level only.** A push now carries many models, so the per-model half
/// (`validate_id("model", ..)`) moved to `validate_models`, which sees the whole
/// list and can therefore also catch the one fault a single id never could — the
/// same model declared twice.
fn validate_ingest(
    state: &Shared,
    schema_version: u32,
    supported_schema: u32,
    project_id: &str,
    taken_at: &str,
) -> Result<(), (StatusCode, String)> {
    if schema_version != supported_schema {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("schema_version {schema_version} not supported; this server speaks {supported_schema}"),
        ));
    }
    if state.settings().settings_for(project_id).is_none() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("no settings configured for project '{project_id}'"),
        ));
    }
    validate_id("project", project_id)?;
    validate_taken_at(taken_at)
}

/// Check the `models` list a multi-model push declares, and answer with the set
/// of ids so the element loop can reject a line naming a model that isn't there.
///
/// Three faults, all of them producer bugs:
///
/// - **an empty list** — a push exists because a run exported something, so
///   declaring no models at all is the multi-model form of the empty-push fault
///   `reject_empty_rooms` catches per model;
/// - **an unsafe id** — `FsStore` builds paths straight from these, so this is
///   the same trust boundary `validate_id` has always guarded, applied per model;
/// - **a duplicate id** — two blocks claiming one model. Whichever stored last
///   would win and the other's rooms would vanish silently, so it is refused
///   rather than merged: merging would be guessing which of two `levels` lists
///   or `model_to_shared` transforms the producer meant.
///
/// Runs off the envelope line, before any element is read, on the same terms as
/// everything else here: a push that will be refused should cost its producer
/// one line, not the whole upload.
fn validate_models<'a>(ids: impl Iterator<Item = &'a str>) -> Result<BTreeSet<String>, (StatusCode, String)> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for id in ids {
        validate_id("model", id)?;
        if !seen.insert(id.to_string()) {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("push declares model {id:?} more than once; each model may appear at most once"),
            ));
        }
    }
    if seen.is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "push declares no models. A push exists because a run exported at least one document — \
             an empty model list is a producer fault, not an empty run."
                .to_string(),
        ));
    }
    Ok(seen)
}

/// Reject an element line naming a model the envelope never declared.
///
/// The line's `model_id` is what files a room or door under a lineage, so an
/// undeclared one has no `levels`, no `model_to_shared` and no boundary regime
/// to be stored against. Refused rather than invented: a room id is unique only
/// within a model, so quietly filing it anywhere else would resolve against
/// real-looking rooms instead of failing.
fn check_declared(declared: &BTreeSet<String>, model_id: &str) -> Result<(), (StatusCode, String)> {
    if declared.contains(model_id) {
        return Ok(());
    }
    Err((
        StatusCode::BAD_REQUEST,
        format!(
            "element names model {model_id:?}, which this push's envelope does not declare (it declares: {})",
            declared.iter().cloned().collect::<Vec<_>>().join(", ")
        ),
    ))
}

/// A rooms push must carry at least one room.
///
/// **This is a producer fault every time, and it used to be stored in silence.**
/// A push exists because someone ran an export against a document that has rooms
/// in it; a payload with none means the export produced nothing, not that the
/// model is empty. On 2026-08-03 a phase filter that matched nothing sent five
/// consecutive zero-room pushes, and every one was accepted, indexed and served
/// as "this model has no rooms" — the only signal was a person noticing the
/// drawing was blank.
///
/// Deliberately **not** the same as an empty *level*, which is ordinary and
/// stays a first-class state the viewer names ("LEVEL 02 has no rooms"). The
/// distinction is whole-payload versus per-level, and only the first is
/// impossible to arrive at honestly.
///
/// A 422 rather than a quarantine: quarantine exists so a *differently-phased*
/// push can be promoted later (PLAN-phasing D6), and there is nothing here worth
/// promoting. Same reasoning that makes an unphased push a hard reject.
///
/// Doors get no equivalent rule: a model with rooms and no doors is a shell, a
/// pre-fit-out phase, or simply a floor without any — all of them legitimate.
fn reject_empty_rooms(
    room_count: usize,
    project_id: &str,
    model_id: &str,
    phase: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    if room_count > 0 {
        return Ok(());
    }
    // Name the phase, because the filter matching nothing is what this is,
    // nearly every time — and "0 rooms" on its own sends people looking at the
    // server.
    let scope = match phase {
        Some(p) => format!("in phase '{p}'"),
        None => "with no phase declared".to_string(),
    };
    Err((
        StatusCode::UNPROCESSABLE_ENTITY,
        format!(
            "push for {project_id}/{model_id} contains no rooms {scope}. A push with an empty room \
             list is a producer fault, not an empty model — check that the phase filter matched \
             something before pushing."
        ),
    ))
}

/// The snapshot id rule lives in the contract (`validate_snapshot_id`:
/// RFC3339, expressed in UTC); this is just its 422 adapter. The date-time
/// requirement subsumes the old per-character filename-safety checks — no
/// RFC3339 string can contain `/`, `\`, or `..` — and the store still
/// sanitises the one filename-hostile character it does contain (`:`) before
/// filesystem use.
fn validate_taken_at(taken_at: &str) -> Result<(), (StatusCode, String)> {
    crate::contract::validate_snapshot_id(taken_at).map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e))
}

/// What ingest decided to do with a push once its phase has been checked
/// against the model's lineage.
enum PhaseDecision {
    /// Store it as a normal, live snapshot.
    Accept,
    /// Quarantine it (`put_pending`) and answer 202: the lineage is phased and
    /// this push names a different phase. Carries the lineage's phase so the
    /// response can say what it disagreed with.
    Quarantine { lineage: String },
}

/// Apply the phase half of the ingest contract. The `schema_version` check in
/// `validate_ingest` runs first and catches a stale producer before this is
/// reached; this is what decides between live, quarantined, and refused.
///
/// The full table (PLAN-phasing.md "P3"):
///
/// | pushed | lineage | result |
/// | --- | --- | --- |
/// | none | *any* | 422 — a producer predating phase support |
/// | some | none | accept; `put` records the lineage's phase |
/// | some | some, agree | accept |
/// | some | some, differ | quarantine, 202 |
///
/// **The two failures are deliberately asymmetric.** A differently-phased push
/// is a correct export of a *different* phase — real, filtered data the user may
/// want to switch the model to, so it is kept and made promotable. A push with
/// no phase at all was never filtered by the phase range test, so it is
/// unfiltered mixed-phase content: there is nothing worth activating, and
/// offering to activate it would be offering to corrupt the model. Hence reject,
/// never quarantine.
///
/// Takes the already-normalized phase (`contract::normalize_phase`), so a
/// blank name has collapsed to `None` and reads as the stale-producer case
/// rather than sneaking through as a phase named "".
fn decide_phase(state: &Shared, key: &ModelKey, pushed: Option<&str>) -> Result<PhaseDecision, (StatusCode, String)> {
    let Some(pushed) = pushed else {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "push for {}/{} carries no phase, which indicates an extractor predating phase support; \
                 its elements were never filtered to a phase. Update the extractor.",
                key.project_id, key.model_id
            ),
        ));
    };

    let lineage = state.model_phase(key).map_err(|e| {
        tracing::error!("failed to read lineage phase: {e:#}");
        (StatusCode::INTERNAL_SERVER_ERROR, format!("could not read model phase: {e}"))
    })?;

    match lineage {
        // First phased push to this lineage — `put` records it from the payload.
        None => Ok(PhaseDecision::Accept),
        Some(stored) if crate::contract::phases_agree(Some(pushed), Some(&stored)) => Ok(PhaseDecision::Accept),
        Some(stored) => Ok(PhaseDecision::Quarantine { lineage: stored }),
    }
}

/// Tolerance for the `model_to_shared` rigidity check: the linear part should be
/// a pure rotation (`|det| ≈ 1`). Generous — its only job is to catch a matrix
/// that has silently picked up scale/shear, not to police float noise.
const MODEL_TO_SHARED_DET_TOL: f64 = 1e-6;

/// Warn (never reject) when a push carries a `model_to_shared` whose linear part
/// isn't a pure rotation — a scaled/sheared transform would silently distort
/// placement. This is advisory only -- the underlay is non-load-bearing, and
/// this is "signal, not error": the geometry still stores and renders, so a
/// 422 would be wrong here. A missing transform is the normal
/// un-placed case and warns nothing.
fn warn_on_transform_drift(model_to_shared: Option<&ModelToShared>, project_id: &str, model_id: &str) {
    if let Some(m) = model_to_shared
        && !m.is_rigid(MODEL_TO_SHARED_DET_TOL)
    {
        tracing::warn!(
            "model_to_shared for {project_id}/{model_id} is not a pure rotation \
             (|det| = {:.6}, expected ≈ 1); placement may be distorted",
            m.determinant().abs()
        );
    }
}

/// Revit posts room data here — one push, one or more models. Returns 200 with
/// a per-model summary, or 422 if the payload fails any pre-flight check. A
/// blank/omitted snapshot id is minted server-side first (`ensure_taken_at`);
/// the response always carries the resolved id so the pusher can attach
/// follow-up uploads to it.
///
/// **Every model in one push shares one `taken_at`**, which is a property worth
/// having rather than an implementation detail: it is what makes "these
/// documents were read together" expressible, and before the bucket existed
/// there was no way to say it — N separate pushes got N timestamps minutes
/// apart.
pub async fn ingest_rooms(
    State(state): State<Shared>,
    Json(mut upload): Json<RoomsUpload>,
) -> Result<(StatusCode, Json<IngestResponse>), (StatusCode, String)> {
    let snapshot_id_generated = crate::contract::ensure_taken_at(&mut upload.snapshot);
    upload.phase = crate::contract::normalize_phase(upload.phase.as_deref());
    validate_ingest(
        &state,
        upload.schema_version,
        SUPPORTED_SCHEMA,
        &upload.project.id,
        &upload.snapshot.taken_at,
    )?;
    validate_models(upload.models.iter().map(|m| m.envelope.model.id.as_str()))?;

    let mut rooms_by_model: Vec<(String, Vec<Room>)> = upload
        .models
        .iter_mut()
        .map(|m| (m.envelope.model.id.clone(), std::mem::take(&mut m.rooms)))
        .collect();
    let models: Vec<RoomModelEnvelope> = upload.models.into_iter().map(|m| m.envelope).collect();
    let decisions = preflight_rooms(&state, &upload.project.id, upload.phase.as_deref(), &models)?;

    // Through the same sinks the streamed route feeds, one room at a time. This
    // route already holds them all, so it gains nothing from streaming -- what it
    // gains is that there is one storage path, and a buffered snapshot and a
    // streamed one cannot come out different.
    let mut sinks = open_room_sinks(
        &state,
        upload.schema_version,
        &upload.project,
        &upload.snapshot,
        upload.phase.as_deref(),
        models,
        decisions,
    )?;
    for (model_id, rooms) in &mut rooms_by_model {
        let sink = sink_for(&mut sinks, model_id, |s| s.model_id.as_str()).expect("every declared model has a sink");
        for room in rooms.drain(..) {
            sink.push(room)?;
        }
    }
    finish_rooms(
        &state,
        &upload.project,
        upload.phase.as_deref(),
        upload.snapshot.taken_at.clone(),
        snapshot_id_generated,
        sinks,
    )
}

/// Decide every model's fate from the envelope alone, before a single element is
/// read.
///
/// **This is what makes a doomed push cost one line instead of the whole
/// upload**, and it is why the phase decision does not simply live inside
/// `store_rooms`: the streaming route has to be able to answer 422 before it
/// starts reading a body it is going to throw away. Both routes call it, so the
/// buffered path cannot decide anything differently.
///
/// Returns one `PhaseDecision` per model, in declaration order.
fn preflight_rooms(
    state: &Shared,
    project_id: &str,
    phase: Option<&str>,
    models: &[RoomModelEnvelope],
) -> Result<Vec<PhaseDecision>, (StatusCode, String)> {
    models
        .iter()
        .map(|envelope| {
            let key = ModelKey { project_id: project_id.to_string(), model_id: envelope.model.id.clone() };
            let decision = decide_phase(state, &key, phase)?;
            warn_on_transform_drift(envelope.model_to_shared.as_ref(), project_id, &envelope.model.id);
            Ok(decision)
        })
        .collect()
}

/// Decompose a rooms push into one stored snapshot per model, and report each
/// one's outcome.
///
/// Shared by the buffered and streaming routes, on the same terms as
/// `validate_ingest` and `store_or_quarantine`: which transport a producer
/// picked must never change what gets stored, and two copies of the
/// decomposition could drift on the order of the phase check, the empty-rooms
/// check and the store.
///
/// **Every model is decided before any is stored.** A model whose phase
/// disagrees with its lineage is quarantined, which is not a failure and must
/// not stop its siblings; but a model contributing no rooms is a 422, and that
/// one has to fire before anything is written, or a bad push would leave half a
/// run live and half refused.
///
/// Takes the decisions `preflight_rooms` already made rather than making them
/// here, so the streaming route can refuse a doomed push from its envelope line
/// alone.
#[allow(clippy::too_many_arguments)]
fn open_room_sinks<'a>(
    state: &'a Shared,
    schema_version: u32,
    project: &Project,
    snapshot: &Snapshot,
    phase: Option<&str>,
    models: Vec<RoomModelEnvelope>,
    decisions: Vec<PhaseDecision>,
) -> Result<Vec<RoomSink<'a>>, (StatusCode, String)> {
    let mut sinks = Vec::with_capacity(models.len());
    for (envelope, decision) in models.into_iter().zip(decisions) {
        let model_id = envelope.model.id.clone();
        let room_boundary = resolved_boundary(state, &project.id, envelope.room_boundary);
        // The payload with an EMPTY rooms list. For a live model that is the
        // envelope the snapshot's header is written from; for a quarantined one
        // it is the payload its rooms accumulate into.
        let payload = envelope.into_payload(
            schema_version,
            project.clone(),
            snapshot.clone(),
            phase.map(str::to_string),
            Vec::new(),
        );
        let dest = match decision {
            PhaseDecision::Accept => RoomDest::Live(state.open_room_snapshot(&payload).map_err(store_failed)?),
            PhaseDecision::Quarantine { lineage } => RoomDest::Quarantined { payload: Box::new(payload), lineage },
        };
        sinks.push(RoomSink { model_id, room_boundary, dest });
    }
    Ok(sinks)
}

/// Where one model's rooms go while a push is being read.
///
/// **A live model streams; a quarantined one buffers**, and the asymmetry is
/// deliberate rather than an omission. Streaming exists to keep a large payload
/// off the heap on its way to disk, and quarantine is at most one push per
/// model, kept only so a human can promote it. Giving it a streaming path too
/// would mean a second streaming entry point on `SnapshotStore` — a parallel
/// method set, which is what the bytes-at-the-boundary rule exists to prevent —
/// in exchange for memory on the one push nobody is reading.
enum RoomDest<'a> {
    Live(StreamingSnapshot<'a>),
    /// Boxed: a `RoomPayload` is an order of magnitude larger than a writer
    /// handle, and every live model -- the overwhelming majority -- would
    /// otherwise carry that size for a variant it never uses.
    Quarantined {
        payload: Box<RoomPayload>,
        lineage: String,
    },
}

/// One model's destination plus what the response owes about it.
struct RoomSink<'a> {
    model_id: String,
    room_boundary: RoomBoundary,
    dest: RoomDest<'a>,
}

impl RoomSink<'_> {
    /// Take one room. Both routes feed sinks through here, so the buffered and
    /// streamed paths cannot store different things.
    fn push(&mut self, room: Room) -> Result<(), (StatusCode, String)> {
        match &mut self.dest {
            RoomDest::Live(snapshot) => snapshot.push(&room).map_err(store_failed),
            RoomDest::Quarantined { payload, .. } => {
                payload.rooms.push(room);
                Ok(())
            }
        }
    }

    fn count(&self) -> usize {
        match &self.dest {
            RoomDest::Live(snapshot) => snapshot.count(),
            RoomDest::Quarantined { payload, .. } => payload.rooms.len(),
        }
    }
}

/// Check every sink, then publish every sink — and report the push.
///
/// **The two loops are not one loop.** `reject_empty_rooms` can only run once a
/// model's rooms have been counted, which is after writing has begun; doing the
/// check and the commit together would publish the models before the offending
/// one and refuse the rest, leaving half a run live. Checking all of them first
/// means a refusal drops every sink uncommitted, and a dropped sink leaves no
/// trace — see `SnapshotWriter`.
fn finish_rooms(
    state: &Shared,
    project: &Project,
    phase: Option<&str>,
    snapshot_taken_at: String,
    snapshot_id_generated: bool,
    sinks: Vec<RoomSink<'_>>,
) -> Result<(StatusCode, Json<IngestResponse>), (StatusCode, String)> {
    for sink in &sinks {
        // A model the envelope declared but that contributed nothing: the
        // producer said it exported this document and then sent none of it.
        reject_empty_rooms(sink.count(), &project.id, &sink.model_id, phase)?;
    }

    let mut results = Vec::with_capacity(sinks.len());
    let mut total = 0usize;
    let mut any_quarantined = false;
    for sink in sinks {
        let count = sink.count();
        total += count;
        let RoomSink { model_id, room_boundary, dest } = sink;
        let key = ModelKey { project_id: project.id.clone(), model_id: model_id.clone() };
        let quarantined = match dest {
            RoomDest::Live(snapshot) => {
                snapshot.commit().map_err(store_failed)?;
                None
            }
            RoomDest::Quarantined { payload, lineage } => {
                let pushed = payload.phase.clone().unwrap_or_default();
                state.set_pending_snapshot(&key, &payload).map_err(store_failed)?;
                tracing::warn!(
                    "quarantined push for {}/{}: phase {:?} disagrees with the model's {:?}",
                    key.project_id,
                    key.model_id,
                    pushed,
                    lineage
                );
                Some(format!(
                    "stored but not live: this push is phase {pushed:?} while the model is {lineage:?}. \
                     A model's phase is fixed once set; activate this push to re-phase the model."
                ))
            }
        };
        any_quarantined |= quarantined.is_some();
        results.push(ModelIngestResult { model_id, room_count: count, room_boundary, quarantined });
    }
    tracing::info!("received {} room(s) across {} model(s)", total, results.len());

    // 202 when *any* model was quarantined: the run as a whole did not go fully
    // live, and a producer that only reads the status must not read that as a
    // clean push. Which models is in `models`.
    let status = if any_quarantined { StatusCode::ACCEPTED } else { StatusCode::OK };
    Ok((
        status,
        Json(IngestResponse {
            accepted: !any_quarantined,
            room_count: total,
            snapshot_taken_at,
            snapshot_id_generated,
            models: results,
        }),
    ))
}

/// A storage failure (unwritable disk, etc.) is a real server error, not a bad
/// request — surface it as 500 rather than swallowing it.
fn store_failed(e: anyhow::Error) -> (StatusCode, String) {
    tracing::error!("failed to store snapshot: {e:#}");
    (StatusCode::INTERNAL_SERVER_ERROR, format!("could not store snapshot: {e}"))
}

/// Find the sink for one element's model. Linear, because a run carries a
/// handful of models and a map would cost more to build than it saves.
fn sink_for<'s, T>(sinks: &'s mut [T], model_id: &str, id_of: impl Fn(&T) -> &str) -> Option<&'s mut T> {
    sinks.iter_mut().find(|s| id_of(s) == model_id)
}

#[derive(Debug, Serialize)]
pub struct IngestResponse {
    /// Whether **every** model in the push went live. False when any was
    /// quarantined — see `models` for which.
    pub accepted: bool,
    /// Rooms stored across the whole push. Kept as a top-level total (rather
    /// than only per model) because it is the number a producer prints, and it
    /// answers the same question it always did: did this run's rooms land.
    pub room_count: usize,
    /// The snapshot id this push was stored under — echoed back (or minted,
    /// see `snapshot_id_generated`) so the pusher can associate follow-up
    /// uploads with this exact snapshot.
    ///
    /// **One id for every model in the push.** See `ingest_rooms`.
    pub snapshot_taken_at: String,
    /// True when the server minted the id above because the payload left it
    /// blank; false when the payload supplied one and the server used it.
    ///
    /// It describes the *id*, not the snapshot: whether a snapshot was stored
    /// is reported by `accepted`/`room_count`. A producer that stamps its own
    /// `taken_at` (as the Revit one always does) therefore sees `false` here
    /// on every successful push.
    pub snapshot_id_generated: bool,

    /// One entry per model the push carried, in declaration order.
    ///
    /// The per-model half of the response, and the reason the top-level fields
    /// above stayed scalars: a producer asking "did my push land" reads
    /// `accepted`, and one asking "what happened to the structural model" reads
    /// this. Collapsing them would have forced the second question through a
    /// total that cannot answer it.
    pub models: Vec<ModelIngestResult>,
}

/// What became of one model inside a multi-model rooms push.
#[derive(Debug, Serialize)]
pub struct ModelIngestResult {
    pub model_id: String,
    pub room_count: usize,

    /// The boundary regime the server **resolved** for this model — what the
    /// model block declared, or, when it declared nothing, what the project's
    /// `[areas] boundary_location` supplied, or finish face.
    ///
    /// Echoed for the same reason `snapshot_taken_at` is: a producer that left
    /// the field off should be able to see what the server assumed on its
    /// behalf, rather than discovering it later in a footprint that came out
    /// the wrong size. The *resolved* value, not the declared one, precisely
    /// because the interesting case is the one the producer did not state.
    ///
    /// Per model, never per push: the regime is one document's Area and Volume
    /// Computations setting, and a run legitimately mixes both.
    pub room_boundary: RoomBoundary,

    /// Why this model was stored but **not** made live, when that happened —
    /// its phase disagrees with the one the model's lineage is fixed to. `None`
    /// on a normal push, and omitted from the JSON entirely so an accepted
    /// entry looks exactly as it always did.
    ///
    /// Its siblings are unaffected: one model disagreeing about phase says
    /// nothing about the rest of the run, so the others still go live and only
    /// the push-level `accepted` turns false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quarantined: Option<String>,
}

/// The regime resolved for one push. An unregistered project cannot reach here
/// (ingest 422s first — see `validate_ingest`), so the `unwrap_or_default`
/// policy is a formality that keeps this total rather than a real fallback.
fn resolved_boundary(state: &Shared, project_id: &str, declared: Option<RoomBoundary>) -> RoomBoundary {
    state
        .settings()
        .settings_for(project_id)
        .map(|b| b.areas.resolve_boundary(declared))
        .unwrap_or(RoomBoundary::FinishFace)
}

/// Streaming ingest for very large models (NDJSON).
/// Reads the request body as a line-delimited stream instead of buffering it
/// whole with `Json<RoomPayload>`, so peak memory is one line, not the entire
/// (possibly >100 MB) payload. Line 1 is the envelope (identity + levels, no
/// rooms); every following line is one `Room`. If `RequestDecompressionLayer`
/// is in front (see `main.rs`), this stream is already the inflated bytes --
/// gzip and streaming compose without either side knowing about the other.
///
/// **Rooms are no longer accumulated.** Each one is written to its model's
/// snapshot as its line is parsed (`SnapshotStore::put_streaming`), so peak
/// memory is one room rather than the whole push -- which used to be the room
/// set *plus* the serialized copy `set_snapshot` made of it, and which the
/// multi-model bucket had made worse by holding a whole run at once.
///
/// One snapshot per model is open at a time, and none of them is visible until
/// `finish_rooms` commits. An early return anywhere below drops the sinks, and a
/// dropped sink leaves no trace -- which matters because the empty-push refusal
/// can only fire *after* rooms have been written.
pub async fn ingest_rooms_stream(
    State(state): State<Shared>,
    body: Body,
) -> Result<(StatusCode, Json<IngestResponse>), (StatusCode, String)> {
    let stream = body.into_data_stream().map(|r| r.map_err(std::io::Error::other));
    let reader = StreamReader::new(stream);
    let mut lines = reader.lines();

    let envelope_line = lines
        .next_line()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("read error: {e}")))?
        .ok_or((StatusCode::BAD_REQUEST, "empty body".into()))?;

    let mut envelope: StreamEnvelope =
        serde_json::from_str(&envelope_line).map_err(|e| (StatusCode::BAD_REQUEST, format!("bad envelope: {e}")))?;

    // Same resolve-then-pre-flight as the buffered path -- run as soon as the
    // envelope is parsed, before the (potentially large) room stream is read. A
    // push that will be refused should cost the producer one line, not a
    // hundred megabytes of upload.
    let snapshot_id_generated = crate::contract::ensure_taken_at(&mut envelope.snapshot);
    envelope.phase = crate::contract::normalize_phase(envelope.phase.as_deref());
    validate_ingest(
        &state,
        envelope.schema_version,
        SUPPORTED_SCHEMA,
        &envelope.project.id,
        &envelope.snapshot.taken_at,
    )?;
    let declared = validate_models(envelope.models.iter().map(|m| m.model.id.as_str()))?;
    let decisions = preflight_rooms(&state, &envelope.project.id, envelope.phase.as_deref(), &envelope.models)?;

    // Open one snapshot per model BEFORE reading any room, so each room can go
    // straight to its model's file as its line is parsed. Nothing any of them
    // wrote becomes visible until `finish_rooms` commits, and an early return
    // anywhere below drops the sinks, which leaves no trace.
    let mut sinks = open_room_sinks(
        &state,
        envelope.schema_version,
        &envelope.project,
        &envelope.snapshot,
        envelope.phase.as_deref(),
        envelope.models,
        decisions,
    )?;

    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("read error: {e}")))?
    {
        if line.trim().is_empty() {
            continue; // tolerate a trailing blank line
        }
        let line: StreamRoom =
            serde_json::from_str(&line).map_err(|e| (StatusCode::BAD_REQUEST, format!("bad room line: {e}")))?;
        check_declared(&declared, &line.model_id)?;
        let sink = sink_for(&mut sinks, &line.model_id, |s| s.model_id.as_str())
            .expect("check_declared already refused any model without a sink");
        sink.push(line.room)?;
    }

    finish_rooms(
        &state,
        &envelope.project,
        envelope.phase.as_deref(),
        envelope.snapshot.taken_at.clone(),
        snapshot_id_generated,
        sinks,
    )
}

/// The doors half of the ingest contract: the one check a doors push has that a
/// rooms push does not, applied after `validate_ingest` and before anything is
/// stored.
///
/// **The phase must match the lineage, and disagreement is refused.** This is
/// where doors deliberately diverge from rooms. A rooms push that disagrees is
/// quarantined and promotable, because promotion is how a model re-phases
/// (PLAN-phasing.md "D6"). A doors push has no such story: promoting it would
/// move the lineage's phase while every rooms snapshot stayed on the old one,
/// stranding the very rooms its references point at. There is nothing worth
/// activating, so — like an unphased push — it is refused outright.
///
/// A doors push naming **no** phase is refused for the same reason a rooms one
/// is: its doors were never filtered by the phase range test, so they are
/// unfiltered mixed-phase content.
///
/// A lineage with **no** phase yet accepts the push and is phased by it, exactly
/// as a first phased rooms push would — including when the doors arrive *first*
/// and there are no rooms behind them yet. That is now an ordinary order rather
/// than a leftover: see below.
///
/// ## The rooms gate that used to be here, and why it is gone
///
/// This function used to refuse a doors push to a model with no rooms snapshot,
/// on the reasoning that `from_room`/`to_room` are `Room.id`s and room ids are
/// unique only within a model, so such a push stores references nothing can
/// resolve. That reasoning was sound while the *only* answer to "which room does
/// this door belong to" was the id the export carried.
///
/// It stops holding once the server resolves a door's rooms itself, from the
/// door's own position and the project's rooms. The question the gate asked —
/// "can these references resolve **now**?" — has a legitimate answer of "not
/// yet", because rooms may arrive in a later push or in a sibling model, and
/// refusing means refusing data that becomes resolvable the moment they do.
///
/// It was also the only place in this codebase where an unresolved
/// cross-reference was an *error* rather than a reported state, against the
/// "signal, not error" rule everything else here follows. Removing it makes the
/// server more consistent, not less strict — the check did not disappear, it
/// moved to `service::validation::door_report`, where it can be re-answered
/// every time the data changes instead of once, at the moment of the push, on
/// the least information anyone will ever have. That report now distinguishes a
/// model whose rooms have not arrived (**pending** — expected, not a finding)
/// from a reference that names a room its model does not have (**dangling** — a
/// finding), which is the distinction this gate could not make at all.
fn check_doors_ingest(state: &Shared, key: &ModelKey, pushed: Option<&str>) -> Result<(), (StatusCode, String)> {
    let Some(pushed) = pushed else {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "doors push for {}/{} carries no phase, which indicates an extractor predating phase support; \
                 its doors were never filtered to a phase. Update the extractor.",
                key.project_id, key.model_id
            ),
        ));
    };

    let lineage = state.model_phase(key).map_err(|e| {
        tracing::error!("failed to read lineage phase: {e:#}");
        (StatusCode::INTERNAL_SERVER_ERROR, format!("could not read model phase: {e}"))
    })?;
    match lineage {
        None => Ok(()),
        Some(stored) if crate::contract::phases_agree(Some(pushed), Some(&stored)) => Ok(()),
        Some(stored) => Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "doors push for {}/{} is phase {pushed:?} while the model is {stored:?}. \
                 Unlike a rooms push, a disagreeing doors push is not quarantined: activating it would \
                 re-phase the model while its rooms stayed on {stored:?}, leaving these doors' room \
                 references pointing at rooms from another phase. Re-phase the model with a rooms push \
                 first, then push its doors.",
                key.project_id, key.model_id
            ),
        )),
    }
}

#[derive(Debug, Serialize)]
pub struct DoorIngestResponse {
    /// Always true on a 200 — a doors push has no quarantine branch, so it
    /// either stored every model or answered an error. Kept as a field anyway,
    /// in lockstep with `IngestResponse`, so a producer reads both responses the
    /// same way rather than remembering which entity can be half-accepted.
    pub accepted: bool,
    /// Doors stored across the whole push.
    pub door_count: usize,
    /// The snapshot id this push was stored under — echoed back (or minted) on
    /// the same terms as `IngestResponse::snapshot_taken_at`, and shared by
    /// every model in the push.
    pub snapshot_taken_at: String,
    pub snapshot_id_generated: bool,
    /// One entry per model the push carried, in declaration order.
    pub models: Vec<DoorModelIngestResult>,
}

/// What became of one model inside a multi-model doors push.
///
/// Shorter than `ModelIngestResult` by exactly the two fields doors have no
/// answer for: there is no boundary regime on a doors push, and no quarantine.
#[derive(Debug, Serialize)]
pub struct DoorModelIngestResult {
    pub model_id: String,
    pub door_count: usize,
}

/// Revit posts door data here — one push, one or more models. Mirrors
/// `ingest_rooms`: same envelope resolution, same pre-flight, same
/// decomposition into one stored snapshot per model, plus `check_doors_ingest`.
///
/// **There is no quarantine branch and so no 202**, unlike rooms: a doors push
/// either goes live or is refused. See `check_doors_ingest` for why.
pub async fn ingest_doors(
    State(state): State<Shared>,
    Json(mut upload): Json<DoorsUpload>,
) -> Result<(StatusCode, Json<DoorIngestResponse>), (StatusCode, String)> {
    let snapshot_id_generated = crate::contract::ensure_taken_at(&mut upload.snapshot);
    upload.phase = crate::contract::normalize_phase(upload.phase.as_deref());
    validate_ingest(
        &state,
        upload.schema_version,
        SUPPORTED_DOOR_SCHEMA,
        &upload.project.id,
        &upload.snapshot.taken_at,
    )?;
    validate_models(upload.models.iter().map(|m| m.envelope.model.id.as_str()))?;

    let mut doors_by_model: Vec<(String, Vec<Opening>)> = upload
        .models
        .iter_mut()
        .map(|m| (m.envelope.model.id.clone(), std::mem::take(&mut m.doors)))
        .collect();
    let models: Vec<DoorModelEnvelope> = upload.models.into_iter().map(|m| m.envelope).collect();
    preflight_doors(&state, &upload.project.id, upload.phase.as_deref(), &models)?;

    // Through the same sinks the streamed route feeds -- see the rooms half of
    // this pair for why the buffered route goes through them too.
    let mut sinks = open_door_sinks(
        &state,
        upload.schema_version,
        &upload.project,
        &upload.snapshot,
        upload.phase.as_deref(),
        models,
    )?;
    for (model_id, doors) in &mut doors_by_model {
        let sink = sink_for(&mut sinks, model_id, |s| s.model_id.as_str()).expect("every declared model has a sink");
        for door in doors.drain(..) {
            sink.push(door)?;
        }
    }
    finish_doors(upload.snapshot.taken_at.clone(), snapshot_id_generated, sinks)
}

/// The doors counterpart of `preflight_rooms`: every refusal a doors push can
/// earn, decided from the envelope line before any door is read.
///
/// Answers nothing, unlike the rooms version — a doors push has no quarantine
/// branch, so there is no per-model decision to carry forward, only a refusal or
/// silence.
fn preflight_doors(
    state: &Shared,
    project_id: &str,
    phase: Option<&str>,
    models: &[DoorModelEnvelope],
) -> Result<(), (StatusCode, String)> {
    for envelope in models {
        let key = ModelKey { project_id: project_id.to_string(), model_id: envelope.model.id.clone() };
        check_doors_ingest(state, &key, phase)?;
        warn_on_transform_drift(envelope.model_to_shared.as_ref(), project_id, &envelope.model.id);
    }
    Ok(())
}

/// Open one streamed snapshot per model — the doors counterpart of
/// `open_room_sinks`, and shorter by exactly the two things doors have no answer
/// for: there is no boundary regime on a doors push, and no quarantine, so every
/// sink is live.
///
/// `preflight_doors` has already refused anything refusable, from the envelope
/// line alone.
fn open_door_sinks<'a>(
    state: &'a Shared,
    schema_version: u32,
    project: &Project,
    snapshot: &Snapshot,
    phase: Option<&str>,
    models: Vec<DoorModelEnvelope>,
) -> Result<Vec<DoorSink<'a>>, (StatusCode, String)> {
    let mut sinks = Vec::with_capacity(models.len());
    for envelope in models {
        let model_id = envelope.model.id.clone();
        let payload = envelope.into_payload(
            schema_version,
            project.clone(),
            snapshot.clone(),
            phase.map(str::to_string),
            Vec::new(),
        );
        let snapshot = state.open_door_snapshot(&payload).map_err(store_failed)?;
        sinks.push(DoorSink { model_id, snapshot });
    }
    Ok(sinks)
}

/// Where one model's doors go while a push is being read.
struct DoorSink<'a> {
    model_id: String,
    snapshot: StreamingSnapshot<'a>,
}

impl DoorSink<'_> {
    fn push(&mut self, door: Opening) -> Result<(), (StatusCode, String)> {
        self.snapshot.push(&door).map_err(store_failed)
    }
}

/// Publish every model's doors and report the push.
///
/// **A model that contributed no doors is still committed**, which is where this
/// deliberately parts company with `finish_rooms`. A rooms push carrying an empty
/// model is a producer fault, because a document with no rooms is not something
/// anyone exports; a model with rooms and no doors is a shell, a pre-fit-out
/// phase, or simply a floor without any. The server cannot tell that from a
/// broken export and does not try — the producer refuses a doorless *run*, which
/// is a question only it can answer.
///
/// So there is no check-then-commit split here: nothing can refuse at this
/// point, and one loop is the honest shape.
fn finish_doors(
    snapshot_taken_at: String,
    snapshot_id_generated: bool,
    sinks: Vec<DoorSink<'_>>,
) -> Result<(StatusCode, Json<DoorIngestResponse>), (StatusCode, String)> {
    let mut results = Vec::with_capacity(sinks.len());
    let mut total = 0usize;
    for sink in sinks {
        let count = sink.snapshot.count();
        total += count;
        sink.snapshot.commit().map_err(store_failed)?;
        results.push(DoorModelIngestResult { model_id: sink.model_id, door_count: count });
    }
    tracing::info!("received {} door(s) across {} model(s)", total, results.len());

    Ok((
        StatusCode::OK,
        Json(DoorIngestResponse {
            accepted: true,
            door_count: total,
            snapshot_taken_at,
            snapshot_id_generated,
            models: results,
        }),
    ))
}

/// Streaming doors ingest (NDJSON), the counterpart to `ingest_rooms_stream`:
/// line 1 is the envelope, every following line is one `Opening`.
///
/// Doors are far fewer than rooms per model, so this is not load-bearing the way
/// the rooms stream is. It exists so a producer can use one transport for both
/// pushes rather than branching per entity — and so the two paths stay provably
/// equivalent, which the tests assert.
pub async fn ingest_doors_stream(
    State(state): State<Shared>,
    body: Body,
) -> Result<(StatusCode, Json<DoorIngestResponse>), (StatusCode, String)> {
    let stream = body.into_data_stream().map(|r| r.map_err(std::io::Error::other));
    let reader = StreamReader::new(stream);
    let mut lines = reader.lines();

    let envelope_line = lines
        .next_line()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("read error: {e}")))?
        .ok_or((StatusCode::BAD_REQUEST, "empty body".into()))?;

    let mut envelope: DoorStreamEnvelope =
        serde_json::from_str(&envelope_line).map_err(|e| (StatusCode::BAD_REQUEST, format!("bad envelope: {e}")))?;

    // Decided from the envelope alone, before a single door line is read — a
    // push that will be refused should cost the producer one line.
    let snapshot_id_generated = crate::contract::ensure_taken_at(&mut envelope.snapshot);
    envelope.phase = crate::contract::normalize_phase(envelope.phase.as_deref());
    validate_ingest(
        &state,
        envelope.schema_version,
        SUPPORTED_DOOR_SCHEMA,
        &envelope.project.id,
        &envelope.snapshot.taken_at,
    )?;
    let declared = validate_models(envelope.models.iter().map(|m| m.model.id.as_str()))?;
    preflight_doors(&state, &envelope.project.id, envelope.phase.as_deref(), &envelope.models)?;

    let mut sinks = open_door_sinks(
        &state,
        envelope.schema_version,
        &envelope.project,
        &envelope.snapshot,
        envelope.phase.as_deref(),
        envelope.models,
    )?;

    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("read error: {e}")))?
    {
        if line.trim().is_empty() {
            continue; // tolerate a trailing blank line
        }
        let line: StreamDoor =
            serde_json::from_str(&line).map_err(|e| (StatusCode::BAD_REQUEST, format!("bad door line: {e}")))?;
        check_declared(&declared, &line.model_id)?;
        let sink = sink_for(&mut sinks, &line.model_id, |s| s.model_id.as_str())
            .expect("check_declared already refused any model without a sink");
        sink.push(line.door)?;
    }

    finish_doors(envelope.snapshot.taken_at.clone(), snapshot_id_generated, sinks)
}

/// `ServiceError` -> `(StatusCode, String)`, the same message-carrying error
/// shape the ingest and settings handlers already use. It replaced a bare
/// `StatusCode` when `ServiceError::Invalid` arrived: a caller-fault status
/// with no body would leave a client unable to tell a malformed filter from a
/// filter that legitimately matched nothing.
fn map_service_error(err: ServiceError) -> (StatusCode, String) {
    match err {
        ServiceError::Internal(e) => {
            tracing::error!("internal service error: {e:#}");
            // The internal detail is logged, never echoed: a storage path or
            // an io error is not the caller's business.
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
        }
        // Caller-fault: the message IS the useful part (which predicate, and
        // why), so it goes back verbatim.
        ServiceError::Invalid(msg) => (StatusCode::BAD_REQUEST, msg),
    }
}

/// Lists every project with at least one stored model — see
/// `service::projects::list_projects`. `200 []` when nothing has been pushed
/// yet: an empty list is a perfectly good answer for a picker, unlike
/// `/rooms`'s 204 (which exists for the poller's specific "nothing posted
/// yet" signal).
pub async fn get_projects(State(state): State<Shared>) -> Result<Json<Vec<ProjectSummary>>, (StatusCode, String)> {
    let projects = projects::list_projects(&state).map_err(map_service_error)?;
    Ok(Json(projects))
}

/// Lists every stored snapshot id for one project, grouped per model — see
/// `service::snapshots::list_project_snapshots`.
pub async fn get_project_snapshots(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
) -> Result<Json<ProjectSnapshotsResponse>, (StatusCode, String)> {
    let result = snapshots::list_project_snapshots(&state, &project_id).map_err(map_service_error)?;
    Ok(Json(result))
}

/// The latest snapshot id for one model — the cheap "what snapshot do I
/// attach my follow-up upload to" call. 404 when the model (or its project)
/// has no latest: unlike the listing's soft empty success, this names one
/// specific resource — see `service::snapshots::latest_snapshot`.
pub async fn get_model_latest_snapshot(
    State(state): State<Shared>,
    Path((project_id, model_id)): Path<(String, String)>,
) -> Result<Json<LatestSnapshot>, (StatusCode, String)> {
    let result = snapshots::latest_snapshot(&state, &project_id, &model_id).map_err(map_service_error)?;
    match result {
        None => Err((StatusCode::NOT_FOUND, "no snapshots stored for that project/model".to_string())),
        Some(latest) => Ok(Json(latest)),
    }
}

/// The quarantined push waiting on one model, if any. 404 when there is none —
/// same one-specific-resource convention as `get_model_latest_snapshot`.
///
/// The read half of the re-phase flow: a push whose phase disagrees with its
/// model's is answered `202` and stored inert, and that response is the only
/// other place the quarantine is ever announced. Without this route, nobody who
/// wasn't watching the push could discover there is something to activate.
pub async fn get_model_pending_snapshot(
    State(state): State<Shared>,
    Path((project_id, model_id)): Path<(String, String)>,
) -> Result<Json<PendingSnapshot>, (StatusCode, String)> {
    let result = snapshots::pending_snapshot(&state, &project_id, &model_id).map_err(map_service_error)?;
    match result {
        None => Err((StatusCode::NOT_FOUND, "no pending push for that project/model".to_string())),
        Some(pending) => Ok(Json(pending)),
    }
}

/// Activate the quarantined push: make it live and re-phase the model to its
/// phase. 404 when nothing is pending.
///
/// The only route in the server that can change a model's phase. A `POST` with
/// no body — the resource being acted on is fully identified by the path, and
/// there is exactly one pending push per model, so there is nothing to select.
pub async fn activate_model_pending_snapshot(
    State(state): State<Shared>,
    Path((project_id, model_id)): Path<(String, String)>,
) -> Result<Json<PendingSnapshot>, (StatusCode, String)> {
    let result = snapshots::activate_pending_snapshot(&state, &project_id, &model_id).map_err(map_service_error)?;
    match result {
        None => Err((StatusCode::NOT_FOUND, "no pending push for that project/model".to_string())),
        Some(activated) => Ok(Json(activated)),
    }
}

/// Lists every uploaded snapshot id for one project's named reference source.
/// Soft-empty for unknown projects, same as the model-snapshot listing.
pub async fn get_reference_snapshots(
    State(state): State<Shared>,
    Path((project_id, source)): Path<(String, String)>,
) -> Result<Json<ReferenceSnapshotList>, (StatusCode, String)> {
    let result = reference::list_reference_snapshots(&state, &project_id, &source).map_err(map_service_error)?;
    Ok(Json(result))
}

/// A parsed summary of the latest uploaded CSV for one project's named
/// reference source. 404 when there is none: this names one specific
/// resource, same convention as `get_model_latest_snapshot`.
pub async fn get_reference_latest(
    State(state): State<Shared>,
    Path((project_id, source)): Path<(String, String)>,
) -> Result<Json<ReferenceSnapshotInfo>, (StatusCode, String)> {
    let result = reference::get_reference_snapshot(&state, &project_id, &source, None).map_err(map_service_error)?;
    match result {
        None => Err((StatusCode::NOT_FOUND, format!("no '{source}' upload stored for that project"))),
        Some(info) => Ok(Json(info)),
    }
}

/// Lists one project's milestones for the viewer's picker — see
/// `service::milestones::list_milestones`.
pub async fn get_project_milestones(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
) -> Result<Json<MilestonesResponse>, (StatusCode, String)> {
    let result = milestones::list_milestones(&state, &project_id).map_err(map_service_error)?;
    Ok(Json(result))
}

/// Lists the distinct "Building" classification values for one project — see
/// `service::projects::list_buildings`.
pub async fn get_project_buildings(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
) -> Result<Json<BuildingsResponse>, (StatusCode, String)> {
    let buildings = projects::list_buildings(&state, &project_id).map_err(map_service_error)?;
    Ok(Json(buildings))
}

/// Optional scoping for `GET /rooms`. All absent keeps today's behaviour:
/// merge every stored model globally (backwards compatible). `milestone`
/// names a per-project milestone; models are then served from the snapshots
/// that milestone pins instead of their latest (see
/// `service::rooms::assemble_rooms`).
#[derive(Deserialize)]
pub struct RoomsQuery {
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub building: Option<String>,
    #[serde(default)]
    pub milestone: Option<String>,
    /// Comma-separated property predicates, all of which must hold, e.g.
    /// `?filter=Department=Cardiology,Area>20`. A value containing a literal
    /// comma must be quoted (`Department="A, B"`). Exists for programmatic
    /// callers — the viewer holds the whole payload and matches locally, so it
    /// never sends this. See `service::rooms::RoomFilter`.
    #[serde(default)]
    pub filter: Option<String>,
}

/// The viewer fetches here — see `service::rooms::assemble_rooms`. Returns
/// 204 when nothing has ever been posted (the service's `None` case); a scope
/// matching nothing still returns 200 with empty arrays; a malformed `filter`
/// is 400 with the parser's message. `RoomsResult` serializes directly — every
/// field is wire shape, so no hand-built JSON is needed here.
///
/// Returns `Response` rather than `Json<..>` so the 204 stays on the `Ok` arm:
/// the error arm now carries a message (`(StatusCode, String)`, the shape the
/// ingest and settings handlers already use), and threading a body-less 204
/// through it would have meant answering the viewer's poll with an empty-bodied
/// error.
/// Turn a data cursor plus the request's own scope into one `ETag` value.
///
/// **The scope has to be in the tag, not just the data.** `service::scope_cursor`
/// answers "which snapshots would this read serve", which is identical for
/// `?building=A` and `?building=B` — the two responses are not. HTTP validates
/// an entity tag against the URL that produced it, but the viewer compares tags
/// in JavaScript across a scope change, so an unqualified tag would let a
/// building switch look like "nothing changed". Hashing the parsed fields rather
/// than the raw query string also means `?a=1&b=2` and `?b=2&a=1` agree, which
/// a raw-string tag would not.
fn etag_for(cursor: &str, scope: [Option<&str>; 4]) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cursor.hash(&mut hasher);
    scope.hash(&mut hasher);
    // Quoted: a bare token is not a legal entity tag, and a proxy that
    // normalises one would break the comparison below.
    format!("\"{:016x}\"", hasher.finish())
}

/// Whether the client already holds this exact entity — an `If-None-Match` hit.
///
/// Only the exact-match case is honoured. `*` and weak comparison exist in the
/// spec, but nothing this server talks to sends them, and a wrong "yes" here
/// serves a stale plan indefinitely; a wrong "no" costs one body.
fn is_fresh(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|hdr| hdr.split(',').any(|candidate| candidate.trim() == etag))
}

/// A 304, carrying the tag so a client that lost track can re-sync from it.
fn not_modified(etag: &str) -> Response {
    ([(header::ETAG, etag)], StatusCode::NOT_MODIFIED).into_response()
}

pub async fn get_rooms(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<RoomsQuery>,
) -> Result<Response, (StatusCode, String)> {
    // Parsed here, in the adapter that holds the raw string, then passed down
    // as a domain type -- `service` never sees the query syntax. The known-
    // source vocabulary comes from the live registry, not a fixed list: a
    // `/rooms` request can span every stored project unscoped, so no single
    // project's config can answer "what's before the dot" (see
    // `SettingsRegistry::known_reference_sources`).
    let known = state.settings().known_reference_sources();
    let filter = query
        .filter
        .as_deref()
        .map(|s| rooms::RoomFilter::parse_query(s, &known))
        .transpose()
        .map_err(|msg| map_service_error(ServiceError::Invalid(msg)))?
        .filter(|f| !f.is_empty());

    let scope = rooms::RoomScope {
        project: query.project.as_deref(),
        building: query.building.as_deref(),
        milestone: query.milestone.as_deref(),
        filter: filter.as_ref(),
    };

    // The cheap half first: if the client's tag still matches, nothing below
    // this line has to run at all. That is the entire point — the assemble is
    // what costs, not the transfer.
    let cursor =
        scope_cursor(&state, scope.project, scope.milestone, &[SnapshotKind::Rooms]).map_err(map_service_error)?;
    let etag = etag_for(
        &cursor,
        [
            query.project.as_deref(),
            query.building.as_deref(),
            query.milestone.as_deref(),
            query.filter.as_deref(),
        ],
    );
    if is_fresh(&headers, &etag) {
        return Ok(not_modified(&etag));
    }

    let result = rooms::assemble_rooms(&state, &scope).map_err(map_service_error)?;

    match result {
        // No tag on a 204: there is no entity to hold, and tagging "nothing"
        // would let the first real push be answered with a 304.
        None => Ok(StatusCode::NO_CONTENT.into_response()),
        Some(result) => Ok(([(header::ETAG, etag)], Json(result)).into_response()),
    }
}

/// Data-quality report for the header's validation panel — see
/// `service::validation::compute_project_validation`.
#[derive(Debug, Deserialize)]
pub struct DoorsQuery {
    #[serde(default)]
    pub project: Option<String>,
    /// Opaque building key from `/projects/{id}/buildings` — a door's building
    /// is its owning room's, so this became answerable when
    /// `[doors] room_attribution` decided which room owns a door.
    #[serde(default)]
    pub building: Option<String>,
    #[serde(default)]
    pub milestone: Option<String>,
    /// Comma-separated property predicates, same grammar as `RoomsQuery::filter`
    /// — plus the door intrinsics `$id`, `$type_id`, `$type_name`, `$level_id`,
    /// `$from_room` and `$to_room`.
    #[serde(default)]
    pub filter: Option<String>,
}

/// The doors read. Mirrors `get_rooms`, minus `?building=` — a door's building
/// depends on which of its rooms owns it, which is an open design question
/// (`service::doors`' module doc).
pub async fn get_doors(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<DoorsQuery>,
) -> Result<Response, (StatusCode, String)> {
    // The same registry-wide source vocabulary `/rooms` parses against, even
    // though no door carries a joined source yet: parsing a filter differently
    // per entity would fork the grammar, and `drofus.NetArea` on a door resolves
    // to `Absent` (matching nothing) rather than erroring. See
    // `impl FilterTarget for DoorResponse`.
    let known = state.settings().known_reference_sources();
    let filter = query
        .filter
        .as_deref()
        .map(|s| rooms::RoomFilter::parse_query(s, &known))
        .transpose()
        .map_err(|msg| map_service_error(ServiceError::Invalid(msg)))?
        .filter(|f| !f.is_empty());

    let scope = doors::DoorScope {
        project: query.project.as_deref(),
        building: query.building.as_deref(),
        milestone: query.milestone.as_deref(),
        filter: filter.as_ref(),
    };

    // Both kinds, because a doors response is not a function of doors alone:
    // ownership and the geometric resolver read the scope's *rooms*
    // (`doors::build_candidates`), so a rooms push changes this body.
    let cursor = scope_cursor(&state, scope.project, scope.milestone, &[SnapshotKind::Rooms, SnapshotKind::Doors])
        .map_err(map_service_error)?;
    let etag = etag_for(
        &cursor,
        [
            query.project.as_deref(),
            query.building.as_deref(),
            query.milestone.as_deref(),
            query.filter.as_deref(),
        ],
    );
    if is_fresh(&headers, &etag) {
        return Ok(not_modified(&etag));
    }

    let result = doors::assemble_doors(&state, &scope).map_err(map_service_error)?;

    match result {
        None => Ok(StatusCode::NO_CONTENT.into_response()),
        Some(result) => Ok(([(header::ETAG, etag)], Json(result)).into_response()),
    }
}

pub async fn get_project_validation(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
) -> Result<Json<ValidationResponse>, (StatusCode, String)> {
    let report = validation::compute_project_validation(&state, &project_id).map_err(map_service_error)?;
    Ok(Json(report))
}

/// Optional building/milestone scoping for `GET /projects/{id}/areas` (the
/// project itself is the path id). Same scoping vocabulary as `/rooms`.
#[derive(Deserialize)]
pub struct AreasQuery {
    #[serde(default)]
    pub building: Option<String>,
    #[serde(default)]
    pub milestone: Option<String>,
}

/// Hierarchy gross-area footprints for one project — see
/// `service::areas::assemble_areas`. 204 when nothing has ever been posted
/// (mirrors `/rooms`, including keeping that 204 on the `Ok` arm so it stays
/// body-less now that the error arm carries a message); a scope matching
/// nothing is 200 with empty `groups`.
pub async fn get_project_areas(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    Query(query): Query<AreasQuery>,
) -> Result<Response, (StatusCode, String)> {
    let result = areas::assemble_areas(&state, &project_id, query.building.as_deref(), query.milestone.as_deref())
        .map_err(map_service_error)?;
    match result {
        None => Ok(StatusCode::NO_CONTENT.into_response()),
        Some(result) => Ok(Json(result).into_response()),
    }
}

/// Scoping for `GET /projects/{id}/adjacency`: the same `?building=` /
/// `?milestone=` vocabulary as `/rooms` and `/areas`, plus the tunable wall
/// tolerance.
///
/// `wall_max` is `Option<String>`, not `Option<f64>`, on purpose. Taken as a
/// float, axum's own `Query` deserialization rejects `?wall_max=abc` *before*
/// this handler runs, with its own wording and no idea that zero is meaningful
/// or that five feet is the ceiling. Parsing it here means every bad tolerance —
/// unparseable, negative, absurd — comes back through one path with one voice,
/// and the range rules stay in `service::adjacency` where both front doors read
/// them.
#[derive(Deserialize)]
pub struct AdjacencyQuery {
    #[serde(default)]
    pub building: Option<String>,
    #[serde(default)]
    pub milestone: Option<String>,
    #[serde(default)]
    pub wall_max: Option<String>,
}

/// Room-to-room adjacency graph for one project — see
/// `service::adjacency::assemble_adjacency`. 204 when nothing has ever been
/// posted (mirrors `/rooms` and `/areas`); a scope matching nothing is 200 with
/// empty `nodes`/`edges`.
///
/// A bad `wall_max` is **400**, not 422: in this codebase 422 is the ingest
/// status (a schema mismatch, a malformed `taken_at`), while a caller-fault read
/// parameter travels as `ServiceError::Invalid` and maps to 400 — exactly how
/// `/rooms` already answers a malformed `?filter=`. Loud over a silent clamp
/// either way; only the number differs.
pub async fn get_project_adjacency(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    Query(query): Query<AdjacencyQuery>,
) -> Result<Response, (StatusCode, String)> {
    let wall_max = match query.wall_max.as_deref() {
        None | Some("") => None,
        Some(raw) => Some(
            raw.parse::<f64>()
                .map_err(|_| map_service_error(ServiceError::Invalid(format!("wall_max {raw:?} is not a number"))))?,
        ),
    };

    let result = adjacency::assemble_adjacency(
        &state,
        &project_id,
        query.building.as_deref(),
        query.milestone.as_deref(),
        wall_max,
    )
    .map_err(map_service_error)?;

    match result {
        None => Ok(StatusCode::NO_CONTENT.into_response()),
        Some(result) => Ok(Json(result).into_response()),
    }
}

/// The baseline milestone plus the milestones to compare against it. A POST
/// body rather than query params because the compared set is a list (repeated
/// query keys don't deserialize cleanly, and milestone names can contain any
/// character). A POST that reads rather than writes — unusual, and the reason
/// it still sits behind the CORS/Host guards like any other mutating route.
#[derive(Deserialize)]
pub struct ComparisonRequest {
    pub baseline: String,
    #[serde(default)]
    pub others: Vec<String>,
}

/// Milestone comparison for one project — see
/// `service::comparison::compare_milestones`. A read, but POST-shaped for its
/// list input. A project with no `comparison_key` configured returns 200 with
/// `comparison_key_configured: false` (a real state the client renders), not an
/// error.
pub async fn compare_project_milestones(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    Json(req): Json<ComparisonRequest>,
) -> Result<Json<ComparisonResponse>, (StatusCode, String)> {
    let result =
        comparison::compare_milestones(&state, &project_id, &req.baseline, &req.others).map_err(map_service_error)?;
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{Level, Model, Project, Snapshot};
    use crate::reference::ReferenceData;
    use crate::state::{AppState, ProjectReferenceSource, ProjectSettings};
    use crate::storage::MemStore;
    use std::collections::BTreeMap;

    fn make_room(id: &str, name: &str) -> Room {
        Room {
            id: id.to_string(),
            name: name.to_string(),
            level_id: "1".to_string(),
            loops: vec![],
            properties: BTreeMap::new(),
        }
    }

    /// One model's rooms as a v7 upload — the shape every rooms ingest test
    /// builds, since a push is now a bucket even when it carries one model.
    fn rooms_upload(model: &str, ts: &str, phase: Option<&str>, rooms: Vec<Room>) -> RoomsUpload {
        rooms_upload_for("p1", model, ts, phase, rooms)
    }

    /// `rooms_upload` with the project named, for the unregistered-project case.
    fn rooms_upload_for(project: &str, model: &str, ts: &str, phase: Option<&str>, rooms: Vec<Room>) -> RoomsUpload {
        RoomsUpload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: project.to_string(), name: "P".to_string() },
            snapshot: Snapshot { taken_at: ts.to_string() },
            phase: phase.map(str::to_string),
            models: vec![room_model(model, rooms)],
        }
    }

    /// One model block, with the two optional per-model facts left unset —
    /// tests that care about them set them on the returned value.
    fn room_model(model: &str, rooms: Vec<Room>) -> crate::contract::RoomModelUpload {
        crate::contract::RoomModelUpload {
            envelope: RoomModelEnvelope {
                model: Model { id: model.to_string(), name: "M".to_string(), source: "revit".to_string() },
                model_to_shared: None,
                room_boundary: None,
                levels: vec![],
            },
            rooms,
        }
    }

    fn make_drofus() -> ReferenceData {
        ReferenceData {
            link_property: "Number".to_string(),
            by_id: BTreeMap::new(),
            reconciliation: BTreeMap::new(),
            all_labels: vec![],
            duplicate_ids: vec![],
            blank_id_rows: 0,
        }
    }

    fn make_bundle() -> ProjectSettings {
        ProjectSettings {
            reference: BTreeMap::from([(
                "drofus".to_string(),
                ProjectReferenceSource {
                    entity: crate::settings::ReferenceEntity::Rooms,
                    data: Some(make_drofus()),
                    fields: vec![],
                },
            )]),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec!["$name".to_string(), "$id".to_string()],
            milestones: vec![],
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            windows: Default::default(),
            hierarchy_exclusions: vec![],
        }
    }

    /// Registers one project's bundle under its id -- the shape
    /// `AppState::new` now takes in place of the old five flat fields.
    fn single_project(project_id: &str) -> std::collections::HashMap<String, ProjectSettings> {
        std::collections::HashMap::from([(project_id.to_string(), make_bundle())])
    }

    /// A `RoomsQuery` with no scoping at all -- the viewer's request shape.
    fn unscoped_query() -> RoomsQuery {
        RoomsQuery { project: None, building: None, milestone: None, filter: None }
    }

    /// An empty store yields 204 through the full handler, not just at the
    /// service layer -- the one behavior that genuinely lives at the HTTP
    /// seam (`service::rooms::assemble_rooms` has no notion of "204"). Also
    /// guards that the 204 stayed on the `Ok` arm when the error type grew a
    /// message: the viewer polls on this status and must not receive a body.
    #[tokio::test]
    async fn test_get_rooms_returns_204_when_store_empty() {
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let response = get_rooms(State(state), HeaderMap::new(), Query(unscoped_query()))
            .await
            .expect("204 is not an error");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    /// One registered project with one room, for the conditional-request tests.
    fn state_with_one_room(taken_at: &str) -> Shared {
        let payload = RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: taken_at.to_string() },
            phase: None,
            model_to_shared: None,
            room_boundary: None,
            levels: vec![Level { id: "l1".to_string(), name: "Level 1".to_string(), elevation: 0.0 }],
            rooms: vec![make_room("r1", "Room A")],
        };
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        state.set_snapshot(payload).unwrap();
        state
    }

    fn if_none_match(etag: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, etag.parse().unwrap());
        headers
    }

    /// The round trip the viewer's poll actually performs: a 200 hands out an
    /// `ETag`, and sending it straight back returns 304 with no body. This is
    /// the whole saving — an idle poll must not cost an assemble.
    #[tokio::test]
    async fn test_get_rooms_answers_304_to_its_own_etag() {
        let state = state_with_one_room("2026-01-01T00:00:00Z");

        let first = get_rooms(State(state.clone()), HeaderMap::new(), Query(unscoped_query())).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let etag = first
            .headers()
            .get(header::ETAG)
            .expect("a 200 must carry a tag")
            .to_str()
            .unwrap()
            .to_string();

        let second = get_rooms(State(state), if_none_match(&etag), Query(unscoped_query())).await.unwrap();

        assert_eq!(second.status(), StatusCode::NOT_MODIFIED);
        let bytes = axum::body::to_bytes(second.into_body(), usize::MAX).await.unwrap();
        assert!(bytes.is_empty(), "a 304 carries no body — that is the point");
    }

    /// A push moves the tag, so a client holding the old one gets a real body.
    /// The failure this rules out is the only *dangerous* direction: a cursor
    /// that kept matching would freeze the viewer on a stale plan indefinitely,
    /// where the opposite error merely costs one needless download.
    #[tokio::test]
    async fn test_get_rooms_etag_moves_on_a_push() {
        let state = state_with_one_room("2026-01-01T00:00:00Z");
        let first = get_rooms(State(state.clone()), HeaderMap::new(), Query(unscoped_query())).await.unwrap();
        let etag = first.headers().get(header::ETAG).unwrap().to_str().unwrap().to_string();

        let mut newer = RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-02-02T00:00:00Z".to_string() },
            phase: None,
            model_to_shared: None,
            room_boundary: None,
            levels: vec![Level { id: "l1".to_string(), name: "Level 1".to_string(), elevation: 0.0 }],
            rooms: vec![make_room("r1", "Room A")],
        };
        newer.rooms.push(make_room("r2", "Room B"));
        state.set_snapshot(newer).unwrap();

        let after = get_rooms(State(state), if_none_match(&etag), Query(unscoped_query())).await.unwrap();

        assert_eq!(after.status(), StatusCode::OK, "the pushed snapshot must not hide behind the old tag");
    }

    /// A tag issued for one scope must not satisfy a request for another. The
    /// data cursor is identical across these two — same store, same snapshots —
    /// so only the scope going into the tag separates them, and the viewer
    /// compares tags in JavaScript across a scope change rather than relying on
    /// HTTP's per-URL validation.
    #[tokio::test]
    async fn test_get_rooms_etag_is_scoped_to_the_request() {
        let state = state_with_one_room("2026-01-01T00:00:00Z");
        let unscoped = get_rooms(State(state.clone()), HeaderMap::new(), Query(unscoped_query())).await.unwrap();
        let etag = unscoped.headers().get(header::ETAG).unwrap().to_str().unwrap().to_string();

        let scoped = RoomsQuery { building: Some("B01".to_string()), ..unscoped_query() };
        let response = get_rooms(State(state), if_none_match(&etag), Query(scoped)).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK, "a building filter is a different entity");
    }

    /// An empty store answers 204 and issues **no** tag. Tagging "nothing"
    /// would let the first real push be answered 304 — the viewer would sit on
    /// "waiting for data" through a successful import.
    #[tokio::test]
    async fn test_get_rooms_204_carries_no_etag() {
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let response = get_rooms(State(state), HeaderMap::new(), Query(unscoped_query())).await.unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(response.headers().get(header::ETAG).is_none());
    }

    fn adjacency_query(wall_max: Option<&str>) -> AdjacencyQuery {
        AdjacencyQuery { building: None, milestone: None, wall_max: wall_max.map(str::to_string) }
    }

    /// Adjacency mirrors `/rooms` and `/areas` on the empty store: 204, and
    /// body-less, so the same poll-shaped client handling works for all three.
    #[tokio::test]
    async fn test_get_adjacency_returns_204_when_store_empty() {
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let response = get_project_adjacency(State(state), Path("p1".to_string()), Query(adjacency_query(None)))
            .await
            .expect("204 is not an error");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    /// **400, not 422.** This is the one behaviour that lives purely at the HTTP
    /// seam, and it is the status the handover originally got wrong: in this
    /// codebase 422 is the *ingest* status (a schema mismatch, a malformed
    /// `taken_at`), while a caller-fault read parameter travels as
    /// `ServiceError::Invalid` and maps to 400 — exactly how `/rooms` answers a
    /// malformed `?filter=`. The message goes back verbatim, because "which
    /// value, and why" is the useful part.
    #[tokio::test]
    async fn test_get_adjacency_rejects_out_of_range_wall_max_with_400() {
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let (status, message) =
            get_project_adjacency(State(state), Path("p1".to_string()), Query(adjacency_query(Some("99"))))
                .await
                .expect_err("99 ft is not a wall");
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(message.contains("99"), "the message names the offending value: {message}");
    }

    /// An unparseable tolerance is rejected here rather than by axum's own
    /// `Query` deserialization — which is the entire reason the field is typed
    /// `Option<String>`. Taken as `Option<f64>` this request would never reach
    /// the handler, and the client would get axum's wording instead of a message
    /// that knows what a wall tolerance is.
    #[tokio::test]
    async fn test_get_adjacency_rejects_unparseable_wall_max_here() {
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let (status, message) =
            get_project_adjacency(State(state), Path("p1".to_string()), Query(adjacency_query(Some("abc"))))
                .await
                .expect_err("not a number");
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(message.contains("abc"), "the message quotes what was sent: {message}");
    }

    /// Zero is a *valid* tolerance — the wall-centreline case, and the more
    /// common of the two boundary regimes. It must reach the service, not be
    /// rejected as "non-positive" or silently treated as "unset" (which would
    /// substitute the 1.5 ft default and quietly answer a different question).
    #[tokio::test]
    async fn test_get_adjacency_accepts_zero_wall_max() {
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        state
            .set_snapshot(RoomPayload {
                schema_version: SUPPORTED_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
                phase: Some("New Construction".to_string()),
                model_to_shared: None,
                room_boundary: None,
                levels: vec![Level { id: "1".to_string(), name: "L".to_string(), elevation: 0.0 }],
                rooms: vec![make_room("r1", "Room 1")],
            })
            .unwrap();

        let response = get_project_adjacency(State(state), Path("p1".to_string()), Query(adjacency_query(Some("0"))))
            .await
            .expect("zero is a real tolerance");
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// The query-string seam: a predicate's own `=` must survive
    /// deserialization. `form_urlencoded` splits each pair at its FIRST `=`,
    /// so `?filter=Department=Cardiology` arrives as the value
    /// "Department=Cardiology" rather than being truncated -- the reason the
    /// grammar can use `=` as an operator in a query param at all.
    #[test]
    fn test_rooms_query_keeps_operators_in_the_filter_value() {
        let uri: axum::http::Uri = "/rooms?project=p1&filter=Department=Cardiology,Area%3E20".parse().unwrap();
        let Query(query) = Query::<RoomsQuery>::try_from_uri(&uri).expect("must deserialize");

        assert_eq!(query.project.as_deref(), Some("p1"));
        assert_eq!(query.filter.as_deref(), Some("Department=Cardiology,Area>20"));
        let filter =
            rooms::RoomFilter::parse_query(query.filter.as_deref().unwrap(), &Default::default()).expect("must parse");
        assert!(!filter.is_empty());
    }

    /// A malformed `?filter=` predicate is a 400 carrying the parser's
    /// message -- the whole point of the message-bearing error type, since a
    /// bare status would leave a client unable to tell a typo from a filter
    /// that legitimately matched nothing.
    #[tokio::test]
    async fn test_get_rooms_rejects_malformed_filter() {
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let query = RoomsQuery { filter: Some("Department".to_string()), ..unscoped_query() };
        let (status, message) = get_rooms(State(state), HeaderMap::new(), Query(query))
            .await
            .expect_err("no operator in the predicate");
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            message.contains("no operator"),
            "the parser's reason must reach the caller, got {message:?}"
        );
    }

    /// A project filter matching nothing still returns 200 with empty
    /// arrays -- distinct from a truly empty store.
    #[tokio::test]
    async fn test_get_rooms_empty_filter_result_is_200_not_204() {
        let payload = RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
            phase: Some("New Construction".to_string()),
            model_to_shared: None,
            room_boundary: None,
            levels: vec![Level { id: "l1".to_string(), name: "Level 1".to_string(), elevation: 0.0 }],
            rooms: vec![make_room("r1", "Room A")],
        };
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        state.set_snapshot(payload).unwrap();

        let query = RoomsQuery { project: Some("nonexistent".to_string()), ..unscoped_query() };
        let response = get_rooms(State(state), HeaderMap::new(), Query(query)).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(body["rooms"].as_array().unwrap().is_empty());
    }

    /// The doors counterpart of `test_get_rooms_empty_filter_result_is_200_not_204`,
    /// added with the scoped store read that made it fragile: the "nothing has
    /// ever been pushed" 204 used to fall out of the merge read being empty, and
    /// a read narrowed to an unknown project is empty for a completely different
    /// reason. Both assemblers now ask the index instead.
    #[tokio::test]
    async fn test_get_doors_unknown_project_is_200_not_204() {
        let state = state_with_one_room("2026-01-01T00:00:00Z");
        state
            .set_door_snapshot(crate::contract::DoorPayload {
                schema_version: crate::contract::SUPPORTED_DOOR_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
                phase: None,
                model_to_shared: None,
                levels: vec![],
                doors: vec![],
            })
            .unwrap();

        let query = DoorsQuery {
            project: Some("nonexistent".to_string()),
            building: None,
            milestone: None,
            filter: None,
        };
        let response = get_doors(State(state), HeaderMap::new(), Query(query)).await.unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "the store has data; this question just has an empty answer"
        );
    }

    /// A project/model id that could escape the storage root as a path
    /// component -- or a `taken_at` that isn't an RFC3339 UTC date-time
    /// (which rules out anything path-shaped) -- is rejected 422 before
    /// anything is written: ids become `root/<project_id>/<model_id>` and
    /// `taken_at` becomes the snapshot filename in `FsStore`.
    #[tokio::test]
    async fn test_ingest_rooms_rejects_path_unsafe_identity() {
        let good_ts = "2026-01-01T00:00:00Z";
        let cases = [
            ("../escape", good_ts),
            ("a/b", good_ts),
            ("a\\b", good_ts),
            ("  ", good_ts),
            ("m1", "..\\..\\evil"),
            ("m1", "2026/01/01"),
            ("m1", "2026-01-01T00:00:00+10:00"), // parses, but not UTC
        ];
        for (model_id, taken_at) in cases {
            let upload = rooms_upload(model_id, taken_at, Some("New Construction"), vec![]);
            let state: Shared =
                std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

            let result = ingest_rooms(State(state), Json(upload)).await;
            match result {
                Err((status, msg)) => {
                    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "model {model_id:?} taken_at {taken_at:?}");
                    assert!(
                        msg.contains("unsafe") || msg.contains("RFC3339") || msg.contains("UTC"),
                        "message names the problem: {msg}"
                    );
                }
                Ok(_) => panic!("expected 422 for model {model_id:?} taken_at {taken_at:?}"),
            }
        }

        // A normal ISO timestamp (with its `:`) still passes, and is echoed
        // back untouched. Carries a room because `reject_empty_rooms` refuses an
        // empty push outright -- the identity cases above still reach their own
        // 422 first, since `validate_ingest` runs before the room count is
        // looked at.
        let upload = rooms_upload("m1", good_ts, Some("New Construction"), vec![make_room("r1", "Room 1")]);
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        let (status, Json(body)) = ingest_rooms(State(state), Json(upload)).await.unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.snapshot_taken_at, good_ts);
        assert!(!body.snapshot_id_generated);
    }

    /// One upload for the phase tests, phase supplied per case.
    fn phase_payload(model: &str, ts: &str, phase: Option<&str>) -> RoomsUpload {
        rooms_upload(model, ts, phase, vec![make_room("r1", "Room A")])
    }

    fn phase_state() -> Shared {
        std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None))
    }

    /// Row 1 of the ingest table: a push with no phase is refused outright,
    /// against an unphased lineage as much as a phased one. It means a producer
    /// predating phase support, whose rooms were never filtered to a phase —
    /// unfiltered mixed-phase content, which is the thing phasing exists to keep
    /// out. The message has to say so, or the operator debugs the wrong thing.
    #[tokio::test]
    async fn test_ingest_rejects_a_push_with_no_phase() {
        let state = phase_state();

        let (status, message) =
            ingest_rooms(State(state.clone()), Json(phase_payload("m1", "2026-01-01T00:00:00Z", None)))
                .await
                .expect_err("an unphased push is refused");
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(message.contains("no phase"), "the message names the cause: {message}");
        assert!(message.contains("extractor"), "and points at the producer: {message}");

        // Nothing was stored -- neither live nor quarantined.
        let key = ModelKey { project_id: "p1".into(), model_id: "m1".into() };
        assert!(state.list_snapshot_ids(&key).unwrap().is_empty());
        assert!(state.pending_snapshot(&key).unwrap().is_none(), "a refused push is not quarantined");
    }

    /// A whitespace-only phase is the same case as an absent one: `normalize_phase`
    /// collapses it, so it cannot sneak past the check as a phase literally
    /// named "   ".
    #[tokio::test]
    async fn test_ingest_treats_a_blank_phase_as_absent() {
        let state = phase_state();
        let (status, _) = ingest_rooms(State(state), Json(phase_payload("m1", "2026-01-01T00:00:00Z", Some("   "))))
            .await
            .expect_err("a blank phase is no phase");
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// Rows 2 and 3: the first phased push sets the lineage's phase, and a later
    /// push naming the same phase joins it. Agreement folds whitespace and case,
    /// so a differently-typed but identical phase is not a disagreement.
    #[tokio::test]
    async fn test_ingest_sets_lineage_phase_then_accepts_agreeing_pushes() {
        let state = phase_state();
        let key = ModelKey { project_id: "p1".into(), model_id: "m1".into() };

        let (status, Json(body)) = ingest_rooms(
            State(state.clone()),
            Json(phase_payload("m1", "2026-01-01T00:00:00Z", Some("New Construction"))),
        )
        .await
        .expect("the first phased push is accepted");
        assert_eq!(status, StatusCode::OK);
        assert!(body.accepted);
        assert!(body.models[0].quarantined.is_none());
        assert_eq!(state.model_phase(&key).unwrap().as_deref(), Some("New Construction"));

        // Same phase, typed differently -- still the same phase.
        let (status, Json(body)) = ingest_rooms(
            State(state.clone()),
            Json(phase_payload("m1", "2026-01-02T00:00:00Z", Some("  NEW CONSTRUCTION "))),
        )
        .await
        .expect("an agreeing push is accepted");
        assert_eq!(status, StatusCode::OK);
        assert!(body.accepted);
        // The lineage keeps the *first* push's casing; later spellings don't
        // rewrite it.
        assert_eq!(state.model_phase(&key).unwrap().as_deref(), Some("New Construction"));
    }

    /// Row 4: a push naming a genuinely different phase is stored but not live.
    /// 202 rather than 200 (accepted, not acted upon) and `accepted: false`, so
    /// a producer that ignores the reason string still knows it did not land.
    /// The live snapshot and the lineage's phase are both untouched.
    #[tokio::test]
    async fn test_ingest_quarantines_a_differently_phased_push() {
        let state = phase_state();
        let key = ModelKey { project_id: "p1".into(), model_id: "m1".into() };
        let _ = ingest_rooms(
            State(state.clone()),
            Json(phase_payload("m1", "2026-01-01T00:00:00Z", Some("New Construction"))),
        )
        .await
        .expect("first push");

        let (status, Json(body)) =
            ingest_rooms(State(state.clone()), Json(phase_payload("m1", "2026-06-01T00:00:00Z", Some("Existing"))))
                .await
                .expect("a disagreeing push is kept, not refused");

        assert_eq!(status, StatusCode::ACCEPTED, "202: stored, not acted upon");
        assert!(!body.accepted, "it did not go live");
        let reason = body.models[0].quarantined.clone().expect("a quarantined push says why");
        assert!(
            reason.contains("Existing") && reason.contains("New Construction"),
            "names both phases: {reason}"
        );

        // The model is unchanged: same phase, same latest snapshot.
        assert_eq!(state.model_phase(&key).unwrap().as_deref(), Some("New Construction"));
        assert_eq!(state.list_snapshot_ids(&key).unwrap(), vec!["2026-01-01T00:00:00Z".to_string()]);
        // But the push is retrievable for promotion.
        let pending = state.pending_snapshot(&key).unwrap().expect("quarantined");
        assert_eq!(pending.phase.as_deref(), Some("Existing"));
    }

    /// The streamed route applies the identical rule, and applies it from the
    /// envelope line alone -- a push that will be refused costs the producer one
    /// line, not the whole upload. Both routes share `decide_phase` precisely so
    /// they cannot drift on this.
    #[tokio::test]
    async fn test_stream_ingest_rejects_an_unphased_envelope() {
        let state = phase_state();
        let body = concat!(
            r#"{"schema_version":7,"project":{"id":"p1","name":"P"},"#,
            r#""snapshot":{"taken_at":"2026-01-01T00:00:00Z"},"#,
            r#""models":[{"id":"m1","name":"M","source":"revit","levels":[]}]}"#,
            "\n",
            r#"{"model_id":"m1","id":"r1","name":"Room A","level_id":"1","loops":[]}"#,
            "\n",
        );

        let (status, message) = ingest_rooms_stream(State(state), Body::from(body))
            .await
            .expect_err("an unphased streamed push is refused too");
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(message.contains("no phase"), "same message as the buffered route: {message}");
    }

    /// The ingest response's JSON keys are the producer-facing contract (the
    /// pyRevit client reads this body), so they're asserted as *text*: the
    /// Rust-side assertions elsewhere in this module would survive a rename
    /// that silently broke every consumer.
    #[test]
    fn test_ingest_response_wire_keys() {
        let json = serde_json::to_string(&IngestResponse {
            accepted: true,
            room_count: 26,
            snapshot_taken_at: "2026-07-15T11:18:58.186000Z".to_string(),
            snapshot_id_generated: false,
            models: vec![ModelIngestResult {
                model_id: "m1".to_string(),
                room_count: 26,
                room_boundary: RoomBoundary::FinishFace,
                quarantined: None,
            }],
        })
        .unwrap();

        assert!(json.contains(r#""snapshot_id_generated":false"#), "unexpected wire shape: {json}");
        // An accepted response is byte-for-byte what it always was: the
        // quarantine key is absent, not present-and-null.
        assert!(!json.contains("quarantined"), "an accepted push carries no quarantine key: {json}");
        assert!(json.contains(r#""snapshot_taken_at":"2026-07-15T11:18:58.186000Z""#));
        assert!(json.contains(r#""room_count":26"#));
        // The snake_case spellings are the producer-facing contract too: an
        // extractor sends `"room_boundary": "finish_face"` and reads it back.
        assert!(json.contains(r#""room_boundary":"finish_face""#), "unexpected wire shape: {json}");
        // The per-model block is the producer-facing half of a multi-model
        // push: a run that sent three documents reads here which of them
        // landed, and under what regime.
        assert!(json.contains(r#""model_id":"m1""#), "unexpected wire shape: {json}");
    }

    /// A blank (or omitted -- serde defaults it to blank) snapshot id is no
    /// longer an error: the server mints one and the response carries it, so
    /// the pusher can attach follow-up uploads to the same snapshot.
    #[tokio::test]
    async fn test_ingest_rooms_generates_snapshot_id_when_blank() {
        let upload = rooms_upload("m1", "", Some("New Construction"), vec![make_room("r1", "Room A")]);
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let (_, Json(body)) = ingest_rooms(State(state.clone()), Json(upload)).await.unwrap();

        assert!(body.snapshot_id_generated);
        assert!(crate::contract::validate_snapshot_id(&body.snapshot_taken_at).is_ok());
        // The store keyed the push under exactly the id the response reports.
        let key = crate::state::ModelKey { project_id: "p1".into(), model_id: "m1".into() };
        assert_eq!(state.list_snapshot_ids(&key).unwrap(), vec![body.snapshot_taken_at.clone()]);
    }

    /// A push for a project with no registered settings (and no default
    /// bundle) is rejected 422, not silently stored -- pairs with
    /// `assemble_rooms`'s "skip on read" for the same case.
    #[tokio::test]
    async fn test_ingest_rooms_rejects_unregistered_project() {
        let upload = rooms_upload_for(
            "unregistered",
            "m1",
            "2026-01-01T00:00:00Z",
            Some("New Construction"),
            vec![make_room("r1", "Room A")],
        );
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let result = ingest_rooms(State(state), Json(upload)).await;
        match result {
            Err((status, _)) => assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY),
            Ok(_) => panic!("expected 422 for an unregistered project"),
        }
    }

    /// A push carrying a `model_to_shared` is accepted and the transform is
    /// stored on the snapshot verbatim -- it rides the envelope end to end.
    #[tokio::test]
    async fn test_ingest_rooms_stores_model_to_shared() {
        let matrix = [
            0.9704980833640151,
            -0.2411088347339701,
            0.2411088347339701,
            0.9704980833640151,
            945737.6,
            20545096.5,
        ];
        let mut upload =
            rooms_upload("m1", "2026-01-01T00:00:00Z", Some("New Construction"), vec![make_room("r1", "Room A")]);
        upload.models[0].envelope.model_to_shared = Some(ModelToShared { matrix });
        let state = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let _ = ingest_rooms(State(state.clone() as Shared), Json(upload)).await.expect("accepted");

        let stored = state.all_snapshots(None).unwrap();
        let (_, payload) = stored.iter().find(|(k, _)| k.model_id == "m1").expect("stored");
        // Compared with tolerance, not bit-exactly: storage takes bytes now, so
        // even `MemStore` round-trips the payload through JSON, and a ~1e7 grid
        // coordinate picks up ULP-scale drift crossing that boundary. Absolute
        // 1e-6 is sub-micron in feet. Same reason (and same tolerance) as
        // `contract::tests::test_model_to_shared_round_trips_and_defaults_to_none`
        // — what this test is about is that the transform survives ingest at
        // all, not that f64 is exact through serde.
        let stored_matrix = payload.model_to_shared.expect("carried through").matrix;
        for (a, b) in stored_matrix.iter().zip(matrix.iter()) {
            assert!((a - b).abs() < 1e-6, "transform drifted through storage: {a} vs {b}");
        }
    }

    /// A `model_to_shared` whose linear part isn't a pure rotation (here a 2×
    /// scale, |det| = 4) is still *accepted* -- the drift is a `tracing::warn!`,
    /// never a 422 (advisory only; the geometry still stores and renders).
    #[tokio::test]
    async fn test_ingest_rooms_accepts_non_rigid_model_to_shared() {
        let mut upload =
            rooms_upload("m1", "2026-01-01T00:00:00Z", Some("New Construction"), vec![make_room("r1", "Room A")]);
        upload.models[0].envelope.model_to_shared = Some(ModelToShared { matrix: [2.0, 0.0, 0.0, 2.0, 0.0, 0.0] });
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let (_, Json(body)) = ingest_rooms(State(state), Json(upload)).await.expect("accepted despite det drift");
        assert!(body.accepted);
    }

    /// A declared `room_boundary` rides the envelope through **both** ingest
    /// routes and lands on the stored snapshot. The streamed half is the one
    /// worth testing: it rebuilds a `RoomPayload` field by field from the
    /// envelope line, so a forgotten field there is a silent regime downgrade
    /// on exactly the large models most likely to use that route.
    #[tokio::test]
    async fn test_ingest_stores_room_boundary_on_both_routes() {
        let state = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let mut buffered = rooms_upload(
            "buffered",
            "2026-01-01T00:00:00Z",
            Some("New Construction"),
            vec![make_room("r1", "Room A")],
        );
        buffered.models[0].envelope.room_boundary = Some(RoomBoundary::FinishFace);
        let _ = ingest_rooms(State(state.clone() as Shared), Json(buffered)).await.expect("accepted");

        let body = concat!(
            r#"{"schema_version":7,"project":{"id":"p1","name":"P"},"#,
            r#""snapshot":{"taken_at":"2026-01-01T00:00:00Z"},"phase":"New Construction","#,
            r#""models":[{"id":"streamed","name":"M","source":"revit","room_boundary":"centreline","levels":[]}]}"#,
            "
",
            r#"{"model_id":"streamed","id":"r1","name":"Room A","level_id":"1","loops":[]}"#,
            "\n",
        );
        let _ = ingest_rooms_stream(State(state.clone() as Shared), Body::from(body))
            .await
            .expect("accepted");

        let stored = state.all_snapshots(None).unwrap();
        let of = |model: &str| stored.iter().find(|(k, _)| k.model_id == model).expect("stored").1.room_boundary;
        assert_eq!(of("buffered"), Some(RoomBoundary::FinishFace));
        assert_eq!(of("streamed"), Some(RoomBoundary::Centreline), "the stream path carries it too");
    }

    // ---------- the multi-model push ----------

    /// **The change this all exists for: one push, several models, one snapshot
    /// each.**
    ///
    /// The bucket is a transport shape and must not become a storage one — every
    /// model keeps its own lineage, its own `levels`, and its own place in the
    /// `(project, model)` key everything downstream reads. What it gains is one
    /// shared `taken_at`, which is how "these documents were read together"
    /// becomes expressible at all.
    #[tokio::test]
    async fn test_one_push_decomposes_into_a_snapshot_per_model() {
        let state = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        let upload = RoomsUpload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
            phase: Some("New Construction".to_string()),
            models: vec![
                room_model("arch", vec![make_room("r1", "Ward 1")]),
                room_model("struct", vec![make_room("r1", "Core"), make_room("r2", "Riser")]),
            ],
        };

        let (status, Json(body)) = ingest_rooms(State(state.clone() as Shared), Json(upload)).await.expect("accepted");
        assert_eq!(status, StatusCode::OK);
        assert!(body.accepted);
        assert_eq!(body.room_count, 3, "the total is across the run");
        assert_eq!(body.models.len(), 2);
        assert_eq!(body.models[0].model_id, "arch");
        assert_eq!(body.models[0].room_count, 1);
        assert_eq!(body.models[1].room_count, 2);

        // Two lineages, not one — and the same room id under each is two
        // different rooms, which is the invariant a stored bucket would break.
        let arch = ModelKey { project_id: "p1".into(), model_id: "arch".into() };
        let structural = ModelKey { project_id: "p1".into(), model_id: "struct".into() };
        assert_eq!(state.list_snapshot_ids(&arch).unwrap(), vec!["2026-01-01T00:00:00Z".to_string()]);
        assert_eq!(state.list_snapshot_ids(&structural).unwrap(), vec!["2026-01-01T00:00:00Z".to_string()]);
        let stored = state.all_snapshots(None).unwrap();
        let of = |m: &str| stored.iter().find(|(k, _)| k.model_id == m).expect("stored").1.rooms.len();
        assert_eq!(of("arch"), 1);
        assert_eq!(of("struct"), 2);
    }

    /// One model quarantined does not hold back its siblings. The disagreement
    /// is a fact about that model's lineage and says nothing about the rest of
    /// the run, so the others go live and only the push-level `accepted` turns
    /// false — with 202 so a producer reading the status alone still knows.
    #[tokio::test]
    async fn test_one_model_quarantined_does_not_stop_its_siblings() {
        let state = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        // `arch` is fixed to New Construction by a first push; `struct` is new.
        let _ = ingest_rooms(
            State(state.clone() as Shared),
            Json(rooms_upload(
                "arch",
                "2026-01-01T00:00:00Z",
                Some("New Construction"),
                vec![make_room("r1", "A")],
            )),
        )
        .await
        .expect("first push");

        let upload = RoomsUpload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
            phase: Some("Existing".to_string()),
            models: vec![
                room_model("arch", vec![make_room("r1", "A")]),
                room_model("struct", vec![make_room("r1", "S")]),
            ],
        };
        let (status, Json(body)) = ingest_rooms(State(state.clone() as Shared), Json(upload)).await.expect("stored");

        assert_eq!(status, StatusCode::ACCEPTED, "202: not every model went live");
        assert!(!body.accepted);
        assert!(body.models[0].quarantined.is_some(), "arch disagrees about phase");
        assert!(body.models[1].quarantined.is_none(), "struct had no lineage to disagree with");

        // arch is untouched and its push is promotable; struct is live.
        let arch = ModelKey { project_id: "p1".into(), model_id: "arch".into() };
        let structural = ModelKey { project_id: "p1".into(), model_id: "struct".into() };
        assert_eq!(state.list_snapshot_ids(&arch).unwrap(), vec!["2026-01-01T00:00:00Z".to_string()]);
        assert!(state.pending_snapshot(&arch).unwrap().is_some());
        assert_eq!(state.model_phase(&structural).unwrap().as_deref(), Some("Existing"));
    }

    /// A push declaring the same model twice is refused. Whichever block stored
    /// last would win and the other's rooms would vanish in silence, and merging
    /// them would mean guessing which of two `levels` lists the producer meant.
    #[tokio::test]
    async fn test_a_model_declared_twice_is_refused() {
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        let upload = RoomsUpload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
            phase: Some("New Construction".to_string()),
            models: vec![
                room_model("arch", vec![make_room("r1", "A")]),
                room_model("arch", vec![make_room("r2", "B")]),
            ],
        };

        let (status, message) = ingest_rooms(State(state.clone()), Json(upload)).await.unwrap_err();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(message.contains("more than once"), "{message}");
        assert!(state.all_snapshots(None).unwrap().is_empty(), "nothing stored");
    }

    /// A push declaring no models at all is refused: a push exists because a run
    /// exported at least one document.
    #[tokio::test]
    async fn test_a_push_with_no_models_is_refused() {
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        let upload = RoomsUpload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
            phase: Some("New Construction".to_string()),
            models: vec![],
        };

        let (status, message) = ingest_rooms(State(state), Json(upload)).await.unwrap_err();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(message.contains("declares no models"), "{message}");
    }

    /// **A room line naming an undeclared model is refused, never filed
    /// anywhere.** Room ids are unique only within a model, so quietly putting
    /// it under some other lineage would make it resolve against real-looking
    /// rooms instead of failing — the failure mode the per-line `model_id`
    /// exists to make impossible.
    #[tokio::test]
    async fn test_a_room_line_naming_an_undeclared_model_is_refused() {
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        let body = concat!(
            r#"{"schema_version":7,"project":{"id":"p1","name":"P"},"#,
            r#""snapshot":{"taken_at":"2026-01-01T00:00:00Z"},"phase":"New Construction","#,
            r#""models":[{"id":"arch","name":"M","source":"revit","levels":[]}]}"#,
            "\n",
            r#"{"model_id":"struct","id":"r1","name":"Room A","level_id":"1","loops":[]}"#,
            "\n",
        );

        let (status, message) = ingest_rooms_stream(State(state.clone()), Body::from(body)).await.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(message.contains("struct") && message.contains("arch"), "names both: {message}");
        assert!(state.all_snapshots(None).unwrap().is_empty(), "nothing stored");
    }

    /// The streamed route decomposes identically to the buffered one — two
    /// models interleaved on the wire land in two lineages, each with only its
    /// own rooms. Interleaving is the case worth writing: the producer emits
    /// model by model today, and nothing on the wire says it must.
    #[tokio::test]
    async fn test_streamed_multi_model_push_decomposes_the_same_way() {
        let state = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        let body = concat!(
            r#"{"schema_version":7,"project":{"id":"p1","name":"P"},"#,
            r#""snapshot":{"taken_at":"2026-01-01T00:00:00Z"},"phase":"New Construction","#,
            r#""models":[{"id":"arch","name":"A","source":"revit","levels":[]},"#,
            r#"{"id":"struct","name":"S","source":"revit","levels":[]}]}"#,
            "\n",
            r#"{"model_id":"arch","id":"r1","name":"Ward","level_id":"1","loops":[]}"#,
            "\n",
            r#"{"model_id":"struct","id":"r1","name":"Core","level_id":"1","loops":[]}"#,
            "\n",
            r#"{"model_id":"arch","id":"r2","name":"Store","level_id":"1","loops":[]}"#,
            "\n",
        );

        let (status, Json(body)) = ingest_rooms_stream(State(state.clone() as Shared), Body::from(body))
            .await
            .expect("accepted");
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.room_count, 3);

        let stored = state.all_snapshots(None).unwrap();
        let of = |m: &str| {
            let (_, p) = stored.iter().find(|(k, _)| k.model_id == m).expect("stored");
            p.rooms.iter().map(|r| r.name.clone()).collect::<Vec<_>>()
        };
        assert_eq!(
            of("arch"),
            vec!["Ward".to_string(), "Store".to_string()],
            "in wire order, and only arch's"
        );
        assert_eq!(of("struct"), vec!["Core".to_string()]);
    }

    /// **One empty model refuses the whole push, and publishes none of it.**
    ///
    /// The property the check-all-then-commit split in `finish_rooms` exists
    /// for. Elements now go to disk as they are read, so by the time the empty
    /// model is recognised the *other* models have already been written -- and
    /// committing them before discovering the fault would leave half a run live
    /// under a snapshot id that claims the whole run was read together.
    ///
    /// The sinks are dropped instead, and a dropped sink leaves no trace.
    #[tokio::test]
    async fn test_one_empty_model_refuses_the_whole_push_and_stores_nothing() {
        let dir = std::env::temp_dir().join(format!("roommate-handler-empty-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let store = crate::storage::FsStore::new(dir.clone()).unwrap();
        let state = std::sync::Arc::new(AppState::new(Box::new(store), single_project("p1"), None));

        let upload = RoomsUpload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
            phase: Some("New Construction".to_string()),
            models: vec![
                room_model("full", vec![make_room("r1", "Ward")]),
                room_model("empty", vec![]),
            ],
        };

        let (status, message) = ingest_rooms(State(state.clone() as Shared), Json(upload)).await.unwrap_err();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(message.contains("contains no rooms"), "{message}");
        assert!(message.contains("empty"), "names the offending model: {message}");

        // Neither model landed -- not even the one whose rooms were written.
        assert!(state.all_snapshots(None).unwrap().is_empty(), "nothing was published");
        let full = ModelKey { project_id: "p1".into(), model_id: "full".into() };
        assert!(state.list_snapshot_ids(&full).unwrap().is_empty(), "the good model was discarded too");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The streamed rooms path writes one element per line and it parses back
    /// unchanged -- properties, geometry and envelope intact.
    ///
    /// Worth asserting on the *file* rather than only through a read: the
    /// snapshot is assembled by hand now (an object, a spliced array key, one
    /// element per line, a closing bracket), so a missing comma or a stray one
    /// would produce a file that no read path could ever recover.
    #[tokio::test]
    async fn test_a_streamed_snapshot_is_one_element_per_line_and_parses_back() {
        let dir = std::env::temp_dir().join(format!("roommate-handler-lines-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let store = crate::storage::FsStore::new(dir.clone()).unwrap();
        let state = std::sync::Arc::new(AppState::new(Box::new(store), single_project("p1"), None));

        let body = concat!(
            r#"{"schema_version":7,"project":{"id":"p1","name":"P"},"#,
            r#""snapshot":{"taken_at":"2026-01-01T00:00:00Z"},"phase":"New Construction","#,
            r#""models":[{"id":"m1","name":"M","source":"revit","room_boundary":"centreline","#,
            r#""levels":[{"id":"lvl1","name":"Level 1","elevation":0.0}]}]}"#,
            "\n",
            r#"{"model_id":"m1","id":"r1","name":"Ward","level_id":"lvl1","loops":[],"properties":{"Number":{"value":"1","storage_type":"String"}}}"#,
            "\n",
            r#"{"model_id":"m1","id":"r2","name":"Store","level_id":"lvl1","loops":[],"properties":{}}"#,
            "\n",
        );
        let (status, _) = ingest_rooms_stream(State(state.clone() as Shared), Body::from(body))
            .await
            .expect("accepted");
        assert_eq!(status, StatusCode::OK);

        // The stored payload round-trips through the normal read path.
        let stored = state.all_snapshots(None).unwrap();
        let (_, payload) = stored.iter().find(|(k, _)| k.model_id == "m1").expect("stored");
        assert_eq!(payload.rooms.len(), 2);
        assert_eq!(payload.rooms[0].name, "Ward");
        assert_eq!(payload.rooms[1].name, "Store");
        assert_eq!(payload.levels.len(), 1, "the envelope survived the splice");
        assert_eq!(payload.room_boundary, Some(RoomBoundary::Centreline));
        assert_eq!(payload.phase.as_deref(), Some("New Construction"));
        assert_eq!(payload.rooms[0].properties.get("Number").map(|v| v.value.as_str()), Some("1"));

        // And the file itself is one room per line, which is what makes it
        // greppable and what a single long line would have cost.
        let file = dir.join("p1").join("m1").join("2026-01-01T00-00-00Z.json");
        let text = std::fs::read_to_string(&file).expect("snapshot file");
        let room_lines = text.lines().filter(|l| l.contains(r#""level_id""#)).count();
        assert_eq!(room_lines, 2, "one room per line, not one long line:\n{text}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A quarantined model buffers while its live siblings stream, and both end
    /// up right: the live one is published, the quarantined one is inert and
    /// promotable. The two destinations are the one place `finish_rooms`
    /// branches, so this is what proves the branch.
    #[tokio::test]
    async fn test_a_quarantined_model_and_a_live_one_in_one_push() {
        let state = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        // `arch` is fixed to New Construction; `struct` has no lineage yet.
        let _ = ingest_rooms(
            State(state.clone() as Shared),
            Json(rooms_upload(
                "arch",
                "2026-01-01T00:00:00Z",
                Some("New Construction"),
                vec![make_room("r1", "A")],
            )),
        )
        .await
        .expect("first push");

        let upload = RoomsUpload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
            phase: Some("Existing".to_string()),
            models: vec![
                room_model("arch", vec![make_room("r1", "A2")]),
                room_model("struct", vec![make_room("r1", "S")]),
            ],
        };
        let (status, Json(body)) = ingest_rooms(State(state.clone() as Shared), Json(upload)).await.expect("stored");
        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(body.models[0].quarantined.is_some(), "arch buffered into quarantine");
        assert!(body.models[1].quarantined.is_none(), "struct streamed live");

        let arch = ModelKey { project_id: "p1".into(), model_id: "arch".into() };
        let pending = state.pending_snapshot(&arch).unwrap().expect("quarantined and promotable");
        assert_eq!(pending.rooms.len(), 1, "the buffered rooms reached the pending slot");
        assert_eq!(pending.rooms[0].name, "A2");
        assert_eq!(pending.phase.as_deref(), Some("Existing"));

        let structural = ModelKey { project_id: "p1".into(), model_id: "struct".into() };
        assert_eq!(
            state.model_phase(&structural).unwrap().as_deref(),
            Some("Existing"),
            "the live one went live"
        );
    }

    // ---------- doors ingest ----------

    fn make_door(id: &str, from_room: Option<&str>, to_room: Option<&str>) -> Opening {
        Opening {
            id: id.to_string(),
            level_id: "1".to_string(),
            loops: vec![],
            from_room: from_room.map(str::to_string),
            to_room: to_room.map(str::to_string),
            // These helpers exercise references and properties, never placement,
            // so `None` here is the honest input rather than a stub: a door with
            // no position and no direction is a state the contract carries.
            insertion_point: None,
            through_wall_normal: None,
            type_id: "t1".to_string(),
            type_name: "Single".to_string(),
            properties: BTreeMap::new(),
            type_properties: BTreeMap::new(),
        }
    }

    fn door_payload(project: &str, model: &str, ts: &str, phase: Option<&str>) -> DoorsUpload {
        DoorsUpload {
            schema_version: SUPPORTED_DOOR_SCHEMA,
            project: Project { id: project.to_string(), name: "P".to_string() },
            snapshot: Snapshot { taken_at: ts.to_string() },
            phase: phase.map(str::to_string),
            models: vec![crate::contract::doors::DoorModelUpload {
                envelope: DoorModelEnvelope {
                    model: Model { id: model.to_string(), name: "M".to_string(), source: "revit".to_string() },
                    model_to_shared: None,
                    levels: vec![],
                },
                doors: vec![make_door("d1", Some("r1"), None)],
            }],
        }
    }

    /// A state whose `p1/m1` lineage already has rooms in `phase` — the
    /// precondition every accepted doors push needs.
    async fn state_with_rooms(phase: Option<&str>) -> Shared {
        let state = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        state
            .set_snapshot(RoomPayload {
                schema_version: SUPPORTED_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
                phase: phase.map(str::to_string),
                model_to_shared: None,
                room_boundary: None,
                levels: vec![],
                rooms: vec![make_room("r1", "Room A")],
            })
            .unwrap();
        state
    }

    /// The happy path: doors land in their own storage slot, leaving the rooms
    /// snapshot exactly as it was.
    #[tokio::test]
    async fn test_ingest_doors_stores_alongside_rooms() {
        let state = state_with_rooms(Some("New Construction")).await;
        let payload = door_payload("p1", "m1", "2026-02-01T00:00:00Z", Some("New Construction"));

        let (status, body) = ingest_doors(State(state.clone()), Json(payload)).await.expect("accepted");
        assert_eq!(status, StatusCode::OK);
        assert!(body.accepted);
        assert_eq!(body.door_count, 1);
        assert!(!body.snapshot_id_generated);

        let key = ModelKey { project_id: "p1".into(), model_id: "m1".into() };
        let stored = state.all_door_snapshots(None).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].1.doors[0].from_room.as_deref(), Some("r1"));
        // The rooms lineage is untouched: same snapshot, same single id.
        assert_eq!(state.list_snapshot_ids(&key).unwrap(), vec!["2026-01-01T00:00:00Z".to_string()]);
        assert_eq!(state.list_door_snapshot_ids(&key).unwrap(), vec!["2026-02-01T00:00:00Z".to_string()]);
    }

    /// **An empty rooms push is refused, and this is the regression it guards.**
    ///
    /// On 2026-08-03 an extractor whose phase filter matched nothing sent five
    /// consecutive zero-room pushes. Every one was accepted, indexed and served
    /// as "this model has no rooms"; the only signal anything was wrong was a
    /// person noticing the drawing was blank.
    ///
    /// Both ingest paths are covered, because they count rooms in different
    /// places — the buffered one off the deserialized payload, the streaming one
    /// as it reads lines — and a guard on only one of them is a guard with a
    /// documented way around it.
    #[tokio::test]
    async fn test_rooms_push_with_no_rooms_is_refused() {
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        let upload = rooms_upload("m1", "2026-01-01T00:00:00Z", Some("New Construction"), vec![]);

        let (status, message) = ingest_rooms(State(state.clone()), Json(upload)).await.unwrap_err();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(message.contains("contains no rooms"), "{message}");
        // Names the phase, because a filter that matched nothing is what this
        // nearly always is, and "0 rooms" alone sends people to the server.
        assert!(message.contains("New Construction"), "message names the phase: {message}");
        // Nothing was stored: a refused push must not leave a lineage behind.
        let key = ModelKey { project_id: "p1".into(), model_id: "m1".into() };
        assert!(state.list_snapshot_ids(&key).unwrap().is_empty(), "nothing stored");
    }

    /// The streaming path counts rooms separately, so it gets the guard
    /// separately. Envelope line, then no room lines at all.
    #[tokio::test]
    async fn test_streamed_rooms_push_with_no_rooms_is_refused() {
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        let envelope = serde_json::json!({
            "schema_version": SUPPORTED_SCHEMA,
            "project": { "id": "p1", "name": "P" },
            "snapshot": { "taken_at": "2026-01-01T00:00:00Z" },
            "phase": "New Construction",
            "models": [{ "id": "m1", "name": "M", "source": "revit", "levels": [] }],
        });
        let body = Body::from(format!(
            "{envelope}
"
        ));

        let (status, message) = ingest_rooms_stream(State(state), body).await.unwrap_err();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(message.contains("contains no rooms"), "{message}");
    }

    /// **A doors push against an EMPTY rooms snapshot is accepted**, and is
    /// reported rather than refused.
    ///
    /// This is where the 2026-08-03 regression used to be caught: the gate
    /// asked whether a rooms snapshot *file* existed, which an empty one does,
    /// so 26 doors referencing 22 room ids were stored against zero rooms. The
    /// gate that fixed it is gone — ingest no longer requires rooms at all —
    /// and the regression is guarded from both ends instead: `reject_empty_rooms`
    /// stops the empty snapshot being *written*, and `door_report` reports every
    /// unresolvable reference on every read rather than once, at the push.
    ///
    /// What this asserts is that the doors still land, because that is what
    /// makes them visible to the report that now owns the question.
    #[tokio::test]
    async fn test_doors_push_against_an_empty_rooms_snapshot_is_stored() {
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        let key = ModelKey { project_id: "p1".into(), model_id: "m1".into() };

        // Write an empty rooms snapshot directly, as the five real ones were —
        // ingest would now refuse it, and that is the point of going round it.
        let empty = RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
            phase: Some("New Construction".to_string()),
            model_to_shared: None,
            room_boundary: None,
            levels: vec![],
            rooms: vec![],
        };
        state.set_snapshot(empty).unwrap();
        assert!(!state.list_snapshot_ids(&key).unwrap().is_empty(), "a snapshot file exists");

        let payload = door_payload("p1", "m1", "2026-02-01T00:00:00Z", Some("New Construction"));
        let (status, _) = ingest_doors(State(state.clone()), Json(payload)).await.expect("stored, not refused");
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            state.all_door_snapshots(None).unwrap().len(),
            1,
            "the doors are on disk for QA to report on"
        );
    }

    /// **Doors may be pushed before their rooms**, which is the whole point of
    /// removing the rooms-first gate.
    ///
    /// The gate used to refuse this, on the reasoning that a door's
    /// `from_room`/`to_room` have nothing to resolve against. That reasoning
    /// only held while the export's ids were the sole answer to "which room";
    /// with the server resolving a door's rooms from its own position, "not yet"
    /// is a legitimate state, and refusing meant refusing data that becomes
    /// resolvable the moment the rooms arrive.
    #[tokio::test]
    async fn test_doors_push_to_a_model_without_rooms_is_accepted() {
        let state = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        let payload = door_payload("p1", "m1", "2026-02-01T00:00:00Z", Some("New Construction"));

        let (status, Json(body)) = ingest_doors(State(state.clone() as Shared), Json(payload))
            .await
            .expect("doors may arrive first");
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.door_count, 1);
        assert_eq!(state.all_door_snapshots(None).unwrap().len(), 1);
        // And it phased the lineage, exactly as a first rooms push would have.
        let key = ModelKey { project_id: "p1".into(), model_id: "m1".into() };
        assert_eq!(state.model_phase(&key).unwrap().as_deref(), Some("New Construction"));
    }

    /// **Doors diverge from rooms here.** A rooms push whose phase disagrees is
    /// quarantined and promotable; a doors push is refused, because promoting it
    /// would re-phase the lineage while the rooms stayed behind, leaving these
    /// doors pointing at another phase's rooms.
    #[tokio::test]
    async fn test_doors_push_with_a_disagreeing_phase_is_refused_not_quarantined() {
        let state = state_with_rooms(Some("New Construction")).await;
        let payload = door_payload("p1", "m1", "2026-02-01T00:00:00Z", Some("Existing"));

        let (status, message) = ingest_doors(State(state.clone()), Json(payload)).await.unwrap_err();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(message.contains("Existing") && message.contains("New Construction"), "{message}");
        assert!(state.all_door_snapshots(None).unwrap().is_empty());
        let key = ModelKey { project_id: "p1".into(), model_id: "m1".into() };
        assert!(state.pending_snapshot(&key).unwrap().is_none(), "never quarantined");
        assert_eq!(state.model_phase(&key).unwrap().as_deref(), Some("New Construction"), "lineage unmoved");
    }

    /// Phase folding is the contract's, not a second implementation: a doors
    /// push differing only in case and surrounding whitespace agrees.
    #[tokio::test]
    async fn test_doors_phase_comparison_folds_case_and_whitespace() {
        let state = state_with_rooms(Some("New Construction")).await;
        let payload = door_payload("p1", "m1", "2026-02-01T00:00:00Z", Some("  new CONSTRUCTION "));

        let (status, _) = ingest_doors(State(state.clone()), Json(payload)).await.expect("accepted");
        assert_eq!(status, StatusCode::OK);
    }

    /// A doors push carrying no phase is refused, exactly as a rooms one is:
    /// unfiltered doors are a mix of every phase and there is no safe default.
    #[tokio::test]
    async fn test_doors_push_without_a_phase_is_refused() {
        let state = state_with_rooms(Some("New Construction")).await;
        let payload = door_payload("p1", "m1", "2026-02-01T00:00:00Z", None);

        let (status, message) = ingest_doors(State(state.clone()), Json(payload)).await.unwrap_err();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(message.contains("carries no phase"), "{message}");
    }

    /// An **unphased** lineage — a model whose rooms were pushed before phasing
    /// existed — accepts a doors push and is phased by it, exactly as a first
    /// phased rooms push would phase it. This is the live case for the House A
    /// sample, whose stored rooms snapshot carries no phase.
    #[tokio::test]
    async fn test_doors_push_phases_an_unphased_lineage() {
        let state = state_with_rooms(None).await;
        let key = ModelKey { project_id: "p1".into(), model_id: "m1".into() };
        assert_eq!(state.model_phase(&key).unwrap(), None);

        let payload = door_payload("p1", "m1", "2026-02-01T00:00:00Z", Some("New Construction"));
        let (status, _) = ingest_doors(State(state.clone()), Json(payload)).await.expect("accepted");
        assert_eq!(status, StatusCode::OK);
        assert_eq!(state.model_phase(&key).unwrap().as_deref(), Some("New Construction"));

        // The rooms snapshot still reports itself unphased — it was, and a later
        // doors push does not retroactively relabel what was stored.
        let stored = state.all_snapshots(None).unwrap();
        assert_eq!(stored[0].1.phase, None);
    }

    /// Doors version independently of rooms: a payload stamped with the *room*
    /// schema is refused, and the message names the doors schema so a producer
    /// is told which number it should be sending.
    #[tokio::test]
    async fn test_doors_push_with_the_room_schema_version_is_refused() {
        let state = state_with_rooms(Some("New Construction")).await;
        let mut payload = door_payload("p1", "m1", "2026-02-01T00:00:00Z", Some("New Construction"));
        payload.schema_version = SUPPORTED_SCHEMA;

        let (status, message) = ingest_doors(State(state.clone()), Json(payload)).await.unwrap_err();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(message.contains(&format!("this server speaks {SUPPORTED_DOOR_SCHEMA}")), "{message}");
    }

    /// A blank `taken_at` is minted server-side and reported, the same
    /// `ensure_taken_at` contract rooms have — not a second implementation.
    #[tokio::test]
    async fn test_doors_push_mints_a_blank_snapshot_id() {
        let state = state_with_rooms(Some("New Construction")).await;
        let payload = door_payload("p1", "m1", "", Some("New Construction"));

        let (_, body) = ingest_doors(State(state.clone()), Json(payload)).await.expect("accepted");
        assert!(body.snapshot_id_generated);
        assert!(crate::contract::validate_snapshot_id(&body.snapshot_taken_at).is_ok());
    }

    /// The buffered and streamed doors routes must store identical results —
    /// which route a producer picked cannot change what is stored, the same
    /// lockstep rule the rooms pair has.
    #[tokio::test]
    async fn test_doors_stream_matches_the_buffered_path() {
        let state = state_with_rooms(Some("New Construction")).await;

        let body = concat!(
            r#"{"schema_version":2,"project":{"id":"p1","name":"P"},"#,
            r#""snapshot":{"taken_at":"2026-02-01T00:00:00Z"},"phase":"New Construction","#,
            r#""models":[{"id":"m1","name":"M","source":"revit"}]}"#,
            "\n",
            r#"{"model_id":"m1","id":"d1","level_id":"1","loops":[],"from_room":"r1","type_id":"t1","type_name":"Single"}"#,
            "\n",
            "\n", // a trailing blank line is tolerated, as on the rooms stream
        );
        let (status, streamed) = ingest_doors_stream(State(state.clone()), Body::from(body)).await.expect("accepted");
        assert_eq!(status, StatusCode::OK);
        assert_eq!(streamed.door_count, 1);

        let key = ModelKey { project_id: "p1".into(), model_id: "m1".into() };
        let stored = state.get_door_snapshot(&key, "2026-02-01T00:00:00Z").unwrap().expect("stored");
        let door = &stored.doors[0];
        assert_eq!(door.id, "d1");
        assert_eq!(door.from_room.as_deref(), Some("r1"));
        assert_eq!(door.to_room, None, "an external door streams as None, not an error");
    }

    /// The stream route refuses on the envelope line alone, before reading any
    /// door lines — a push that will be refused should cost the producer one
    /// line, not the whole body. The malformed line below is never parsed, and
    /// that is the assertion: the error is the envelope's, not the parser's.
    ///
    /// The refusal under test is the **phase** one. It used to be the rooms
    /// gate, which no longer exists — see `check_doors_ingest`.
    #[tokio::test]
    async fn test_doors_stream_refuses_from_the_envelope_alone() {
        let state = state_with_rooms(Some("New Construction")).await;
        let body = concat!(
            r#"{"schema_version":2,"project":{"id":"p1","name":"P"},"#,
            r#""snapshot":{"taken_at":"2026-02-01T00:00:00Z"},"phase":"Existing","#,
            r#""models":[{"id":"m1","name":"M","source":"revit"}]}"#,
            "\n",
            r#"{"this is not a door and is never parsed"#, // malformed on purpose
        );

        let (status, message) = ingest_doors_stream(State(state.clone()), Body::from(body)).await.unwrap_err();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "the gate ran, not the line parser");
        assert!(message.contains("Existing") && message.contains("New Construction"), "{message}");
    }

    /// An unregistered project is refused for doors exactly as for rooms — a
    /// project must be onboarded before it can push anything.
    #[tokio::test]
    async fn test_doors_push_to_an_unregistered_project_is_refused() {
        let state = state_with_rooms(Some("New Construction")).await;
        let payload = door_payload("nope", "m1", "2026-02-01T00:00:00Z", Some("New Construction"));

        let (status, message) = ingest_doors(State(state.clone()), Json(payload)).await.unwrap_err();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(message.contains("no settings configured"), "{message}");
    }
}
