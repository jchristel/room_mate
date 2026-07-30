//! roommate — the Axum HTTP server binary. Deliberately thin: parse args,
//! build shared state via `roommate::bootstrap`, wire the router. All the
//! substance lives in the `roommate` lib crate (see its own header for the
//! module index). The `mcp` binary (`src/bin/mcp.rs`) is the other consumer
//! of that lib crate, over stdio instead of HTTP.

use std::path::PathBuf;

use anyhow::Context;
use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};
use clap::Parser;
use axum::http::{Method, StatusCode};
use axum::response::IntoResponse;
use tower_http::{
    cors::{Any, CorsLayer},
    decompression::RequestDecompressionLayer,
    services::ServeDir,
    trace::TraceLayer,
};

use roommate::bootstrap::build_state;
use roommate::handlers::{
    compare_project_milestones, get_model_latest_snapshot, get_project_adjacency, get_project_areas,
    get_project_buildings, get_project_milestones, get_project_snapshots, get_project_validation,
    get_projects, get_reference_latest, get_reference_snapshots, get_rooms, ingest_rooms,
    ingest_rooms_stream,
};
use roommate::settings_api::{
    http_create_project, http_reference_check, http_get_project, http_get_project_resolved,
    http_list_projects, http_update_project, http_upload_reference,
};
use roommate::{DEFAULT_HTTP_HOST, DEFAULT_HTTP_PORT};

/// Cap on the buffered `/rooms` body -- applies to the DECOMPRESSED size, since
/// `RequestDecompressionLayer` inflates before this limit is checked. FFE
/// exports run >100 MB uncompressed; sized generously above that rather than
/// tuned tight, since the streaming route (`/rooms/stream`) is the intended
/// home for anything approaching this ceiling anyway. See HANDOVER-gzip.md.
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
    /// project, each declaring its own `project_id`). See
    /// HANDOVER-per-project-settings.md.
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
    /// The host is deliberately not configurable — see `DEFAULT_HTTP_HOST`.
    #[arg(long, env = "PORT", default_value_t = DEFAULT_HTTP_PORT)]
    port: u16,
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
/// project files and reads dRofus CSVs from paths the caller supplies. With
/// permissive CORS, `POST /api/settings/reference-check {"path": "C:/Windows/win.ini"}`
/// from a hostile page returned that file's second line. Loopback binding did
/// nothing to stop it; only the CORS policy could.
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
/// it with an origin allowlist if this ever runs anywhere but a workstation,
/// and put real authentication in front of `/api/settings` before binding
/// anything but loopback — which is why `DEFAULT_HTTP_HOST` is a constant with
/// no flag behind it, while the port next to it is configurable.
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
}

/// The host names this server will answer to. Anything else is a request that
/// reached our socket under a name we do not own — see `guard_host`.
const ALLOWED_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// Strip the `:port` suffix and any IPv6 brackets from a `Host` header value.
/// IPv6 literals are bracketed (`[::1]:5151`), so the last colon is only a
/// port separator when it falls outside the brackets.
fn host_name(value: &str) -> &str {
    let bare = if let Some(end) = value.find(']') {
        &value[..end + 1]
    } else if value.matches(':').count() == 1 {
        value.split(':').next().unwrap_or(value)
    } else {
        // A bare IPv6 literal with no brackets and no port.
        value
    };
    bare.trim_start_matches('[').trim_end_matches(']')
}

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
/// anything but loopback (see `DEFAULT_HTTP_HOST`).
async fn guard_host(req: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    let claimed = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .or_else(|| req.uri().host());

    if let Some(claimed) = claimed
        && !ALLOWED_HOSTS.contains(&host_name(claimed))
    {
        tracing::warn!("refused request claiming Host {claimed:?} — not a loopback name this server answers to");
        return (
            StatusCode::FORBIDDEN,
            "this server answers only to localhost; the Host header named something else\n",
        )
            .into_response();
    }
    next.run(req).await
}

