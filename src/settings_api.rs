//! Project-settings read/save API: the machinery behind the settings UI
//! (`static/settings.html`).
//!
//! Layout mirrors the codebase's handler/service split inside one module: a
//! transport-agnostic core at the top (plain functions over `projects_dir`,
//! typed `SettingsError` results), thin Axum adapters at the bottom. The MCP
//! binary reuses the core's *read* functions for its `list_project_settings`
//! / `get_project_settings` tools; **writes stay HTTP-only** — the MCP server
//! is a separate process, so a write from it could not hot-swap this
//! process's registry (instant split-brain), and mutation stays behind the
//! human UI per mcp.rs's read-only contract.
//!
//! The TOML files remain the single source of truth: reads parse them fresh
//! per call (no filename bookkeeping in `AppState`), and a save validates the
//! candidate through the exact startup pipeline (`bootstrap::load_project_bundle`)
//! before installing the file and hot-swapping the registry — a file this API
//! accepts can never fail the next boot. Access control is the server's
//! `127.0.0.1` bind, same trust model as ingest.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use axum::{
    extract::{Path as UrlPath, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::bootstrap::{load_project_bundle, load_project_settings_dir};
use crate::contract::{ensure_taken_at, validate_snapshot_id, Snapshot};
use crate::reference::load_reference_from_bytes;
use crate::settings::{validate_reference_fields, ReferenceOrigin, Settings};
use crate::state::{is_path_safe_component, AppState, SettingsRegistry, Shared};

/// One project-settings file as the UI's list sees it. A file that fails to
/// parse still gets a row (with `error` set) rather than breaking the whole
/// list — the settings UI is exactly the tool you'd reach for to notice a
/// rotten file, so it must stay usable when one exists.
#[derive(Serialize)]
pub struct ProjectFileSummary {
    /// File name within the projects dir (not a full path).
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// The project's display name, absent when the file sets none — consumers
    /// fall back to `project_id`. Carried on the summary so the pyRevit push
    /// picker can both label a project and send `project.name` from one call
    /// (see room_mate's `fetch_projects`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub is_default: bool,
    /// Every reference source this file declares, by name — the list form of
    /// what `drofus_configured: bool` used to answer for one hardcoded source.
    /// A bool could only ever say "is dRofus there", which was already the
    /// wrong question once a project could configure several.
    pub reference_sources: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Typed failure for the core functions; each transport maps it itself
/// (HTTP below, MCP in `bin/mcp.rs`) — same seam discipline as `ServiceError`.
#[derive(Debug)]
pub enum SettingsError {
    /// The state wasn't built from a settings directory (in-memory tests).
    NotFileBacked,
    /// No settings file exists for the requested project id.
    NotFound(String),
    /// A create collides with an existing project id or file.
    Conflict(String),
    /// The candidate settings failed validation — message is the same loud
    /// text startup would print for the same mistake.
    Invalid(String),
    Internal(anyhow::Error),
}

/// Serialises every save end-to-end. Two concurrent admin saves are a
/// non-case in practice, but the lock makes the scan-then-write race
/// structurally impossible rather than merely unlikely.
static SAVE_LOCK: Mutex<()> = Mutex::new(());

/// Parse one settings file RAW — `toml::from_str`, no relative-path
/// resolution — so a dRofus path round-trips exactly as authored. (The
/// resolving parse, `load_settings`, is for *running* against a file; this
/// one is for *editing* it.)
fn read_raw(path: &Path) -> anyhow::Result<Settings> {
    let raw = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&raw)?)
}

/// Every `*.toml` directly in the projects dir, sorted by file name.
fn settings_files(projects_dir: &Path) -> Result<Vec<PathBuf>, SettingsError> {
    let entries = std::fs::read_dir(projects_dir)
        .map_err(|e| SettingsError::Internal(anyhow::anyhow!("could not read {}: {e}", projects_dir.display())))?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("toml"))
        .collect();
    files.sort();
    Ok(files)
}

fn file_name(path: &Path) -> String {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string()
}

/// List every project-settings file with its headline facts (or its parse
/// error). Shared by `GET /api/settings/projects` and the MCP
/// `list_project_settings` tool.
pub fn list_project_files(projects_dir: &Path) -> Result<Vec<ProjectFileSummary>, SettingsError> {
    let mut out = Vec::new();
    for path in settings_files(projects_dir)? {
        match read_raw(&path) {
            Ok(settings) => out.push(ProjectFileSummary {
                file: file_name(&path),
                project_id: Some(settings.project_id),
                name: settings.name,
                is_default: settings.is_default,
                reference_sources: settings.sources.reference.keys().cloned().collect(),
                error: None,
            }),
            Err(e) => out.push(ProjectFileSummary {
                file: file_name(&path),
                project_id: None,
                name: None,
                is_default: false,
                reference_sources: vec![],
                error: Some(format!("{e:#}")),
            }),
        }
    }
    Ok(out)
}

/// Find and parse the settings file whose `project_id` matches. Shared by
/// `GET /api/settings/projects/{id}` and the MCP `get_project_settings` tool.
/// Returns the file name alongside so the UI can show where the project lives.
pub fn get_project_file(projects_dir: &Path, project_id: &str) -> Result<(String, Settings), SettingsError> {
    for path in settings_files(projects_dir)? {
        if let Ok(settings) = read_raw(&path)
            && settings.project_id == project_id
        {
            return Ok((file_name(&path), settings));
        }
    }
    Err(SettingsError::NotFound(format!("no settings file declares project_id '{project_id}'")))
}

/// Like `get_project_file`, but with the default-fallback semantics of
/// `SettingsRegistry::settings_for`: when no file's `project_id` matches
/// exactly, fall back to the file marked `is_default = true`.
///
/// The viewer resolves colour plans by the *payload* project id (e.g.
/// `"130486"`), which is not a settings `project_id` (e.g. `"Riverside ..."`),
/// so the exact match 404s and it needs this fallback. The editors
/// (`settings.html`, `comparison.html`) must NOT get the fallback — they GET and
/// PUT the same path by the real `project_id`, and a silent default-fallback
/// could load or overwrite the wrong file — which is why this is a separate
/// function feeding a separate route rather than a change to `get_project_file`.
/// Returns the parsed `Settings` (so `colour_plans` ride along) and its file.
pub fn resolve_project_file(projects_dir: &Path, project_id: &str) -> Result<(String, Settings), SettingsError> {
    if let Ok(found) = get_project_file(projects_dir, project_id) {
        return Ok(found);
    }
    for path in settings_files(projects_dir)? {
        if let Ok(settings) = read_raw(&path)
            && settings.is_default
        {
            return Ok((file_name(&path), settings));
        }
    }
    Err(SettingsError::NotFound(format!(
        "no settings file declares project_id '{project_id}' and none is marked is_default"
    )))
}

