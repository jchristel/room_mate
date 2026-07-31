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

use crate::contract::{ModelToShared, Room, RoomBoundary, RoomPayload, StreamEnvelope, SUPPORTED_SCHEMA};
use crate::service::adjacency;
use crate::service::areas;
use crate::service::comparison::{self, ComparisonResponse};
use crate::service::milestones::MilestonesResponse;
use crate::service::projects::{BuildingsResponse, ProjectSummary};
use crate::service::reference::{ReferenceSnapshotInfo, ReferenceSnapshotList};
use crate::service::snapshots::{LatestSnapshot, ProjectSnapshotsResponse};
use crate::service::validation::ValidationResponse;
use crate::service::{milestones, projects, reference, rooms, snapshots, validation, ServiceError};
use crate::state::Shared;

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
/// route can run it from the envelope line alone, before reading any rooms.
fn validate_ingest(
    state: &Shared,
    schema_version: u32,
    project_id: &str,
    model_id: &str,
    taken_at: &str,
) -> Result<(), (StatusCode, String)> {
    if schema_version != SUPPORTED_SCHEMA {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("schema_version {schema_version} not supported; this server speaks {SUPPORTED_SCHEMA}"),
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
) -> Result<Json<IngestResponse>, (StatusCode, String)> {
    let snapshot_id_generated = crate::contract::ensure_taken_at(&mut payload.snapshot);
    validate_ingest(
        &state,
        payload.schema_version,
        &payload.project.id,
        &payload.model.id,
        &payload.snapshot.taken_at,
    )?;
    warn_on_transform_drift(payload.model_to_shared.as_ref(), &payload.project.id, &payload.model.id);

    let count = payload.rooms.len();
    let snapshot_taken_at = payload.snapshot.taken_at.clone();
    let room_boundary = resolved_boundary(&state, &payload.project.id, payload.room_boundary);
    tracing::info!("received {} room(s)", count);

    // Persist. A storage failure (unwritable disk, etc.) is a real server error,
    // not a bad request — surface it as 500 rather than swallowing it.
    state.set_snapshot(payload).map_err(|e| {
        tracing::error!("failed to store snapshot: {e:#}");
        (StatusCode::INTERNAL_SERVER_ERROR, format!("could not store snapshot: {e}"))
    })?;

    Ok(Json(IngestResponse {
        accepted: true,
        room_count: count,
        snapshot_taken_at,
        snapshot_id_generated,
        room_boundary,
    }))
}

#[derive(Serialize)]
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
) -> Result<Json<IngestResponse>, (StatusCode, String)> {
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
    validate_ingest(
        &state,
        envelope.schema_version,
        &envelope.project.id,
        &envelope.model.id,
        &envelope.snapshot.taken_at,
    )?;
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
        model_to_shared: envelope.model_to_shared,
        room_boundary: envelope.room_boundary,
        levels: envelope.levels,
        rooms,
    };

    state.set_snapshot(payload).map_err(|e| {
        tracing::error!("failed to store snapshot: {e:#}");
        (StatusCode::INTERNAL_SERVER_ERROR, format!("could not store snapshot: {e}"))
    })?;

    Ok(Json(IngestResponse {
        accepted: true,
        room_count: count,
        snapshot_taken_at,
        snapshot_id_generated,
        room_boundary,
    }))
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
            schema_version: 5,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
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
            model_to_shared: None,
            room_boundary: None,
            levels: vec![],
            rooms: vec![],
        };
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));
        let response = ingest_rooms(State(state), Json(payload)).await.unwrap();
        assert_eq!(response.0.snapshot_taken_at, good_ts);
        assert!(!response.0.snapshot_id_generated);
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
        })
        .unwrap();

        assert!(json.contains(r#""snapshot_id_generated":false"#), "unexpected wire shape: {json}");
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
            model_to_shared: None,
            room_boundary: None,
            levels: vec![],
            rooms: vec![make_room("r1", "Room A")],
        };
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let response = ingest_rooms(State(state.clone()), Json(payload)).await.unwrap();

        assert!(response.0.snapshot_id_generated);
        assert!(crate::contract::validate_snapshot_id(&response.0.snapshot_taken_at).is_ok());
        // The store keyed the push under exactly the id the response reports.
        let key = crate::state::ModelKey { project_id: "p1".into(), model_id: "m1".into() };
        assert_eq!(state.list_snapshot_ids(&key).unwrap(), vec![response.0.snapshot_taken_at.clone()]);
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
            model_to_shared: Some(ModelToShared { matrix }),
            room_boundary: None,
            levels: vec![],
            rooms: vec![make_room("r1", "Room A")],
        };
        let state = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let _ = ingest_rooms(State(state.clone() as Shared), Json(payload)).await.expect("accepted");

        let stored = state.all_snapshots().unwrap();
        let (_, payload) = stored.iter().find(|(k, _)| k.model_id == "m1").expect("stored");
        assert_eq!(payload.model_to_shared.expect("carried through").matrix, matrix);
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
            model_to_shared: Some(ModelToShared { matrix: [2.0, 0.0, 0.0, 2.0, 0.0, 0.0] }),
            room_boundary: None,
            levels: vec![],
            rooms: vec![make_room("r1", "Room A")],
        };
        let state: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), single_project("p1"), None));

        let response = ingest_rooms(State(state), Json(payload)).await.expect("accepted despite det drift");
        assert!(response.0.accepted);
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
            model_to_shared: None,
            room_boundary: Some(RoomBoundary::FinishFace),
            levels: vec![],
            rooms: vec![make_room("r1", "Room A")],
        };
        let _ = ingest_rooms(State(state.clone() as Shared), Json(buffered)).await.expect("accepted");

        let body = concat!(
            r#"{"schema_version":5,"project":{"id":"p1","name":"P"},"model":{"id":"streamed","name":"M","source":"revit"},"#,
            r#""snapshot":{"taken_at":"2026-01-01T00:00:00Z"},"room_boundary":"centreline","levels":[]}"#,
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
}
