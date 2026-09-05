//! Transport-agnostic domain layer: the derive/assemble logic that used to
//! live inside the `/rooms` and validation handlers.
//!
//! Domain logic never imports a transport crate -- no `axum`, no `rmcp`, no
//! `StatusCode` in here. `ServiceError` is the seam: each transport (the Axum
//! `handlers`, the MCP server in `src/bin/mcp.rs`) maps it to its own
//! convention. That mapping is deliberately kept *out* of this module -- it
//! belongs in the adapter, not the domain. (CODING-CONVENTIONS.md,
//! "Dependency direction is the seam".)

pub mod adjacency;
pub mod areas;
pub mod comparison;
pub mod entity_scope;
pub mod milestones;
pub mod openings;
pub mod projects;
pub mod reference;
pub mod room_locator;
pub mod rooms;
pub mod snapshots;
pub mod validation;

/// A content cursor for a scoped read: which snapshot each in-scope model would
/// contribute, hashed, **without opening one of them**.
///
/// **Why this exists.** `/rooms` and `/doors` already return a `revision` the
/// viewer compares to decide whether to re-render — but it arrives at the bottom
/// of the body, so a poll that changes nothing still costs a full assemble and a
/// full download. At RHH scale that is 16 MB and 73 MB every two seconds, per
/// open tab: measured, ~4.7 s of work against a 2 s tick, three concurrent
/// `/doors` reads taking the server from 245 MB to 904 MB resident. This is the
/// same question answered from the index instead, so the adapters can settle an
/// unchanged poll with a 304 and no body at all.
///
/// **It is deliberately conservative, and the asymmetry is the safety
/// property.** A cursor that changes when the body did not costs one needless
/// body — correct, just unhelpful. A cursor that *matches* when the body
/// changed would serve a stale plan forever. So every approximation here errs
/// towards over-reporting change:
///
/// - It covers every model the scope admits, where `scoped_revision` covers only
///   models that *contributed a row*. A building or property filter can exclude
///   a model with no matching rooms; the cursor still counts it, so its next
///   push moves the cursor even though the body might not change.
/// - Under a milestone it hashes the pinned ids, since that is what the read
///   would serve — but a model whose pin dangles is counted rather than skipped
///   (`scope_payloads` warns and drops it), which again only over-reports.
///
/// The one thing it does **not** track is settings: a colour plan or a dRofus
/// mapping changes derived data without touching a stored snapshot. That is not
/// a new gap — `scoped_revision` documents the same exclusion, and the viewer
/// re-fetches on a settings change through its own trigger rather than the poll.
///
/// `kinds` is a list because a doors read is not doors-only: `assemble_doors`
/// resolves ownership and geometry against the *rooms* of the same scope
/// (`build_candidates`), so a rooms push changes a doors body. Passing only
/// `Doors` there would be exactly the unsafe direction above.
pub(crate) fn scope_cursor(
    state: &crate::state::AppState,
    project: Option<&str>,
    milestone: Option<&str>,
    kinds: &[crate::storage::SnapshotKind],
) -> Result<String, ServiceError> {
    use std::hash::{Hash, Hasher};

    let index = state.model_index().map_err(ServiceError::Internal)?;
    let registry = state.settings();

    let mut parts: Vec<(&str, &str, &str, String)> = Vec::new();
    for row in &index {
        if project.is_some_and(|p| row.key.project_id != p) {
            continue;
        }
        // Same "skip on read" policy `scope_payloads` applies: an unregistered
        // project contributes nothing, so its pushes must not move the cursor.
        let Some(bundle) = registry.settings_for(&row.key.project_id) else {
            continue;
        };
        for kind in kinds {
            let id = match milestone {
                None => row.latest.get(kind).cloned(),
                // A milestone pins one id per model and is not per kind — the
                // pin names a rooms snapshot and the doors read follows it, the
                // same substitution `scope_payloads` performs.
                Some(wanted) => bundle
                    .milestones
                    .iter()
                    .find(|m| m.name == wanted)
                    .and_then(|m| m.attachments.get(&row.key.model_id))
                    .cloned(),
            };
            if let Some(id) = id {
                parts.push((row.key.project_id.as_str(), row.key.model_id.as_str(), kind.label(), id));
            }
        }
    }
    // Sorted before hashing for the same reason `scoped_revision` sorts: model
    // iteration order is a directory-walk artefact, not content.
    parts.sort_unstable();

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        DoorPayload, Level, Model, Project, Room, RoomPayload, Snapshot, SUPPORTED_DOOR_SCHEMA, SUPPORTED_SCHEMA,
    };
    use crate::state::{AppState, ProjectSettings};
    use crate::storage::{MemStore, SnapshotKind};

    fn make_bundle() -> ProjectSettings {
        ProjectSettings {
            reference: std::collections::BTreeMap::new(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec!["$name".to_string()],
            milestones: vec![],
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            windows: Default::default(),
            ffe: Default::default(),
            hierarchy_exclusions: vec![],
        }
    }

    fn rooms_payload(taken_at: &str) -> RoomPayload {
        RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: taken_at.to_string() },
            phase: None,
            model_to_shared: None,
            room_boundary: None,
            levels: vec![Level { id: "l1".to_string(), name: "Level 1".to_string(), elevation: 0.0 }],
            rooms: vec![Room {
                id: "r1".to_string(),
                name: "Room".to_string(),
                level_id: "l1".to_string(),
                loops: vec![],
                properties: std::collections::BTreeMap::new(),
            }],
        }
    }

    fn doors_payload(taken_at: &str) -> DoorPayload {
        DoorPayload {
            schema_version: SUPPORTED_DOOR_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: taken_at.to_string() },
            phase: None,
            model_to_shared: None,
            levels: vec![],
            doors: vec![],
        }
    }

    fn state() -> AppState {
        let registry = std::collections::HashMap::from([("p1".to_string(), make_bundle())]);
        AppState::new(Box::new(MemStore::new()), registry, None)
    }

    /// Idle reads agree, a push does not — the two halves of being a usable
    /// cursor at all. A cursor that drifted while nothing happened would make
    /// every poll a full read again and quietly undo the whole optimisation.
    #[test]
    fn test_cursor_is_stable_while_idle_and_moves_on_a_push() {
        let state = state();
        state.set_snapshot(rooms_payload("2026-01-01T00:00:00Z")).unwrap();
        let cursor = |s: &AppState| scope_cursor(s, Some("p1"), None, &[SnapshotKind::Rooms]).unwrap();

        let first = cursor(&state);
        assert_eq!(first, cursor(&state), "nothing happened between these two reads");

        state.set_snapshot(rooms_payload("2026-02-02T00:00:00Z")).unwrap();
        assert_ne!(first, cursor(&state), "a new snapshot is new content");
    }

    /// **A ROOMS push moves the DOORS cursor**, and this is the property most
    /// likely to be broken by someone tidying the kind list down to the entity
    /// the endpoint is named after. A doors response is not a function of doors:
    /// `doors::build_candidates` resolves ownership and geometry against the
    /// scope's rooms, so rooms moving underneath changes the body. Passing only
    /// `Doors` would serve that changed body a 304 forever.
    #[test]
    fn test_doors_cursor_tracks_rooms_too() {
        let state = state();
        state.set_snapshot(rooms_payload("2026-01-01T00:00:00Z")).unwrap();
        state.set_door_snapshot(doors_payload("2026-01-01T00:00:00Z")).unwrap();
        let kinds = [SnapshotKind::Rooms, SnapshotKind::Doors];

        let before = scope_cursor(&state, Some("p1"), None, &kinds).unwrap();
        state.set_snapshot(rooms_payload("2026-03-03T00:00:00Z")).unwrap();
        let after = scope_cursor(&state, Some("p1"), None, &kinds).unwrap();

        assert_ne!(before, after, "the doors body depends on the rooms it resolves against");
    }

    /// An unregistered project is invisible to every read, so its pushes must
    /// not move the cursor either — otherwise a dev seed nothing can display
    /// would invalidate every poll for the projects that can.
    #[test]
    fn test_cursor_ignores_an_unregistered_project() {
        let state = state();
        state.set_snapshot(rooms_payload("2026-01-01T00:00:00Z")).unwrap();
        let before = scope_cursor(&state, None, None, &[SnapshotKind::Rooms]).unwrap();

        let mut ghost = rooms_payload("2026-04-04T00:00:00Z");
        ghost.project = Project { id: "ghost".to_string(), name: "Unregistered".to_string() };
        state.set_snapshot(ghost).unwrap();

        assert_eq!(before, scope_cursor(&state, None, None, &[SnapshotKind::Rooms]).unwrap());
    }

    /// Scoping to a project excludes another project's pushes. Without this the
    /// cursor would be storewide, and RHH — the reason any of this exists —
    /// would invalidate House A's poll on every import.
    #[test]
    fn test_cursor_is_scoped_to_its_project() {
        let registry =
            std::collections::HashMap::from([("p1".to_string(), make_bundle()), ("p2".to_string(), make_bundle())]);
        let state = AppState::new(Box::new(MemStore::new()), registry, None);
        state.set_snapshot(rooms_payload("2026-01-01T00:00:00Z")).unwrap();
        let before = scope_cursor(&state, Some("p1"), None, &[SnapshotKind::Rooms]).unwrap();

        let mut other = rooms_payload("2026-05-05T00:00:00Z");
        other.project = Project { id: "p2".to_string(), name: "Other".to_string() };
        state.set_snapshot(other).unwrap();

        assert_eq!(before, scope_cursor(&state, Some("p1"), None, &[SnapshotKind::Rooms]).unwrap());
    }
}
