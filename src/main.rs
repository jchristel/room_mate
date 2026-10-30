//! roommate — the Axum HTTP server binary. Deliberately thin: parse args,
//! build shared state via `roommate::bootstrap`, wire the router. All the
//! substance lives in the `roommate` lib crate (see its own header for the
//! module index). The `mcp` binary (`src/bin/mcp.rs`) is the other consumer
//! of that lib crate, over stdio instead of HTTP.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::IntoResponse;
use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, post},
    Router,
};
use clap::Parser;
use tower::ServiceBuilder;
use tower_http::{
    cors::{Any, CorsLayer},
    decompression::RequestDecompressionLayer,
    services::ServeDir,
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};

use roommate::assignments_api::{
    http_assign, http_assign_many, http_delete_assignment, http_list_assignments, http_solve_blueprint,
};
use roommate::blueprints_api::{
    http_accept_pending, http_add_revision, http_capture_blueprint, http_create_blueprint, http_get_blueprint,
    http_list_blueprints, http_push_from_revit, http_push_revision, http_reject_pending, http_reshape_blueprint,
    http_resolve_blueprint, http_update_blueprint,
};
use roommate::bootstrap::build_state;
use roommate::connections_api::{http_get_connections, http_put_connections};
use roommate::handlers::{
    activate_model_pending_snapshot, build_project_report, compare_project_milestones, get_ceilings, get_doors,
    get_ffe, get_floors, get_model_latest_snapshot, get_model_pending_snapshot, get_project_adjacency,
    get_project_areas, get_project_buildings, get_project_connectivity, get_project_milestones, get_project_snapshots,
    get_project_stack, get_project_validation, get_projects, get_reference_latest, get_reference_snapshots,
    get_report_columns, get_rooms, get_spaces, get_windows, ingest_ceilings, ingest_ceilings_stream, ingest_doors,
    ingest_doors_stream, ingest_ffe, ingest_ffe_stream, ingest_floors, ingest_floors_stream, ingest_rooms,
    ingest_rooms_stream, ingest_spaces, ingest_spaces_stream, ingest_windows, ingest_windows_stream,
    post_project_reach,
};
use roommate::network::{self, HostPolicy};
use roommate::reports_api::{http_delete_report, http_get_report, http_list_reports, http_save_report};
use roommate::settings_api::{
    http_create_project, http_get_project, http_get_project_resolved, http_list_projects, http_update_project,
    http_upload_reference,
};
use roommate::sheets_api::{
    http_accept_sheet_push, http_delete_sheet, http_get_sheet, http_list_sheets, http_push_sheets,
    http_reject_sheet_push, http_sheet_elevations, http_suggest_sheets, http_update_sheet,
};
use roommate::tag_rules_api::{http_delete_tag_rule, http_get_tag_rule, http_list_tag_rules, http_observe_tags};
use roommate::tasks_api::{
    http_claim_tasks, http_create_tasks, http_dismiss_task, http_get_task, http_list_tasks, http_preview_rls,
    http_report_tasks, http_resend_task,
};
use roommate::{DEFAULT_HTTP_HOST, DEFAULT_HTTP_PORT};

/// Cap on the buffered `/rooms` body -- applies to the DECOMPRESSED size, since
/// `RequestDecompressionLayer` inflates before this limit is checked. FFE
/// exports run >100 MB uncompressed; sized generously above that rather than
/// tuned tight, since the streaming route (`/rooms/stream`) is the intended
/// home for anything approaching this ceiling anyway.
const ROOMS_BODY_LIMIT_BYTES: usize = 512 * 1024 * 1024;

/// Cap on a reference-source CSV upload body (decompressed, same as above).
/// Real exports are a few MB of CSV; 32 MB is generous headroom. Without an
/// explicit layer this route would get axum's silent 2 MB default.
const REFERENCE_BODY_LIMIT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Parser)]
struct Args {
    /// Path to the server-wide TOML settings file (`[storage]`, `[test_data]`).
    #[arg(long)]
    server_settings: PathBuf,

    /// Path to a directory of per-project TOML settings files (one per
    /// project, each declaring its own `project_id`).
    #[arg(long)]
    project_settings: PathBuf,

    /// TCP port to listen on. Defaults to `DEFAULT_HTTP_PORT` (5151).
    ///
    /// Also read from the **`PORT` environment variable**, which is what lets a
    /// harness that assigns ports (the `.claude/launch.json` preview runner,
    /// most PaaS runtimes) place the server without editing a command line. The
    /// flag wins over the variable; clap's `--help` shows which one supplied the
    /// value, so a surprising port is diagnosable rather than mysterious.
    ///
    /// Two things do **not** follow the port automatically, and both are worth
    /// knowing before moving it: `bin/mcp.rs` defaults `--server-url` to the
    /// *default* port, so an MCP server talking to a relocated HTTP server needs
    /// that flag passed explicitly; and any external producer (the pyRevit
    /// pusher) posts to whatever URL it was configured with. That is the reason
    /// the checked-in launch config pins the port instead of letting the harness
    /// assign one — now a choice rather than a constraint.
    ///
    #[arg(long, env = "PORT", default_value_t = DEFAULT_HTTP_PORT)]
    port: u16,

    /// The interface to listen on. Defaults to loopback (`127.0.0.1`): only this machine can reach
    /// the server. `0.0.0.0` listens on every interface, so another computer on the same network
    /// (a Revit machine running the pyRevit extension) can reach it; a single address limits it to
    /// that interface. **There is no authentication**, so use it only on a network you trust (see
    /// `network`). Also read from `ROOMMATE_BIND`.
    #[arg(long, env = "ROOMMATE_BIND", default_value = DEFAULT_HTTP_HOST)]
    bind: IpAddr,

