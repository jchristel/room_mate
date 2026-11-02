//! An MCP server for dRofus: the tools a client (Claude, or any MCP host) can call to read what a
//! dRofus database holds.
//!
//! **Standalone on purpose.** This crate depends on nothing else in the repository it lives in (no
//! path dependency, no shared types, its own lint table), so `crates/drofus-mcp` can be copied out
//! and become its own repository with no edit. Anything it needs from the roommate server is
//! copied, not imported.
//!
//! - [`config`] -- where the dRofus server is and how this process was told, with secrets kept out
//!   of every `Debug` and log line.
//! - [`endpoints`] -- the table of read operations (generated from the OpenAPI definition) and how a
//!   tool's arguments become a request.
//! - [`client`] -- the one generic HTTP path, its error mapping and its size caps.
//! - [`approval`] -- the human gate any future write must pass: dry run, out-of-band approval, audit log.
//!   No tool uses it yet.
//! - [`auth`] -- the authentication seam, with nothing implemented behind it.
//! - [`server`] -- the MCP tools, one method each, and the server's identity.
//!
//! The binary (`src/main.rs`) only parses flags and serves the tools over stdio.

pub mod approval;
pub mod auth;
pub mod client;
pub mod config;
pub mod endpoints;
pub mod server;
