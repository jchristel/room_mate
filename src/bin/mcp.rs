//! roommate's MCP server: exposes the read side as MCP tools over stdio, one
//! per existing HTTP read route -- `list_projects`, `list_buildings`,
//! `get_rooms`, `get_validation`, `get_hierarchy_areas`, `get_adjacency`,
//! `list_snapshots`, `get_latest_snapshot`, `get_pending_snapshot`,
//! `list_milestones`, `compare_milestones`, `list_reference_snapshots`,
//! `get_reference_snapshot`, `get_doors` --
//! plus two settings *reads* off `settings_api`'s transport-agnostic core
//! (`list_project_settings`, `get_project_settings`) and the one forwarded
//! mutation (`upload_reference`, below). Seventeen in total; keep this list and
//! STRATEGY-MCP.md's in step when adding one. Each tool is a thin adapter over
//! `roommate::service` -- parse params, call one service function, serialize
//! the result -- exactly like the Axum handlers in `roommate::handlers`, just a
//! second transport over the same domain layer.
//!
//! Ingest (`POST /rooms`) has no MCP equivalent here: an MCP client asking an
//! LLM to push a full room snapshot isn't a realistic flow, and the HTTP
//! server remains the ingest path.
//!
//! The one mutating tool, `upload_reference`, doesn't break that rule: it never
//! writes this process's state or the store — it reads a CSV file and
//! *forwards it over HTTP* to the running server (`--server-url`, default the
//! shared default address), which stays the single writer and hot-swaps
//! its own registry. The `reqwest` dependency this adds is an HTTP *client*;
//! the "no transport crate leaks into the other binary" rule is about server
//! frameworks (`mcp.rs` still never imports `axum`), and `main.rs` still
//! never imports `rmcp` or `reqwest`.
//!
//! Run as a client-spawned subprocess (e.g. from an MCP host's config) --
//! stdout is reserved for the JSON-RPC stream, so all logging goes to
//! stderr. This is a distinct OS process from any running HTTP server: it
//! only sees the same room data if pointed at the same `[storage]` root via
//! `--server-settings`, since `MemStore` state isn't shared across processes.

use std::path::PathBuf;

use clap::Parser;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServerHandler, ServiceExt,
};

