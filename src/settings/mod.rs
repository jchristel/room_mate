//! Startup configuration: the TOML settings files and everything parsed from them.
//!
//! **The types are not here — they are in `roommate_shared::settings`**,
//! re-exported whole below so every `crate::settings::…` path the server uses
//! still resolves. They moved so a consumer outside the server can use them
//! without this crate's runtime dependencies (see `roommate_shared`'s header).
//!
//! What stays is what cannot move: **`load`**, the TOML loaders and
//! settings-file-relative path resolution. Reading a directory off disk is
//! exactly the dependency the shared crate refuses, and it is the only half of
//! this module that needs it.

pub use roommate_shared::settings::*;

mod load;

pub use load::{load_server_config, load_settings};
