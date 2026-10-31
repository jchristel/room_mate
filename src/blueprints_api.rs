//! HTTP adapters for blueprints -- the thin half of `crate::blueprints`, as
//! `connections_api` is of `connections`.
//!
//! Reads: the list (with the count of sheets bound to each), one blueprint, and a **capture** of a
//! room, which is a read -- it stores nothing and answers what a blueprint made from that room
//! would hold. The Revit push captures the room here and hands the sheets and tags on as a bundle
//! (`blueprint_bundle`): applied at once for a new blueprint, kept with a pending revision otherwise.
//! Writes (create, add a revision, rename, move the current marker or mark not in use) are
//! HTTP-only, for the reason settings and connections writes are: the MCP server is a
//! separate process and reads the same files.

use axum::{
    extract::{Path as UrlPath, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::blueprint_bundle::{self, Applied, Bundle};
use crate::blueprints::{
    self, AddRevisionRequest, Blueprint, BlueprintError, BlueprintSummary, CreateRequest, DecideRequest, PushRequest,
    SourceRoom, UpdateRequest,
};
use crate::service::blueprint_capture::{capture_room, qualify_room};
use crate::service::blueprint_model::{BlueprintSheet, RevisionContent, Skipped};
use crate::service::blueprint_resolve::{resolve_in_room_with, EdgeMap, Resolution};
use crate::sheets::SheetRoom;
use crate::state::Shared;

pub fn to_http(err: BlueprintError) -> (StatusCode, String) {
    match err {
        BlueprintError::NotFileBacked => (
            StatusCode::SERVICE_UNAVAILABLE,
            "this server was not started from a settings directory".to_string(),
        ),
        BlueprintError::Invalid(msg) => (StatusCode::BAD_REQUEST, msg),
        BlueprintError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
        BlueprintError::Conflict(msg) => (StatusCode::CONFLICT, msg),
        BlueprintError::Internal(e) => {
            tracing::error!("blueprints error: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
        }
    }
}

fn dir(state: &Shared) -> Result<std::path::PathBuf, (StatusCode, String)> {
    state.projects_dir().cloned().ok_or(BlueprintError::NotFileBacked).map_err(to_http)
}

/// The project's blueprints with the number of sheets bound to each: the store holds no sheets
/// (plan decision 27), so the count is read from the library. Shared by the HTTP list and MCP.
pub fn list_with_sheet_counts(
    projects_dir: &std::path::Path,
    project: &str,
) -> Result<Vec<BlueprintSummary>, BlueprintError> {
    let mut rows = blueprints::list(projects_dir, project)?;
    let sheets = crate::sheets::list(projects_dir, project)
        .map_err(|e| BlueprintError::Internal(anyhow::anyhow!("the sheet library cannot be read: {e:?}")))?;
    for row in &mut rows {
        row.sheets = sheets.iter().filter(|s| s.blueprint.as_deref() == Some(row.id.as_str())).count();
    }
    Ok(rows)
}

pub async fn http_list_blueprints(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
) -> Result<Json<Vec<BlueprintSummary>>, (StatusCode, String)> {
    list_with_sheet_counts(&dir(&state)?, &project_id).map(Json).map_err(to_http)
}

pub async fn http_get_blueprint(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
) -> Result<Json<Blueprint>, (StatusCode, String)> {
    blueprints::load(&dir(&state)?, &project_id, &id).map(Json).map_err(to_http)
}

pub async fn http_create_blueprint(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(request): Json<CreateRequest>,
) -> Result<(StatusCode, Json<Blueprint>), (StatusCode, String)> {
    let bundle = parse_bundle(request.bundle.clone())?;
    let created = blueprints::create(&dir(&state)?, &project_id, request).map_err(to_http)?;
    if let Some(bundle) = bundle {
        // An imported blueprint: its sheets and rules go with it, bound to the new blueprint.
        let room = source_sheet_room(&created);
        blueprint_bundle::apply(&state, &project_id, &created.id, room.as_ref(), &bundle)?;
    }
    Ok((StatusCode::CREATED, Json(created)))
}

fn parse_bundle(value: Option<serde_json::Value>) -> Result<Option<Bundle>, (StatusCode, String)> {
    value
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("the bundle cannot be read: {e}")))
}

/// The room a blueprint was captured from, as a sheet's room link. An imported blueprint may have
/// none, and its sheets are then bound without a room link.
fn source_sheet_room(b: &Blueprint) -> Option<SheetRoom> {
    b.source
        .as_ref()
        .map(|s| SheetRoom { model_id: s.model_id.clone(), room_id: s.room_id.clone() })
}

pub async fn http_add_revision(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
    Json(request): Json<AddRevisionRequest>,
) -> Result<Json<Blueprint>, (StatusCode, String)> {
    blueprints::add_revision(&dir(&state)?, &project_id, &id, request)
        .map(Json)
        .map_err(to_http)
}

/// A revision arriving from elsewhere: stored as pending, never current. See `blueprints`.
pub async fn http_push_revision(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
    Json(request): Json<PushRequest>,
) -> Result<(StatusCode, Json<Blueprint>), (StatusCode, String)> {
    blueprints::push(&dir(&state)?, &project_id, &id, request)
        .map(|b| (StatusCode::ACCEPTED, Json(b)))
        .map_err(to_http)
}

pub async fn http_accept_pending(
    State(state): State<Shared>,
    UrlPath((project_id, id, pending_id)): UrlPath<(String, String, u32)>,
    Json(request): Json<DecideRequest>,
) -> Result<Json<Blueprint>, (StatusCode, String)> {
    let projects = dir(&state)?;
    // The sheets and tags that came with the revision wait with it; read them before it becomes a revision.
    let waiting = blueprints::load(&projects, &project_id, &id)
        .map_err(to_http)?
        .pending
        .iter()
        .find(|p| p.pending_id == pending_id)
        .and_then(|p| p.bundle.clone());
    let accepted = blueprints::accept(&projects, &project_id, &id, pending_id, request).map_err(to_http)?;
    if let Some(bundle) = parse_bundle(waiting)? {
        let room = source_sheet_room(&accepted);
        blueprint_bundle::apply(&state, &project_id, &accepted.id, room.as_ref(), &bundle).map_err(
            |(status, message)| {
                (
                    status,
                    format!("the revision was accepted, but its sheets and tags could not be applied: {message}"),
                )
            },
        )?;
    }
    Ok(Json(accepted))
}

pub async fn http_reject_pending(
    State(state): State<Shared>,
    UrlPath((project_id, id, pending_id)): UrlPath<(String, String, u32)>,
    Json(request): Json<DecideRequest>,
) -> Result<Json<Blueprint>, (StatusCode, String)> {
    blueprints::reject(&dir(&state)?, &project_id, &id, pending_id, request)
        .map(Json)
        .map_err(to_http)
}

pub async fn http_update_blueprint(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
    Json(request): Json<UpdateRequest>,
) -> Result<Json<Blueprint>, (StatusCode, String)> {
    blueprints::update(&dir(&state)?, &project_id, &id, request).map(Json).map_err(to_http)
}

#[derive(Deserialize)]
pub struct CaptureParams {
    /// The room's model. Left out, the room id is looked up among the project's rooms and is
    /// refused if more than one model has it.
    #[serde(default)]
    pub model_id: Option<String>,
    pub room_id: String,
}

/// What capturing a room would produce, and nothing stored.
#[derive(Serialize)]
pub struct CaptureResponse {
    pub room_name: String,
    /// The room's model, as the server resolved it, so a caller that named only the room id
    /// can record where the blueprint came from.
    pub model_id: String,
    #[serde(flatten)]
    pub content: RevisionContent,
    /// What capture left out, as numbers.
    pub skipped: Skipped,
}

pub async fn http_capture_blueprint(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Query(params): Query<CaptureParams>,
) -> Result<Json<CaptureResponse>, (StatusCode, String)> {
    let room = match qualify_room(&state, &project_id, &params.room_id, params.model_id.as_deref()) {
        Ok(Some(room)) => room,
        Ok(None) => return Err((StatusCode::NOT_FOUND, "no such room in this project".to_string())),
        Err(e) => return Err(service_error(e)),
    };
    // Reads the project's rooms and FF&E, which can take seconds on a large one; the other
    // service reads run inline in their handlers too.
    let result = capture_room(&state, &project_id, &room);
    match result {
        Ok(Some((captured, room_name))) => Ok(Json(CaptureResponse {
            room_name,
            model_id: room.model_id.clone(),
            content: captured.content,
            skipped: captured.skipped,
        })),
        Ok(None) => Err((StatusCode::NOT_FOUND, "no such room in this project".to_string())),
        Err(crate::service::ServiceError::Invalid(msg)) => Err((StatusCode::BAD_REQUEST, msg)),
        Err(crate::service::ServiceError::Internal(e)) => {
            tracing::error!("capture error: {e:#}");
            Err((StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string()))
        }
    }
}

#[derive(Deserialize)]
pub struct ResolveParams {
    #[serde(default)]
    pub model_id: Option<String>,
    pub room_id: String,
    /// The revision to place; the blueprint's current one when left out.
    #[serde(default)]
    pub revision: Option<u32>,
    /// Pin the edge mapping (blueprint edge `i` to room edge `(i + shift) % edges`); left out,
    /// the best fit by shape is used.
    #[serde(default)]
    pub shift: Option<usize>,
    /// Map the edges by hand: the room edge for each blueprint edge, comma separated
    /// (`2,3,0,1`). Wins over `shift`, and is how a room with a different edge count is placed.
    #[serde(default)]
    pub edge_map: Option<String>,
    /// A room's item overrides to preview, as a JSON array (`[{"item_id":"7","dx_mm":20}]`): what
    /// the assign view sends so the drawing is the server's answer and not a second copy of the
    /// rule. Checked as an assignment's are.
    #[serde(default)]
    pub overrides: Option<String>,
    /// A box laid over the room by hand, as JSON (`{"dx_mm":0,"dy_mm":0,"rotate_deg":0,"sx":1,"sy":1}`,
    /// see `assignments::Frame`). Wins over `edge_map` and `shift`.
    #[serde(default)]
    pub frame: Option<String>,
}

/// The previewed overrides of a resolve, parsed and checked.
fn overrides_of(params: &ResolveParams) -> Result<Vec<crate::assignments::ItemOverride>, (StatusCode, String)> {
    const MAX_CHARS: usize = 200_000;
    let Some(text) = params.overrides.as_deref().filter(|t| !t.trim().is_empty()) else {
        return Ok(Vec::new());
    };
    if text.len() > MAX_CHARS {
        return Err((StatusCode::BAD_REQUEST, format!("overrides is at most {MAX_CHARS} characters")));
    }
    let parsed: Vec<crate::assignments::ItemOverride> = serde_json::from_str(text)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("overrides is not a JSON list of item overrides: {e}")))?;
    crate::assignments::clean_overrides(parsed).map_err(to_http)
}

