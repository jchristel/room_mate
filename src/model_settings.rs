//! A hand-off between a Revit session and a browser tab: the RLS settings of ONE model, edited on
//! a page and read back by the plug-in.
//!
//! **The project file is the source of truth; this is the form it is edited on.** The settings live
//! in the Revit model's extensible storage (`room_m.project_settings`). The plug-in cannot show a
//! grid of its own without risking Revit (WPF from IronPython crashed on large grids), so it
//! posts the model's current settings here with the choices only Revit knows (`options`: the
//! title block families, view types, tag types ...), opens `/model-settings/` in the browser, and
//! when the person has pressed Save it reads the draft back and writes it into the model.
//! Nothing here is a second copy to maintain: a draft is overwritten by the next session, and a
//! settings object this server has never seen is simply what the draft holds.
//!
//! **Opaque on purpose.** The server types neither `settings` nor `options`: the page and the
//! plug-in own their shape (it is still changing, and typing it here would pin the server to a
//! plug-in version, the reason a sheet body is opaque too). Both must be JSON objects, and a draft
//! is capped at [`MAX_BYTES`].
//!
//! **A draft is `saved` only by the page's Save.** Posting starts a fresh, unsaved draft, so a
//! plug-in that asks "is it saved?" can never be answered by an earlier session's Save.
//!
//! **Proof of concept**: keyed by project and model id (the document title), one file each under
//! `modelsettings/<project>/`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use axum::{
    extract::{Path as UrlPath, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::state::{is_path_safe_component, Shared};

const MAX_BYTES: usize = 500_000;
const MAX_MODEL_ID_CHARS: usize = 300;
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub model_id: String,
    /// The settings as the model holds them, then as the person edited them.
    pub settings: Map<String, Value>,
    /// What Revit offers to choose from. Written by the plug-in, read by the page.
    pub options: Map<String, Value>,
    pub saved: bool,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StartBody {
    pub model_id: String,
    #[serde(default)]
    pub settings: Map<String, Value>,
    #[serde(default)]
    pub options: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct SaveBody {
    pub model_id: String,
    pub settings: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct ModelQuery {
    pub model: String,
}

#[derive(Debug)]
pub enum Error {
    NotFileBacked,
    Invalid(String),
    NotFound,
    Internal(String),
}

fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

/// FNV-1a, 64 bit: a file name for a model id that may hold any character, stable across runs.
/// The id itself is stored in the file and checked on load, so a collision reads as not found.
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

fn path_of(projects_dir: &Path, project: &str, model: &str) -> Result<PathBuf, Error> {
    if !is_path_safe_component(project) {
        return Err(Error::Invalid(format!("'{project}' is not a usable project id")));
    }
    if model.trim().is_empty() || model.chars().count() > MAX_MODEL_ID_CHARS {
        return Err(Error::Invalid("the model id is empty or too long".to_string()));
    }
    Ok(projects_dir.join("modelsettings").join(project).join(format!("{:016x}.json", fnv(model))))
}

fn write(path: &Path, draft: &Draft) -> Result<(), Error> {
    let json = serde_json::to_vec_pretty(draft).map_err(|e| Error::Internal(e.to_string()))?;
    if json.len() > MAX_BYTES {
        return Err(Error::Invalid(format!("the settings are larger than {MAX_BYTES} bytes")));
    }
    std::fs::create_dir_all(path.parent().expect("a draft path has a parent"))
        .map_err(|e| Error::Internal(e.to_string()))?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &json).map_err(|e| Error::Internal(e.to_string()))?;
    std::fs::rename(&temp, path).map_err(|e| Error::Internal(e.to_string()))
}

pub fn load(projects_dir: &Path, project: &str, model: &str) -> Result<Draft, Error> {
    let path = path_of(projects_dir, project, model)?;
    let bytes = std::fs::read(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Error::NotFound,
        _ => Error::Internal(e.to_string()),
    })?;
    let draft: Draft = serde_json::from_slice(&bytes).map_err(|e| Error::Internal(e.to_string()))?;
    if draft.model_id != model {
        return Err(Error::NotFound);
    }
    Ok(draft)
}

/// A fresh, unsaved draft: what the plug-in posts when the person opens the settings.
pub fn start(projects_dir: &Path, project: &str, body: StartBody) -> Result<Draft, Error> {
    let _guard = LOCK.lock().unwrap();
    let path = path_of(projects_dir, project, &body.model_id)?;
    let draft = Draft {
        model_id: body.model_id,
        settings: body.settings,
        options: body.options,
        saved: false,
        created_at: now(),
        saved_at: None,
    };
    write(&path, &draft)?;
    Ok(draft)
}

/// The page's Save: the edited settings replace the draft's and it counts as saved. The options
/// stay as the plug-in posted them.
pub fn save(projects_dir: &Path, project: &str, body: SaveBody) -> Result<Draft, Error> {
    let _guard = LOCK.lock().unwrap();
    let mut draft = load(projects_dir, project, &body.model_id)?;
    draft.settings = body.settings;
    draft.saved = true;
    draft.saved_at = Some(now());
    write(&path_of(projects_dir, project, &body.model_id)?, &draft)?;
    Ok(draft)
}

fn to_http(e: Error) -> (StatusCode, String) {
    match e {
        Error::NotFileBacked => (
            StatusCode::SERVICE_UNAVAILABLE,
            "this server was not started from a settings directory".into(),
        ),
        Error::Invalid(m) => (StatusCode::BAD_REQUEST, m),
        Error::NotFound => (
            StatusCode::NOT_FOUND,
            "no settings draft for this model: open the settings from Revit first".into(),
        ),
        Error::Internal(m) => {
            tracing::error!("model settings error: {m}");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
        }
    }
}

fn dir(state: &Shared) -> Result<PathBuf, (StatusCode, String)> {
    state.projects_dir().cloned().ok_or(Error::NotFileBacked).map_err(to_http)
}

pub async fn http_start(
    State(state): State<Shared>,
    UrlPath(project): UrlPath<String>,
    Json(body): Json<StartBody>,
) -> Result<Json<Draft>, (StatusCode, String)> {
    start(&dir(&state)?, &project, body).map(Json).map_err(to_http)
}

pub async fn http_get(
    State(state): State<Shared>,
    UrlPath(project): UrlPath<String>,
    Query(q): Query<ModelQuery>,
) -> Result<Json<Draft>, (StatusCode, String)> {
    load(&dir(&state)?, &project, &q.model).map(Json).map_err(to_http)
}

pub async fn http_save(
    State(state): State<Shared>,
    UrlPath(project): UrlPath<String>,
    Json(body): Json<SaveBody>,
) -> Result<Json<Draft>, (StatusCode, String)> {
    save(&dir(&state)?, &project, body).map(Json).map_err(to_http)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roommate-modelsettings-{}-{}", std::process::id(), fnv(&now())));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn object(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn started(dir: &Path, model: &str) -> Draft {
        start(
            dir,
            "p",
            StartBody {
                model_id: model.into(),
                settings: object(json!({"a": 1})),
                options: object(json!({"title_blocks": ["A1"]})),
            },
        )
        .unwrap()
    }

    #[test]
    fn a_started_draft_is_unsaved_and_reads_back() {
        let dir = temp();
        let d = started(&dir, "House A");
        assert!(!d.saved);
        assert_eq!(load(&dir, "p", "House A").unwrap(), d);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_replaces_the_settings_keeps_the_options_and_marks_it_saved() {
        let dir = temp();
        started(&dir, "House A");
        let d = save(&dir, "p", SaveBody { model_id: "House A".into(), settings: object(json!({"a": 2})) }).unwrap();
        assert!(d.saved && d.saved_at.is_some());
        assert_eq!(d.settings["a"], 2);
        assert_eq!(d.options["title_blocks"][0], "A1");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn starting_again_forgets_an_earlier_save() {
        let dir = temp();
        started(&dir, "House A");
        save(&dir, "p", SaveBody { model_id: "House A".into(), settings: Map::new() }).unwrap();
        assert!(!started(&dir, "House A").saved, "an earlier session's Save must not answer this one");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn models_are_kept_apart_and_odd_ids_are_fine() {
        let dir = temp();
        started(&dir, "A/B: c*d");
        assert!(matches!(load(&dir, "p", "other"), Err(Error::NotFound)));
        assert!(load(&dir, "p", "A/B: c*d").is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_without_a_draft_and_bad_ids_are_refused() {
        let dir = temp();
        assert!(matches!(
            save(&dir, "p", SaveBody { model_id: "x".into(), settings: Map::new() }),
            Err(Error::NotFound)
        ));
        assert!(matches!(load(&dir, "../x", "m"), Err(Error::Invalid(_))));
        assert!(matches!(load(&dir, "p", " "), Err(Error::Invalid(_))));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_oversized_draft_is_refused() {
        let dir = temp();
        let big = "x".repeat(MAX_BYTES);
        let r = start(
            &dir,
            "p",
            StartBody { model_id: "m".into(), settings: object(json!({"k": big})), options: Map::new() },
        );
        assert!(matches!(r, Err(Error::Invalid(_))));
        std::fs::remove_dir_all(&dir).ok();
    }
}