/// Build the application router. Split out of `main` so the CORS policy above
/// is reachable from a test — a security control with no regression test is one
/// refactor away from silently reverting to `permissive()`.
fn build_router(state: roommate::state::Shared) -> Router {
    Router::new()
        .route(
            "/rooms",
            post(ingest_rooms).get(get_rooms).layer(DefaultBodyLimit::max(ROOMS_BODY_LIMIT_BYTES)),
        )
        // Streaming NDJSON ingest for models too large to buffer whole (see
        // HANDOVER-streaming.md) -- disables the body limit entirely and relies
        // on line-by-line reading to keep peak memory low instead.
        .route(
            "/rooms/stream",
            post(ingest_rooms_stream).layer(DefaultBodyLimit::disable()),
        )
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
        // Milestone comparison: a baseline-vs-each-other diff of rooms and a
        // user-defined property set. POST (not GET) for its list body — see
        // `handlers::compare_project_milestones`.
        .route("/projects/{id}/comparison", post(compare_project_milestones))
        .route(
            "/projects/{project_id}/models/{model_id}/snapshots/latest",
            get(get_model_latest_snapshot),
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
        // Settings read/save API behind static/settings.html — see
        // `settings_api`'s module doc for the save pipeline and trust model.
        .route("/api/settings/projects", get(http_list_projects).post(http_create_project))
        .route("/api/settings/projects/{id}", get(http_get_project).put(http_update_project))
        // Viewer-only resolving read: same as the GET above but falls back to the
        // is_default file, so the viewer's payload id (not a settings project_id)
        // still finds its colour plans. Editors keep the strict route above.
        .route("/api/settings/resolve/{id}", get(http_get_project_resolved))
        .route("/api/settings/reference-check", post(http_reference_check))
        // Serves the viewer page at "/" from ./static.
        .fallback_service(ServeDir::new("static"))
        // Inflate gzip request bodies (Content-Encoding: gzip) before Json/NDJSON
        // parsing sees them. Transparent: a non-gzip body passes through
        // untouched, so an uncompressed sender still works -- purely additive.
        // Added before Cors/Trace so it sits innermost (Router::layer wraps
        // outward: the layer added last runs first on the request path), i.e.
        // decompression happens right before the body reaches a handler.
        .layer(RequestDecompressionLayer::new())
        // Lets a browser viewer served from elsewhere READ /rooms — and nothing
        // more than read. See `read_only_cors`.
        .layer(read_only_cors())
        // Outside CORS, so it runs FIRST on the request path (Router::layer
        // wraps outward). A DNS-rebound request is same-origin and never
        // reaches the CORS layer's decision at all, so the name check has to
        // sit in front of it rather than behind. See `guard_host`.
        .layer(axum::middleware::from_fn(guard_host))
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
    let app = build_router(state);

    let addr = format!("{DEFAULT_HTTP_HOST}:{}", args.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.with_context(|| {
        format!("could not bind {addr} — is another roommate already listening on port {}?", args.port)
    })?;
    tracing::info!("viewer on http://{addr}  (POST room JSON to http://{addr}/rooms)");
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
    use std::sync::Arc;
    use tower::ServiceExt;

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
        let argv = ["roommate", "--server-settings", "s.toml", "--project-settings", "p", "--port", "70000"];
        assert!(Args::try_parse_from(argv).is_err());
    }

    /// The bind address is loopback plus the chosen port, and the host is not
    /// reachable from a flag — see `DEFAULT_HTTP_HOST` for why that is a
    /// security decision rather than an oversight.
    #[test]
    fn test_bind_address_is_loopback_plus_chosen_port() {
        assert_eq!(format!("{DEFAULT_HTTP_HOST}:{}", parse(&["--port", "9000"]).port), "127.0.0.1:9000");
        assert_eq!(roommate::default_http_addr(), format!("127.0.0.1:{DEFAULT_HTTP_PORT}"));
    }

    fn router() -> Router {
        let state: roommate::state::Shared =
            Arc::new(AppState::new(Box::new(MemStore::new()), Default::default(), None));
        build_router(state)
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
    /// `POST /api/settings/reference-check {"path": "C:/Windows/win.ini"}` and read
    /// back that file's second line — loopback binding is no defence, because
    /// the browser making the request is itself local.
    ///
    /// No cross-origin caller may be granted a mutating method on any route.
    /// Asserted over the write routes rather than just the one that leaked: the
    /// bug was the blanket policy, not that single endpoint.
    #[tokio::test]
    async fn test_cross_origin_writes_are_never_granted() {
        for path in [
            "/api/settings/reference-check",
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
        router()
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
        for host in ["evil.example", "evil.example:5151", "192.168.1.10:5151", "roommate.attacker.test"] {
            assert_eq!(
                with_host("/api/settings/reference-check", "POST", host).await,
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
        for host in ["localhost", "localhost:5151", "127.0.0.1", "127.0.0.1:5151", "[::1]", "[::1]:5151"] {
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

    /// The port split has to survive IPv6 literals, where colons are part of
    /// the address rather than a port separator.
    #[test]
    fn test_host_name_strips_port_not_address() {
        assert_eq!(host_name("localhost:5151"), "localhost");
        assert_eq!(host_name("127.0.0.1"), "127.0.0.1");
        assert_eq!(host_name("[::1]:5151"), "::1");
        assert_eq!(host_name("[::1]"), "::1");
        assert_eq!(host_name("::1"), "::1");
    }
}
