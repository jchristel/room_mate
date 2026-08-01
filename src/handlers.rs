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
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncBufReadExt;
use tokio_util::io::StreamReader;

use crate::contract::{
    Door, DoorPayload, DoorStreamEnvelope, ModelToShared, Room, RoomBoundary, RoomPayload, StreamEnvelope,
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
use crate::service::{doors, milestones, projects, reference, rooms, snapshots, validation, ServiceError};
use crate::state::{ModelKey, Shared};

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
fn validate_ingest(
    state: &Shared,
    schema_version: u32,
    supported_schema: u32,
    project_id: &str,
    model_id: &str,
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
    validate_id("model", model_id)?;
    validate_taken_at(taken_at)
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

/// Revit posts room data here. Returns 200 with a short summary, or 422 if the
/// payload fails any `validate_ingest` check. A blank/omitted snapshot id is
/// minted server-side first (`ensure_taken_at`); the response always carries
/// the resolved id so the pusher can attach follow-up uploads to it.
pub async fn ingest_rooms(
    State(state): State<Shared>,
    Json(mut payload): Json<RoomPayload>,
) -> Result<(StatusCode, Json<IngestResponse>), (StatusCode, String)> {
    let snapshot_id_generated = crate::contract::ensure_taken_at(&mut payload.snapshot);
    payload.phase = crate::contract::normalize_phase(payload.phase.as_deref());
    validate_ingest(
        &state,
        payload.schema_version,
        SUPPORTED_SCHEMA,
        &payload.project.id,
        &payload.model.id,
        &payload.snapshot.taken_at,
    )?;
    let key = ModelKey::from_payload(&payload);
    let decision = decide_phase(&state, &key, payload.phase.as_deref())?;
    warn_on_transform_drift(payload.model_to_shared.as_ref(), &payload.project.id, &payload.model.id);

    let count = payload.rooms.len();
    let snapshot_taken_at = payload.snapshot.taken_at.clone();
    let room_boundary = resolved_boundary(&state, &payload.project.id, payload.room_boundary);
    tracing::info!("received {} room(s)", count);

    store_or_quarantine(&state, &key, payload, decision).map(|(status, quarantined)| {
        (
            status,
            Json(IngestResponse {
                accepted: quarantined.is_none(),
                room_count: count,
                snapshot_taken_at,
                snapshot_id_generated,
                room_boundary,
                quarantined,
            }),
        )
    })
}

/// Commit one push according to its `PhaseDecision`: live via `set_snapshot`, or
/// quarantined via `set_pending_snapshot`. Returns the status to answer with and
/// the quarantine reason, if any.
///
/// Shared by both ingest routes so the buffered and streamed paths cannot
/// disagree about what a phase disagreement does — the same reason
/// `validate_ingest` is shared.
fn store_or_quarantine(
    state: &Shared,
    key: &ModelKey,
    payload: RoomPayload,
    decision: PhaseDecision,
) -> Result<(StatusCode, Option<String>), (StatusCode, String)> {
    // A storage failure (unwritable disk, etc.) is a real server error, not a
    // bad request — surface it as 500 rather than swallowing it.
    let fail = |e: anyhow::Error| {
        tracing::error!("failed to store snapshot: {e:#}");
        (StatusCode::INTERNAL_SERVER_ERROR, format!("could not store snapshot: {e}"))
    };

    match decision {
        PhaseDecision::Accept => {
            state.set_snapshot(payload).map_err(fail)?;
            Ok((StatusCode::OK, None))
        }
        PhaseDecision::Quarantine { lineage } => {
            let pushed = payload.phase.clone().unwrap_or_default();
            state.set_pending_snapshot(key, &payload).map_err(fail)?;
            tracing::warn!(
                "quarantined push for {}/{}: phase {:?} disagrees with the model's {:?}",
                key.project_id,
                key.model_id,
                pushed,
                lineage
            );
            // 202, not 200: the payload was accepted and stored, but has not
            // been acted upon — it is inert until someone promotes it.
            Ok((
                StatusCode::ACCEPTED,
                Some(format!(
                    "stored but not live: this push is phase {pushed:?} while the model is {lineage:?}. \
                     A model's phase is fixed once set; activate this push to re-phase the model."
                )),
            ))
        }
    }
}

#[derive(Debug, Serialize)]
pub struct IngestResponse {
    pub accepted: bool,
    pub room_count: usize,
    /// The snapshot id this push was stored under — echoed back (or minted,
    /// see `snapshot_id_generated`) so the pusher can associate follow-up
    /// uploads with this exact snapshot.
    pub snapshot_taken_at: String,
    /// True when the server minted the id above because the payload left it
    /// blank; false when the payload supplied one and the server used it.
    ///
    /// It describes the *id*, not the snapshot: whether a snapshot was stored
    /// is reported by `accepted`/`room_count`. A producer that stamps its own
    /// `taken_at` (as the Revit one always does) therefore sees `false` here
    /// on every successful push.
    pub snapshot_id_generated: bool,

    /// Why this push was stored but **not** made live, when that happened —
    /// its phase disagrees with the one the model's lineage is fixed to. `None`
    /// on a normal push, and omitted from the JSON entirely so an accepted
    /// response looks exactly as it always did.
    ///
    /// Paired with a `202 Accepted` rather than the usual `200`, and with
    /// `accepted: false`: the data is safely stored and promotable, but nothing
    /// reads it yet. A producer that ignores this field sees `accepted: false`
    /// and knows its push did not go live, which is the important half.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quarantined: Option<String>,

    /// The boundary regime the server **resolved** for this model — what the
    /// envelope declared, or, when it declared nothing, what the project's
    /// `[areas] boundary_location` supplied, or finish face.
    ///
    /// Echoed for the same reason `snapshot_taken_at` is: a producer that left
    /// the field off should be able to see what the server assumed on its
    /// behalf, rather than discovering it later in a footprint that came out
    /// the wrong size. The *resolved* value, not the declared one, precisely
    /// because the interesting case is the one the producer did not state.
    pub room_boundary: RoomBoundary,
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
/// Rooms are still accumulated into a `Vec` before handing the assembled
/// `RoomPayload` to the existing store, so storage and everything downstream
/// stays byte-for-byte identical to the buffered path -- streaming changes
/// only how the body is *read*. Honest limitation: peak memory is therefore
/// the in-memory room set, not the raw JSON text (still a real win, since the
/// text is ~40% empty-string overhead). If even that Vec is too large, the
/// next step is a `SnapshotStore::put_streaming` that writes rooms to disk as
/// they arrive -- deferred until the Vec itself is the ceiling.
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
    // envelope is parsed, before the (potentially large) room stream is read.
    let snapshot_id_generated = crate::contract::ensure_taken_at(&mut envelope.snapshot);
    envelope.phase = crate::contract::normalize_phase(envelope.phase.as_deref());
    validate_ingest(
        &state,
        envelope.schema_version,
        SUPPORTED_SCHEMA,
        &envelope.project.id,
        &envelope.model.id,
        &envelope.snapshot.taken_at,
    )?;
    // Decided from the envelope alone, before a single room line is read: a
    // push that will be refused should cost the producer one line, not a
    // hundred megabytes of upload.
    let key = ModelKey { project_id: envelope.project.id.clone(), model_id: envelope.model.id.clone() };
    let decision = decide_phase(&state, &key, envelope.phase.as_deref())?;
    warn_on_transform_drift(envelope.model_to_shared.as_ref(), &envelope.project.id, &envelope.model.id);

    let mut rooms: Vec<Room> = Vec::new();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("read error: {e}")))?
    {
        if line.trim().is_empty() {
            continue; // tolerate a trailing blank line
        }
        let room: Room =
            serde_json::from_str(&line).map_err(|e| (StatusCode::BAD_REQUEST, format!("bad room line: {e}")))?;
        rooms.push(room);
    }

    let count = rooms.len();
    tracing::info!("streamed {} room(s)", count);

    let snapshot_taken_at = envelope.snapshot.taken_at.clone();
    let room_boundary = resolved_boundary(&state, &envelope.project.id, envelope.room_boundary);
    let payload = RoomPayload {
        schema_version: envelope.schema_version,
        project: envelope.project,
        model: envelope.model,
        snapshot: envelope.snapshot,
        phase: envelope.phase,
        model_to_shared: envelope.model_to_shared,
        room_boundary: envelope.room_boundary,
        levels: envelope.levels,
        rooms,
    };

    store_or_quarantine(&state, &key, payload, decision).map(|(status, quarantined)| {
        (
            status,
            Json(IngestResponse {
                accepted: quarantined.is_none(),
                room_count: count,
                snapshot_taken_at,
                snapshot_id_generated,
                room_boundary,
                quarantined,
            }),
        )
    })
}

