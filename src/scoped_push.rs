//! Level-scoped pushes: a producer sends one or a few LEVELS of a model and the server keeps the rest.
//!
//! **Why**: the slow half of a push is the Revit export, and most of the time only one level has
//! changed. A model block of a streamed push may carry `"scope": {"levels": ["<level id>", ...]}`:
//! the elements sent replace what the model's latest snapshot holds on those levels, and every
//! element on other levels stays. The stored result is a COMPLETE snapshot like any other, so
//! reads, milestones, comparisons and the property dictionary never learn that a push was partial.
//!
//! **How**: a middleware in front of the seven `/<entity>/stream` routes. It reads the envelope
//! line, and for each scoped model decodes the latest snapshot and appends the elements NOT on the
//! scoped levels to the request body as ordinary lines (`AppState::carried_elements`). The handler
//! behind it does not change and cannot tell: carried elements pass through the same validation,
//! the same empty-push rule and the same encoding as sent ones, so a scoped push cannot store what
//! an unscoped one would refuse. The `scope` key is removed from the envelope before the handler
//! reads it.
//!
//! **Level ids, not names.** A level id is per document and stable between pushes of one model, so
//! within a model's own history it identifies a level exactly; the name-and-elevation rule exists to
//! compare levels ACROSS models and is not needed here. A level deleted in Revit leaves its elements
//! in the snapshot until an unscoped push (or one naming it) replaces them.
//!
//! **A scoped model may send elements only on its scoped levels.** An element on any other level
//! would duplicate one carried over, so the push is refused (400, naming the element's level) as
//! soon as such a line is read rather than stored with two of an id.
//!
//! **The first push of a model cannot be scoped to anything**: with no earlier snapshot there is
//! nothing to carry, and the push is what it sent (and the usual empty-push rule applies).
//!
//! **Order and races.** Two scoped pushes to one model are each read against the snapshot that was
//! latest when they began; the one stored last wins whole, and the other's levels are replaced by
//! the older state. A producer that needs more should push unscoped. Not guarded in this version.

use std::collections::HashMap;

use axum::{
    body::{Body, Bytes},
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use tokio::io::AsyncBufReadExt;
use tokio_util::io::StreamReader;

use crate::state::{Carried, ModelKey, Shared};
use crate::storage::SnapshotKind;

/// The entity a streamed push route carries, from its path.
fn kind_of(path: &str) -> Option<SnapshotKind> {
    Some(match path {
        "/rooms/stream" => SnapshotKind::Rooms,
        "/doors/stream" => SnapshotKind::Doors,
        "/windows/stream" => SnapshotKind::Windows,
        "/ffe/stream" => SnapshotKind::Ffe,
        "/spaces/stream" => SnapshotKind::Spaces,
        "/ceilings/stream" => SnapshotKind::Ceilings,
        "/floors/stream" => SnapshotKind::Floors,
        _ => return None,
    })
}

type Refusal = (StatusCode, String);

/// What the envelope line asks for: the line without its `scope` keys, and each scoped model's levels.
struct Scoped {
    envelope: String,
    project_id: String,
    levels: HashMap<String, Vec<String>>,
}

/// Read the scopes out of the envelope line. `None` when no model is scoped.
fn scopes_of(line: &str) -> Result<Option<Scoped>, Refusal> {
    let Ok(mut envelope) = serde_json::from_str::<Value>(line) else {
        // Not ours to judge: the handler reports a bad envelope in its own words.
        return Ok(None);
    };
    let project_id = envelope.pointer("/project/id").and_then(Value::as_str).unwrap_or_default().to_string();
    let Some(models) = envelope.get_mut("models").and_then(Value::as_array_mut) else {
        return Ok(None);
    };
    let mut levels = HashMap::new();
    for model in models.iter_mut() {
        let Some(map) = model.as_object_mut() else { continue };
        let Some(scope) = map.remove("scope") else { continue };
        let id = map.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        let ids: Vec<String> = scope
            .get("levels")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        let declared = scope.get("levels").and_then(Value::as_array).map_or(0, Vec::len);
        if id.is_empty() || ids.is_empty() || ids.len() != declared || ids.iter().any(|l| l.trim().is_empty()) {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("model '{id}': a scope needs a non-empty list of level ids, as strings"),
            ));
        }
        levels.insert(id, ids);
    }
    if levels.is_empty() {
        return Ok(None);
    }
    let envelope = serde_json::to_string(&envelope).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Some(Scoped { envelope, project_id, levels }))
}

#[derive(Deserialize)]
struct Probe {
    model_id: String,
    level_id: String,
}

/// A line of a scoped model on a level outside its scope: the refusal, else `Ok`. A line that does
/// not parse this far is left to the handler, which says what is wrong with it.
fn check_line(line: &str, scopes: &HashMap<String, Vec<String>>) -> Result<(), String> {
    if scopes.is_empty() || line.trim().is_empty() {
        return Ok(());
    }
    let Ok(probe) = serde_json::from_str::<Probe>(line) else {
        return Ok(());
    };
    match scopes.get(&probe.model_id) {
        Some(levels) if !levels.contains(&probe.level_id) => Err(format!(
            "model '{}': an element is on level '{}', which is not in the push's scope ({}); \
             it would duplicate one carried over from the earlier snapshot",
            probe.model_id,
            probe.level_id,
            levels.join(", ")
        )),
        _ => Ok(()),
    }
}

fn refuse(r: Refusal) -> Response {
    r.into_response()
}