fn edge_map_of(params: &ResolveParams) -> Result<EdgeMap, (StatusCode, String)> {
    if let Some(text) = params.frame.as_deref().filter(|t| !t.trim().is_empty()) {
        let frame: crate::assignments::Frame =
            serde_json::from_str(text).map_err(|e| (StatusCode::BAD_REQUEST, format!("frame is not a box ({e})")))?;
        let frame = frame.checked().map_err(|m| (StatusCode::BAD_REQUEST, m))?;
        return Ok(EdgeMap::Frame(frame));
    }
    if let Some(text) = params.edge_map.as_deref().filter(|t| !t.trim().is_empty()) {
        let parsed: Result<Vec<usize>, _> = text.split(',').map(|n| n.trim().parse::<usize>()).collect();
        return parsed.map(EdgeMap::Explicit).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                format!("edge_map {text:?} is not a comma separated list of edge numbers"),
            )
        });
    }
    Ok(params.shift.map_or(EdgeMap::Auto, EdgeMap::Shift))
}

#[derive(Serialize)]
pub struct ResolveResponse {
    pub blueprint_id: String,
    pub revision: u32,
    pub room_name: String,
    #[serde(flatten)]
    pub resolution: Resolution,
}

/// Where a blueprint revision's items go in one room. A read: nothing is stored or applied.
pub async fn http_resolve_blueprint(
    State(state): State<Shared>,
    UrlPath((project_id, id)): UrlPath<(String, String)>,
    Query(params): Query<ResolveParams>,
) -> Result<Json<ResolveResponse>, (StatusCode, String)> {
    let blueprint = blueprints::load(&dir(&state)?, &project_id, &id).map_err(to_http)?;
    let number = params.revision.unwrap_or(blueprint.current_revision);
    let Some(revision) = blueprint.revision(number) else {
        return Err((StatusCode::NOT_FOUND, format!("the blueprint has no revision {number}")));
    };
    let room = match qualify_room(&state, &project_id, &params.room_id, params.model_id.as_deref()) {
        Ok(Some(room)) => room,
        Ok(None) => return Err((StatusCode::NOT_FOUND, "no such room in this project".to_string())),
        Err(e) => return Err(service_error(e)),
    };
    let mapping = edge_map_of(&params)?;
    let overrides = overrides_of(&params)?;
    match resolve_in_room_with(&state, &project_id, &revision.content, &room, &mapping, &overrides) {
        Ok(Some((room_name, resolution))) => Ok(Json(ResolveResponse {
            blueprint_id: blueprint.id.clone(),
            revision: number,
            room_name,
            resolution,
        })),
        Ok(None) => Err((StatusCode::NOT_FOUND, "no such room in this project".to_string())),
        Err(crate::service::ServiceError::Invalid(msg)) => Err((StatusCode::BAD_REQUEST, msg)),
        Err(crate::service::ServiceError::Internal(e)) => {
            tracing::error!("resolve error: {e:#}");
            Err((StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string()))
        }
    }
}