/// The doors half of the ingest contract: the two checks a doors push has that
/// a rooms push does not, applied after `validate_ingest` and before anything
/// is stored.
///
/// **1. The model must already have rooms.** A door's `from_room`/`to_room` are
/// `Room.id`s, and room ids are unique only *within* a model — so a doors push
/// to a model with no rooms stores references nothing can ever resolve. Scoped
/// to the `(project, model)` lineage rather than the project for exactly that
/// reason: rooms under a *sibling* model are the wrong id space, so they would
/// satisfy a project-wide gate while resolving nothing.
///
/// **2. The phase must match the lineage, and disagreement is refused.** This is
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
/// as a first phased rooms push would. That is the live case for a model whose
/// rooms were pushed before phasing existed: its rooms snapshot keeps reporting
/// itself unphased (it was), and the QA phase report keeps saying so.
fn check_doors_ingest(state: &Shared, key: &ModelKey, pushed: Option<&str>) -> Result<(), (StatusCode, String)> {
    let has_rooms = state.has_room_snapshot(key).map_err(|e| {
        tracing::error!("failed to read rooms index: {e:#}");
        (StatusCode::INTERNAL_SERVER_ERROR, format!("could not read rooms index: {e}"))
    })?;
    if !has_rooms {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "no rooms have been pushed for {}/{}, so this model's doors have nothing to link to. \
                 Push rooms for this model first — a door's from_room/to_room are room ids, and room \
                 ids are unique only within one model.",
                key.project_id, key.model_id
            ),
        ));
    }

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
    pub accepted: bool,
    pub door_count: usize,
    /// The snapshot id this push was stored under — echoed back (or minted) on
    /// the same terms as `IngestResponse::snapshot_taken_at`.
    pub snapshot_taken_at: String,
    pub snapshot_id_generated: bool,
}

