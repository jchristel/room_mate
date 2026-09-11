//! The types a consumer outside the server needs, with none of the server's
//! runtime attached.
//!
//! **Why this crate exists.** It was split out on 2026-09-11 for the framework
//! comparison recorded in STRATEGY-BROWSER.md ("UI growth"), where a Rust+WASM
//! page compiled these types directly — something `roommate` cannot offer,
//! because it depends on axum, tokio, rmcp and reqwest. That page lost the
//! comparison and was deleted. The crate was kept: it is the natural source for
//! the generated TypeScript wire types the comparison left open, and it costs
//! the server nothing.
//!
//! **The dependency rule is the whole design:** nothing here may need a socket,
//! a filesystem or a runtime — serde, anyhow and chrono's parsing half, and that
//! is all. A type that needs more stays in `roommate`. `settings` is the worked
//! example: the project-file *types* and their validation live here; reading the
//! files off disk (`roommate::settings::load`) does not.
//!
//! **The module paths mirror the server's on purpose.** `roommate::settings`
//! glob re-exports `settings`, and `roommate::contract` and
//! `roommate::settings_api` re-export what moved from them, so every path the
//! server already used still resolves. That is also why the moved code still
//! says `crate::contract::RoomBoundary`: the path means the same thing in either
//! crate.
//!
//! Grows only when a consumer outside the server needs a type — not in
//! anticipation of one. Moving a type here is cheap; moving one back out, once
//! something else depends on it, is not.

pub mod contract;
pub mod settings;
pub mod settings_api;