pub fn service_error(err: crate::service::ServiceError) -> (StatusCode, String) {
    match err {
        crate::service::ServiceError::Invalid(msg) => (StatusCode::BAD_REQUEST, msg),
        crate::service::ServiceError::Internal(e) => {
            tracing::error!("blueprints error: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
        }
    }
}

/// What the Revit push sends: a room, the sheets chosen for it, and the name of the blueprint.
/// **The room data is not sent**: the server captures it from the rooms and FF&E already pushed
/// (one capture rule, in Rust), so the plugin only says which room and which sheets.
#[derive(Deserialize)]
pub struct RevitPushBody {
    pub name: String,
    pub room_id: String,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// Who pushed, for the timeline (for example the Revit document's title).
    #[serde(default)]
    pub origin: Option<String>,
    pub sheets: Vec<BlueprintSheet>,
    /// The tags read from the sheets (`tag_rules::Observation`), or absent when the push did not read
    /// them; with the parameter their type codes were read from.
    #[serde(default)]
    pub observations: Option<serde_json::Value>,
    #[serde(default)]
    pub type_code_parameter: String,
}

#[derive(Serialize)]
pub struct RevitPushOutcome {
    pub blueprint_id: String,
    pub name: String,
    /// `created` (a new blueprint, revision 1 current) or `pending` (a revision of the blueprint
    /// of that name, waiting for a person to accept it).
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_id: Option<u32>,
    pub room_name: String,
    pub skipped: Skipped,
    /// How many of the push's sheets the sheet library took, bound to the blueprint (created,
    /// updated or held for a decision). 0 for a pending revision: its sheets and tags wait for it.
    pub library_sheets: usize,
    /// Sheets that were bound to the blueprint and are not in this push: generic now.
    pub unlinked_sheets: usize,
    /// What the tags became, for a new blueprint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<crate::tag_rules::ObservationOutcome>,
    /// Tag rules that were bound and are no longer written: merged into generic.
    pub unlinked_tag_rules: usize,
    /// The sheets and tags are held with the pending revision until it is accepted.
    pub waiting: bool,
}

/// A blueprint from Revit: the room as the server holds it, plus the sheets the user chose.
/// **The only way a blueprint is made** (plan decision 18), so sheets are required; they go to the
/// sheet library bound to the blueprint, with its tags (`blueprint_bundle`, decision 27). A name the
/// project already has becomes a pending revision of that blueprint, never an overwrite (decision
/// 10), and its sheets and tags wait with it until it is accepted; a new name creates the blueprint
/// and applies them at once.
pub async fn http_push_from_revit(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(body): Json<RevitPushBody>,
) -> Result<(StatusCode, Json<RevitPushOutcome>), (StatusCode, String)> {
    if body.sheets.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "a blueprint push carries the room's layout sheets; pick at least one".to_string(),
        ));
    }
    let room = match qualify_room(&state, &project_id, &body.room_id, body.model_id.as_deref()) {
        Ok(Some(room)) => room,
        Ok(None) => return Err((StatusCode::NOT_FOUND, "no such room in this project".to_string())),
        Err(e) => return Err(service_error(e)),
    };
    let (captured, room_name) = match capture_room(&state, &project_id, &room) {
        Ok(Some(found)) => found,
        Ok(None) => return Err((StatusCode::NOT_FOUND, "no such room in this project".to_string())),
        Err(e) => return Err(service_error(e)),
    };
    let content = captured.content;
    let projects = dir(&state)?;
    let name = body.name.trim().to_string();
    let existing = blueprints::list(&projects, &project_id)
        .map_err(to_http)?
        .into_iter()
        .find(|b| b.name == name);
    let sheet_room = SheetRoom::from(&room);
    let source = SourceRoom { model_id: room.model_id, room_id: room.room_id, room_name: room_name.clone() };
    let bundle = Bundle {
        sheets: body.sheets,
        type_code_parameter: body.type_code_parameter,
        observations: body.observations,
        rules: None,
    };
    let mut applied = Applied::default();
    let (blueprint, outcome, pending_id) = match existing {
        Some(found) => {
            let pushed = blueprints::push(
                &projects,
                &project_id,
                &found.id,
                PushRequest {
                    origin: body.origin,
                    note: body.note,
                    bundle: Some(serde_json::to_value(&bundle).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?),
                    content,
                },
            )
            .map_err(to_http)?;
            let pending_id = pushed.last_pending_id;
            (pushed, "pending", Some(pending_id))
        }
        None => {
            let created = blueprints::create(
                &projects,
                &project_id,
                CreateRequest { name, source: Some(source), note: body.note, bundle: None, content },
            )
            .map_err(to_http)?;
            applied = blueprint_bundle::apply(&state, &project_id, &created.id, Some(&sheet_room), &bundle)?;
            (created, "created", None)
        }
    };
    let status = if outcome == "created" { StatusCode::CREATED } else { StatusCode::ACCEPTED };
    Ok((
        status,
        Json(RevitPushOutcome {
            blueprint_id: blueprint.id,
            name: blueprint.name,
            outcome,
            pending_id,
            room_name,
            skipped: captured.skipped,
            library_sheets: applied.sheets,
            unlinked_sheets: applied.unlinked_sheets,
            tags: applied.tags,
            unlinked_tag_rules: applied.unlinked_tag_rules,
            waiting: pending_id.is_some(),
        }),
    ))
}