/// Render `settings` as the file to write, carrying the previous file's
/// comments across.
///
/// A save regenerates the whole project file from the typed `Settings`, and a
/// generated file has no comments in it. For these files the comments are most
/// of the content — why a door key is `$id` and not `Mark`, which of two
/// measured options a setting records, what a tier would break if it were keyed
/// on the code instead of the name. None of it is recoverable from the values,
/// none of it is written down anywhere else, and losing it to an unrelated edit
/// is the same class of loss `merge_over_stored` fixes one layer down.
///
/// **Content and formatting come from `toml::to_string_pretty`; only decor
/// comes from the old file.** The generated text is reparsed and decorated
/// rather than being built through `toml_edit`'s own serializer, which keeps
/// exactly one thing deciding what a settings file looks like — the same call,
/// still guarded by `test_toml_serializer_hoists_values_above_tables`. The
/// worst case here is a comment dropped or misplaced; a *value* cannot come out
/// different, because no value passes through this.
///
/// `previous` absent — a create, or a file too broken to parse — simply means
/// nothing to carry.
fn render_settings(settings: &Settings, previous: Option<&str>) -> Result<String, SettingsError> {
    let generated = toml::to_string_pretty(settings)
        .map_err(|e| SettingsError::Invalid(format!("settings do not serialize to TOML: {e}")))?;

    let (Some(previous), Ok(mut next)) = (previous, generated.parse::<toml_edit::DocumentMut>()) else {
        return Ok(generated);
    };
    let Ok(previous) = previous.parse::<toml_edit::DocumentMut>() else {
        return Ok(generated);
    };

    carry_comments(previous.as_table(), next.as_table_mut());
    next.set_trailing(previous.trailing().clone());
    Ok(next.to_string())
}

/// Copy one document's comments onto a freshly generated one, matched by key
/// path. A key the new document no longer has loses its comment along with the
/// setting it described; a key the old document never had gets none. Both are
/// the right answer, which is why neither is special-cased.
fn carry_comments(previous: &toml_edit::Table, next: &mut toml_edit::Table) {
    for (mut key, item) in next.iter_mut() {
        let name = key.get().to_string();
        let Some(old_item) = previous.get(&name) else { continue };

        // A scalar's comment block sits on its KEY; a `[table]`'s sits on the
        // table header, handled in `carry_table_comments`. A key is only ever
        // one of the two, so copying both is not double-counting.
        if let Some(old_key) = previous.key(&name)
            && let Some(prefix) = old_key.leaf_decor().prefix()
        {
            key.leaf_decor_mut().set_prefix(prefix.clone());
        }

        match (old_item, item) {
            (toml_edit::Item::Table(old), toml_edit::Item::Table(new)) => carry_table_comments(old, new),
            (toml_edit::Item::ArrayOfTables(old), toml_edit::Item::ArrayOfTables(new)) => {
                for (index, entry) in new.iter_mut().enumerate() {
                    let Some(old_entry) = matching_entry(old, entry, index) else {
                        continue;
                    };
                    carry_table_comments(old_entry, entry);
                }
            }
            _ => {}
        }
    }
}

fn carry_table_comments(previous: &toml_edit::Table, next: &mut toml_edit::Table) {
    if let Some(prefix) = previous.decor().prefix() {
        next.decor_mut().set_prefix(prefix.clone());
    }
    carry_comments(previous, next);
}

/// Which entry of the previous array-of-tables described this one.
///
/// By **identity** wherever the entries carry one — `name` for a hierarchy
/// tier, milestone or colour plan, `canonical` for a builtin property, `label`
/// for a reference field — and by position only for entries that carry none.
/// Position alone breaks the first time somebody reorders tiers in the UI: the
/// comments would stay where they were and start describing their new
/// neighbours, which is worse than losing them. An identified entry with no
/// match in the old document is new, and correctly gets nothing.
fn matching_entry<'a>(
    previous: &'a toml_edit::ArrayOfTables,
    entry: &toml_edit::Table,
    index: usize,
) -> Option<&'a toml_edit::Table> {
    for field in ["name", "canonical", "label"] {
        if let Some(id) = entry.get(field).and_then(toml_edit::Item::as_str) {
            return previous.iter().find(|old| old.get(field).and_then(toml_edit::Item::as_str) == Some(id));
        }
    }
    previous.get(index)
}

/// Fill a partial update from the file it is about to replace: any top-level
/// key the request did not send keeps the value already on disk.
///
/// **This exists because a save is a whole-file write and the settings page
/// only edits part of a project.** `static/settings.html` has no controls for
/// `[doors]`, `[areas]` or `hierarchy_exclusions` and never sends them, and
/// all three are `#[serde(default, skip_serializing_if = ...)]` — so an unsent
/// section did not stay unedited, it was **deleted**, silently, by editing
/// something unrelated. RHH lost `[doors] room_resolution = "same_model"` that
/// way and put 2084 doors back to homeless; nothing anywhere said so.
///
/// The merge is deliberately **top-level only**. A deep merge would resurrect
/// what the client meant to remove — a source dropped from `sources`, a tier
/// dropped from `hierarchy` — because "absent from a list the client sent" and
/// "absent from the request" would stop being distinguishable. At this depth
/// they still are: a key the client sent wins whole, a key it omitted is
/// inherited whole.
///
/// It runs on the JSON, before typing, because that is the last point where
/// the distinction exists at all. Once the body is a `Settings`,
/// `#[serde(default)]` has already turned "not sent" into "the default value"
/// and no code downstream can tell those apart.
///
/// Update only. A create has no stored file to inherit from, and giving one a
/// silent parent would be a different feature.
fn merge_over_stored(
    projects_dir: &Path,
    project_id: &str,
    incoming: serde_json::Value,
) -> Result<Settings, SettingsError> {
    let serde_json::Value::Object(mut merged) = incoming else {
        return Err(SettingsError::Invalid("settings body must be a JSON object".to_string()));
    };

    // A project this lookup cannot find has nothing to carry forward. The
    // error is not raised here: `save_project` owns the update path's 404 and
    // its wording, and raising a second one would fork that message.
    if let Ok((_, stored)) = get_project_file(projects_dir, project_id) {
        let stored = serde_json::to_value(&stored)
            .map_err(|e| SettingsError::Internal(anyhow::anyhow!("could not re-encode stored settings: {e}")))?;
        if let serde_json::Value::Object(stored) = stored {
            for (key, value) in stored {
                merged.entry(key).or_insert(value);
            }
        }
    }

    serde_json::from_value(serde_json::Value::Object(merged))
        .map_err(|e| SettingsError::Invalid(format!("settings body is not valid project settings: {e}")))
}