    /// An extra `Host` name to answer to when serving a network (repeat it, or comma-separate; also
    /// `ROOMMATE_ALLOW_HOSTS`). This machine's own hostname and any IP address are already allowed
    /// with a non-loopback `--bind`; list a DNS name the machine is reached by if it has one.
    #[arg(long = "allow-host", env = "ROOMMATE_ALLOW_HOSTS", value_delimiter = ',')]
    allow_host: Vec<String>,
}

/// Cross-origin policy: **read-only, and that is the security boundary.**
///
/// This used to be `CorsLayer::permissive()`, which answers a preflight for any
/// method from any origin. That is a bigger grant than it looks, because
/// "binds loopback" does not mean "only things you trust can call it" — your
/// browser is local, so any page you visit can issue requests to
/// `localhost:5151` and, under a permissive policy, **read the responses**.
///
/// The server has no authentication of any kind, and `/api/settings` writes
/// project files. With permissive CORS, `POST /api/settings/drofus-check
/// {"path": "C:/Windows/win.ini"}` from a hostile page returned that file's
/// second line. Loopback binding did nothing to stop it; only the CORS policy
/// could. That endpoint has since been deleted along with the `file` reference
/// origin, so the specific read is gone — but the policy it forced still
/// guards every remaining write route, which is why it is asserted over all of
/// them below rather than over the one that leaked.
///
/// So: allow `GET`/`HEAD` cross-origin — the documented reason the layer exists
/// at all, so a viewer served from somewhere else can still read `/rooms` — and
/// grant nothing else. Every mutating route, and every request carrying
/// `Content-Type: application/json` (which forces a preflight), is now refused
/// at the preflight for a cross-origin caller.
///
/// **This costs non-browser clients nothing.** CORS is enforced by browsers and
/// ignored by everything else, so the pyRevit pusher, `curl` and the MCP
/// binary's HTTP client are unaffected. The settings UI is served same-origin
/// from `ServeDir`, so it never involved CORS to begin with.
///
/// Cross-origin `GET` still exposes room data to any page you visit. That is a
/// deliberate, narrower tradeoff kept for the dev convenience above — revisit
/// it with an origin allowlist if this ever runs anywhere but a workstation.
/// Leaving loopback is `--bind`, an explicit opt-in for a CLOSED network with no
/// authentication (see `network`): the pyRevit pusher is not a browser, so CORS
/// never applies to it, and the `Host` check below is what stays on.
///
/// **This layer is half the control, not all of it.** It decides from the
/// `Origin`, and a DNS-rebinding attack is built precisely to make that field
/// harmless — a rebound request is same-origin, so it arrives with no `Origin`
/// at all and this policy never runs. `guard_host` below is the other half, and
/// neither is sufficient alone.
fn read_only_cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::HEAD])
        .allow_headers(Any)
        // `ETag` has to be exposed or the conditional-poll saving is same-origin
        // only: a cross-origin reader may *send* `If-None-Match` (allowed above,
        // at the cost of a preflight) but cannot read the tag back off a 200 to
        // send next time, so it would never have one to send. Response headers
        // are opaque to cross-origin JS unless named here, and the safelist does
        // not include `ETag`.
        .expose_headers([header::ETAG])
}

// The host names this server answers to are `network::HostPolicy`'s: loopback, and, when serving
// a network, IP literals, this machine's own names and the operator's. Anything else is a request
// that reached our socket under a name we do not own — see `guard_host`.

/// **The defence `read_only_cors` cannot provide: DNS rebinding.**
///
/// CORS is decided from the *`Origin`*, and a rebinding attack is designed to
/// make that field harmless. A hostile page on `evil.example` is served with a
/// one-second-TTL DNS record; once loaded, the record is re-answered as
/// `127.0.0.1`. The page then fetches `http://evil.example:5151/...`, which the
/// browser now routes to this server — and because the page's own origin *is*
/// `evil.example`, the request is **same-origin**. No preflight is sent, no
/// `Origin` header is attached, and `read_only_cors` never gets a say. The
/// arbitrary-file read it was written to stop is reachable again by a different
/// road.
///
/// The `Host` header is what survives that trick: the browser fills it with the
/// name the page asked for (`evil.example`), not the address it resolved to. So
/// the check is simply "is this a name we own" — loopback, in the three spellings
/// a browser can produce. A rebound request announces someone else's hostname and
/// is refused before it reaches a handler.
///
/// **A missing `Host` is allowed.** HTTP/1.1 requires the header and every
/// browser sends it, so absence means a non-browser client (the pyRevit pusher,
/// `curl`, the MCP binary's HTTP client, and the `oneshot` router tests below),
/// which is not the threat this guards — nothing a rebinding attacker controls
/// can *omit* it.
///
/// This is a name check, not authentication: it stops a remote page from driving
/// this server, and does nothing about a hostile process already on the machine.
/// Real auth in front of `/api/settings` remains the prerequisite for binding
/// anything but a closed network (see `network`).
async fn guard_host(
    axum::extract::State(policy): axum::extract::State<Arc<HostPolicy>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let claimed = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .or_else(|| req.uri().host());

    if let Some(claimed) = claimed
        && !policy.allows(claimed)
    {
        tracing::warn!("refused request claiming Host {claimed:?} — not a name this server answers to");
        return (
            StatusCode::FORBIDDEN,
            "this server does not answer to that Host name; use an IP address, this machine's name, or list it with --allow-host\n",
        )
            .into_response();
    }
    next.run(req).await
}