/// Revit posts door data here. Mirrors `ingest_rooms` — same envelope
/// resolution, same pre-flight — plus `check_doors_ingest`.
///
/// **There is no quarantine branch and so no 202**, unlike rooms: a doors push
/// either goes live or is refused. See `check_doors_ingest` for why.
pub async fn ingest_doors(
    State(state): State<Shared>,
    Json(mut payload): Json<DoorPayload>,
) -> Result<(StatusCode, Json<DoorIngestResponse>), (StatusCode, String)> {
    let snapshot_id_generated = crate::contract::ensure_taken_at(&mut payload.snapshot);
    payload.phase = crate::contract::normalize_phase(payload.phase.as_deref());
    validate_ingest(
        &state,
        payload.schema_version,
        SUPPORTED_DOOR_SCHEMA,
        &payload.project.id,
        &payload.model.id,
        &payload.snapshot.taken_at,
    )?;
    let key = ModelKey::from_door_payload(&payload);
    check_doors_ingest(&state, &key, payload.phase.as_deref())?;
    warn_on_transform_drift(payload.model_to_shared.as_ref(), &payload.project.id, &payload.model.id);

    let count = payload.doors.len();
    let snapshot_taken_at = payload.snapshot.taken_at.clone();
    tracing::info!("received {} door(s)", count);

    state.set_door_snapshot(payload).map_err(|e| {
        tracing::error!("failed to store doors snapshot: {e:#}");
        (StatusCode::INTERNAL_SERVER_ERROR, format!("could not store doors snapshot: {e}"))
    })?;

    Ok((
        StatusCode::OK,
        Json(DoorIngestResponse { accepted: true, door_count: count, snapshot_taken_at, snapshot_id_generated }),
    ))
}