/// What the box editor asks: this content, with its reference outline made these nodes, or the
/// rectangle round the room, or the room's own outline.
#[derive(Deserialize)]
pub struct ReshapeBody {
    pub content: RevisionContent,
    #[serde(default)]
    pub nodes: Option<Vec<crate::service::blueprint_model::Point>>,
    /// `rectangle` or `room`, when no nodes are given.
    #[serde(default)]
    pub to: Option<String>,
}

/// Write a blueprint's items and doors again against an edited reference outline, and answer with
/// the content. **Stores nothing**: the page keeps the answer as its draft and a save is the
/// ordinary new revision (`blueprint_reshape`).
pub async fn http_reshape_blueprint(
    UrlPath(_project_id): UrlPath<String>,
    Json(body): Json<ReshapeBody>,
) -> Result<Json<RevisionContent>, (StatusCode, String)> {
    use crate::service::blueprint_reshape::{reshape, Target};
    let target = match (body.nodes, body.to.as_deref()) {
        (Some(nodes), None) => Target::Nodes(nodes.into_iter().map(Into::into).collect()),
        (None, Some("rectangle")) => Target::Rectangle,
        (None, Some("room")) => Target::RoomOutline,
        _ => return Err((StatusCode::BAD_REQUEST, "give either `nodes` or `to` (`rectangle` or `room`)".to_string())),
    };
    reshape(&body.content, target).map(Json).map_err(service_error)
}