/// Save one project's settings: validate through the startup pipeline, write
/// the file atomically, hot-swap the running registry. `existing_id` is
/// `Some` for an update (`PUT`) and `None` for a create (`POST`).
///
/// Takes a whole `Settings` and writes a whole file. An update that means to
/// preserve what it does not mention must go through `merge_over_stored`
/// first — see that function for why the merge cannot live in here.
///
/// Ordering is deliberate: nothing is installed until the candidate passed
/// the exact validation startup runs, and the registry is only swapped from a
/// full, successful re-load of the whole directory — the running server can
/// never observe a half-updated state, and a file this function accepts can
/// never fail the next boot.
pub fn save_project(
    state: &AppState,
    existing_id: Option<&str>,
    settings: Settings,
) -> Result<Settings, SettingsError> {
    let projects_dir = state.projects_dir().ok_or(SettingsError::NotFileBacked)?.clone();
    let _guard = SAVE_LOCK.lock().unwrap();

    let id = settings.project_id.clone();
    if !is_path_safe_component(&id) {
        return Err(SettingsError::Invalid(format!(
            "project_id {id:?} is empty or contains characters unsafe for file names"
        )));
    }

    // Resolve the target file. Update: the file that currently declares this
    // id (the id is the identity — renaming is a new project, not an edit).
    // Create: a fresh `<id>.toml`, rejecting a collision with any existing
    // declaration or file.
    let target = match existing_id {
        Some(existing) => {
            if existing != id {
                return Err(SettingsError::Invalid(format!(
                    "project_id cannot change ('{existing}' -> '{id}'): the id is the project's identity — create a new project instead"
                )));
            }
            let (file, _) = get_project_file(&projects_dir, existing)?;
            projects_dir.join(file)
        }
        None => {
            if let Ok((file, _)) = get_project_file(&projects_dir, &id) {
                return Err(SettingsError::Conflict(format!("project '{id}' already exists (in {file})")));
            }
            let target = projects_dir.join(format!("{id}.toml"));
            if target.exists() {
                return Err(SettingsError::Conflict(format!(
                    "file {} already exists but declares a different project",
                    file_name(&target)
                )));
            }
            target
        }
    };

    // Cross-file checks against every OTHER file: a second `is_default` is
    // the same startup-loud error `load_project_settings_dir` raises.
    // (Duplicate project_id is already excluded by the create path above and
    // impossible on update, where the id equals the target file's own.)
    if settings.is_default {
        for path in settings_files(&projects_dir)? {
            if path == target {
                continue;
            }
            if let Ok(other) = read_raw(&path)
                && other.is_default
            {
                return Err(SettingsError::Invalid(format!(
                    "another settings file already sets is_default = true: {} ('{}')",
                    file_name(&path),
                    other.project_id
                )));
            }
        }
    }

    // Serialize and stage the candidate as a temp file IN the projects dir
    // (so relative dRofus paths resolve exactly as they will at startup) with
    // a non-.toml extension (so a crash mid-save can't leave a file the next
    // startup scan would pick up).
    // The file being replaced is read for its comments only — see
    // `render_settings`. A create has no such file and reads `None`.
    let previous = std::fs::read_to_string(&target).ok();
    let toml_text = render_settings(&settings, previous.as_deref())?;
    let temp = projects_dir.join(format!(".{id}.candidate.tmp"));
    std::fs::write(&temp, &toml_text)
        .map_err(|e| SettingsError::Internal(anyhow::anyhow!("could not write candidate file: {e}")))?;

    // Full standalone validation — the same pipeline startup runs, so the
    // rejection message is the same loud text a bad boot would print. The
    // store is passed through because an `upload`-sourced project's dRofus
    // labels live in its latest stored CSV, not in any file path.
    if let Err(e) = load_project_bundle(&temp, state.store()) {
        std::fs::remove_file(&temp).ok();
        return Err(SettingsError::Invalid(format!("{e:#}")));
    }

    // Atomic install (std::fs::rename replaces an existing target on both
    // Unix and Windows), then rebuild the registry from the whole directory
    // and swap it in.
    std::fs::rename(&temp, &target)
        .map_err(|e| SettingsError::Internal(anyhow::anyhow!("could not install settings file: {e}")))?;

    reload_and_swap(state, &projects_dir)?;
    tracing::info!("settings saved and applied: {} ({})", id, file_name(&target));
    Ok(settings)
}

/// Rebuild the registry from the whole projects directory and swap it in —
/// the hot-reload tail shared by `save_project` and `upload_reference`, so the
/// two mutating paths can never diverge on how a registry is rebuilt. A
/// reload failure here means some file rotted underneath us — surface it
/// loudly and keep serving the old registry.
fn reload_and_swap(state: &AppState, projects_dir: &Path) -> Result<(), SettingsError> {
    match load_project_settings_dir(projects_dir, state.store()) {
        Ok((by_project, default)) => {
            state.swap_registry(SettingsRegistry { by_project, default });
            Ok(())
        }
        Err(e) => Err(SettingsError::Internal(anyhow::anyhow!(
            "change installed, but reloading the settings directory failed (another file may be broken): {e:#} — \
             the running server keeps its previous settings until this is fixed"
        ))),
    }
}

/// Result of one dRofus CSV upload, echoed to the uploader. Carries the
/// resolved snapshot id (minted when the request supplied none — same
/// contract as rooms ingest) and the parsed CSV's headline facts so the
/// settings UI can refresh its label dropdowns without a second call.
#[derive(Debug, Serialize)]
pub struct ReferenceUploadResult {
    pub accepted: bool,
    /// False when a dRofus snapshot with this `taken_at` already existed —
    /// the upload was skipped (never overwritten), same duplicate rule as
    /// rooms ingest.
    pub stored: bool,
    /// Records actually stored — **the deduplicated count**, which is why the
    /// two fields below matter: a 200-row CSV with five repeated ids reports
    /// 195 here, and without them nothing would say where the other five went.
    pub record_count: usize,
    /// Ids the CSV repeated; only the last row of each survived. Reported at
    /// upload because that is the moment the operator is looking at the file.
    pub duplicate_ids: Vec<String>,
    /// Rows whose id cell was empty, skipped by the loader.
    pub blank_id_rows: usize,
    pub link_property: String,
    pub labels: Vec<String>,
    pub snapshot_taken_at: String,
    pub snapshot_id_generated: bool,
}

/// Store one uploaded CSV against a project's named reference source and
/// hot-swap it into the running registry.
///
/// Ordering is load-bearing: the CSV is parsed and validated against that
/// source's declared fields BEFORE anything is stored — a stored CSV is
/// hydrated at every boot, so accepting a bad one here would fail the next
/// startup of both binaries. Everything runs under `SAVE_LOCK` so an upload
/// can never race a settings save's own scan-then-swap.
pub fn upload_reference(
    state: &AppState,
    project_id: &str,
    source: &str,
    taken_at: Option<&str>,
    csv: &[u8],
) -> Result<ReferenceUploadResult, SettingsError> {
    let projects_dir = state.projects_dir().ok_or(SettingsError::NotFileBacked)?.clone();
    let _guard = SAVE_LOCK.lock().unwrap();

    // Resolve the snapshot id through the shared contract functions — minted
    // when absent, validated always, echoed back either way.
    let mut snapshot = Snapshot { taken_at: taken_at.unwrap_or_default().to_string() };
    let snapshot_id_generated = ensure_taken_at(&mut snapshot);
    validate_snapshot_id(&snapshot.taken_at).map_err(SettingsError::Invalid)?;

    // The target project must exist and declare the upload source — an
    // upload against a `file`-sourced (or unconfigured) source would store
    // data nothing ever reads.
    let (_, settings) = get_project_file(&projects_dir, project_id)?;
    let reference_source = settings.sources.reference.get(source);
    match reference_source.map(|s| &s.origin) {
        Some(ReferenceOrigin::Upload) => {}
        _ => {
            return Err(SettingsError::Invalid(format!(
                "project '{project_id}' does not declare [sources.reference.{source}] type = \"upload\" — \
                 set that source's type to \"upload\" in its settings first"
            )));
        }
    }

    // Parse + validate before storing (see the doc comment).
    let data = load_reference_from_bytes(csv).map_err(|e| SettingsError::Invalid(format!("{e:#}")))?;
    let fields = reference_source.map(|s| s.fields.clone()).unwrap_or_default();
    validate_reference_fields(&fields, &data.all_labels).map_err(|e| SettingsError::Invalid(format!("{e:#}")))?;

    let stored = state
        .put_reference(project_id, source, &snapshot.taken_at, csv)
        .map_err(SettingsError::Internal)?;

    // Rebuild + swap so the upload is live without a restart. The bundle
    // re-hydrates from the store's *latest* — a backfilled older `taken_at`
    // correctly does not displace a newer one.
    reload_and_swap(state, &projects_dir)?;
    tracing::info!(
        "'{}' upload applied: {} @ {} ({} record(s))",
        source,
        project_id,
        snapshot.taken_at,
        data.by_id.len()
    );

    Ok(ReferenceUploadResult {
        accepted: true,
        stored,
        record_count: data.by_id.len(),
        duplicate_ids: data.duplicate_ids,
        blank_id_rows: data.blank_id_rows,
        link_property: data.link_property,
        labels: data.all_labels,
        snapshot_taken_at: snapshot.taken_at,
        snapshot_id_generated,
    })
}

