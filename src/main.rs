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
use tower_http::{cors::CorsLayer, decompression::RequestDecompressionLayer, services::ServeDir, trace::TraceLayer};

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
use roommate::{DEFAULT_HTTP_HOST, DEFAULT_HTTP_PORT};

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

    let app = Router::new()
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
        // Lets the browser viewer call /rooms even if served from elsewhere.
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

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
}
