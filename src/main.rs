//! roommate — the Axum HTTP server binary. Deliberately thin: parse args,
//! build shared state via `roommate::bootstrap`, wire the router. All the
//! substance lives in the `roommate` lib crate (see its own header for the
//! module index). The `mcp` binary (`src/bin/mcp.rs`) is the other consumer
//! of that lib crate, over stdio instead of HTTP.

use std::path::PathBuf;

use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};
use clap::Parser;
use axum::http::Method;
use tower_http::{
    cors::{Any, CorsLayer},
    decompression::RequestDecompressionLayer,
    services::ServeDir,
    trace::TraceLayer,
};

use roommate::bootstrap::build_state;
use roommate::handlers::{
    compare_project_milestones, get_drofus_latest, get_drofus_snapshots, get_model_latest_snapshot,
    get_project_adjacency, get_project_areas, get_project_buildings, get_project_milestones,
    get_project_snapshots, get_project_validation, get_projects, get_rooms, ingest_rooms,
    ingest_rooms_stream,
};
use roommate::settings_api::{
    http_create_project, http_drofus_check, http_get_project, http_get_project_resolved,
    http_list_projects, http_update_project, http_upload_drofus,
};
use roommate::DEFAULT_HTTP_ADDR;

/// Cap on the buffered `/rooms` body -- applies to the DECOMPRESSED size, since
/// `RequestDecompressionLayer` inflates before this limit is checked. FFE
/// exports run >100 MB uncompressed; sized generously above that rather than
/// tuned tight, since the streaming route (`/rooms/stream`) is the intended
/// home for anything approaching this ceiling anyway. See HANDOVER-gzip.md.
const ROOMS_BODY_LIMIT_BYTES: usize = 512 * 1024 * 1024;

/// Cap on a dRofus CSV upload body (decompressed, same as above). Real dRofus
/// exports are a few MB of CSV; 32 MB is generous headroom. Without an
/// explicit layer this route would get axum's silent 2 MB default.
const DROFUS_BODY_LIMIT_BYTES: usize = 32 * 1024 * 1024;

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
/// permissive CORS, `POST /api/settings/drofus-check {"path": "C:/Windows/win.ini"}`
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
/// anything but loopback (see `DEFAULT_HTTP_ADDR`).
fn read_only_cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::HEAD])
        .allow_headers(Any)
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
        // dRofus upload ingest + its read side: uploaded CSVs are timestamped
        // project-scoped snapshots in the store (see settings_api's
        // `upload_drofus` for the validate-before-store pipeline).
        .route(
            "/projects/{id}/drofus",
            post(http_upload_drofus).layer(DefaultBodyLimit::max(DROFUS_BODY_LIMIT_BYTES)),
        )
        .route("/projects/{id}/drofus/snapshots", get(get_drofus_snapshots))
        .route("/projects/{id}/drofus/latest", get(get_drofus_latest))
        // Settings read/save API behind static/settings.html — see
        // `settings_api`'s module doc for the save pipeline and trust model.
        .route("/api/settings/projects", get(http_list_projects).post(http_create_project))
        .route("/api/settings/projects/{id}", get(http_get_project).put(http_update_project))
        // Viewer-only resolving read: same as the GET above but falls back to the
        // is_default file, so the viewer's payload id (not a settings project_id)
        // still finds its colour plans. Editors keep the strict route above.
        .route("/api/settings/resolve/{id}", get(http_get_project_resolved))
        .route("/api/settings/drofus-check", post(http_drofus_check))
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

    let addr = DEFAULT_HTTP_ADDR;
    let listener = tokio::net::TcpListener::bind(addr).await?;
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
    /// `POST /api/settings/drofus-check {"path": "C:/Windows/win.ini"}` and read
    /// back that file's second line — loopback binding is no defence, because
    /// the browser making the request is itself local.
    ///
    /// No cross-origin caller may be granted a mutating method on any route.
    /// Asserted over the write routes rather than just the one that leaked: the
    /// bug was the blanket policy, not that single endpoint.
    #[tokio::test]
    async fn test_cross_origin_writes_are_never_granted() {
        for path in [
            "/api/settings/drofus-check",
            "/api/settings/projects",
            "/api/settings/projects/p1",
            "/rooms",
            "/rooms/stream",
            "/projects/p1/drofus",
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
}