/// The middleware: scoped pushes get the rest of their model carried over, others pass untouched.
pub async fn scope_stream(State(state): State<Shared>, req: Request, next: Next) -> Response {
    let Some(kind) = kind_of(req.uri().path()).filter(|_| req.method() == axum::http::Method::POST) else {
        return next.run(req).await;
    };
    let (parts, body) = req.into_parts();
    let mut reader = StreamReader::new(body.into_data_stream().map(|r| r.map_err(std::io::Error::other)));
    let mut first = String::new();
    if let Err(e) = reader.read_line(&mut first).await {
        return refuse((StatusCode::BAD_REQUEST, format!("read error: {e}")));
    }
    let scoped = match scopes_of(&first) {
        Ok(s) => s,
        Err(r) => return refuse(r),
    };

    let (envelope, levels, carried) = match scoped {
        None => (first, HashMap::new(), Vec::new()),
        Some(s) => {
            let mut carried: Vec<Carried> = Vec::new();
            for (model_id, levels) in &s.levels {
                let key = ModelKey { project_id: s.project_id.clone(), model_id: model_id.clone() };
                match state.carried_elements(kind, &key, levels) {
                    Ok(Some(c)) => {
                        tracing::info!(
                            "scoped {} push for {}/{}: replacing level(s) {}, carrying over {} of {} element(s)",
                            kind.label(),
                            key.project_id,
                            key.model_id,
                            levels.join(", "),
                            c.kept,
                            c.total
                        );
                        carried.push(c);
                    }
                    Ok(None) => tracing::info!(
                        "scoped {} push for {}/{}: no earlier snapshot, so nothing to carry over",
                        kind.label(),
                        key.project_id,
                        key.model_id
                    ),
                    Err(e) => {
                        tracing::error!("could not read the earlier snapshot for a scoped push: {e:#}");
                        return refuse((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("could not read the earlier snapshot: {e}"),
                        ));
                    }
                }
            }
            (format!("{}\n", s.envelope), s.levels, carried)
        }
    };

    let envelope_line = futures_util::stream::once(async move { Ok::<Bytes, std::io::Error>(Bytes::from(envelope)) });
    let sent = futures_util::stream::unfold((reader, levels), |(mut reader, levels)| async move {
        let mut line = String::new();
        match reader.read_line(&mut line).await {
            Ok(0) => None,
            Ok(_) => {
                if let Err(message) = check_line(&line, &levels) {
                    return Some((Err(std::io::Error::other(message)), (reader, levels)));
                }
                if !line.ends_with('\n') {
                    line.push('\n');
                }
                Some((Ok(Bytes::from(line)), (reader, levels)))
            }
            Err(e) => Some((Err(e), (reader, levels))),
        }
    });
    let carry = futures_util::stream::iter(carried.into_iter().flatten())
        .map(|line| line.map(Bytes::from).map_err(std::io::Error::other));
    let body = Body::from_stream(envelope_line.chain(sent).chain(carry));
    next.run(Request::from_parts(parts, body)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn envelope(scope: Value) -> String {
        json!({
            "schema_version": 1, "project": {"id": "p"},
            "models": [{"id": "m", "name": "M", "scope": scope}, {"id": "n", "name": "N"}]
        })
        .to_string()
    }

    #[test]
    fn a_scope_is_read_and_removed_from_the_envelope() {
        let s = scopes_of(&envelope(json!({"levels": ["1", "2"]}))).unwrap().unwrap();
        assert_eq!(s.levels["m"], vec!["1", "2"]);
        assert_eq!(s.project_id, "p");
        assert!(!s.envelope.contains("scope"));
        assert!(s.envelope.contains("\"id\":\"n\""));
    }

    #[test]
    fn no_scope_is_no_change() {
        let line = json!({"schema_version": 1, "project": {"id": "p"}, "models": [{"id": "m"}]}).to_string();
        assert!(scopes_of(&line).unwrap().is_none());
        assert!(scopes_of("{not json").unwrap().is_none(), "the handler reports a bad envelope itself");
    }

    #[test]
    fn a_scope_that_names_no_level_or_a_blank_one_is_refused() {
        for bad in [
            json!({"levels": []}),
            json!({}),
            json!({"levels": [5]}),
            json!({"levels": ["  "]}),
            json!("x"),
        ] {
            assert!(scopes_of(&envelope(bad.clone())).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_element_outside_the_scope_is_refused_and_another_model_is_not_judged() {
        let scopes = HashMap::from([("m".to_string(), vec!["1".to_string()])]);
        assert!(check_line(r#"{"model_id":"m","level_id":"1","id":"a"}"#, &scopes).is_ok());
        let err = check_line(r#"{"model_id":"m","level_id":"2","id":"a"}"#, &scopes).unwrap_err();
        assert!(err.contains("level '2'") && err.contains("duplicate"));
        assert!(
            check_line(r#"{"model_id":"n","level_id":"2","id":"a"}"#, &scopes).is_ok(),
            "model n is not scoped"
        );
        assert!(check_line("", &scopes).is_ok());
        assert!(check_line("{not json", &scopes).is_ok(), "the handler says what is wrong with it");
    }

    #[test]
    fn only_the_stream_routes_are_ours() {
        assert_eq!(kind_of("/rooms/stream"), Some(SnapshotKind::Rooms));
        assert_eq!(kind_of("/floors/stream"), Some(SnapshotKind::Floors));
        assert_eq!(kind_of("/rooms"), None);
        assert_eq!(kind_of("/projects/p/rooms/stream"), None);
    }
}