/// Streaming doors ingest (NDJSON), the counterpart to `ingest_rooms_stream`:
/// line 1 is the envelope, every following line is one `Door`.
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

    let snapshot_id_generated = crate::contract::ensure_taken_at(&mut envelope.snapshot);
    envelope.phase = crate::contract::normalize_phase(envelope.phase.as_deref());
    validate_ingest(
        &state,
        envelope.schema_version,
        SUPPORTED_DOOR_SCHEMA,
        &envelope.project.id,
        &envelope.model.id,
        &envelope.snapshot.taken_at,
    )?;
    // Decided from the envelope alone, before a single door line is read — a
    // push that will be refused should cost the producer one line.
    let key = ModelKey { project_id: envelope.project.id.clone(), model_id: envelope.model.id.clone() };
    check_doors_ingest(&state, &key, envelope.phase.as_deref())?;
    warn_on_transform_drift(envelope.model_to_shared.as_ref(), &envelope.project.id, &envelope.model.id);

    let mut doors: Vec<Door> = Vec::new();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("read error: {e}")))?
    {
        if line.trim().is_empty() {
            continue; // tolerate a trailing blank line
        }
        let door: Door =
            serde_json::from_str(&line).map_err(|e| (StatusCode::BAD_REQUEST, format!("bad door line: {e}")))?;
        doors.push(door);
    }

    let count = doors.len();
    tracing::info!("streamed {} door(s)", count);

    let snapshot_taken_at = envelope.snapshot.taken_at.clone();
    let payload = DoorPayload {
        schema_version: envelope.schema_version,
        project: envelope.project,
        model: envelope.model,
        snapshot: envelope.snapshot,
        phase: envelope.phase,
        model_to_shared: envelope.model_to_shared,
        doors,
    };

    state.set_door_snapshot(payload).map_err(|e| {
        tracing::error!("failed to store doors snapshot: {e:#}");
        (StatusCode::INTERNAL_SERVER_ERROR, format!("could not store doors snapshot: {e}"))
    })?;

    Ok((
        StatusCode::OK,
        Json(DoorIngestResponse { accepted: true, door_count: count, snapshot_taken_at, snapshot_id_generated }),
    ))
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
pub async fn get_rooms(
    State(state): State<Shared>,
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
    let result = rooms::assemble_rooms(&state, &scope).map_err(map_service_error)?;

    match result {
        None => Ok(StatusCode::NO_CONTENT.into_response()),
        Some(result) => Ok(Json(result).into_response()),
    }
}

