//! What a store's error means over HTTP, said once.
//!
//! The authored-data stores (blueprints, sheets, tag rules, tasks, connections, saved reports) each
//! have their own error enum with the same handful of outcomes, and each HTTP adapter used to carry
//! a copy of the same `match` to turn it into a status and a message, and a copy of "give me the
//! projects directory or say the server has none". A policy that lives in six copies is changed in
//! five of them. Now a store says what its error IS (`StoreError::failure`) and this module says what
//! that looks like on the wire.
//!
//! **Deliberately not here: `settings_api`.** Its statuses differ on purpose (a missing directory is a
//! 404 there, an invalid save a 422, and an internal error shows its text, since the settings page is
//! how a person finds out what is wrong with the file they hand-edit), so it keeps its own mapping.

use std::path::PathBuf;

use axum::http::StatusCode;

use crate::blueprints::BlueprintError;
use crate::connections::ConnectionsError;
use crate::reports_api::ReportsError;
use crate::sheets::SheetError;
use crate::state::Shared;
use crate::tag_rules::TagRuleError;
use crate::tasks::TaskError;

/// A status and the text for the body: the error type every adapter's handlers return.
pub type HttpError = (StatusCode, String);

/// The outcomes every store reports. A store with no such outcome (tag rules have no conflict) simply
/// never produces it.
pub enum Failure {
    /// The server was not started from a settings directory (in-memory tests, a bare binary).
    NotFileBacked,
    Invalid(String),
    NotFound(String),
    /// The document moved since the caller read it.
    Conflict(String),
    Internal(anyhow::Error),
}

/// A store's error, as one of the shared outcomes. `LABEL` names the store in the log line of an
/// internal error, which is all a caller ever learns of one (the body says only "internal error").
pub trait StoreError {
    const LABEL: &'static str;
    fn failure(self) -> Failure;
}

/// The status and message for a store's error.
pub fn to_http<E: StoreError>(err: E) -> HttpError {
    match err.failure() {
        Failure::NotFileBacked => (
            StatusCode::SERVICE_UNAVAILABLE,
            "this server was not started from a settings directory".to_string(),
        ),
        Failure::Invalid(msg) => (StatusCode::BAD_REQUEST, msg),
        Failure::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
        Failure::Conflict(msg) => (StatusCode::CONFLICT, msg),
        Failure::Internal(e) => {
            tracing::error!("{} error: {e:#}", E::LABEL);
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
        }
    }
}

/// The directory authored documents live in, or the 503 every adapter answers when there is none.
pub fn projects_dir(state: &Shared) -> Result<PathBuf, HttpError> {
    state.projects_dir().cloned().ok_or_else(|| to_http(Failure::NotFileBacked))
}

/// A `Failure` is itself a store error, so "no directory" is built the same way a store would say it.
impl StoreError for Failure {
    const LABEL: &'static str = "server";
    fn failure(self) -> Failure {
        self
    }
}

impl StoreError for BlueprintError {
    const LABEL: &'static str = "blueprints";
    fn failure(self) -> Failure {
        match self {
            Self::NotFileBacked => Failure::NotFileBacked,
            Self::Invalid(m) => Failure::Invalid(m),
            Self::NotFound(m) => Failure::NotFound(m),
            Self::Conflict(m) => Failure::Conflict(m),
            Self::Internal(e) => Failure::Internal(e),
        }
    }
}

impl StoreError for SheetError {
    const LABEL: &'static str = "sheets";
    fn failure(self) -> Failure {
        match self {
            Self::NotFileBacked => Failure::NotFileBacked,
            Self::Invalid(m) => Failure::Invalid(m),
            Self::NotFound(m) => Failure::NotFound(m),
            Self::Conflict(m) => Failure::Conflict(m),
            Self::Internal(e) => Failure::Internal(e),
        }
    }
}