use roommate::bootstrap::build_state;
use roommate::default_http_addr;
use roommate::service::{
    adjacency, areas, comparison, doors, milestones, projects, reference, rooms, snapshots, validation, ServiceError,
};
use roommate::settings_api::{self, SettingsError};
use roommate::state::Shared;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ProjectIdParams {
    /// The project id, as returned by `list_projects`.
    project_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ModelIdParams {
    /// The project id, as returned by `list_projects`.
    project_id: String,
    /// The model id, as returned by `list_snapshots`.
    model_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct GetRoomsParams {
    /// Scope the merge to one project id. Omit to merge every stored model.
    #[serde(default)]
    project: Option<String>,
    /// Opaque building key from `list_buildings`. Omit for no building filter.
    #[serde(default)]
    building: Option<String>,
    /// Milestone name from `list_milestones`: serve the snapshots that
    /// milestone pins instead of each model's latest. Omit for latest.
    #[serde(default)]
    milestone: Option<String>,
    /// Property predicates, ALL of which must hold (AND), one per element:
    /// ["Department=Cardiology", "Area>=20", "drofus.NetArea>20"]. Operators:
    /// = != > >= < <= and ~ (case-insensitive contains). An unqualified name
    /// is a canonical room property (as listed by get_project_settings'
    /// builtin_properties), plus $name / $id for the room's own fields; a
    /// source-prefixed name (e.g. "drofus.") reads that joined record's field label
    /// (as listed by get_reference_snapshot). A room missing the property -- or
    /// with no joined record for that source at all -- never matches, negative
    /// operators included. Quote a value containing spaces if in doubt.
    /// Omit for no filter.
    #[serde(default)]
    filter: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct GetDoorsParams {
    /// Scope the merge to one project id. Omit to merge every stored model.
    #[serde(default)]
    project: Option<String>,
    /// Milestone name from `list_milestones`: serve the doors snapshots that
    /// milestone pins instead of each model's latest. Omit for latest.
    #[serde(default)]
    milestone: Option<String>,
    /// Property predicates, ALL of which must hold (AND), one per element:
    /// ["$to_room=2621156", "Mark=29"]. Same operators as get_rooms' filter.
    /// An unqualified name reads the door's own properties, instance tier then
    /// family type tier; the intrinsics are $id, $type_id, $type_name,
    /// $level_id, $from_room and $to_room. A door missing the property never
    /// matches, negative operators included -- so an external door (null on one
    /// side) does not match "$to_room!=x" either. Omit for no filter.
    #[serde(default)]
    filter: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct AdjacencyParams {
    /// The project id, as returned by `list_projects`.
    project_id: String,
    /// Opaque building key from `list_buildings`. Omit for no building filter.
    #[serde(default)]
    building: Option<String>,
    /// Milestone name from `list_milestones`: build the graph from the
    /// snapshots that milestone pins instead of each model's latest. Omit for
    /// latest.
    #[serde(default)]
    milestone: Option<String>,
    /// Largest gap between two room boundaries, in decimal feet, still counted
    /// as a shared wall. **Omit it unless you have a reason not to:** the
    /// server then derives the tolerance from the project's declared
    /// `[areas] max_wall_thickness` and the boundary regime the models
    /// declared — zero when every level in scope is drawn to wall centrelines
    /// (neighbours touch exactly), the declared thickness otherwise. The
    /// response echoes the value actually applied. Pass one only to probe a
    /// different tolerance than the project declares; must be between 0 and 5.
    #[serde(default)]
    wall_max: Option<f64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct AreasParams {
    /// The project id, as returned by `list_projects`.
    project_id: String,
    /// Opaque building key from `list_buildings`. Omit for no building filter.
    #[serde(default)]
    building: Option<String>,
    /// Milestone name from `list_milestones`: measure the snapshots that
    /// milestone pins instead of each model's latest. Omit for latest.
    #[serde(default)]
    milestone: Option<String>,
}

/// Serialize any service response into a single text content block -- the
/// same `Serialize` types the HTTP handlers already return as JSON, just
/// wrapped for MCP instead of `axum::Json`.
fn json_result<T: serde::Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let json = serde_json::to_string(value)
        .map_err(|e| McpError::internal_error(format!("failed to serialize response: {e}"), None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
}

/// Minimal percent-encoding for URL path/query components. The values that
/// pass through here are constrained by construction — project ids are
/// path-safe (`is_path_safe_component`) and `taken_at` is RFC3339, whose only
/// URL-reserved character is the `+` of a numeric offset — so a tiny
/// encode-everything-non-unreserved loop beats a dependency.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Which reference source a reference tool call means.
///
/// The tools used to hardcode `"drofus"`, which quietly made every other
/// configured source unreachable over MCP. Making `source` *required* instead
/// would have been the honest fix and a bad trade: the overwhelmingly common
/// project configures exactly one source, and an agent should not have to make
/// a `get_project_settings` round trip to learn a name the server can infer.
///
/// So: an explicit `source` always wins. Omitted, it resolves to the project's
/// sole configured source. Ambiguity is refused rather than guessed — picking
/// "the first one" would silently answer about the wrong dataset, and the
/// error names the actual choices so the retry is obvious.
fn resolve_source(state: &Shared, project_id: &str, requested: Option<&str>) -> Result<String, McpError> {
    if let Some(name) = requested {
        return Ok(name.to_string());
    }
    let registry = state.settings();
    let names: Vec<&String> = registry
        .settings_for(project_id)
        .map(|bundle| bundle.reference.keys().collect())
        .unwrap_or_default();
    match names.as_slice() {
        [only] => Ok((*only).clone()),
        [] => Err(McpError::invalid_params(
            format!("project '{project_id}' configures no reference source"),
            None,
        )),
        many => Err(McpError::invalid_params(
            format!(
                "project '{project_id}' configures {} reference sources ({}); pass `source` to say which",
                many.len(),
                many.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            ),
            None,
        )),
    }
}

/// `ServiceError` -> `McpError`: an internal failure becomes `internal_error`,
/// a malformed request `invalid_params` -- the MCP counterpart of the HTTP
/// adapter's 500/400 split, mapped here in the adapter rather than in the
/// domain layer.
fn to_mcp_error(err: ServiceError) -> McpError {
    match err {
        ServiceError::Internal(e) => {
            tracing::error!("internal service error: {e:#}");
            McpError::internal_error(e.to_string(), None)
        }
        // Caller-addressable, and worth passing verbatim: the message names
        // the offending predicate, which is what lets the client fix it
        // instead of guessing why a result was empty.
        ServiceError::Invalid(msg) => McpError::invalid_params(msg, None),
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct CompareMilestonesParams {
    /// The project id, as returned by `list_projects`.
    project_id: String,
    /// The baseline milestone name (from `list_milestones`) every other is
    /// compared against.
    baseline: String,
    /// The milestone names to compare against the baseline. Any equal to the
    /// baseline is skipped.
    #[serde(default)]
    others: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ReferenceSourceParams {
    /// The project id, as returned by `list_projects`.
    project_id: String,
    /// Which reference source, e.g. "drofus". Omit when the project
    /// configures exactly one — see `resolve_source`.
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct GetReferenceSnapshotParams {
    /// The project id, as returned by `list_projects`.
    project_id: String,
    /// Which reference source, e.g. "drofus". Omit when the project
    /// configures exactly one — see `resolve_source`.
    #[serde(default)]
    source: Option<String>,
    /// A snapshot id from `list_reference_snapshots`. Omit for the latest.
    #[serde(default)]
    taken_at: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct UploadReferenceParams {
    /// The project id, as returned by `list_projects`. Its settings must
    /// declare `[sources.reference.<source>] type = "upload"`.
    project_id: String,
    /// Which reference source to upload for, e.g. "drofus". Omit when the
    /// project configures exactly one — see `resolve_source`.
    #[serde(default)]
    source: Option<String>,
    /// Absolute path to the CSV export to upload.
    path: String,
    /// Snapshot id (RFC3339 UTC date-time) to store the upload under. Omit
    /// to let the server mint one; the result reports the resolved id.
    #[serde(default)]
    taken_at: Option<String>,
}

// `tool_router` is read by the `#[tool_handler]`-generated dispatch code,
// but rustc's dead-code analysis doesn't see through that -- same false
// positive the rmcp SDK's own examples suppress this way.
#[allow(dead_code)]
#[derive(Clone)]
struct RoommateMcp {
    state: Shared,
    server_url: String,
    tool_router: ToolRouter<RoommateMcp>,
}

#[tool_router]
impl RoommateMcp {
    fn new(state: Shared, server_url: String) -> Self {
        Self { state, server_url, tool_router: Self::tool_router() }
    }

    /// Lists every project with at least one stored model -- see
    /// `service::projects::list_projects`.
    #[tool(description = "List every project with at least one stored model")]
    fn list_projects(&self) -> Result<CallToolResult, McpError> {
        let result = projects::list_projects(&self.state).map_err(to_mcp_error)?;
        json_result(&result)
    }

    /// Lists the distinct "Building" classification values for one project
    /// -- see `service::projects::list_buildings`.
    #[tool(description = "List the distinct Building classification values found in one project's rooms")]
    fn list_buildings(&self, Parameters(p): Parameters<ProjectIdParams>) -> Result<CallToolResult, McpError> {
        let result = projects::list_buildings(&self.state, &p.project_id).map_err(to_mcp_error)?;
        json_result(&result)
    }

    /// Merges every stored model's levels and rooms, optionally scoped by
    /// project and building -- see `service::rooms::assemble_rooms`. The
    /// service's `None` ("nothing has ever been pushed" -- the HTTP 204 case)
    /// has no MCP status-code equivalent, so it becomes a short plain-text
    /// answer instead of a JSON body; an LLM client reads either just fine.
    #[tool(
        description = "Fetch merged rooms and levels across stored models, optionally scoped by project id, building key, milestone name, and property filter. \
                          A project whose hierarchy has no 'Building' tier matches nothing under a building filter (check list_buildings' tier_configured before filtering); \
                          under a milestone filter, models are served from the snapshots that milestone pins instead of their latest. \
                          Prefer the 'filter' parameter over fetching every room and matching client-side -- it answers property questions ('which rooms are Department = Cardiology?') \
                          server-side, against the same canonical property names the rest of the settings use. Note that a room missing the filtered property never matches, \
                          negative operators included, so an empty result can mean 'no room has that property' rather than 'no room has that value'. \
                          IMPORTANT -- results are scoped to ONE Revit phase per model, not the whole model: each model's rooms were filtered at export to the phase named in \
                          'phase_by_model' (project id -> model id -> phase). This is a partial view of the building by design, so do not read it as a complete model. \
                          Different models of one project CAN be on different phases, in which case the merged result spans two phases -- compare the values in 'phase_by_model' \
                          before aggregating across models, and see get_validation's 'phases.disagree' for that as a reported finding. A null phase means a model pushed before \
                          phasing existed, whose rooms were never filtered to any phase at all."
    )]
    fn get_rooms(&self, Parameters(p): Parameters<GetRoomsParams>) -> Result<CallToolResult, McpError> {
        // Parsed here, in the adapter holding the raw strings, then passed
        // down as a domain type -- `service` never sees the filter syntax.
        // The known-source vocabulary comes from the live registry, not a
        // fixed list -- see `handlers::get_rooms`'s matching call.
        let known = self.state.settings().known_reference_sources();
        let filter =
            rooms::RoomFilter::parse(&p.filter, &known).map_err(|msg| to_mcp_error(ServiceError::Invalid(msg)))?;
        let scope = rooms::RoomScope {
            project: p.project.as_deref(),
            building: p.building.as_deref(),
            milestone: p.milestone.as_deref(),
            filter: Some(&filter).filter(|f| !f.is_empty()),
        };
        let result = rooms::assemble_rooms(&self.state, &scope).map_err(to_mcp_error)?;
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no snapshots have been pushed to this server yet",
            )])),
            Some(result) => json_result(&result),
        }
    }

    /// Merges every stored model's doors, optionally scoped -- see
    /// `service::doors::assemble_doors`. Same `None` -> plain-text handling as
    /// `get_rooms`, for the same reason.
    #[tool(
        description = "Fetch merged doors across stored models, optionally scoped by project id, milestone name, and property filter. Each door carries its own instance \
                          properties AND its family type's properties, its footprint, its level, and BOTH room references: 'from_room' and 'to_room'. \
                          A null on one side is an EXTERNAL door -- a normal state, not missing data. Use '$from_room=<room id>' or '$to_room=<room id>' in the filter to ask \
                          which doors touch a given room; the other intrinsics are $id, $type_id, $type_name and $level_id, and any other unqualified name reads the door's \
                          properties, INSTANCE tier first then the family TYPE tier (a blank instance value does not hide the type's). \
                          Room references are model-scoped: a room id is unique only within one model, so resolve 'from_room'/'to_room' against rooms of the SAME 'model_id' \
                          this door carries. get_validation's 'doors' section reports the ones that resolve to nothing. \
                          Doors carry no joined reference sources yet, so a source-prefixed filter (e.g. 'drofus.') matches no door rather than erroring. \
                          There is no building scope here, unlike get_rooms: a door's building would depend on which of its two rooms owns it, which is deliberately undecided. \
                          IMPORTANT -- like get_rooms, results are scoped to ONE Revit phase per model, named in 'phase_by_model'. Do not read them as a complete door schedule."
    )]
    fn get_doors(&self, Parameters(p): Parameters<GetDoorsParams>) -> Result<CallToolResult, McpError> {
        let known = self.state.settings().known_reference_sources();
        let filter =
            rooms::RoomFilter::parse(&p.filter, &known).map_err(|msg| to_mcp_error(ServiceError::Invalid(msg)))?;
        let scope = doors::DoorScope {
            project: p.project.as_deref(),
            milestone: p.milestone.as_deref(),
            filter: Some(&filter).filter(|f| !f.is_empty()),
        };
        let result = doors::assemble_doors(&self.state, &scope).map_err(to_mcp_error)?;
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no doors have been pushed to this server yet",
            )])),
            Some(result) => json_result(&result),
        }
    }

    /// Lists one project's milestones (named dated snapshot pins) -- see
    /// `service::milestones::list_milestones`.
    #[tool(
        description = "List one project's milestones: named dates with data snapshots pinned to them, newest first — each carries its model-pin count and `reference_snapshots`, a map of reference source name to the snapshot id that milestone pins for it (absent sources are joined at their current data). Pass a milestone's name to get_rooms to view the project as captured at that milestone, rooms AND every reference source."
    )]
    fn list_milestones(&self, Parameters(p): Parameters<ProjectIdParams>) -> Result<CallToolResult, McpError> {
        let result = milestones::list_milestones(&self.state, &p.project_id).map_err(to_mcp_error)?;
        json_result(&result)
    }

    /// Compares N milestones against a baseline for one project -- see
    /// `service::comparison::compare_milestones`. A project with no
    /// `comparison_key` configured returns `comparison_key_configured: false`.
    #[tool(
        description = "Compare milestones for one project: one baseline milestone versus each of the others (a star diff, not all-pairs). \
                          Reports rooms added and removed relative to the baseline, and per-property differences on rooms present in both, \
                          over the project's configured comparison property set. Rooms are matched by the project's user-defined comparison_key \
                          property (its own setting, NOT any reference source's link property); if none is configured the result is comparison_key_configured: false. \
                          A separate 'doors' section reports the same diff over the project's DOORS -- doors_added, doors_removed and changed_doors, each changed door carrying \
                          its model_id because a door id is unique only within one model. Doors are configured independently, by [doors] comparison_key and [doors] \
                          comparison_properties, so a project can compare rooms and not doors or the reverse: check 'doors.comparison_key_configured' separately from the \
                          top-level one. Comparing '$to_room'/'$from_room' is how a door that MOVED between rooms shows up; losing a room reference entirely is reported as a \
                          missing property rather than a difference against an empty value. Doors come from the snapshots a milestone pins in door_attachments, which are \
                          separate pins from the rooms ones."
    )]
    fn compare_milestones(
        &self,
        Parameters(p): Parameters<CompareMilestonesParams>,
    ) -> Result<CallToolResult, McpError> {
        let result =
            comparison::compare_milestones(&self.state, &p.project_id, &p.baseline, &p.others).map_err(to_mcp_error)?;
        json_result(&result)
    }

    /// Lists every stored snapshot id for one project, grouped per model --
    /// see `service::snapshots::list_project_snapshots`.
    #[tool(
        description = "List every stored snapshot id (RFC3339 UTC taken_at) for one project, grouped per model, each group carrying its latest"
    )]
    fn list_snapshots(&self, Parameters(p): Parameters<ProjectIdParams>) -> Result<CallToolResult, McpError> {
        let result = snapshots::list_project_snapshots(&self.state, &p.project_id).map_err(to_mcp_error)?;
        json_result(&result)
    }

    /// The latest snapshot id for one model -- see
    /// `service::snapshots::latest_snapshot`. The service's `None` (the HTTP
    /// 404 case) becomes a short plain-text answer, same convention as
    /// `get_rooms`' empty-store case.
    #[tool(description = "Get the latest snapshot id (taken_at) for one model of one project")]
    fn get_latest_snapshot(&self, Parameters(p): Parameters<ModelIdParams>) -> Result<CallToolResult, McpError> {
        let result = snapshots::latest_snapshot(&self.state, &p.project_id, &p.model_id).map_err(to_mcp_error)?;
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no snapshots stored for that project/model",
            )])),
            Some(latest) => json_result(&latest),
        }
    }

    /// The quarantined push waiting on one model -- see
    /// `service::snapshots::pending_snapshot`. Read-only: activating one is a
    /// mutation and stays HTTP-only, same line ingest draws.
    #[tool(
        description = "Get the push waiting to be activated for one model, if any. A model's Revit phase is fixed by its first phased push, so a push declaring a different phase is stored but NOT made live -- nothing reads it until someone activates it over HTTP. Returns its snapshot id, the phase it would move the model to ('phase'), the phase the model is on now ('current_phase'), and its room count."
    )]
    fn get_pending_snapshot(&self, Parameters(p): Parameters<ModelIdParams>) -> Result<CallToolResult, McpError> {
        let result = snapshots::pending_snapshot(&self.state, &p.project_id, &p.model_id).map_err(to_mcp_error)?;
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no pending push for that project/model",
            )])),
            Some(pending) => json_result(&pending),
        }
    }

    /// Runs the reference reconciliation QA report for one project -- see
    /// `service::validation::compute_project_validation`.
    #[tool(
        description = "Run the reference reconciliation validation report for one project. Returns one report per configured reference source under 'sources' (keyed by source name, each with its own link_property, discrepancy lists, field coverage and 'error_rooms' room_id -> number/name/link value for the flagged rooms), plus a cross-source 'discrepancies' summary for a one-shot count. \
                       Unmatched is reported in BOTH directions and they mean different things: 'rooms_unmatched' lists room ids whose link value finds no record, while 'reference_unmatched' lists the source's own link values that no room resolves to (bare values, not room ids -- there is no room, which is the finding). A value shared by several rooms counts as matched and is reported once under 'duplicate_link_values' instead. \
                       An empty 'sources' map means the project reconciles against nothing -- normal, not an error. \
                       Separately from any reference source, 'phases' reports which Revit phase each of the project's models was filtered to ('by_model') and whether they \
                       disagree ('disagree'). A true 'disagree' means /rooms is merging rooms from two different phases into one plan that will nonetheless look complete; \
                       an unphased model (null) counts as a distinct value there, since its rooms were never filtered at all. This is reported, never rejected. \
                       Also separate from any reference source, 'doors' reports whether the project's doors link to rooms that exist: 'doors_without_room_reference' lists doors \
                       naming no room on either side, and 'doors_unresolved_room' lists room references naming a room the door's own model does not have (one entry per dangling \
                       side, each carrying model_id, door_id, side and room_id). References resolve WITHIN one model, because room ids are unique only within a model. \
                       A door with a room on exactly one side is an EXTERNAL door -- normal, counted under 'doors_external', and deliberately not a discrepancy. \
                       Door findings have their own 'doors.discrepancies' and are NOT included in the top-level 'discrepancies', which counts reference sources only."
    )]
    fn get_validation(&self, Parameters(p): Parameters<ProjectIdParams>) -> Result<CallToolResult, McpError> {
        let result = validation::compute_project_validation(&self.state, &p.project_id).map_err(to_mcp_error)?;
        json_result(&result)
    }

    /// Hierarchy gross-area footprints for one project -- see
    /// `service::areas::assemble_areas`. Shares the exact read logic the HTTP
    /// `GET /projects/{id}/areas` uses; `None` (nothing pushed) mirrors
    /// `get_rooms`' empty-store message.
    #[tool(
        description = "Compute per-level, per-tier dissolved area footprints for one project, optionally scoped by building key and milestone name. Each group carries its resolved classification path, its measured footprint area (an aggregated ROOM FOOTPRINT — room area PLUS the enclosed wall bands between rooms, MINUS any genuine void like a courtyard or atrium; NOT net area and NOT a standards gross), whether it counts toward tiers above it (a settings exclusion can withhold a group), and its footprint polygons (each an exterior ring plus any interior rings for open voids). How much wall a footprint contains follows the boundary regime each model declared: a finish-face level fills the gaps between its rooms up to the project's declared max_wall_thickness, while a centreline level already has its walls inside the room polygons and is dissolved with no fill at all. The response echoes the gap applied per level (wall_gap_by_level) and the project's declared measurement_standard, which may be null — the figure is a house convention, not a standards gross, so read measurement_standard before quoting it. A void wider than a wall stays open and is excluded from the area."
    )]
    fn get_hierarchy_areas(&self, Parameters(p): Parameters<AreasParams>) -> Result<CallToolResult, McpError> {
        let result = areas::assemble_areas(&self.state, &p.project_id, p.building.as_deref(), p.milestone.as_deref())
            .map_err(to_mcp_error)?;
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no snapshots have been pushed to this server yet",
            )])),
            Some(result) => json_result(&result),
        }
    }

    /// Room-to-room adjacency graph for one project -- see
    /// `service::adjacency::assemble_adjacency`. Shares the exact read logic
    /// the HTTP `GET /projects/{id}/adjacency` uses, including the tolerance
    /// validation, so the two front doors cannot disagree on what a valid
    /// `wall_max` is.
    #[tool(
        description = "Compute the room-to-room adjacency graph for one project: which rooms share a wall, and how much wall they share. Optionally scoped by building key and milestone name. Returns nodes (one per room, with its level, centroid, classification path and any joined reference records, each flattened under its source name) and undirected edges (a room pair, their level, and the accumulated shared wall length in feet). Same level only — no cross-floor adjacency. Adjacency here means SHARED WALL geometry, NOT door connectivity: two rooms can share a wall with no door in it, and a door can connect two rooms sharing almost no wall. Doors ARE collected — use get_doors, where every door names its from_room and to_room — but they are a separate edge set over the same rooms, not a refinement of this graph. The `wall_max` parameter is the gap tolerance and matters: a Revit model whose room boundaries sit on wall centrelines has neighbours touching exactly (use 0), while one using finish faces separates them by the wall thickness (use roughly that). Too large a value bridges rooms that merely face each other across a corridor."
    )]
    fn get_adjacency(&self, Parameters(p): Parameters<AdjacencyParams>) -> Result<CallToolResult, McpError> {
        let result = adjacency::assemble_adjacency(
            &self.state,
            &p.project_id,
            p.building.as_deref(),
            p.milestone.as_deref(),
            p.wall_max,
        )
        .map_err(to_mcp_error)?;
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no snapshots have been pushed to this server yet",
            )])),
            Some(result) => json_result(&result),
        }
    }

    /// Lists every uploaded snapshot id for one project's reference source --
    /// see `service::reference::list_reference_snapshots`.
    #[tool(
        description = "List every uploaded CSV snapshot id (RFC3339 UTC taken_at) for one project's reference source, ascending, with the latest. \
                          `source` names the reference source (e.g. \"drofus\") and may be omitted when the project configures exactly one; \
                          when it configures several, the error names them. \
                          Reads the shared store fresh, so an upload forwarded moments ago shows here immediately."
    )]
    fn list_reference_snapshots(
        &self,
        Parameters(p): Parameters<ReferenceSourceParams>,
    ) -> Result<CallToolResult, McpError> {
        let source = resolve_source(&self.state, &p.project_id, p.source.as_deref())?;
        let result = reference::list_reference_snapshots(&self.state, &p.project_id, &source).map_err(to_mcp_error)?;
        json_result(&result)
    }

    /// A parsed summary of one uploaded reference CSV -- see
    /// `service::reference::get_reference_snapshot`. The service's `None` (the
    /// HTTP 404 case) becomes a short plain-text answer, same convention as
    /// `get_latest_snapshot`.
    #[tool(
        description = "Get a parsed summary (record count, link property, field labels) of one uploaded reference CSV -- the given taken_at, or the latest when omitted. \
                          `source` names the reference source (e.g. \"drofus\") and may be omitted when the project configures exactly one. \
                          Reads the shared store fresh."
    )]
    fn get_reference_snapshot(
        &self,
        Parameters(p): Parameters<GetReferenceSnapshotParams>,
    ) -> Result<CallToolResult, McpError> {
        let source = resolve_source(&self.state, &p.project_id, p.source.as_deref())?;
        let result = reference::get_reference_snapshot(&self.state, &p.project_id, &source, p.taken_at.as_deref())
            .map_err(to_mcp_error)?;
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "no such '{source}' upload stored for that project"
            ))])),
            Some(info) => json_result(&info),
        }
    }

    /// Uploads a reference CSV by FORWARDING it to the running HTTP server --
    /// this process never writes the store itself (see the module doc): the
    /// server validates, stores, and hot-swaps its own registry, staying the
    /// single writer.
    #[tool(description = "Upload a reference-source CSV export (given as an absolute file path) for one project. \
                          `source` names the reference source (e.g. \"drofus\") and may be omitted when the project configures exactly one. \
                          Forwards the file over HTTP to the running roommate server, which validates it against that source's \
                          declared fields before storing it as a dated snapshot and applying it live -- so the HTTP server must be running. \
                          The project's settings must declare [sources.reference.<source>] type = \"upload\". \
                          Note the staleness asymmetry: after an upload, this process's own get_rooms/get_validation still join the \
                          data loaded at ITS startup; list_reference_snapshots/get_reference_snapshot read the store fresh and see the new upload immediately.")]
    async fn upload_reference(
        &self,
        Parameters(p): Parameters<UploadReferenceParams>,
    ) -> Result<CallToolResult, McpError> {
        let source = resolve_source(&self.state, &p.project_id, p.source.as_deref())?;
        let bytes = std::fs::read(&p.path)
            .map_err(|e| McpError::invalid_params(format!("could not read CSV file {:?}: {e}", p.path), None))?;

        let mut url = format!(
            "{}/projects/{}/reference/{}",
            self.server_url.trim_end_matches('/'),
            urlencode(&p.project_id),
            urlencode(&source)
        );
        if let Some(taken_at) = &p.taken_at {
            url.push_str(&format!("?taken_at={}", urlencode(taken_at)));
        }

        let response = reqwest::Client::new()
            .post(&url)
            .header("Content-Type", "text/csv")
            .body(bytes)
            .send()
            .await
            .map_err(|e| {
                McpError::internal_error(
                    format!(
                        "the roommate HTTP server is not reachable at {} ({e}) -- \
                         start it (it is the single writer for uploads) and retry",
                        self.server_url
                    ),
                    None,
                )
            })?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(CallToolResult::success(vec![ContentBlock::text(body)]))
        } else {
            // The server's rejection text is the real validator -- pass it on
            // verbatim, marked caller-addressable.
            Err(McpError::invalid_params(format!("server answered {status}: {body}"), None))
        }
    }

    // Settings tools are READ-ONLY by design: this is a separate process from
    // the HTTP server, so a write from here could not hot-swap that server's
    // in-memory registry -- the file and the serving process would silently
    // disagree until a restart. Mutation stays behind the HTTP settings UI
    // (see `settings_api`'s module doc), matching the "Read-only access
    // against local state" contract `get_info` declares. `upload_reference`
    // above is not an exception: it forwards to that HTTP server rather than
    // writing anything from this process.

    /// Lists every project settings file with its headline facts -- see
    /// `settings_api::list_project_files`.
    #[tool(
        description = "List every project settings file (project id, is_default, and reference_sources -- the names of the reference sources it declares). \
                          Reads the files fresh, so a settings change saved through the HTTP UI shows here immediately; \
                          this process's own get_rooms/get_validation behavior still reflects the settings loaded at its startup."
    )]
    fn list_project_settings(&self) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let result = settings_api::list_project_files(&dir).map_err(settings_to_mcp_error)?;
        json_result(&result)
    }

    /// One project's parsed settings as JSON -- see
    /// `settings_api::get_project_file`.
    #[tool(
        description = "Get one project's settings (hierarchy, reference sources, builtin properties, room label, QA fields) as JSON. \
                          Reads the file fresh, so a settings change saved through the HTTP UI shows here immediately; \
                          this process's own get_rooms/get_validation behavior still reflects the settings loaded at its startup."
    )]
    fn get_project_settings(&self, Parameters(p): Parameters<ProjectIdParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let (file, settings) = settings_api::get_project_file(&dir, &p.project_id).map_err(settings_to_mcp_error)?;
        json_result(&serde_json::json!({ "file": file, "settings": settings }))
    }

    /// The `--project-settings` directory this process was started with --
    /// always present for this binary (the arg is required), so the error arm
    /// is defensive only.
    fn projects_dir(&self) -> Result<std::path::PathBuf, McpError> {
        self.state
            .projects_dir()
            .cloned()
            .ok_or_else(|| McpError::internal_error("no project settings directory configured", None))
    }
}