/// Data-quality report for the header's validation panel — see
/// `service::validation::compute_project_validation`.
#[derive(Debug, Deserialize)]
pub struct DoorsQuery {
    #[serde(default)]
    pub project: Option<String>,
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
        milestone: query.milestone.as_deref(),
        filter: filter.as_ref(),
    };
    let result = doors::assemble_doors(&state, &scope).map_err(map_service_error)?;

    match result {
        None => Ok(StatusCode::NO_CONTENT.into_response()),
        Some(result) => Ok(Json(result).into_response()),
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
                ProjectReferenceSource { data: Some(make_drofus()), fields: vec![] },
            )]),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec!["$name".to_string(), "$id".to_string()],
            milestones: vec![],
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
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

        let response = get_rooms(State(state), Query(unscoped_query())).await.expect("204 is not an error");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
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
        let (status, message) = get_rooms(State(state), Query(query)).await.expect_err("no operator in the predicate");
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
        let response = get_rooms(State(state), Query(query)).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(body["rooms"].as_array().unwrap().is_empty());
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
            let payload = RoomPayload {
                schema_version: SUPPORTED_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: model_id.to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: taken_at.to_string() },
                phase: Some("New Construction".to_string()),
                model_to_shared: None,
                room_boundary: None,
                levels: vec![],
                rooms: vec![],
            };
            let state: Shared =
                std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

            let result = ingest_rooms(State(state), Json(payload)).await;
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
        // back untouched.
        let payload = RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: good_ts.to_string() },
            phase: Some("New Construction".to_string()),
            model_to_shared: None,
            room_boundary: None,
            levels: vec![],
            rooms: vec![],
        };
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        let (status, Json(body)) = ingest_rooms(State(state), Json(payload)).await.unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.snapshot_taken_at, good_ts);
        assert!(!body.snapshot_id_generated);
    }

    /// One payload for the phase tests, phase supplied per case.
    fn phase_payload(model: &str, ts: &str, phase: Option<&str>) -> RoomPayload {
        RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: model.to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: ts.to_string() },
            phase: phase.map(str::to_string),
            model_to_shared: None,
            room_boundary: None,
            levels: vec![],
            rooms: vec![make_room("r1", "Room A")],
        }
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
        assert!(body.quarantined.is_none());
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
        let reason = body.quarantined.expect("a quarantined push says why");
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
            r#"{"schema_version":6,"project":{"id":"p1","name":"P"},"model":{"id":"m1","name":"M","source":"revit"},"#,
            r#""snapshot":{"taken_at":"2026-01-01T00:00:00Z"},"levels":[]}"#,
            "\n",
            r#"{"id":"r1","name":"Room A","level_id":"1","loops":[]}"#,
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
            room_boundary: RoomBoundary::FinishFace,
            quarantined: None,
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
    }

    /// A blank (or omitted -- serde defaults it to blank) snapshot id is no
    /// longer an error: the server mints one and the response carries it, so
    /// the pusher can attach follow-up uploads to the same snapshot.
    #[tokio::test]
    async fn test_ingest_rooms_generates_snapshot_id_when_blank() {
        let payload = RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "".to_string() },
            phase: Some("New Construction".to_string()),
            model_to_shared: None,
            room_boundary: None,
            levels: vec![],
            rooms: vec![make_room("r1", "Room A")],
        };
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let (_, Json(body)) = ingest_rooms(State(state.clone()), Json(payload)).await.unwrap();

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
        let payload = RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "unregistered".to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
            phase: Some("New Construction".to_string()),
            model_to_shared: None,
            room_boundary: None,
            levels: vec![],
            rooms: vec![make_room("r1", "Room A")],
        };
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let result = ingest_rooms(State(state), Json(payload)).await;
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
        let payload = RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
            phase: Some("New Construction".to_string()),
            model_to_shared: Some(ModelToShared { matrix }),
            room_boundary: None,
            levels: vec![],
            rooms: vec![make_room("r1", "Room A")],
        };
        let state = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let _ = ingest_rooms(State(state.clone() as Shared), Json(payload)).await.expect("accepted");

        let stored = state.all_snapshots().unwrap();
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
        let payload = RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
            phase: Some("New Construction".to_string()),
            model_to_shared: Some(ModelToShared { matrix: [2.0, 0.0, 0.0, 2.0, 0.0, 0.0] }),
            room_boundary: None,
            levels: vec![],
            rooms: vec![make_room("r1", "Room A")],
        };
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let (_, Json(body)) = ingest_rooms(State(state), Json(payload)).await.expect("accepted despite det drift");
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

        let buffered = RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: "buffered".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
            phase: Some("New Construction".to_string()),
            model_to_shared: None,
            room_boundary: Some(RoomBoundary::FinishFace),
            levels: vec![],
            rooms: vec![make_room("r1", "Room A")],
        };
        let _ = ingest_rooms(State(state.clone() as Shared), Json(buffered)).await.expect("accepted");

        let body = concat!(
            r#"{"schema_version":6,"project":{"id":"p1","name":"P"},"model":{"id":"streamed","name":"M","source":"revit"},"#,
            r#""snapshot":{"taken_at":"2026-01-01T00:00:00Z"},"phase":"New Construction","room_boundary":"centreline","levels":[]}"#,
            "\n",
            r#"{"id":"r1","name":"Room A","level_id":"1","loops":[]}"#,
            "\n",
        );
        let _ = ingest_rooms_stream(State(state.clone() as Shared), Body::from(body))
            .await
            .expect("accepted");

        let stored = state.all_snapshots().unwrap();
        let of = |model: &str| stored.iter().find(|(k, _)| k.model_id == model).expect("stored").1.room_boundary;
        assert_eq!(of("buffered"), Some(RoomBoundary::FinishFace));
        assert_eq!(of("streamed"), Some(RoomBoundary::Centreline), "the stream path carries it too");
    }

    // ---------- doors ingest ----------

    fn make_door(id: &str, from_room: Option<&str>, to_room: Option<&str>) -> Door {
        Door {
            id: id.to_string(),
            level_id: "1".to_string(),
            loops: vec![],
            from_room: from_room.map(str::to_string),
            to_room: to_room.map(str::to_string),
            type_id: "t1".to_string(),
            type_name: "Single".to_string(),
            properties: BTreeMap::new(),
            type_properties: BTreeMap::new(),
        }
    }

    fn door_payload(project: &str, model: &str, ts: &str, phase: Option<&str>) -> DoorPayload {
        DoorPayload {
            schema_version: SUPPORTED_DOOR_SCHEMA,
            project: Project { id: project.to_string(), name: "P".to_string() },
            model: Model { id: model.to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: ts.to_string() },
            phase: phase.map(str::to_string),
            model_to_shared: None,
            doors: vec![make_door("d1", Some("r1"), None)],
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
        let stored = state.all_door_snapshots().unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].1.doors[0].from_room.as_deref(), Some("r1"));
        // The rooms lineage is untouched: same snapshot, same single id.
        assert_eq!(state.list_snapshot_ids(&key).unwrap(), vec!["2026-01-01T00:00:00Z".to_string()]);
        assert_eq!(state.list_door_snapshot_ids(&key).unwrap(), vec!["2026-02-01T00:00:00Z".to_string()]);
    }

    /// **The rooms-first gate.** Without rooms in this model there is nothing a
    /// door's `from_room`/`to_room` could resolve against, so the push is
    /// refused rather than stored to dangle.
    #[tokio::test]
    async fn test_doors_push_to_a_model_without_rooms_is_refused() {
        let state = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        let payload = door_payload("p1", "m1", "2026-02-01T00:00:00Z", Some("New Construction"));

        let (status, message) = ingest_doors(State(state.clone()), Json(payload)).await.unwrap_err();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(message.contains("no rooms have been pushed"), "{message}");
        assert!(state.all_door_snapshots().unwrap().is_empty(), "nothing was stored");
    }

    /// The gate is scoped to the `(project, model)` lineage, not the project.
    /// Rooms under a *sibling* model are a different room-id space, so they
    /// cannot satisfy it — a project-wide gate would accept this push and leave
    /// every reference unresolvable.
    #[tokio::test]
    async fn test_rooms_under_a_sibling_model_do_not_satisfy_the_gate() {
        let state = state_with_rooms(Some("New Construction")).await; // rooms on m1
        let payload = door_payload("p1", "m2", "2026-02-01T00:00:00Z", Some("New Construction"));

        let (status, message) = ingest_doors(State(state.clone()), Json(payload)).await.unwrap_err();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(message.contains("p1/m2"), "names the model that lacks rooms: {message}");
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
        assert!(state.all_door_snapshots().unwrap().is_empty());
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
        let stored = state.all_snapshots().unwrap();
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
            r#"{"schema_version":1,"project":{"id":"p1","name":"P"},"model":{"id":"m1","name":"M","source":"revit"},"#,
            r#""snapshot":{"taken_at":"2026-02-01T00:00:00Z"},"phase":"New Construction"}"#,
            "\n",
            r#"{"id":"d1","level_id":"1","loops":[],"from_room":"r1","type_id":"t1","type_name":"Single"}"#,
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
    /// line, not the whole body.
    #[tokio::test]
    async fn test_doors_stream_refuses_from_the_envelope_alone() {
        let state = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        let body = concat!(
            r#"{"schema_version":1,"project":{"id":"p1","name":"P"},"model":{"id":"m1","name":"M","source":"revit"},"#,
            r#""snapshot":{"taken_at":"2026-02-01T00:00:00Z"},"phase":"New Construction"}"#,
            "\n",
            r#"{"this is not a door and is never parsed"#, // malformed on purpose
        );

        let (status, message) = ingest_doors_stream(State(state.clone()), Body::from(body)).await.unwrap_err();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "the gate ran, not the line parser");
        assert!(message.contains("no rooms have been pushed"), "{message}");
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