/// Where the viewer's HTML/CSS/JS live: **beside the executable**, falling back
/// to `./static` relative to the working directory.
///
/// This was a bare `ServeDir::new("static")`, which is right exactly when the
/// process was launched from the crate root -- true under `cargo run` and false
/// for every installed build, because a Start Menu shortcut sets whatever
/// working directory it likes. The failure mode is the expensive kind: the API
/// keeps answering normally and only the *pages* 404, so the server looks
/// healthy while the website is simply missing.
///
/// Exe-relative is tried first because that is the shape an installer ships --
/// `roommate.exe` and `static/` in one directory, launched from anywhere. The
/// working-directory fallback is what keeps the repo working, where the binary
/// sits in `target/debug` with no `static/` beside it. Note the fallback is a
/// *relative* path on purpose: it must stay wrong in the same way it was
/// before, so nothing that already relies on being run from the crate root
/// changes behaviour.
///
/// Existence, not contents, decides the branch. A half-copied install is not
/// worth distinguishing from a missing one -- both want the same answer from
/// whoever reads the startup log, which is why `main` prints the path it chose.
fn viewer_root() -> PathBuf {
    let exe = std::env::current_exe().ok();
    resolve_viewer_root(exe.as_deref().and_then(Path::parent))
}

/// The decision in `viewer_root`, with the one environment lookup lifted out so
/// both branches are reachable from a test. `std::env::current_exe` cannot be
/// pointed somewhere else, and under `cargo test` it resolves to
/// `target/debug/deps` -- which has no `static/` beside it, so an un-split
/// `viewer_root` could only ever exercise the fallback.
fn resolve_viewer_root(exe_dir: Option<&Path>) -> PathBuf {
    exe_dir
        .map(|dir| dir.join("static"))
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(|| PathBuf::from("static"))
}

/// What the viewer's static files are served with, and **the bug it fixes.**
///
/// `ServeDir` sends `last-modified` and nothing else. With no `cache-control`,
/// a browser is free to apply *heuristic* freshness -- roughly a tenth of the
/// file's age -- and reuse a cached copy **without revalidating**, so a plain
/// reload does not re-fetch. That is not a theoretical staleness: the viewer is
/// two independently cached documents, `index.html` and the renderer bundle,
/// and the failure mode when they disagree is silent rather than broken.
///
/// On 2026-09-04 a browser held a NEW `index.html` against a CACHED
/// pre-windows `renderer.bundle.js`. The drawing code lives in the bundle and
/// the controls live in the HTML, so the window toggle appeared, the payload
/// loaded, and the plan drew nothing -- with no console error and nothing wrong
/// on the server. Only a hard reload cleared it, which is not something a
/// reader can be expected to guess.
///
/// `no-cache` is "store it, but revalidate before every use", NOT "do not
/// store": the conditional request still answers 304 from `last-modified`, so
/// the bytes are re-sent only when they actually changed. The cost is one
/// round trip per file on loopback; the alternative is a viewer that can be
/// silently half a release old.
fn viewer_cache_control() -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))
}

/// Build the application router. Split out of `main` so the CORS policy above
/// is reachable from a test — a security control with no regression test is one
/// refactor away from silently reverting to `permissive()`.
/// The routes for data a person authors rather than reads off the model: blueprints and the
/// room layout sheet library. Split out of `build_router` only to keep that one readable.
fn authored_routes() -> Router<roommate::state::Shared> {
    Router::new()
        // Blueprints: a room and its FF&E as recipes against the room's edges. Their own
        // endpoint, one file each, append-only revisions. `capture` is a read: it
        // stores nothing. See `blueprints`.
        .route("/projects/{id}/blueprints", get(http_list_blueprints).post(http_create_blueprint))
        .route("/projects/{id}/blueprints/capture", get(http_capture_blueprint))
        // Writes a draft's items against an edited reference outline; stores nothing.
        .route("/projects/{id}/blueprints/reshape", post(http_reshape_blueprint))
        .route(
            "/projects/{id}/blueprints/{blueprint}",
            get(http_get_blueprint).put(http_update_blueprint),
        )
        .route("/projects/{id}/blueprints/{blueprint}/revisions", post(http_add_revision))
        // Where a revision's items go in one room: a read, nothing stored or applied.
        .route("/projects/{id}/blueprints/{blueprint}/resolve", get(http_resolve_blueprint))
        // The Revit push: a room and its sheets, the server captures the room. See `blueprints_api`.
        .route("/projects/{id}/blueprints/from-revit", post(http_push_from_revit))
        // A revision from elsewhere is stored PENDING and changes nothing current; a person
        // accepts it (it becomes the next revision) or rejects it (kept, marked rejected).
        .route("/projects/{id}/blueprints/{blueprint}/pending", post(http_push_revision))
        .route(
            "/projects/{id}/blueprints/{blueprint}/pending/{pending}/accept",
            post(http_accept_pending),
        )
        .route(
            "/projects/{id}/blueprints/{blueprint}/pending/{pending}/reject",
            post(http_reject_pending),
        )
        // Which blueprint is placed in which room, one per room. See `assignments`.
        .route("/projects/{id}/assignments", get(http_list_assignments).post(http_assign))
        .route("/projects/{id}/assignments/{assignment}", delete(http_delete_assignment))
        .route("/projects/{id}/assignments/many", post(http_assign_many))
        // Room tasks: work for a Revit session. See `tasks`. `roomtasks` is the session's side
        // (a POST, because asking marks the tasks in progress); `tasks` is the person's.
        .route("/projects/{id}/tasks", get(http_list_tasks).post(http_create_tasks))
        .route("/projects/{id}/tasks/rls-preview", post(http_preview_rls))
        .route("/projects/{id}/tasks/{task}", get(http_get_task))
        .route("/projects/{id}/tasks/{task}/dismiss", post(http_dismiss_task))
        .route("/projects/{id}/tasks/{task}/resend", post(http_resend_task))
        // Tag rules: where a tag goes relative to the element it tags, one rule per key. POST is
        // the extractor's observations. See `tag_rules`.
        .route("/projects/{id}/tagrules", get(http_list_tag_rules))
        .route("/projects/{id}/tagrules/observations", post(http_observe_tags))
        .route("/projects/{id}/tagrules/{rule}", get(http_get_tag_rule).delete(http_delete_tag_rule))
        .route("/projects/{id}/roomtasks", post(http_claim_tasks))
        .route("/projects/{id}/roomtasks/report", post(http_report_tasks))
        .route("/projects/{id}/blueprints/{blueprint}/solve", post(http_solve_blueprint))
        // The room layout sheet library: sheets a person chose to export, kept as exported.
        // Its own endpoint, apart from blueprints. POST is the extractor's push; `suggest`
        // is a read, asked for in sheet creation mode only. See `sheets`.
        .route("/projects/{id}/sheets", get(http_list_sheets).post(http_push_sheets))
        .route("/projects/{id}/sheets/suggest", get(http_suggest_sheets))
        .route(
            "/projects/{id}/sheets/{sheet}",
            get(http_get_sheet).put(http_update_sheet).delete(http_delete_sheet),
        )
        .route("/projects/{id}/sheets/{sheet}/elevations", get(http_sheet_elevations))
        .route("/projects/{id}/sheets/{sheet}/pending/{pending}/accept", post(http_accept_sheet_push))
        .route("/projects/{id}/sheets/{sheet}/pending/{pending}/reject", post(http_reject_sheet_push))
}