/// `SettingsError` -> `McpError`: caller-addressable problems (unknown id,
/// invalid input) become `invalid_params`; the rest `internal_error`.
fn settings_to_mcp_error(err: SettingsError) -> McpError {
    match err {
        SettingsError::NotFound(msg) | SettingsError::Invalid(msg) | SettingsError::Conflict(msg) => {
            McpError::invalid_params(msg, None)
        }
        SettingsError::NotFileBacked => McpError::internal_error("no project settings directory configured", None),
        SettingsError::Internal(e) => {
            tracing::error!("settings read error: {e:#}");
            McpError::internal_error(e.to_string(), None)
        }
    }
}

#[tool_handler]
impl ServerHandler for RoommateMcp {
    fn get_info(&self) -> ServerInfo {
        // Not `Implementation::from_build_env()` -- it's a plain fn whose body
        // bakes in `env!()` at *rmcp's own* compile time, so it always reports
        // "rmcp"/rmcp's version rather than ours (confirmed via a stdio smoke
        // test). Name and version explicitly instead.
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("roommate-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Read-only access to roommate's stored room and reference data -- this process never \
                 writes its own state or the store. The one mutating tool, upload_reference, forwards \
                 the CSV over HTTP to the running roommate server, which stays the single writer \
                 and hot-swaps its own registry. Requires the same [storage] root as the HTTP \
                 server (via --server-settings) to see real data -- this process does not share \
                 memory with it."
                    .to_string(),
            )
    }
}

