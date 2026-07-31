//! roommate — Revit → Rust → browser room viewer.
//!
//! Library crate shared by two binaries: `roommate` (the Axum HTTP server,
//! `src/main.rs`) and `mcp` (the MCP stdio server, `src/bin/mcp.rs`). Neither
//! binary's transport concerns leak into these modules — `service/` never
//! imports `axum` or `rmcp`, and `bootstrap` (settings → running `AppState`)
//! is shared verbatim so the two entry points can't drift on how a store gets
//! picked or a snapshot gets seeded. Each module carries its own rationale at
//! the top:
//!
//! - `contract`  — the JSON contract shared with the Revit extractor + the
//!   cross-tier property lookup both consumers use.
//! - `settings`  — startup TOML config (sources, test seed, hierarchy defn).
//! - `reference` — reference-data loader + join dataset.
//! - `classify`  — room → full-depth classification path.
//! - `state`     — shared app state: settings registry + the snapshot store
//!   behind its trait, plus the startup seed.
//! - `storage`   — the `SnapshotStore` trait and its two impls (`FsStore`
//!   on disk, `MemStore` volatile).
//! - `bootstrap` — settings file path -> running `Shared` state, reused by
//!   both binaries' `main()`.
//! - `handlers`  — thin Axum adapters: the `/rooms` push (plus the streaming
//!   `/rooms/stream` push for large models) and the read-side routes, which
//!   call into `service`.
//! - `service`   — transport-agnostic derive/assemble logic (dRofus join,
//!   classification, validation), shared by `handlers` and the MCP binary.
//! - `settings_api` — read/save API behind the settings UI: transport-agnostic
//!   core (reads shared with the MCP binary) + the `/api/settings` Axum
//!   adapters; saves hot-swap the registry.

/// The interface the HTTP server binds to. **Loopback on purpose, and not
/// configurable**: the server has no authentication of any kind, and the
/// settings API can write project files, so binding a routable interface would
/// expose that to the network. Serving beyond localhost is a deliberate
/// decision with prerequisites, not a flag.
pub const DEFAULT_HTTP_HOST: &str = "127.0.0.1";

/// Default port for the HTTP server, overridable with `--port` (or the `PORT`
/// environment variable — see `main.rs`). A *default*, not a constant the code
/// depends on: it exists so the common case needs no flag, and so `bin/mcp.rs`
/// has something to point `--server-url` at.
pub const DEFAULT_HTTP_PORT: u16 = 5151;

/// The default bind address, `host:port`.
///
/// A function rather than a third constant. Writing `"127.0.0.1:5151"` out
/// again beside the two values it is made of would be the same physical fact in
/// two places, free to drift — the exact shape of bug `[areas]
/// max_wall_thickness` was introduced to remove (see
/// STRATEGY-AREA-CALCULATION.md). Rust cannot concatenate a `&str` and an
/// integer in a `const` without a macro, so it formats at the one call site
/// that needs the joined form.
pub fn default_http_addr() -> String {
    format!("{DEFAULT_HTTP_HOST}:{DEFAULT_HTTP_PORT}")
}

pub mod bootstrap;
pub mod classify;
pub mod contract;
pub mod handlers;
pub mod reference;
pub mod service;
pub mod settings;
pub mod settings_api;
pub mod state;
pub mod storage;