// ---------- Axum adapters ----------

fn to_http(err: SettingsError) -> (StatusCode, String) {
    match err {
        SettingsError::NotFileBacked => (
            StatusCode::NOT_FOUND,
            "this server has no --project-settings directory (settings editing unavailable)".to_string(),
        ),
        SettingsError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
        SettingsError::Conflict(msg) => (StatusCode::CONFLICT, msg),
        SettingsError::Invalid(msg) => (StatusCode::UNPROCESSABLE_ENTITY, msg),
        SettingsError::Internal(e) => {
            tracing::error!("settings API internal error: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}

fn require_dir(state: &AppState) -> Result<PathBuf, (StatusCode, String)> {
    state.projects_dir().cloned().ok_or_else(|| to_http(SettingsError::NotFileBacked))
}

/// `GET /api/settings/projects`
pub async fn http_list_projects(
    State(state): State<Shared>,
) -> Result<Json<Vec<ProjectFileSummary>>, (StatusCode, String)> {
    let dir = require_dir(&state)?;
    list_project_files(&dir).map(Json).map_err(to_http)
}

/// Wire shape of one project's settings: the parsed `Settings` plus which
/// file it lives in.
#[derive(Serialize)]
pub struct ProjectSettingsResponse {
    pub file: String,
    pub settings: Settings,
}

/// `GET /api/settings/projects/{id}`
pub async fn http_get_project(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
) -> Result<Json<ProjectSettingsResponse>, (StatusCode, String)> {
    let dir = require_dir(&state)?;
    let (file, settings) = get_project_file(&dir, &project_id).map_err(to_http)?;
    Ok(Json(ProjectSettingsResponse { file, settings }))
}

/// `GET /api/settings/resolve/{id}` — the viewer's read-only, default-falling-
/// back variant of `http_get_project`. Same response shape; only the lookup
/// differs (`resolve_project_file`). The viewer, which holds a payload id that
/// isn't a settings `project_id`, is the sole intended caller; editors keep the
/// strict `/api/settings/projects/{id}` route for GET and PUT.
pub async fn http_get_project_resolved(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
) -> Result<Json<ProjectSettingsResponse>, (StatusCode, String)> {
    let dir = require_dir(&state)?;
    let (file, settings) = resolve_project_file(&dir, &project_id).map_err(to_http)?;
    Ok(Json(ProjectSettingsResponse { file, settings }))
}

/// Save response: the settings as installed, plus the hot-reload confirmation
/// the UI shows ("saved & applied live").
#[derive(Serialize)]
pub struct SaveResponse {
    pub applied: bool,
    pub settings: Settings,
}

/// `POST /api/settings/projects` (create)
pub async fn http_create_project(
    State(state): State<Shared>,
    Json(settings): Json<Settings>,
) -> Result<Json<SaveResponse>, (StatusCode, String)> {
    let settings = save_project(&state, None, settings).map_err(to_http)?;
    Ok(Json(SaveResponse { applied: true, settings }))
}

/// `PUT /api/settings/projects/{id}` (update)
///
/// Body is taken as raw JSON, not as `Settings`, so `merge_over_stored` can
/// still see which keys the client actually sent — typing it here would
/// collapse "not sent" into "default" before anything could preserve it.
pub async fn http_update_project(
    State(state): State<Shared>,
    UrlPath(project_id): UrlPath<String>,
    Json(incoming): Json<serde_json::Value>,
) -> Result<Json<SaveResponse>, (StatusCode, String)> {
    let dir = require_dir(&state)?;
    let settings = merge_over_stored(&dir, &project_id, incoming).map_err(to_http)?;
    let settings = save_project(&state, Some(&project_id), settings).map_err(to_http)?;
    Ok(Json(SaveResponse { applied: true, settings }))
}

/// Optional `?taken_at=` on a dRofus upload — the snapshot-id half of the
/// upload envelope, carried as a query param because a raw CSV body has no
/// JSON envelope to put it in.
#[derive(Deserialize)]
pub struct ReferenceUploadQuery {
    #[serde(default)]
    pub taken_at: Option<String>,
}

/// `POST /projects/{id}/reference/{source}` — raw `text/csv` body (buffered
/// `Bytes`: real reference exports are a few MB of CSV, not the >100 MB FFE
/// case that forced `/rooms/stream` to stream), for any reference source the
/// project configures with an `upload` origin.
pub async fn http_upload_reference(
    State(state): State<Shared>,
    UrlPath((project_id, source)): UrlPath<(String, String)>,
    Query(query): Query<ReferenceUploadQuery>,
    body: axum::body::Bytes,
) -> Result<Json<ReferenceUploadResult>, (StatusCode, String)> {
    upload_reference(&state, &project_id, &source, query.taken_at.as_deref(), &body)
        .map(Json)
        .map_err(to_http)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::RoomResolution;
    use crate::storage::MemStore;
    use std::collections::HashMap;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roommate-settings-api-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file_backed_state(dir: &Path) -> AppState {
        AppState::new(Box::new(MemStore::new()), HashMap::new(), None).with_projects_dir(dir.to_path_buf())
    }

    fn minimal_settings(id: &str) -> Settings {
        toml::from_str(&format!("project_id = \"{id}\"\n")).unwrap()
    }

    /// A `Settings` value survives serialize-to-TOML → parse — proves the new
    /// `Serialize` derives and toml's table ordering handle the full shape
    /// (values, tables, arrays-of-tables interleaved in struct order).
    ///
    /// Long by necessity, not by neglect, which is why it takes the `#[allow]`
    /// the conventions reserve for a cohesive function rather than being split:
    /// what it asserts is that the **whole** shape round-trips at once, so its
    /// length is a function of how many fields `Settings` has. Splitting it into
    /// per-section tests would test each section against a *separately built*
    /// document and lose the property that actually matters — the TOML
    /// value-before-table ordering footgun (CODING-CONVENTIONS.md) only bites
    /// when the sections are interleaved in one file.
    #[allow(clippy::too_many_lines)]
    #[test]
    fn test_settings_toml_round_trip() {
        let source: Settings = toml::from_str(
            // `r##"…"##` (not `r#"…"#`): a colour value like `"#5a7a4f"` contains
            // the `"#` sequence that would otherwise close a single-hash raw string.
            r##"
project_id = "p1"
is_default = true
room_label = ["$name", "Area"]
comparison_key = "Number"
comparison_properties = ["Area", "Department"]

[sources.reference.drofus]
type = "upload"

[[hierarchy]]
name = "Building"
code_property = "bldg_code"

[[builtin_properties]]
canonical = "Area"
by_source = { revit = "Area" }

[[milestones]]
name = "Design Freeze"
date = "2026-06-30"
[milestones.attachments]
"model-guid" = "2026-06-29T10:00:00Z"

[[sources.reference.drofus.fields]]
label = "LastSync"
type = "date"
format = "%Y-%m-%d"
qa = "ignore"

[[colour_plans]]
name = "Area check"
active = true
[colour_plans.mode]
kind = "propertycompare"
property_a = "Area"
property_b = "d_net_area"
op = "diff"
[colour_plans.mode.colouring]
style = "bands"
bands = [
    { hi = 0.0, colour = "#5a7a4f" },
    { lo = 0.0, hi = 1.0, colour = "#e4ddc9" },
    { lo = 1.0, colour = "#b4541f" },
]

[[colour_plans]]
name = "By department"
[colour_plans.mode]
kind = "hierarchy"
tiers = ["Building", "Department"]
scheme = "Set2"

[[colour_plans]]
name = "By sync date"
[colour_plans.mode]
kind = "daterange"
property = "LastSync"
near_date = "2026-06-30"
scheme = "RdYlGn"
format = "%Y-%m-%d"

[[hierarchy_exclusions]]
match = "group"
tier = "Department"
value = "Outdoor"

[[hierarchy_exclusions]]
match = "rooms"
ids = ["12345", "67890"]
"##,
        )
        .unwrap();

        let text = toml::to_string_pretty(&source).unwrap();
        let reparsed: Settings = toml::from_str(&text).unwrap();

        assert_eq!(reparsed.project_id, "p1");
        assert!(reparsed.is_default);
        assert_eq!(reparsed.room_label, vec!["$name".to_string(), "Area".to_string()]);
        // Comparison settings survive the round-trip. See
        // `test_toml_serializer_hoists_values_above_tables` below for *why*
        // that holds regardless of where they sit in the struct.
        assert_eq!(reparsed.comparison_key.as_deref(), Some("Number"));
        assert_eq!(reparsed.comparison_properties, vec!["Area".to_string(), "Department".to_string()]);
        assert!(matches!(
            reparsed.sources.reference.get("drofus").map(|s| &s.origin),
            Some(ReferenceOrigin::Upload)
        ));
        assert_eq!(reparsed.hierarchy.len(), 1);
        assert_eq!(reparsed.builtin_properties.len(), 1);
        assert_eq!(reparsed.sources.reference["drofus"].fields.len(), 1);
        assert_eq!(reparsed.milestones.len(), 1);
        assert_eq!(reparsed.milestones[0].name, "Design Freeze");
        assert_eq!(reparsed.milestones[0].attachments["model-guid"], "2026-06-29T10:00:00Z");

        // Colour plans survive the full TOML round-trip — all three modes'
        // nested internally-tagged `mode`/`colouring` enums, the `Bands` list,
        // and the date-range `format` (the serde-tagging decision this test is
        // the gate for).
        use crate::settings::{ColourMode, Colouring};
        assert_eq!(reparsed.colour_plans.len(), 3);
        match &reparsed.colour_plans[0].mode {
            ColourMode::PropertyCompare { property_a, colouring, .. } => {
                assert_eq!(property_a, "Area");
                match colouring {
                    Colouring::Bands { bands } => assert_eq!(bands.len(), 3),
                    other => panic!("expected Bands, got {other:?}"),
                }
            }
            other => panic!("expected PropertyCompare, got {other:?}"),
        }
        match &reparsed.colour_plans[1].mode {
            ColourMode::Hierarchy { tiers, scheme } => {
                assert_eq!(tiers, &vec!["Building".to_string(), "Department".to_string()]);
                assert_eq!(scheme, "Set2");
            }
            other => panic!("expected Hierarchy, got {other:?}"),
        }
        match &reparsed.colour_plans[2].mode {
            ColourMode::DateRange { property, format, .. } => {
                assert_eq!(property, "LastSync");
                assert_eq!(format.as_deref(), Some("%Y-%m-%d"));
            }
            other => panic!("expected DateRange, got {other:?}"),
        }

        // Hierarchy exclusions survive the round-trip even though they are the
        // LAST array-of-tables, emitted after the colour-plan tables — the exact
        // ordering the `skip_serializing_if` guard protects. Both match kinds
        // (internally tagged on `match`) parse back to their variants.
        use crate::settings::HierarchyExclusion;
        assert_eq!(reparsed.hierarchy_exclusions.len(), 2);
        match &reparsed.hierarchy_exclusions[0] {
            HierarchyExclusion::Group { tier, value } => {
                assert_eq!(tier, "Department");
                assert_eq!(value, "Outdoor");
            }
            other => panic!("expected Group, got {other:?}"),
        }
        match &reparsed.hierarchy_exclusions[1] {
            HierarchyExclusion::Rooms { ids } => assert_eq!(ids, &vec!["12345".to_string(), "67890".to_string()]),
            other => panic!("expected Rooms, got {other:?}"),
        }
    }

    /// **The TOML ordering footgun, pinned.** `toml 0.8`'s serializer emits all
    /// value-typed fields (scalars and inline arrays) *before* any table or
    /// array-of-tables, regardless of the order they are declared in the struct.
    /// That is what makes a value declared after a map safe today.
    ///
    /// It was not always so, and CODING-CONVENTIONS.md still teaches declaring
    /// scalars first — correctly, as belt and braces. But a discipline nothing
    /// measures is one nobody can tell has stopped being necessary, and the
    /// reverse is worse: if a future `toml` release, a switch to
    /// `toml_edit`, or a hand-rolled writer ever restores source-order
    /// emission, every settings file written after that point silently gains a
    /// value nested inside the wrong table. The value still round-trips —
    /// nothing is lost, the key just *moves* — which is exactly why
    /// `test_settings_toml_round_trip` above cannot catch it and why this is a
    /// separate test.
    ///
    /// Deliberately declared in the WRONG order, so it fails the day the
    /// serializer stops compensating.
    #[test]
    fn test_toml_serializer_hoists_values_above_tables() {
        #[derive(serde::Serialize)]
        struct WrongOrder {
            table: std::collections::BTreeMap<String, String>,
            array_of_tables: Vec<Inner>,
            scalar_declared_last: String,
            inline_array_declared_last: Vec<String>,
        }
        #[derive(serde::Serialize)]
        struct Inner {
            a: String,
        }

        let value = WrongOrder {
            table: std::collections::BTreeMap::from([("k".to_string(), "v".to_string())]),
            array_of_tables: vec![Inner { a: "x".to_string() }],
            scalar_declared_last: "top".to_string(),
            inline_array_declared_last: vec!["also-top".to_string()],
        };

        let text = toml::to_string_pretty(&value).expect("serializes");
        let doc: toml::Table = toml::from_str(&text).expect("re-parses");

        for key in ["scalar_declared_last", "inline_array_declared_last"] {
            assert!(
                doc.contains_key(key),
                "`{key}` is no longer emitted at the top level: the TOML serializer has \
                 stopped hoisting values above tables, so every settings struct whose \
                 scalars sit below a map or Vec<Struct> now writes them INTO that table. \
                 Re-order those structs' fields (CODING-CONVENTIONS.md, \"TOML footgun\") \
                 before this ships.\n--- emitted ---\n{text}"
            );
        }
    }

    /// The same guarantee for the storage manifest, which is the other TOML
    /// document this server writes. `ModelEntry` gained a scalar (`phase`) and
    /// will gain more as entities are added, so it is worth asserting rather
    /// than assuming — a corrupt `project.toml` fails every read *and* the next
    /// boot, while the snapshots beside it stay perfectly intact.
    #[test]
    fn test_project_manifest_scalars_stay_top_level() {
        use crate::storage::{ModelEntry, ProjectManifest};

        let manifest = ProjectManifest {
            name: "Hospital Job".to_string(),
            models: std::collections::BTreeMap::from([(
                "m1".to_string(),
                ModelEntry {
                    name: "ARCH".to_string(),
                    phase: Some("New Construction".to_string()),
                    snapshots: vec!["2026-01-01T00:00:00Z".to_string()],
                    doors: vec![],
                    windows: vec![],
                    ffe: vec![],
                },
            )]),
            reference_snapshots: std::collections::BTreeMap::from([(
                "drofus".to_string(),
                vec!["2026-01-01T00:00:00Z".to_string()],
            )]),
        };

        let text = toml::to_string_pretty(&manifest).expect("serializes");
        let doc: toml::Table = toml::from_str(&text).expect("re-parses");
        assert!(doc.contains_key("name"), "the project name must stay top level:\n{text}");

        // And the per-model scalars stay inside their own model's table rather
        // than sliding into a sibling.
        let reparsed: ProjectManifest = toml::from_str(&text).expect("round-trips");
        let entry = &reparsed.models["m1"];
        assert_eq!(entry.name, "ARCH");
        assert_eq!(entry.phase.as_deref(), Some("New Construction"));
        assert_eq!(entry.snapshots, vec!["2026-01-01T00:00:00Z".to_string()]);
        assert_eq!(reparsed.reference_snapshots["drofus"].len(), 1);
    }

    /// A settings file with no `colour_plans` key deserializes to an empty
    /// `Vec` — the `#[serde(default)]` back-compat net for every project file
    /// saved before this feature existed.
    #[test]
    fn test_settings_without_colour_plans_defaults_empty() {
        let source: Settings = toml::from_str("project_id = \"p1\"\n").unwrap();
        assert!(source.colour_plans.is_empty());
    }

    /// Create → list → get round-trip through the core, and the saved project
    /// is immediately resolvable via the hot-swapped registry (no restart).
    #[test]
    fn test_save_applies_live_and_reads_back() {
        let dir = temp_dir("save-live");
        let state = file_backed_state(&dir);

        assert!(state.settings().settings_for("p1").is_none(), "not registered before save");

        save_project(&state, None, minimal_settings("p1")).unwrap();

        assert!(state.settings().settings_for("p1").is_some(), "registered without a restart");
        let list = list_project_files(&dir).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].project_id.as_deref(), Some("p1"));
        let (file, settings) = get_project_file(&dir, "p1").unwrap();
        assert_eq!(file, "p1.toml");
        assert_eq!(settings.project_id, "p1");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Creating an id that already exists is a conflict, not an overwrite.
    #[test]
    fn test_create_duplicate_id_conflicts() {
        let dir = temp_dir("dup-id");
        let state = file_backed_state(&dir);
        save_project(&state, None, minimal_settings("p1")).unwrap();

        match save_project(&state, None, minimal_settings("p1")) {
            Err(SettingsError::Conflict(msg)) => assert!(msg.contains("p1")),
            other => panic!("expected Conflict, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A second `is_default` is rejected with the file that already claims it
    /// named — same rule startup enforces, caught before anything is written.
    #[test]
    fn test_second_default_rejected() {
        let dir = temp_dir("second-default");
        let state = file_backed_state(&dir);
        let mut first = minimal_settings("p1");
        first.is_default = true;
        save_project(&state, None, first).unwrap();

        let mut second = minimal_settings("p2");
        second.is_default = true;
        match save_project(&state, None, second) {
            Err(SettingsError::Invalid(msg)) => assert!(msg.contains("is_default") && msg.contains("p1")),
            other => panic!("expected Invalid, got {other:?}"),
        }
        assert!(get_project_file(&dir, "p2").is_err(), "nothing was written for the rejected save");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// An invalid candidate (here: a `date` field with no `format` — the same
    /// startup-loud rule) leaves the existing file byte-identical on disk and
    /// the registry unswapped.
    #[test]
    fn test_invalid_update_leaves_file_intact() {
        let dir = temp_dir("invalid-update");
        let state = file_backed_state(&dir);
        save_project(&state, None, minimal_settings("p1")).unwrap();
        let before = std::fs::read_to_string(dir.join("p1.toml")).unwrap();

        let bad: Settings = toml::from_str(
            "project_id = \"p1\"\n\n[sources.reference.drofus]\ntype = \"upload\"\n\n[[sources.reference.drofus.fields]]\nlabel = \"X\"\ntype = \"date\"\n",
        )
        .unwrap();
        match save_project(&state, Some("p1"), bad) {
            Err(SettingsError::Invalid(msg)) => assert!(msg.contains("format")),
            other => panic!("expected Invalid, got {other:?}"),
        }

        assert_eq!(std::fs::read_to_string(dir.join("p1.toml")).unwrap(), before, "file untouched");
        assert!(!dir.join(".p1.candidate.tmp").exists(), "candidate temp file cleaned up");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The save path rejects a bad comparison namespace — verifying (not
    /// assuming) that re-running `load_project_bundle` covers the new
    /// namespace validation, so the API 422 really does come for free.
    #[test]
    fn test_save_rejects_bad_comparison_namespace() {
        let dir = temp_dir("cmp-ns");
        let state = file_backed_state(&dir);
        save_project(&state, None, minimal_settings("p1")).unwrap();
        let before = std::fs::read_to_string(dir.join("p1.toml")).unwrap();

        let mut bad = minimal_settings("p1");
        bad.comparison_key = Some("drofuss.NetArea".to_string());
        match save_project(&state, Some("p1"), bad) {
            Err(SettingsError::Invalid(msg)) => {
                assert!(msg.contains("unknown data source"), "names the problem: {msg}");
                assert!(msg.contains("drofus"), "names the known sources: {msg}");
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
        assert_eq!(std::fs::read_to_string(dir.join("p1.toml")).unwrap(), before, "file untouched");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A stored file holding two sections the settings page cannot edit
    /// (`[doors]`) and one it can (`hierarchy`), so one fixture exercises both
    /// halves of the merge rule.
    const DOORS_TOML: &str = r#"
project_id = "p1"
room_label = ["$name"]

[doors]
comparison_key = "$id"
room_resolution = "same_model"

[[hierarchy]]
name = "Building"
name_property = "Building"

[[hierarchy]]
name = "Department"
name_property = "Department"
"#;

    /// The regression this whole merge exists for: a body shaped like what
    /// `settings.html` actually sends — no `doors` key anywhere — must not
    /// delete `[doors]`. Before `merge_over_stored`, editing a room label
    /// silently dropped `room_resolution` and re-homeless'd 2084 RHH doors.
    #[test]
    fn test_update_preserves_sections_the_client_did_not_send() {
        let dir = temp_dir("merge-keep");
        std::fs::write(dir.join("p1.toml"), DOORS_TOML).unwrap();

        let body = serde_json::json!({
            "project_id": "p1",
            "room_label": ["$name", "Area"],
            "hierarchy": [{ "name": "Building", "name_property": "Building" }],
        });
        let merged = merge_over_stored(&dir, "p1", body).unwrap();

        assert_eq!(merged.doors.comparison_key.as_deref(), Some("$id"), "unsent section inherited");
        assert_eq!(merged.doors.room_resolution, RoomResolution::SameModel);
        // Top-level only: the one tier the client sent wins WHOLE. A deep
        // merge would put the deleted "Department" tier back.
        assert_eq!(merged.hierarchy.len(), 1, "a sent list is not back-filled");
        assert_eq!(merged.room_label, vec!["$name".to_string(), "Area".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The other half of the same rule: a section the client DID send replaces
    /// the stored one entirely, defaults included. Sending `doors` without
    /// `room_resolution` means off — inheriting the stored `same_model` field
    /// by field is the deep merge this deliberately isn't.
    #[test]
    fn test_update_lets_a_sent_section_win_whole() {
        let dir = temp_dir("merge-override");
        std::fs::write(dir.join("p1.toml"), DOORS_TOML).unwrap();

        let body = serde_json::json!({
            "project_id": "p1",
            "doors": { "comparison_key": "Mark" },
        });
        let merged = merge_over_stored(&dir, "p1", body).unwrap();

        assert_eq!(merged.doors.comparison_key.as_deref(), Some("Mark"));
        assert_eq!(merged.doors.room_resolution, RoomResolution::Off, "replaced whole, not field by field");
        // Untouched keys still come from disk.
        assert_eq!(merged.hierarchy.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A project with no stored file has nothing to inherit, and the body is
    /// typed exactly as it arrived. The 404 belongs to `save_project`, so this
    /// path must not raise one of its own.
    #[test]
    fn test_update_merge_tolerates_an_unknown_project() {
        let dir = temp_dir("merge-ghost");
        let merged = merge_over_stored(&dir, "ghost", serde_json::json!({ "project_id": "ghost" })).unwrap();
        assert_eq!(merged.project_id, "ghost");
        assert_eq!(merged.doors.room_resolution, RoomResolution::Off);

        // A body that isn't an object cannot be merged into one.
        assert!(matches!(
            merge_over_stored(&dir, "ghost", serde_json::json!([1, 2, 3])),
            Err(SettingsError::Invalid(_))
        ));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A commented file in the shape the real project files take: a header
    /// block, a comment on a scalar, one on a table, and one on each entry of
    /// an array-of-tables.
    const COMMENTED_TOML: &str = r#"# Rouse Hill Hospital — the file header.
# Second line of it.
project_id = "p1"

# Room identity across milestones.
comparison_key = "Number"
room_label = ["$name"]

# Doors are pushed for six of the seven models.
[doors]
comparison_key = "$id"
room_resolution = "same_model"

# The top tier.
[[hierarchy]]
name = "Building"
name_property = "Building"

# The middle tier, keyed on name only.
[[hierarchy]]
name = "Department"
name_property = "Department"
"#;

    /// Comments survive a save. Before this, every save regenerated the file
    /// from the typed struct and the reasoning in it was simply gone — the
    /// values were preserved and the only record of WHY they were those values
    /// was not.
    #[test]
    fn test_render_carries_comments_onto_a_regenerated_file() {
        let mut settings: Settings = toml::from_str(COMMENTED_TOML).unwrap();
        settings.room_label = vec!["$name".to_string(), "Area".to_string()];

        let rendered = render_settings(&settings, Some(COMMENTED_TOML)).unwrap();

        assert!(rendered.contains("# Rouse Hill Hospital — the file header."), "header block: {rendered}");
        assert!(rendered.contains("# Second line of it."));
        assert!(rendered.contains("# Room identity across milestones."), "scalar key comment");
        assert!(
            rendered.contains("# Doors are pushed for six of the seven models."),
            "table header comment"
        );
        assert!(rendered.contains("# The top tier."), "array-of-tables entry comment");
        assert!(rendered.contains("# The middle tier, keyed on name only."));

        // The edit still landed, and the file still parses back to it.
        let back: Settings = toml::from_str(&rendered).unwrap();
        assert_eq!(back.room_label, vec!["$name".to_string(), "Area".to_string()]);
        assert_eq!(back.doors.room_resolution, RoomResolution::SameModel);
    }

    /// Reordering array entries moves their comments with them. Matching by
    /// position instead would leave "# The top tier." sitting on Department —
    /// a comment that now says something false, which is worse than none.
    #[test]
    fn test_render_matches_array_entries_by_identity_not_position() {
        let mut settings: Settings = toml::from_str(COMMENTED_TOML).unwrap();
        settings.hierarchy.reverse();

        let rendered = render_settings(&settings, Some(COMMENTED_TOML)).unwrap();

        let department = rendered.find("# The middle tier, keyed on name only.").expect("kept");
        let building = rendered.find("# The top tier.").expect("kept");
        assert!(department < building, "comments followed their tiers:\n{rendered}");
        assert!(
            rendered[department..building].contains("name = \"Department\""),
            "Department's comment sits on Department:\n{rendered}"
        );
    }

    /// A section that is gone takes its comment with it, and a create has
    /// nothing to carry — the two ends of "decor follows content".
    #[test]
    fn test_render_drops_comments_for_removed_sections_and_skips_a_create() {
        let mut settings: Settings = toml::from_str(COMMENTED_TOML).unwrap();
        settings.doors = Default::default();
        settings.hierarchy.clear();

        let rendered = render_settings(&settings, Some(COMMENTED_TOML)).unwrap();
        assert!(!rendered.contains("# Doors are pushed"), "comment left without its section:\n{rendered}");
        assert!(!rendered.contains("# The top tier."));
        assert!(rendered.contains("# Room identity across milestones."), "surviving keys keep theirs");

        // No previous file: byte-identical to what the plain serializer writes,
        // so a create is unchanged by any of this.
        let created = render_settings(&settings, None).unwrap();
        assert_eq!(created, toml::to_string_pretty(&settings).unwrap());
    }

    /// An unparseable previous file must not fail the save it precedes — the
    /// settings UI is exactly the tool you would use to fix a rotten file, so
    /// overwriting one has to keep working. Comments are simply not carried.
    #[test]
    fn test_render_falls_back_when_the_previous_file_is_broken() {
        let settings = minimal_settings("p1");
        let rendered = render_settings(&settings, Some("this is not toml [[")).unwrap();
        assert_eq!(rendered, toml::to_string_pretty(&settings).unwrap());
    }

    const UPLOAD_TOML: &str = "project_id = \"p1\"\n\n[sources.reference.drofus]\ntype = \"upload\"\n";
    const UPLOAD_CSV: &[u8] = b"DrofusRoomId,NetArea\nNumber,Area\n1,25.5\n";

    /// Upload happy path: supplied `taken_at` echoed, minted when absent, and
    /// the uploaded data is live in the registry without a restart.
    #[test]
    fn test_upload_reference_applies_live() {
        let dir = temp_dir("up-happy");
        std::fs::write(dir.join("p1.toml"), UPLOAD_TOML).unwrap();
        let state = file_backed_state(&dir);

        let res = upload_reference(&state, "p1", "drofus", Some("2026-01-01T10:00:00Z"), UPLOAD_CSV).unwrap();
        assert!(res.accepted && res.stored);
        assert!(!res.snapshot_id_generated);
        assert_eq!(res.snapshot_taken_at, "2026-01-01T10:00:00Z");
        assert_eq!(res.record_count, 1);
        assert_eq!(res.link_property, "Number");
        assert_eq!(res.labels, vec!["NetArea".to_string()]);

        // Hot-swap: the running registry now joins the uploaded data.
        let registry = state.settings();
        let drofus = registry.settings_for("p1").unwrap().reference["drofus"]
            .data
            .as_ref()
            .expect("hydrated live");
        assert_eq!(drofus.by_id["1"].fields.get("NetArea"), Some(&"25.5".to_string()));

        // Omitted taken_at: minted server-side and reported as such.
        let minted = upload_reference(&state, "p1", "drofus", None, UPLOAD_CSV).unwrap();
        assert!(minted.snapshot_id_generated);
        assert!(crate::contract::validate_snapshot_id(&minted.snapshot_taken_at).is_ok());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Every rejection path installs nothing: bad snapshot id, unknown
    /// project, a project whose source isn't `upload`, an unparseable CSV,
    /// and a CSV whose labels break the declared `drofus_fields`.
    #[test]
    fn test_upload_reference_rejections_store_nothing() {
        let dir = temp_dir("up-reject");
        std::fs::write(
            dir.join("p1.toml"),
            "project_id = \"p1\"\n\n[sources.reference.drofus]\ntype = \"upload\"\n\n[[sources.reference.drofus.fields]]\nlabel = \"NoSuchColumn\"\nqa = \"exact\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("p2.toml"), "project_id = \"p2\"\n").unwrap();
        let state = file_backed_state(&dir);

        // Non-UTC offset — the same 422 rule rooms ingest applies.
        assert!(matches!(
            upload_reference(&state, "p1", "drofus", Some("2026-01-01T10:00:00+10:00"), UPLOAD_CSV),
            Err(SettingsError::Invalid(_))
        ));
        // Unknown project.
        assert!(matches!(
            upload_reference(&state, "ghost", "drofus", None, UPLOAD_CSV),
            Err(SettingsError::NotFound(_))
        ));
        // Registered, but not upload-sourced — told to set the source first.
        match upload_reference(&state, "p2", "drofus", None, UPLOAD_CSV) {
            Err(SettingsError::Invalid(msg)) => assert!(msg.contains("upload")),
            other => panic!("expected Invalid, got {other:?}"),
        }
        // Unparseable CSV (no row 2).
        assert!(matches!(
            upload_reference(&state, "p1", "drofus", None, b"OnlyOneRow\n"),
            Err(SettingsError::Invalid(_))
        ));
        // Parseable CSV that doesn't carry the declared drofus_fields label.
        match upload_reference(&state, "p1", "drofus", None, UPLOAD_CSV) {
            Err(SettingsError::Invalid(msg)) => assert!(msg.contains("NoSuchColumn")),
            other => panic!("expected Invalid, got {other:?}"),
        }

        // None of the failures stored anything.
        assert!(state.list_reference_snapshot_ids("p1", "drofus").unwrap().is_empty());
        assert!(state.list_reference_snapshot_ids("p2", "drofus").unwrap().is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A backfilled older `taken_at` is stored (history) but does not
    /// displace the newer latest in the registry — needs the FsStore, since
    /// MemStore keeps no history by design.
    #[test]
    fn test_upload_reference_older_backfill_does_not_displace_latest() {
        let dir = temp_dir("up-backfill");
        let store_dir = temp_dir("up-backfill-store");
        std::fs::write(dir.join("p1.toml"), UPLOAD_TOML).unwrap();
        let state =
            AppState::new(Box::new(crate::storage::FsStore::new(store_dir.clone()).unwrap()), HashMap::new(), None)
                .with_projects_dir(dir.to_path_buf());

        upload_reference(
            &state,
            "p1",
            "drofus",
            Some("2026-01-02T10:00:00Z"),
            b"DrofusRoomId,NetArea\nNumber,Area\n1,99.9\n",
        )
        .unwrap();
        upload_reference(&state, "p1", "drofus", Some("2026-01-01T10:00:00Z"), UPLOAD_CSV).unwrap();

        // Both stored, newer still the live one.
        assert_eq!(state.list_reference_snapshot_ids("p1", "drofus").unwrap().len(), 2);
        let registry = state.settings();
        let drofus = registry.settings_for("p1").unwrap().reference["drofus"].data.as_ref().unwrap();
        assert_eq!(drofus.by_id["1"].fields.get("NetArea"), Some(&"99.9".to_string()));

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&store_dir).ok();
    }

    /// Save interplay, both directions: an `upload` project saves fine before
    /// any upload (shape-only field validation), and a save whose
    /// `drofus_fields` reference a label absent from the latest stored CSV is
    /// rejected — the labels now come from the store.
    #[test]
    fn test_save_upload_project_validates_fields_against_stored_csv() {
        let dir = temp_dir("up-save");
        let state = file_backed_state(&dir);

        // Before any upload: accepted (labels unknowable, shapes checked).
        let pre: Settings = toml::from_str(
            "project_id = \"p1\"\n\n[sources.reference.drofus]\ntype = \"upload\"\n\n[[sources.reference.drofus.fields]]\nlabel = \"NetArea\"\nqa = \"exact\"\n",
        )
        .unwrap();
        save_project(&state, None, pre).unwrap();

        upload_reference(&state, "p1", "drofus", Some("2026-01-01T10:00:00Z"), UPLOAD_CSV).unwrap();

        // After the upload, a label the stored CSV doesn't have is rejected.
        let bad: Settings = toml::from_str(
            "project_id = \"p1\"\n\n[sources.reference.drofus]\ntype = \"upload\"\n\n[[sources.reference.drofus.fields]]\nlabel = \"NoSuchColumn\"\nqa = \"exact\"\n",
        )
        .unwrap();
        match save_project(&state, Some("p1"), bad) {
            Err(SettingsError::Invalid(msg)) => assert!(msg.contains("NoSuchColumn")),
            other => panic!("expected Invalid, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Updating may not change the project id — the id is the identity.
    #[test]
    fn test_update_cannot_rename() {
        let dir = temp_dir("no-rename");
        let state = file_backed_state(&dir);
        save_project(&state, None, minimal_settings("p1")).unwrap();

        match save_project(&state, Some("p1"), minimal_settings("p2")) {
            Err(SettingsError::Invalid(msg)) => assert!(msg.contains("identity")),
            other => panic!("expected Invalid, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Updating an unknown id is NotFound; a path-unsafe id never touches disk.
    #[test]
    fn test_update_unknown_and_unsafe_ids() {
        let dir = temp_dir("unknown-unsafe");
        let state = file_backed_state(&dir);

        assert!(matches!(
            save_project(&state, Some("ghost"), minimal_settings("ghost")),
            Err(SettingsError::NotFound(_))
        ));
        assert!(matches!(
            save_project(&state, None, minimal_settings("a/b")),
            Err(SettingsError::Invalid(_))
        ));
        assert!(settings_files(&dir).unwrap().is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The dRofus dry-run reports records and labels; a bogus path is the
    /// same loud error a bad startup source would raise.
    #[test]
    fn test_list_reports_broken_file() {
        let dir = temp_dir("broken-file");
        std::fs::write(dir.join("good.toml"), "project_id = \"p1\"\n").unwrap();
        std::fs::write(dir.join("bad.toml"), "this is not toml [[").unwrap();

        let list = list_project_files(&dir).unwrap();
        assert_eq!(list.len(), 2);
        let bad = list.iter().find(|s| s.file == "bad.toml").unwrap();
        assert!(bad.error.is_some());
        let good = list.iter().find(|s| s.file == "good.toml").unwrap();
        assert_eq!(good.project_id.as_deref(), Some("p1"));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The viewer's resolving read: an exact `project_id` match wins even when a
    /// different file is the default (never silently prefer the default over a
    /// real match).
    #[test]
    fn test_resolve_project_file_exact_match_wins() {
        let dir = temp_dir("resolve-exact");
        std::fs::write(dir.join("real.toml"), "project_id = \"Riverside\"\n").unwrap();
        std::fs::write(dir.join("default.toml"), "project_id = \"other\"\nis_default = true\n").unwrap();

        let (file, settings) = resolve_project_file(&dir, "Riverside").unwrap();
        assert_eq!(file, "real.toml");
        assert_eq!(settings.project_id, "Riverside");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// An unknown id (the viewer's payload id case) falls back to the file marked
    /// `is_default` — this is what makes the colour picker resolve.
    #[test]
    fn test_resolve_project_file_falls_back_to_default() {
        let dir = temp_dir("resolve-default");
        std::fs::write(dir.join("real.toml"), "project_id = \"Riverside\"\nis_default = true\n").unwrap();

        let (file, settings) = resolve_project_file(&dir, "130486").unwrap();
        assert_eq!(file, "real.toml");
        assert_eq!(settings.project_id, "Riverside", "fell back to the is_default file");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// An unknown id with no default file is `NotFound` — the fallback doesn't
    /// invent a match.
    #[test]
    fn test_resolve_project_file_unknown_no_default_is_not_found() {
        let dir = temp_dir("resolve-none");
        std::fs::write(dir.join("real.toml"), "project_id = \"Riverside\"\n").unwrap();

        assert!(matches!(resolve_project_file(&dir, "130486"), Err(SettingsError::NotFound(_))));

        std::fs::remove_dir_all(&dir).ok();
    }
}