impl StoreError for TagRuleError {
    const LABEL: &'static str = "tag rules";
    fn failure(self) -> Failure {
        match self {
            Self::NotFileBacked => Failure::NotFileBacked,
            Self::Invalid(m) => Failure::Invalid(m),
            Self::NotFound(m) => Failure::NotFound(m),
            Self::Internal(e) => Failure::Internal(e),
        }
    }
}

impl StoreError for TaskError {
    const LABEL: &'static str = "tasks";
    fn failure(self) -> Failure {
        match self {
            Self::NotFileBacked => Failure::NotFileBacked,
            Self::Invalid(m) => Failure::Invalid(m),
            Self::NotFound(m) => Failure::NotFound(m),
            Self::Conflict(m) => Failure::Conflict(m),
            Self::Internal(e) => Failure::Internal(e),
        }
    }
}

impl StoreError for ConnectionsError {
    const LABEL: &'static str = "connections";
    fn failure(self) -> Failure {
        match self {
            Self::NotFileBacked => Failure::NotFileBacked,
            Self::Invalid(m) => Failure::Invalid(m),
            Self::Conflict(m) => Failure::Conflict(m),
            Self::Internal(e) => Failure::Internal(e),
        }
    }
}

impl StoreError for ReportsError {
    const LABEL: &'static str = "saved reports";
    fn failure(self) -> Failure {
        match self {
            Self::NotFileBacked => Failure::NotFileBacked,
            Self::Invalid(m) => Failure::Invalid(m),
            Self::NotFound(m) => Failure::NotFound(m),
            Self::Internal(e) => Failure::Internal(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use crate::storage::MemStore;

    #[test]
    fn test_every_store_reads_the_same_outcome_the_same_way() {
        let invalid = (StatusCode::BAD_REQUEST, "bad".to_string());
        assert_eq!(to_http(BlueprintError::Invalid("bad".into())), invalid);
        assert_eq!(to_http(SheetError::Invalid("bad".into())), invalid);
        assert_eq!(to_http(TagRuleError::Invalid("bad".into())), invalid);
        assert_eq!(to_http(TaskError::Invalid("bad".into())), invalid);
        assert_eq!(to_http(ConnectionsError::Invalid("bad".into())), invalid);
        assert_eq!(to_http(ReportsError::Invalid("bad".into())), invalid);
        let gone = (StatusCode::NOT_FOUND, "gone".to_string());
        assert_eq!(to_http(BlueprintError::NotFound("gone".into())), gone);
        assert_eq!(to_http(SheetError::NotFound("gone".into())), gone);
        assert_eq!(to_http(TaskError::NotFound("gone".into())), gone);
        assert_eq!(to_http(ReportsError::NotFound("gone".into())), gone);
        let moved = (StatusCode::CONFLICT, "moved".to_string());
        assert_eq!(to_http(BlueprintError::Conflict("moved".into())), moved);
        assert_eq!(to_http(SheetError::Conflict("moved".into())), moved);
        assert_eq!(to_http(TaskError::Conflict("moved".into())), moved);
        assert_eq!(to_http(ConnectionsError::Conflict("moved".into())), moved);
    }

    #[test]
    fn test_an_internal_error_shows_nothing_of_itself_and_no_directory_is_a_503() {
        let (status, body) = to_http(SheetError::Internal(anyhow::anyhow!("disk on fire at /secret/path")));
        assert_eq!((status, body.as_str()), (StatusCode::INTERNAL_SERVER_ERROR, "internal error"));
        let (status, body) = to_http(TagRuleError::NotFileBacked);
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(body.contains("settings directory"));

        let bare: Shared = std::sync::Arc::new(AppState::new(Box::new(MemStore::new()), Default::default(), None));
        assert_eq!(projects_dir(&bare).unwrap_err().0, StatusCode::SERVICE_UNAVAILABLE);
        let dir = std::env::temp_dir();
        let backed: Shared = std::sync::Arc::new(
            AppState::new(Box::new(MemStore::new()), Default::default(), None).with_projects_dir(dir.clone()),
        );
        assert_eq!(projects_dir(&backed).unwrap(), dir);
    }
}