#[derive(Parser)]
struct Args {
    /// Path to the server-wide TOML settings file (same file the HTTP server
    /// uses via `--server-settings`).
    #[arg(long)]
    server_settings: PathBuf,

    /// Path to the directory of per-project TOML settings files (same
    /// directory the HTTP server uses via `--project-settings`).
    #[arg(long)]
    project_settings: PathBuf,

    /// Base URL of the running roommate HTTP server, used only by the
    /// `upload_reference` tool (which forwards uploads to it). Defaults to the
    /// address the server binary binds by default.
    ///
    /// It tracks the server's *default* port, not its actual one: the server
    /// takes `--port` (and `$PORT`), and nothing here can observe that choice.
    /// Move the server and this flag has to move with it — deliberately loud
    /// rather than a second env lookup that would silently disagree with
    /// whatever the server actually did.
    #[arg(long, default_value_t = format!("http://{}", default_http_addr()))]
    server_url: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // stderr, never stdout -- stdout is the JSON-RPC transport. Filter on the
    // *current* crate name (this said "revit_viewer", the crate's old name,
    // which silently dropped every log event). RUST_LOG still wins when set.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("roommate=info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    let state = build_state(&args.server_settings, &args.project_settings)?;

    let service = RoommateMcp::new(state, args.server_url).serve(stdio()).await.inspect_err(|e| {
        tracing::error!("serving error: {e:?}");
    })?;
    service.waiting().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roommate::state::{AppState, ProjectSettings};
    use roommate::storage::MemStore;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// A registered project configuring `sources` reference sources by name.
    fn state_with(sources: &[&str]) -> Shared {
        let bundle = ProjectSettings {
            reference: sources
                .iter()
                .map(|name| (name.to_string(), roommate::state::ProjectReferenceSource { data: None, fields: vec![] }))
                .collect(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec![],
            milestones: vec![],
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            hierarchy_exclusions: vec![],
        };
        Arc::new(AppState::new(
            Box::new(MemStore::new()),
            HashMap::from([("p1".to_string(), bundle)]),
            None,
        ))
    }

    /// The common case: one configured source, so a caller that names none
    /// still gets the right one. This is what keeps `source` optional instead
    /// of forcing a `get_project_settings` round trip before every call.
    #[test]
    fn test_sole_source_resolves_without_being_named() {
        assert_eq!(resolve_source(&state_with(&["drofus"]), "p1", None).unwrap(), "drofus");
    }

    /// An explicit source always wins — including one the project does not
    /// configure. Whether that source has data is the service's answer to
    /// give (a soft-empty listing), not this resolver's to pre-empt.
    #[test]
    fn test_explicit_source_wins() {
        let state = state_with(&["drofus", "ffe"]);
        assert_eq!(resolve_source(&state, "p1", Some("ffe")).unwrap(), "ffe");
        assert_eq!(resolve_source(&state, "p1", Some("nope")).unwrap(), "nope");
    }

    /// **Ambiguity is refused, not guessed.** Silently taking the first source
    /// would answer confidently about the wrong dataset, which is worse than
    /// an error. The message names the real choices so the retry is obvious.
    #[test]
    fn test_ambiguous_source_is_refused_and_names_the_choices() {
        let err = resolve_source(&state_with(&["drofus", "ffe"]), "p1", None).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("drofus") && msg.contains("ffe"), "must name the choices: {msg}");
    }

    /// A project with no reference source, and an unregistered project, both
    /// fail with a caller-addressable message rather than defaulting to a
    /// name that would then 404 deeper in.
    #[test]
    fn test_no_configured_source_is_an_error() {
        assert!(resolve_source(&state_with(&[]), "p1", None).is_err());
        assert!(resolve_source(&state_with(&["drofus"]), "ghost", None).is_err());
    }

    /// Path and query components are percent-encoded before they reach the
    /// forwarded upload URL — a source or project name with a space or slash
    /// must not split the path.
    #[test]
    fn test_urlencode_escapes_path_hostile_characters() {
        assert_eq!(urlencode("drofus"), "drofus");
        assert_eq!(urlencode("a b/c"), "a%20b%2Fc");
        assert_eq!(urlencode("2026-01-01T10:00:00+10:00"), "2026-01-01T10%3A00%3A00%2B10%3A00");
    }
}