fn build_router(state: roommate::state::Shared, hosts: HostPolicy) -> Router {
    Router::new()
        .route(
            "/rooms",
            post(ingest_rooms).get(get_rooms).layer(DefaultBodyLimit::max(ROOMS_BODY_LIMIT_BYTES)),
        )
        // Streaming NDJSON ingest for models too large to buffer whole --
        // disables the body limit entirely and relies
        // on line-by-line reading to keep peak memory low instead.
        .route("/rooms/stream", post(ingest_rooms_stream).layer(DefaultBodyLimit::disable()))
        // Doors: the second primary entity, same body limits and same streaming
        // pair as rooms. Doors may be pushed BEFORE their rooms -- the gate that
        // once required them is deliberately gone, and an unresolvable room
        // reference is reported by `opening_report` rather than refused here. What
        // `handlers::check_doors_ingest` still refuses is an unphased push, and
        // a doors push that disagrees on an already-established phase.
        .route(
            "/doors",
            post(ingest_doors).get(get_doors).layer(DefaultBodyLimit::max(ROOMS_BODY_LIMIT_BYTES)),
        )
        .route("/doors/stream", post(ingest_doors_stream).layer(DefaultBodyLimit::disable()))
        // Windows: the third primary entity, and the same pair again. The only
        // route-level difference from doors is the name -- the phase gate, the
        // sinks and the read assembly behind these are the doors ones with a
        // SnapshotKind passed in. A facade model is the case that motivated
        // them: 158 windows and no rooms at all, which only resolves once
        // [windows] room_resolution can see the project's other models.
        .route(
            "/windows",
            post(ingest_windows).get(get_windows).layer(DefaultBodyLimit::max(ROOMS_BODY_LIMIT_BYTES)),
        )
        .route("/windows/stream", post(ingest_windows_stream).layer(DefaultBodyLimit::disable()))
        // FF&E, on the openings ingest shape and the rooms read shape: one
        // buffered route, one streamed, one read. `/ffe` is a collective noun
        // and stays singular-looking on purpose -- the element key on the wire
        // is `ffe` too, so the route and the payload agree.
        .route("/ffe", post(ingest_ffe).get(get_ffe).layer(DefaultBodyLimit::max(ROOMS_BODY_LIMIT_BYTES)))
        .route("/ffe/stream", post(ingest_ffe_stream).layer(DefaultBodyLimit::disable()))
        // Spaces: the fifth primary entity, and the first whose record is the
        // ROOMS record rather than the openings one. The read carries the plan's
        // spaces layer; the space-to-room reconciliation that gives the entity
        // something to SAY rides `/validation` instead, because it answers about
        // a project's models rather than about one model's spaces -- see
        // `service::spaces::space_report`. Two rules here are this entity's alone: an empty
        // spaces list is accepted because "audited, and holds none" is the
        // finding, and a push whose phase disagrees is QUARANTINED like a rooms
        // push rather than refused like an openings one, because a space carries
        // no room id to strand. See `handlers::preflight_spaces`.
        .route(
            "/spaces",
            post(ingest_spaces).get(get_spaces).layer(DefaultBodyLimit::max(ROOMS_BODY_LIMIT_BYTES)),
        )
        .route("/spaces/stream", post(ingest_spaces_stream).layer(DefaultBodyLimit::disable()))
        // Ceilings: the sixth entity, and the first whose room association is
        // purely geometric -- a ceiling has no room parameter and a room has no
        // ceiling parameter, so `service::surfaces` derives the join from
        // polygon overlap on every read and stores none of it. A disagreeing
        // phase is QUARANTINED rather than refused: unlike a doors push, this
        // one carries no room reference for a promotion to strand. See
        // `handlers::begin_surface_ingest`.
        .route(
            "/ceilings",
            post(ingest_ceilings)
                .get(get_ceilings)
                .layer(DefaultBodyLimit::max(ROOMS_BODY_LIMIT_BYTES)),
        )
        .route("/ceilings/stream", post(ingest_ceilings_stream).layer(DefaultBodyLimit::disable()))
        // Floors: the seventh entity and the second slab. The same record, the
        // same ingest and the same read as ceilings, under their own key --
        // see `service::surfaces`.
        .route(
            "/floors",
            post(ingest_floors).get(get_floors).layer(DefaultBodyLimit::max(ROOMS_BODY_LIMIT_BYTES)),
        )
        .route("/floors/stream", post(ingest_floors_stream).layer(DefaultBodyLimit::disable()))
        .route("/projects", get(get_projects))
        .route("/projects/{id}/buildings", get(get_project_buildings))
        .route("/projects/{id}/validation", get(get_project_validation))
        // Snapshot history: everything per project (grouped by model), and the
        // per-model latest a follow-up upload attaches its data to.
        .route("/projects/{id}/snapshots", get(get_project_snapshots))
        // Milestones: named dated pins over snapshots, defined per project in
        // its settings file; the viewer's dropdown reads this list.
        .route("/projects/{id}/milestones", get(get_project_milestones))
        // Hierarchy gross-area footprints: dissolved per-tier polygons + areas,
        // scoped by ?building=/?milestone= like /rooms. See service::areas.
        .route("/projects/{id}/areas", get(get_project_areas))
        // Room-to-room adjacency: shared-wall graph for one level's rooms,
        // scoped like /rooms plus a tunable ?wall_max= (the wall tolerance,
        // which is what spans Revit's two room-boundary regimes). Fetched on a
        // room SELECTION change, not the 2s poll -- its own trigger and its own
        // consumer, which is why it is not part of /rooms. See service::adjacency.
        .route("/projects/{id}/adjacency", get(get_project_adjacency))
        // Door connectivity and the shortest route between two rooms: its own
        // trigger (a pair of clicks) and its own consumer, so not part of /rooms.
        .route("/projects/{id}/connectivity", get(get_project_connectivity))
        // How far a walk of a given length reaches from some start points, as a
        // grid over the plan (coverage from start points). POST for its list body, like
        // /comparison; it reads and writes nothing. See service::reach.
        .route("/projects/{id}/reach", post(post_project_reach))
        // The rooms over and under one room, ranked: candidates for a vertical link.
        .route("/projects/{id}/stack", get(get_project_stack))
        // The authored half of connectivity: open zones. PUT replaces the whole
        // list and names the version it read, so two editors cannot overwrite
        // each other unseen. See `connections`.
        .route("/projects/{id}/connections", get(http_get_connections).put(http_put_connections))
        .merge(authored_routes())
        // Milestone comparison: a baseline-vs-each-other diff of rooms and a
        // user-defined property set. POST (not GET) for its list body — see
        // `handlers::compare_project_milestones`.
        .route("/projects/{id}/comparison", post(compare_project_milestones))
        .route("/projects/{id}/reports", post(build_project_report))
        .route("/projects/{id}/reports/columns", get(get_report_columns))
        // Saved reports are documents the reports page owns, so they sit under
        // /api/ beside the settings API rather than under the project read
        // routes -- what they are is config-shaped, even though what they SAY
        // is a question rather than a policy.
        .route("/api/reports/projects/{project}", get(http_list_reports))
        .route(
            "/api/reports/projects/{project}/{report}",
            get(http_get_report).put(http_save_report).delete(http_delete_report),
        )
        .route(
            "/projects/{project_id}/models/{model_id}/snapshots/latest",
            get(get_model_latest_snapshot),
        )
        // The re-phase flow. A model's phase is fixed by its first phased push,
        // so a push naming a different one is stored inert (202) rather than
        // refused; these two routes are how it is then discovered and made live.
        // `activate` is the only route that can change a model's phase.
        .route(
            "/projects/{project_id}/models/{model_id}/snapshots/pending",
            get(get_model_pending_snapshot),
        )
        .route(
            "/projects/{project_id}/models/{model_id}/snapshots/pending/activate",
            post(activate_model_pending_snapshot),
        )
        // Reference-source upload ingest + its read side, for any source a
        // project configures with an `upload` origin: uploaded CSVs are
        // timestamped project-scoped snapshots in the store (see
        // settings_api's `upload_reference` for the validate-before-store
        // pipeline).
        .route(
            "/projects/{id}/reference/{source}",
            post(http_upload_reference).layer(DefaultBodyLimit::max(REFERENCE_BODY_LIMIT_BYTES)),
        )
        .route("/projects/{id}/reference/{source}/snapshots", get(get_reference_snapshots))
        .route("/projects/{id}/reference/{source}/latest", get(get_reference_latest))
        // Settings read/save API behind the settings page (src-js/settings) — see
        // `settings_api`'s module doc for the save pipeline and trust model.
        .route("/api/settings/projects", get(http_list_projects).post(http_create_project))
        .route("/api/settings/projects/{id}", get(http_get_project).put(http_update_project))
        // Viewer-only resolving read: same as the GET above but falls back to the
        // is_default file, so the viewer's payload id (not a settings project_id)
        // still finds its colour plans. Editors keep the strict route above.
        .route("/api/settings/resolve/{id}", get(http_get_project_resolved))
        // Serves the viewer page at "/" from the viewer root -- see `viewer_root`
        // for why that is not simply "static". The cache-control layer is
        // scoped to THIS service rather than added to the router: the API
        // routes answer with their own ETags and revision fields, and a blanket
        // no-cache would be making a caching decision for them too. See
        // `viewer_cache_control`.
        .fallback_service(ServiceBuilder::new().layer(viewer_cache_control()).service(ServeDir::new(viewer_root())))
        // Inflate gzip request bodies (Content-Encoding: gzip) before Json/NDJSON
        // parsing sees them. Transparent: a non-gzip body passes through
        // untouched, so an uncompressed sender still works -- purely additive.
        // Added before Cors/Trace so it sits innermost (Router::layer wraps
        // outward: the layer added last runs first on the request path), i.e.
        // decompression happens right before the body reaches a handler.
        // Level-scoped pushes: carries the rest of a model over before the stream handlers read it.
        // Inside decompression, so it reads the inflated body. See `scoped_push`.
        .route_layer(axum::middleware::from_fn_with_state(state.clone(), roommate::scoped_push::scope_stream))
        .layer(RequestDecompressionLayer::new())
        // Lets a browser viewer served from elsewhere READ /rooms — and nothing
        // more than read. See `read_only_cors`.
        .layer(read_only_cors())
        // Outside CORS, so it runs FIRST on the request path (Router::layer
        // wraps outward). A DNS-rebound request is same-origin and never
        // reaches the CORS layer's decision at all, so the name check has to
        // sit in front of it rather than behind. See `guard_host`.
        .layer(axum::middleware::from_fn_with_state(Arc::new(hosts), guard_host))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Filter on the *current* crate name -- this said "revit_viewer" (the
    // crate's old name) for a while, which silently dropped every log event
    // the server emitted. RUST_LOG still wins when set.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("roommate=info,tower_http=info")),
        )
        .init();

    let args = Args::parse();
    let state = build_state(&args.server_settings, &args.project_settings)?;
    // Say, once, which settings name a property the data no longer has.
    roommate::settings_check::log_startup(&state, &args.project_settings);
    let hostname = network::machine_hostname();
    let hosts = HostPolicy::for_bind(args.bind, hostname.as_deref(), &args.allow_host);
    let app = build_router(state, hosts);

    let addr = SocketAddr::new(args.bind, args.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.with_context(|| {
        format!("could not bind {addr} — is another roommate already listening on port {}?", args.port)
    })?;
    // Logged because a wrong answer here is otherwise invisible: the API still
    // works and only the pages 404. See `viewer_root`.
    tracing::info!("viewer files from {}", viewer_root().display());
    tracing::info!("viewer on http://{addr}  (POST room JSON to http://{addr}/rooms)");
    if network::serves_network(args.bind) {
        // The address a person types into the pyRevit side, and the one thing worth saying out
        // loud about the choice: nothing here checks who is asking.
        for url in network::reachable_urls(addr, hostname.as_deref()) {
            tracing::info!("reachable from other computers at {url}");
        }
        tracing::warn!(
            "serving a network: there is no authentication, so anyone who can reach this port can read and change the data. Use a network you trust."
        );
    }
    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use roommate::state::AppState;
    use roommate::storage::MemStore;
    use tower::ServiceExt;

    /// The installed shape: `static/` beside the binary wins, and the result is
    /// absolute -- which is the whole point, since the caller's working
    /// directory is a Start Menu shortcut's business, not ours.
    #[test]
    fn test_viewer_root_prefers_a_static_dir_beside_the_exe() {
        let tmp = std::env::temp_dir().join("roommate_viewer_root_test");
        std::fs::create_dir_all(tmp.join("static")).expect("temp dir");

        let resolved = resolve_viewer_root(Some(&tmp));

        assert_eq!(resolved, tmp.join("static"));
        assert!(resolved.is_absolute(), "an installed build is launched from anywhere");
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// No `static/` beside the binary -- the `cargo run` case -- falls back to
    /// the historic **relative** path. Relative deliberately: this branch has to
    /// stay wrong in exactly the way it was before, or the repo workflow that
    /// depends on being launched from the crate root changes under it.
    #[test]
    fn test_viewer_root_falls_back_to_the_working_directory() {
        let tmp = std::env::temp_dir().join("roommate_viewer_root_empty");
        std::fs::create_dir_all(&tmp).expect("temp dir");

        assert_eq!(resolve_viewer_root(Some(&tmp)), PathBuf::from("static"));
        assert_eq!(resolve_viewer_root(None), PathBuf::from("static"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    fn parse(extra: &[&str]) -> Args {
        let mut argv = vec!["roommate", "--server-settings", "s.toml", "--project-settings", "p"];
        argv.extend_from_slice(extra);
        Args::parse_from(argv)
    }

    /// Omitting `--port` keeps the historic 5151, so every existing command
    /// line, script and launch config behaves exactly as before this flag
    /// existed. The whole point of the change is that the port stopped being
    /// *fixed*, not that it moved.
    #[test]
    fn test_port_defaults_to_the_historic_value() {
        assert_eq!(parse(&[]).port, DEFAULT_HTTP_PORT);
        assert_eq!(DEFAULT_HTTP_PORT, 5151, "moving this silently relocates every default install");
    }

    /// `--port` is honoured, including 0 — which asks the OS for an ephemeral
    /// port and is the one value a range check would have been tempted to
    /// reject. It is genuinely useful (parallel test servers), and unlike
    /// `[areas] max_wall_thickness`, a zero here means something to the OS
    /// rather than contradicting a declared fact.
    #[test]
    fn test_port_flag_is_honoured() {
        assert_eq!(parse(&["--port", "8080"]).port, 8080);
        assert_eq!(parse(&["--port", "0"]).port, 0);
        assert_eq!(parse(&["--port", "65535"]).port, 65535);
    }

    /// A port outside `u16` is rejected by parsing rather than truncated —
    /// clap's own error, so `--port 70000` cannot quietly become 4464.
    #[test]
    fn test_out_of_range_port_is_rejected() {
        let argv = [
            "roommate",
            "--server-settings",
            "s.toml",
            "--project-settings",
            "p",
            "--port",
            "70000",
        ];
        assert!(Args::try_parse_from(argv).is_err());
    }

    /// With no `--bind` the address is loopback plus the chosen port: leaving this machine is an
    /// explicit opt-in (see `DEFAULT_HTTP_HOST`), never a default.
    #[test]
    fn test_bind_address_is_loopback_plus_chosen_port() {
        let args = parse(&["--port", "9000"]);
        assert_eq!(SocketAddr::new(args.bind, args.port).to_string(), "127.0.0.1:9000");
        assert_eq!(DEFAULT_HTTP_HOST, "127.0.0.1");
        assert_eq!(roommate::default_http_addr(), format!("127.0.0.1:{DEFAULT_HTTP_PORT}"));
    }

    fn router() -> Router {
        router_for(HostPolicy::loopback_only())
    }

    fn router_for(hosts: HostPolicy) -> Router {
        let state: roommate::state::Shared =
            Arc::new(AppState::new(Box::new(MemStore::new()), Default::default(), None));
        build_router(state, hosts)
    }

    /// **The regression guard for the silent-stale-viewer bug.** Without
    /// `cache-control`, a browser may reuse a cached bundle without
    /// revalidating, which on 2026-09-04 paired a new `index.html` with a
    /// pre-windows renderer: the toggle was there, the payload loaded, nothing
    /// drew. Asserted on a 404 from the static service because that is still a
    /// response the layer wrapped -- the point is that the header is attached to
    /// the viewer service at all, and a fixture file would make this a test
    /// about `static/`'s contents instead.
    #[tokio::test]
    async fn test_viewer_files_are_served_revalidate_always() {
        let response = router()
            .oneshot(Request::builder().uri("/index.html").body(Body::empty()).unwrap())
            .await
            .unwrap();

        let cache_control = response.headers().get(header::CACHE_CONTROL);
        assert_eq!(
            cache_control.map(|v| v.to_str().unwrap()),
            Some("no-cache"),
            "a viewer file cached without revalidation goes stale silently",
        );
    }

    /// The API is NOT swept up by the layer above. `/rooms` answers with its own
    /// revision and ETag, and a blanket no-cache would be this module deciding
    /// caching policy for handlers that already have one.
    #[tokio::test]
    async fn test_api_routes_are_left_to_their_own_caching() {
        let response = router()
            .oneshot(Request::builder().uri("/projects").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert!(
            response.headers().get(header::CACHE_CONTROL).is_none(),
            "the static layer must not reach the API routes",
        );
    }

    /// One CORS preflight, as a browser would send it.
    async fn preflight(path: &str, method: &str) -> (StatusCode, Option<String>) {
        let response = router()
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri(path)
                    .header(header::ORIGIN, "https://evil.example")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, method)
                    .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let allowed = response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_METHODS)
            .map(|v| v.to_str().unwrap().to_string());
        (response.status(), allowed)
    }

    /// **The regression guard for a real vulnerability.** Under
    /// `CorsLayer::permissive()` a page on any origin could preflight and then
    /// `POST /api/settings/drofus-check {"path": "C:/Windows/win.ini"}` and read
    /// back that file's second line — loopback binding is no defence, because
    /// the browser making the request is itself local. That endpoint is gone,
    /// but the routes below can still write project files.
    ///
    /// No cross-origin caller may be granted a mutating method on any route.
    /// Asserted over the write routes rather than just the one that leaked: the
    /// bug was the blanket policy, not that single endpoint.
    #[tokio::test]
    async fn test_cross_origin_writes_are_never_granted() {
        for path in [
            "/api/settings/projects",
            "/api/settings/projects/p1",
            "/rooms",
            "/rooms/stream",
            "/projects/p1/reference/drofus",
            "/projects/p1/comparison",
        ] {
            for method in ["POST", "PUT"] {
                let (_, allowed) = preflight(path, method).await;
                let allowed = allowed.unwrap_or_default();
                assert!(
                    !allowed.contains(method) && !allowed.contains('*'),
                    "{method} {path}: cross-origin preflight granted {allowed:?}"
                );
            }
        }
    }

    /// The half that is deliberately kept: a viewer served from another origin
    /// can still READ. If this ever fails, the CORS layer was tightened past
    /// what the layer exists for, and the dev workflow it supports is broken.
    #[tokio::test]
    async fn test_cross_origin_reads_are_still_allowed() {
        let (status, allowed) = preflight("/rooms", "GET").await;
        assert!(status.is_success(), "preflight rejected outright: {status}");
        assert!(allowed.unwrap_or_default().contains("GET"), "cross-origin GET must stay allowed");
    }

    /// Same-origin traffic never involves CORS, so the settings UI (served from
    /// `ServeDir` on this very origin) is untouched by the policy above — a
    /// request with no `Origin` header reaches its handler normally.
    #[tokio::test]
    async fn test_same_origin_requests_are_unaffected() {
        let response = router()
            .oneshot(Request::builder().uri("/projects").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// One request announcing `host`, as a browser fills the header in.
    async fn with_host(path: &str, method: &str, host: &str) -> StatusCode {
        with_host_under(HostPolicy::loopback_only(), path, method, host).await
    }

    async fn with_host_under(hosts: HostPolicy, path: &str, method: &str, host: &str) -> StatusCode {
        router_for(hosts)
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header(header::HOST, host)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    /// **The regression guard for the hole CORS cannot cover.** A DNS-rebound
    /// request is same-origin, so it carries no `Origin` and never reaches the
    /// CORS layer's decision — `test_cross_origin_writes_are_never_granted`
    /// would pass while the arbitrary-file read was wide open again. What the
    /// attacker cannot forge is the `Host`: the browser writes the name the
    /// page asked for, and that name is not ours.
    #[tokio::test]
    async fn test_rebound_host_is_refused() {
        for host in [
            "evil.example",
            "evil.example:5151",
            "192.168.1.10:5151",
            "roommate.attacker.test",
        ] {
            assert_eq!(
                with_host("/api/settings/projects", "POST", host).await,
                StatusCode::FORBIDDEN,
                "Host {host:?} must not reach a handler"
            );
            // Reads too: rebinding is just as good for exfiltrating /rooms.
            assert_eq!(with_host("/projects", "GET", host).await, StatusCode::FORBIDDEN, "Host {host:?}");
        }
    }

    /// Every spelling of loopback a browser can put in the header still works,
    /// with and without the port, including the bracketed IPv6 literal. If this
    /// fails the guard has locked the operator out of their own viewer.
    #[tokio::test]
    async fn test_loopback_hosts_are_allowed() {
        for host in [
            "localhost",
            "localhost:5151",
            "127.0.0.1",
            "127.0.0.1:5151",
            "[::1]",
            "[::1]:5151",
        ] {
            assert_eq!(with_host("/projects", "GET", host).await, StatusCode::OK, "Host {host:?} must be allowed");
        }
    }

    /// A client that sends no `Host` at all is not the threat — nothing a
    /// rebinding attacker controls can omit it, while `curl`, the pyRevit
    /// pusher and the MCP binary's HTTP client all reach the server this way.
    #[tokio::test]
    async fn test_missing_host_is_allowed() {
        let response = router()
            .oneshot(Request::builder().uri("/projects").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// **Serving a network must not switch the rebinding defence off.** With a non-loopback bind
    /// the pyRevit pusher's `Host: 192.168.x.x:5151` reaches a handler, and so does this machine's
    /// own name, while a made-up domain (what a rebinding page presents) is still refused, on
    /// reads and on the settings writes alike.
    #[tokio::test]
    async fn test_a_network_bind_answers_ip_literals_and_still_refuses_other_domains() {
        let net = || HostPolicy::for_bind("0.0.0.0".parse().unwrap(), Some("Mac-Studio.lan"), &[]);
        for host in [
            "192.168.1.20:5151",
            "10.0.0.5",
            "[fe80::1]:5151",
            "mac-studio.local:5151",
            "localhost:5151",
        ] {
            assert_eq!(with_host_under(net(), "/projects", "GET", host).await, StatusCode::OK, "Host {host:?}");
        }
        for host in ["evil.example", "evil.example:5151", "192.168.1.20.evil.example"] {
            for (path, method) in [("/projects", "GET"), ("/api/settings/projects", "POST")] {
                assert_eq!(
                    with_host_under(net(), path, method, host).await,
                    StatusCode::FORBIDDEN,
                    "{method} {path} with Host {host:?}"
                );
            }
        }
    }

    /// The default is unchanged: a LAN address is refused until the operator opts in with `--bind`.
    #[tokio::test]
    async fn test_without_bind_a_lan_address_is_still_refused() {
        assert_eq!(with_host("/projects", "GET", "192.168.1.20:5151").await, StatusCode::FORBIDDEN);
    }

    /// The refusal tells the operator what to do, because the usual cause is a Host spelled with a
    /// DNS name nobody listed.
    #[tokio::test]
    async fn test_a_refused_host_says_how_to_allow_it() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/projects")
                    .header(header::HOST, "plans.internal")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("--allow-host"));
    }

    /// An end-to-end check over a real socket on every interface: a request that names the machine
    /// by an address other than `localhost` (as another computer's pyRevit would) is answered.
    #[tokio::test]
    async fn test_a_server_bound_to_every_interface_answers_a_request_by_address() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let hosts = HostPolicy::for_bind("0.0.0.0".parse().unwrap(), None, &[]);
        let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, router_for(hosts)).await.unwrap() });
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let request = format!("GET /projects HTTP/1.1\r\nHost: 192.168.1.20:{port}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).await.unwrap();
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    }

    /// `--bind` defaults to loopback, takes an address (IPv4 or IPv6) and refuses a name.
    #[test]
    fn test_bind_defaults_to_loopback_and_takes_an_address() {
        assert_eq!(parse(&[]).bind, "127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(parse(&["--bind", "0.0.0.0"]).bind, "0.0.0.0".parse::<IpAddr>().unwrap());
        assert_eq!(parse(&["--bind", "192.168.1.20"]).bind, "192.168.1.20".parse::<IpAddr>().unwrap());
        assert_eq!(parse(&["--bind", "::"]).bind, "::".parse::<IpAddr>().unwrap());
        let bad = Args::try_parse_from([
            "roommate",
            "--server-settings",
            "s",
            "--project-settings",
            "p",
            "--bind",
            "my-mac",
        ]);
        assert!(bad.is_err(), "a name is not an interface");
    }

    #[test]
    fn test_allowed_hosts_are_repeatable_and_comma_separated() {
        assert_eq!(
            parse(&["--allow-host", "a.internal", "--allow-host", "b"]).allow_host,
            vec!["a.internal", "b"]
        );
        assert_eq!(parse(&["--allow-host", "a.internal,b"]).allow_host, vec!["a.internal", "b"]);
        assert!(parse(&[]).allow_host.is_empty());
    }
}
