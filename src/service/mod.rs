//! Transport-agnostic domain layer: the derive/assemble logic that used to
//! live inside the `/rooms` and validation handlers.
//!
//! Domain logic never imports a transport crate -- no `axum`, no `rmcp`, no
//! `StatusCode` in here. `ServiceError` is the seam: each transport (the Axum
//! `handlers`, the MCP server in `src/bin/mcp.rs`) maps it to its own
//! convention. That mapping is deliberately kept *out* of this module -- it
//! belongs in the adapter, not the domain. See HANDOVER-service-layer.md.

pub mod areas;
pub mod comparison;
pub mod drofus;
pub mod milestones;
pub mod projects;
pub mod rooms;
pub mod snapshots;
pub mod validation;

/// Domain-level failure, independent of how a caller reports it.
///
/// Most failure paths are an unexpected internal error (a storage read).
/// Caller-fault variants are added only with a producer: the read endpoints
/// answer an unknown project with a soft "not configured" success by design
/// (see `list_buildings` / `compute_project_validation`), not an error, which
/// is why `Invalid` did not exist until something could actually be malformed.
#[derive(Debug)]
pub enum ServiceError {
    /// An unexpected internal failure (e.g. a storage read error).
    Internal(anyhow::Error),

    /// The request itself is malformed — today, an unparseable room filter
    /// predicate (`rooms::RoomFilter::parse`), the first caller-fault input any
    /// read path accepts. The string is caller-addressable text meant to be
    /// shown verbatim: each adapter maps it to its own convention (HTTP 400,
    /// MCP `invalid_params`), and swallowing it would leave a client with an
    /// empty result and no way to tell a typo from a genuine no-match.
    Invalid(String),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceError::Internal(e) => write!(f, "internal error: {e}"),
            ServiceError::Invalid(msg) => write!(f, "invalid request: {msg}"),
        }
    }
}

impl std::error::Error for ServiceError {}
