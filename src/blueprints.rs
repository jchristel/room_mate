//! Authored blueprints: a room and its FF&E written as recipes against the room's
//! edges, so the same layout can be put in another room (`docs/PLAN-blueprints.md`).
//!
//! **Their own endpoint, not part of the connections document** (decision 8). A
//! connections save replaces the whole list; a blueprint has a history. So there is one
//! JSON file per blueprint, under `<projects_dir>/blueprints/<project>/<id>.json`,
//! written the way connections are: temp then rename, a copy of the replaced file under
//! `.backups/`, one lock, and this module is the only code that reads or writes them.
//!
//! **Revisions are append-only.** An edit is a new revision, never a change to an old
//! one, because an assignment pins a revision and editing a blueprint must not silently
//! move rooms already assigned. `current_revision` says which one new assignments use,
//! and moving it is the one way to change that. A revision is never deleted; removing an
//! item from a blueprint marks it `excluded` in the next revision.
//!
//! **Optimistic concurrency, per blueprint.** A write names the `taken_at` of the
//! blueprint it read (`base`), and a mismatch is a conflict rather than an overwrite --
//! two people editing one layout is the ordinary case for authored data.
//!
//! **A blueprint is never validated against the model on save.** Authored data outlives
//! the geometry it came from: the room it was captured from may have moved or gone, and
//! `source` is a record of where it came from, not a reference that must resolve.
//!
//! **A push never becomes current by itself** (decision 10). A blueprint that arrives from
//! elsewhere (an import now, a Revit-originated one later) is stored beside the blueprint it
//! is for as a *pending* revision: it appears on the timeline, changes nothing about what is
//! current or what any assignment pins, and waits for a person to **accept** it (it becomes
//! the next revision and the current one) or **reject** it (it is kept, marked rejected, never
//! deleted). Backups under `.backups/` are a different thing: the safety copy made when a
//! file is rewritten, not the review step.
//!
//! **A blueprint holds no sheets and no tags** (plan decision 27). Its room layout sheets live in the
//! sheet library, bound to the blueprint by id (`sheets::SheetRecord::blueprint`), so there is one
//! copy of a sheet and a blueprint's revisions are about the room alone.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::service::blueprint_model::RevisionContent;
use crate::state::is_path_safe_component;

pub const SCHEMA_VERSION: u32 = 1;

/// Hard limits, so a hostile or runaway save cannot make every later read expensive.
/// Far above any real room.
const MAX_BLUEPRINTS_PER_PROJECT: usize = 5_000;
const MAX_REVISIONS: usize = 500;
const MAX_PENDING: usize = 100;
const MAX_ITEMS: usize = 5_000;
const MAX_DOORS: usize = 200;
const MAX_OUTLINE_POINTS: usize = 2_000;
const MAX_PROPERTIES: usize = 2_000;
const MAX_NAME_CHARS: usize = 120;
const MAX_NOTE_CHARS: usize = 2_000;

/// Where a blueprint was captured from. A record, not a reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRoom {
    pub model_id: String,
    pub room_id: String,
    /// The room's name when it was captured.
    #[serde(default)]
    pub room_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Revision {
    /// 1, 2, 3 ...: never reused and never reordered.
    pub number: u32,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(flatten)]
    pub content: RevisionContent,
}

/// A revision that has arrived and not been reviewed. Not part of the history until accepted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingRevision {
    /// Identifies this push among the blueprint's pending and rejected ones. Never reused.
    pub pending_id: u32,
    pub received_at: String,
    /// What sent it, as the sender described itself: a project, a person, a tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The sheets and tags that came with the revision, kept opaque here (`blueprint_bundle::Bundle`):
    /// they wait with it and are applied when it is accepted (plan decision 27). Dropped on rejection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<serde_json::Value>,
    #[serde(flatten)]
    pub content: RevisionContent,
}

/// A pushed revision a person declined. Kept, so the timeline can still show it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectedRevision {
    pub rejected_at: String,
    #[serde(flatten)]
    pub revision: PendingRevision,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Blueprint {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    /// When this blueprint was last saved: its version, and the `base` the next write names.
    pub taken_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceRoom>,
    /// The revision new assignments use. Always one of `revisions`.
    pub current_revision: u32,
    /// Whether the blueprint is in use (plan decision 27). **A blueprint is never deleted, only marked
    /// not in use**: it then takes no new assignment and is left out of suggestions, while what is bound
    /// to it (its sheets, its tags) and every assignment already made stay as they are. A file written
    /// before this is in use.
    #[serde(default = "in_use_by_default")]
    pub in_use: bool,
    pub revisions: Vec<Revision>,
    /// Pushed revisions waiting for review. Absent in a file written before pushes existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<PendingRevision>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected: Vec<RejectedRevision>,
    /// The last `pending_id` handed out. A counter and not a maximum over what is on file,
    /// because an accepted revision leaves the pending list and its id must not come back.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub last_pending_id: u32,
}

fn in_use_by_default() -> bool {
    true
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Blueprint {
    pub fn revision(&self, number: u32) -> Option<&Revision> {
        self.revisions.iter().find(|r| r.number == number)
    }

    pub fn current(&self) -> Option<&Revision> {
        self.revision(self.current_revision)
    }
}

/// A blueprint without its revisions' bodies: what a list shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlueprintSummary {
    pub id: String,
    pub name: String,
    pub taken_at: String,
    pub source: Option<SourceRoom>,
    pub current_revision: u32,
    pub in_use: bool,
    pub revisions: usize,
    /// Pushed revisions waiting for review.
    pub pending: usize,
    /// Items in the current revision, excluded ones not counted.
    pub items: usize,
    /// Sheets bound to this blueprint in the sheet library (plan decision 27). The store does not
    /// hold sheets, so this is 0 here and the caller fills it (`blueprints_api::with_sheet_counts`).
    pub sheets: usize,
}

impl From<&Blueprint> for BlueprintSummary {
    fn from(b: &Blueprint) -> Self {
        BlueprintSummary {
            id: b.id.clone(),
            name: b.name.clone(),
            taken_at: b.taken_at.clone(),
            source: b.source.clone(),
            current_revision: b.current_revision,
            in_use: b.in_use,
            revisions: b.revisions.len(),
            pending: b.pending.len(),
            items: b.current().map_or(0, |r| r.content.items.iter().filter(|i| !i.excluded).count()),
            sheets: 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateRequest {
    pub name: String,
    #[serde(default)]
    pub source: Option<SourceRoom>,
    #[serde(default)]
    pub note: Option<String>,
    /// Sheets and tags to apply once the blueprint exists. The store ignores it: the API layer applies it.
    #[serde(default)]
    pub bundle: Option<serde_json::Value>,
    #[serde(flatten)]
    pub content: RevisionContent,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddRevisionRequest {
    /// The `taken_at` of the blueprint the caller read.
    pub base: String,
    #[serde(default)]
    pub note: Option<String>,
    /// Whether the new revision becomes the current one. Default true: an edit is
    /// normally the new state of the layout.
    #[serde(default = "yes")]
    pub make_current: bool,
    #[serde(flatten)]
    pub content: RevisionContent,
}

fn yes() -> bool {
    true
}

/// A revision arriving from elsewhere. Carries no `base`: it adds to the pending list and
/// changes nothing a reader has seen of the history.
#[derive(Debug, Clone, Deserialize)]
pub struct PushRequest {
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// The revision's sheets and tags, kept with it until it is accepted.
    #[serde(default)]
    pub bundle: Option<serde_json::Value>,
    #[serde(flatten)]
    pub content: RevisionContent,
}

/// Accept or reject one pending revision.
#[derive(Debug, Clone, Deserialize)]
pub struct DecideRequest {
    pub base: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateRequest {
    pub base: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub current_revision: Option<u32>,
    /// Mark the blueprint in use or not in use.
    #[serde(default)]
    pub in_use: Option<bool>,
}

#[derive(Debug)]
pub enum BlueprintError {
    /// The state wasn't built from a settings directory (in-memory tests).
    NotFileBacked,
    Invalid(String),
    NotFound(String),
    /// The blueprint moved since the caller read it.
    Conflict(String),
    Internal(anyhow::Error),
}

/// Serialises every write, so the read-compare-write below cannot interleave.
static SAVE_LOCK: Mutex<()> = Mutex::new(());
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn bad<T>(msg: String) -> Result<T, BlueprintError> {
    Err(BlueprintError::Invalid(msg))
}

fn project_dir(projects_dir: &Path, project: &str) -> Result<PathBuf, BlueprintError> {
    if !is_path_safe_component(project) {
        return bad(format!("project id {project:?} cannot be a file name"));
    }
    Ok(projects_dir.join("blueprints").join(project))
}

fn blueprint_path(projects_dir: &Path, project: &str, id: &str) -> Result<PathBuf, BlueprintError> {
    if !is_path_safe_component(id) {
        return bad(format!("blueprint id {id:?} cannot be a file name"));
    }
    Ok(project_dir(projects_dir, project)?.join(format!("{id}.json")))
}

fn internal<E: Into<anyhow::Error>>(e: E) -> BlueprintError {
    BlueprintError::Internal(e.into())
}

/// Every blueprint of the project, by name. A file that cannot be read is an error, not a
/// gap: silently skipping it would make a corrupt blueprint look like a deleted one.
pub fn list(projects_dir: &Path, project: &str) -> Result<Vec<BlueprintSummary>, BlueprintError> {
    let dir = project_dir(projects_dir, project)?;
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(internal(e)),
    };
    let mut out = Vec::new();
    for entry in entries {
        let path = entry.map_err(internal)?.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        out.push(BlueprintSummary::from(&read(&path)?));
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    Ok(out)
}

fn read(path: &Path) -> Result<Blueprint, BlueprintError> {
    let bytes = std::fs::read(path).map_err(internal)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| BlueprintError::Internal(anyhow::anyhow!("{} is not a blueprint: {e}", path.display())))
}

pub fn load(projects_dir: &Path, project: &str, id: &str) -> Result<Blueprint, BlueprintError> {
    let path = blueprint_path(projects_dir, project, id)?;
    match std::fs::read(&path) {
        Ok(_) => read(&path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(BlueprintError::NotFound(format!("no blueprint {id:?} in project {project:?}")))
        }
        Err(e) => Err(internal(e)),
    }
}

fn clean_name(name: &str) -> Result<String, BlueprintError> {
    let name = name.trim();
    if name.is_empty() {
        return bad("a blueprint needs a name".to_string());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return bad(format!("a blueprint's name is at most {MAX_NAME_CHARS} characters"));
    }
    Ok(name.to_string())
}

fn clean_note(note: Option<String>) -> Result<Option<String>, BlueprintError> {
    let note = note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    if note.as_ref().is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS) {
        return bad(format!("a note is at most {MAX_NOTE_CHARS} characters"));
    }
    Ok(note)
}

fn finite(label: &str, values: &[f64]) -> Result<(), BlueprintError> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        bad(format!("{label} must be a finite number"))
    }
}

/// The one rule set for what a revision may hold. Pure, so it is testable without a disk.
pub fn validate_content(content: &RevisionContent) -> Result<(), BlueprintError> {
    let outline = &content.outline;
    if outline.coarse.len() < 3 {
        return bad("the coarse outline needs at least three corners".to_string());
    }
    if outline.coarse.len() > MAX_OUTLINE_POINTS || outline.fine.len() > MAX_OUTLINE_POINTS {
        return bad(format!("an outline is at most {MAX_OUTLINE_POINTS} points"));
    }
    for p in outline.coarse.iter().chain(&outline.fine) {
        finite("an outline point", &[p.x, p.y])?;
    }
    if content.doors.len() > MAX_DOORS {
        return bad(format!("a revision holds at most {MAX_DOORS} doors"));
    }
    for door in &content.doors {
        if door.edge >= outline.coarse.len() {
            return bad(format!("door {:?} is on edge {}, which the outline does not have", door.id, door.edge));
        }
        finite("a door's position", &[door.along_mm, door.edge_len_mm, door.width_mm])?;
    }
    if content.items.len() > MAX_ITEMS {
        return bad(format!("a revision holds at most {MAX_ITEMS} items"));
    }
    let mut seen = std::collections::HashSet::new();
    for item in &content.items {
        if item.id.trim().is_empty() {
            return bad("an item needs an id".to_string());
        }
        if !seen.insert(item.id.as_str()) {
            return bad(format!("item id {:?} appears twice", item.id));
        }
    }
    for item in &content.items {
        let p = &item.placement;
        if p.edge >= outline.coarse.len() {
            return bad(format!(
                "item {:?} is on edge {}, but the outline has {}",
                item.id,
                p.edge,
                outline.coarse.len()
            ));
        }
        finite(
            &format!("item {:?}'s placement", item.id),
            &[
                p.along_mm,
                p.off_mm,
                p.edge_len_mm,
                p.depth_mm,
                p.facing_deg,
                item.size_mm[0],
                item.size_mm[1],
            ],
        )?;
        if p.edge_len_mm <= 0.0 || p.depth_mm <= 0.0 {
            return bad(format!("item {:?}: the edge length and depth at capture must be positive", item.id));
        }
        if item.size_mm.iter().any(|s| *s < 0.0) {
            return bad(format!("item {:?}: a size cannot be negative", item.id));
        }
        if let Some(f) = &item.follows {
            if f.item == item.id {
                return bad(format!("item {:?} cannot follow itself", item.id));
            }
            if !seen.contains(f.item.as_str()) {
                return bad(format!("item {:?} follows {:?}, which is not in the revision", item.id, f.item));
            }
            finite(&format!("item {:?}'s follow gap", item.id), &[f.gap_mm])?;
        }
        if item.properties.len() > MAX_PROPERTIES || item.type_properties.len() > MAX_PROPERTIES {
            return bad(format!("item {:?} has more than {MAX_PROPERTIES} properties", item.id));
        }
    }
    // A chain of followers must end: an item cannot (through others) follow itself. The resolver
    // survives a loop and reports it, but a saved revision should not hold one.
    let follows: std::collections::HashMap<&str, &str> = content
        .items
        .iter()
        .filter_map(|i| i.follows.as_ref().map(|f| (i.id.as_str(), f.item.as_str())))
        .collect();
    for item in &content.items {
        let mut at = item.id.as_str();
        for _ in 0..=content.items.len() {
            match follows.get(at) {
                Some(&next) => at = next,
                None => break,
            }
            if at == item.id {
                return bad(format!("item {:?} follows itself through a loop of followers", item.id));
            }
        }
    }
    Ok(())
}

fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

/// A version stamp that sorts after the one it replaces, even on a clock that did not move.
fn next_stamp(previous: &str) -> String {
    let stamp = now();
    if stamp.as_str() <= previous {
        format!("{previous}~")
    } else {
        stamp
    }
}

fn new_id(dir: &Path) -> String {
    loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let id = format!("bp-{:x}-{n:x}", chrono::Utc::now().timestamp_micros());
        if !dir.join(format!("{id}.json")).exists() {
            return id;
        }
    }
}

/// Write `blueprint`, copying the file it replaces aside first. Order is load-bearing, as in
/// `connections::save`: everything that can refuse runs before this, the old file is copied
/// while it is still in place, and the new one is installed by rename so a reader never sees
/// half a document.
fn write(projects_dir: &Path, project: &str, blueprint: &Blueprint) -> Result<(), BlueprintError> {
    let path = blueprint_path(projects_dir, project, &blueprint.id)?;
    let dir = path.parent().expect("a blueprint path always has a parent");
    std::fs::create_dir_all(dir).map_err(internal)?;
    let json = serde_json::to_vec_pretty(blueprint).map_err(internal)?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &json).map_err(internal)?;
    let stem = format!("blueprint-{project}-{}", blueprint.id);
    if path.exists()
        && let Err(e) = crate::backups::back_up(projects_dir, &stem, "json", &path)
    {
        std::fs::remove_file(&temp).ok();
        return Err(BlueprintError::Internal(e));
    }
    std::fs::rename(&temp, &path).map_err(internal)?;
    crate::backups::prune(projects_dir, &stem, "json");
    Ok(())
}

pub fn create(projects_dir: &Path, project: &str, request: CreateRequest) -> Result<Blueprint, BlueprintError> {
    let name = clean_name(&request.name)?;
    let note = clean_note(request.note)?;
    validate_content(&request.content)?;
    let _guard = SAVE_LOCK.lock().unwrap();
    let dir = project_dir(projects_dir, project)?;
    if list(projects_dir, project)?.len() >= MAX_BLUEPRINTS_PER_PROJECT {
        return bad(format!("a project holds at most {MAX_BLUEPRINTS_PER_PROJECT} blueprints"));
    }
    let taken_at = now();
    let blueprint = Blueprint {
        schema_version: SCHEMA_VERSION,
        id: new_id(&dir),
        name,
        taken_at: taken_at.clone(),
        source: request.source,
        current_revision: 1,
        in_use: true,
        revisions: vec![Revision { number: 1, created_at: taken_at, note, content: request.content }],
        pending: Vec::new(),
        rejected: Vec::new(),
        last_pending_id: 0,
    };
    write(projects_dir, project, &blueprint)?;
    tracing::info!("created blueprint {} ({:?}) in project {project}", blueprint.id, blueprint.name);
    Ok(blueprint)
}

fn conflict(base: &str, current: &str) -> BlueprintError {
    BlueprintError::Conflict(format!(
        "the blueprint changed since you read it (yours is from {base:?}, the server has {current:?}); reload and try again"
    ))
}

/// Append a revision. Revisions are never edited in place.
pub fn add_revision(
    projects_dir: &Path,
    project: &str,
    id: &str,
    request: AddRevisionRequest,
) -> Result<Blueprint, BlueprintError> {
    let note = clean_note(request.note)?;
    validate_content(&request.content)?;
    let _guard = SAVE_LOCK.lock().unwrap();
    let mut blueprint = load(projects_dir, project, id)?;
    if blueprint.taken_at != request.base {
        return Err(conflict(&request.base, &blueprint.taken_at));
    }
    if blueprint.revisions.len() >= MAX_REVISIONS {
        return bad(format!("a blueprint holds at most {MAX_REVISIONS} revisions"));
    }
    let number = blueprint.revisions.iter().map(|r| r.number).max().unwrap_or(0) + 1;
    blueprint.taken_at = next_stamp(&blueprint.taken_at);
    let content = request.content;
    blueprint
        .revisions
        .push(Revision { number, created_at: blueprint.taken_at.clone(), note, content });
    if request.make_current {
        blueprint.current_revision = number;
    }
    write(projects_dir, project, &blueprint)?;
    Ok(blueprint)
}

/// Store a revision that arrived from elsewhere as **pending**. It is not a revision, is not
/// current, and no assignment can pin it; it waits for `accept` or `reject`.
pub fn push(projects_dir: &Path, project: &str, id: &str, request: PushRequest) -> Result<Blueprint, BlueprintError> {
    let note = clean_note(request.note)?;
    let origin = clean_note(request.origin)?;
    validate_content(&request.content)?;
    let _guard = SAVE_LOCK.lock().unwrap();
    let mut blueprint = load(projects_dir, project, id)?;
    if blueprint.pending.len() >= MAX_PENDING {
        return bad(format!("a blueprint holds at most {MAX_PENDING} pending revisions; review some first"));
    }
    let pending_id = blueprint.last_pending_id + 1;
    blueprint.last_pending_id = pending_id;
    blueprint.taken_at = next_stamp(&blueprint.taken_at);
    blueprint.pending.push(PendingRevision {
        pending_id,
        received_at: blueprint.taken_at.clone(),
        origin,
        note,
        bundle: request.bundle,
        content: request.content,
    });
    write(projects_dir, project, &blueprint)?;
    tracing::info!("blueprint {id} in project {project} received pending revision {pending_id}");
    Ok(blueprint)
}

fn take_pending(blueprint: &mut Blueprint, pending_id: u32) -> Result<PendingRevision, BlueprintError> {
    match blueprint.pending.iter().position(|p| p.pending_id == pending_id) {
        Some(i) => Ok(blueprint.pending.remove(i)),
        None => bad(format!("the blueprint has no pending revision {pending_id}")),
    }
}

/// Accept a pending revision: it becomes the next revision and the current one.
pub fn accept(
    projects_dir: &Path,
    project: &str,
    id: &str,
    pending_id: u32,
    request: DecideRequest,
) -> Result<Blueprint, BlueprintError> {
    let _guard = SAVE_LOCK.lock().unwrap();
    let mut blueprint = load(projects_dir, project, id)?;
    if blueprint.taken_at != request.base {
        return Err(conflict(&request.base, &blueprint.taken_at));
    }
    if blueprint.revisions.len() >= MAX_REVISIONS {
        return bad(format!("a blueprint holds at most {MAX_REVISIONS} revisions"));
    }
    let accepted = take_pending(&mut blueprint, pending_id)?;
    let number = blueprint.revisions.iter().map(|r| r.number).max().unwrap_or(0) + 1;
    blueprint.taken_at = next_stamp(&blueprint.taken_at);
    blueprint.revisions.push(Revision {
        number,
        created_at: blueprint.taken_at.clone(),
        note: accepted.note,
        content: accepted.content,
    });
    blueprint.current_revision = number;
    write(projects_dir, project, &blueprint)?;
    Ok(blueprint)
}

/// Reject a pending revision. It is kept, marked rejected, and the history is untouched.
pub fn reject(
    projects_dir: &Path,
    project: &str,
    id: &str,
    pending_id: u32,
    request: DecideRequest,
) -> Result<Blueprint, BlueprintError> {
    let _guard = SAVE_LOCK.lock().unwrap();
    let mut blueprint = load(projects_dir, project, id)?;
    if blueprint.taken_at != request.base {
        return Err(conflict(&request.base, &blueprint.taken_at));
    }
    let mut declined = take_pending(&mut blueprint, pending_id)?;
    declined.bundle = None; // a rejected revision's sheets and tags are never applied, so are not kept
    blueprint.taken_at = next_stamp(&blueprint.taken_at);
    blueprint
        .rejected
        .push(RejectedRevision { rejected_at: blueprint.taken_at.clone(), revision: declined });
    write(projects_dir, project, &blueprint)?;
    Ok(blueprint)
}

/// Rename, and/or move which revision is current.
pub fn update(
    projects_dir: &Path,
    project: &str,
    id: &str,
    request: UpdateRequest,
) -> Result<Blueprint, BlueprintError> {
    let name = request.name.as_deref().map(clean_name).transpose()?;
    let _guard = SAVE_LOCK.lock().unwrap();
    let mut blueprint = load(projects_dir, project, id)?;
    if blueprint.taken_at != request.base {
        return Err(conflict(&request.base, &blueprint.taken_at));
    }
    if let Some(n) = request.current_revision {
        if blueprint.revision(n).is_none() {
            return bad(format!("the blueprint has no revision {n}"));
        }
        blueprint.current_revision = n;
    }
    if let Some(name) = name {
        blueprint.name = name;
    }
    if let Some(in_use) = request.in_use {
        blueprint.in_use = in_use;
    }
    blueprint.taken_at = next_stamp(&blueprint.taken_at);
    write(projects_dir, project, &blueprint)?;
    Ok(blueprint)
}

/// Remove a blueprint's file, for the tests that need a blueprint that has gone (an assignment then
/// reports `blueprint_missing`). **There is no such operation in the product**: a blueprint is marked
/// not in use, never deleted (plan decision 27).
#[cfg(test)]
pub(crate) fn remove_for_test(projects_dir: &Path, project: &str, id: &str) {
    std::fs::remove_file(blueprint_path(projects_dir, project, id).unwrap()).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::blueprint_capture::{capture, SourceItem};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roommate-blueprints-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok(); // a stale one from a run with the same process id would read as saved data
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn content(items: usize) -> RevisionContent {
        let ring = vec![(0.0, 0.0), (4000.0, 0.0), (4000.0, 3000.0), (0.0, 3000.0)];
        let sources = (0..items)
            .map(|i| {
                let c = (500.0 + 700.0 * i as f64, 300.0);
                SourceItem {
                    id: format!("i{i}"),
                    family_name: Some("Chair".into()),
                    type_name: "Std".into(),
                    category: "Furniture".into(),
                    footprint: vec![
                        (c.0 - 250.0, c.1 - 250.0),
                        (c.0 + 250.0, c.1 - 250.0),
                        (c.0 + 250.0, c.1 + 250.0),
                        (c.0 - 250.0, c.1 + 250.0),
                    ],
                    facing: Some((0.0, 1.0)),
                    nested: false,
                    properties: vec![],
                    type_properties: vec![],
                }
            })
            .collect();
        capture(&ring, sources).unwrap().content
    }

    fn create_req(name: &str, items: usize) -> CreateRequest {
        CreateRequest {
            name: name.into(),
            source: None,
            note: None,
            bundle: None,
            content: content(items),
        }
    }

    #[test]
    fn test_a_pending_revision_keeps_its_bundle_until_rejected_and_accepting_hands_it_on() {
        let dir = temp_dir("bundle");
        let v1 = create(&dir, "p1", create_req("A", 1)).unwrap();
        let bundle = serde_json::json!({ "sheets": [] });
        let with = |n| PushRequest { origin: None, note: None, bundle: Some(bundle.clone()), content: content(n) };
        let a = push(&dir, "p1", &v1.id, with(2)).unwrap();
        let b = push(&dir, "p1", &v1.id, with(3)).unwrap();
        assert!(b.pending.iter().all(|p| p.bundle.as_ref() == Some(&bundle)), "waits with the revision");
        let rejected =
            reject(&dir, "p1", &v1.id, a.pending[0].pending_id, DecideRequest { base: b.taken_at.clone() }).unwrap();
        assert!(
            rejected.rejected[0].revision.bundle.is_none(),
            "a rejected revision's sheets and tags are dropped"
        );
        assert!(rejected.pending[0].bundle.is_some(), "the other one still waits");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_project_with_no_blueprints_lists_none() {
        let dir = temp_dir("empty");
        assert!(list(&dir, "p1").unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_create_then_load_round_trips_and_lists() {
        let dir = temp_dir("round-trip");
        let made = create(&dir, "p1", create_req("1 BED B", 3)).unwrap();
        assert_eq!((made.current_revision, made.revisions.len()), (1, 1));
        assert_eq!(load(&dir, "p1", &made.id).unwrap(), made);
        let listed = list(&dir, "p1").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0].name.as_str(), listed[0].items, listed[0].revisions), ("1 BED B", 3, 1));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// An edit is a new revision; the old one is untouched, and an assignment pinned to
    /// it keeps meaning what it meant.
    #[test]
    fn test_a_new_revision_is_appended_and_the_old_one_is_untouched() {
        let dir = temp_dir("append");
        let v1 = create(&dir, "p1", create_req("A", 2)).unwrap();
        let v2 = add_revision(
            &dir,
            "p1",
            &v1.id,
            AddRevisionRequest {
                base: v1.taken_at.clone(),
                note: Some("moved the bed".into()),
                make_current: true,
                content: content(3),
            },
        )
        .unwrap();
        assert_eq!((v2.current_revision, v2.revisions.len()), (2, 2));
        assert_eq!(
            v2.revision(1).unwrap(),
            v1.revision(1).unwrap(),
            "revision 1 is byte for byte what it was"
        );
        assert!(v2.taken_at > v1.taken_at);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_revision_can_be_added_without_becoming_current_and_current_can_move_back() {
        let dir = temp_dir("current");
        let v1 = create(&dir, "p1", create_req("A", 1)).unwrap();
        let v2 = add_revision(
            &dir,
            "p1",
            &v1.id,
            AddRevisionRequest { base: v1.taken_at.clone(), note: None, make_current: false, content: content(2) },
        )
        .unwrap();
        assert_eq!(v2.current_revision, 1);
        let moved = update(
            &dir,
            "p1",
            &v1.id,
            UpdateRequest { base: v2.taken_at.clone(), name: None, current_revision: Some(2), in_use: None },
        )
        .unwrap();
        assert_eq!(moved.current_revision, 2);
        let back = update(
            &dir,
            "p1",
            &v1.id,
            UpdateRequest {
                base: moved.taken_at.clone(),
                name: None,
                current_revision: Some(1),
                in_use: None,
            },
        )
        .unwrap();
        assert_eq!(
            (back.current_revision, back.revisions.len()),
            (1, 2),
            "nothing was lost by moving the marker"
        );
        assert!(update(
            &dir,
            "p1",
            &v1.id,
            UpdateRequest { base: back.taken_at, name: None, current_revision: Some(9), in_use: None }
        )
        .is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_stale_base_is_a_conflict_not_an_overwrite() {
        let dir = temp_dir("conflict");
        let v1 = create(&dir, "p1", create_req("A", 1)).unwrap();
        let first = update(
            &dir,
            "p1",
            &v1.id,
            UpdateRequest {
                base: v1.taken_at.clone(),
                name: Some("B".into()),
                current_revision: None,
                in_use: None,
            },
        )
        .unwrap();
        let stale = update(
            &dir,
            "p1",
            &v1.id,
            UpdateRequest {
                base: v1.taken_at.clone(),
                name: Some("C".into()),
                current_revision: None,
                in_use: None,
            },
        );
        assert!(matches!(stale, Err(BlueprintError::Conflict(_))));
        assert_eq!(load(&dir, "p1", &v1.id).unwrap(), first, "the earlier write is untouched");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_blueprint_is_marked_not_in_use_and_back_and_keeps_everything() {
        let dir = temp_dir("in-use");
        let v1 = create(&dir, "p1", create_req("A", 1)).unwrap();
        assert!(v1.in_use && list(&dir, "p1").unwrap()[0].in_use);
        let off = update(
            &dir,
            "p1",
            &v1.id,
            UpdateRequest {
                base: v1.taken_at.clone(),
                name: None,
                current_revision: None,
                in_use: Some(false),
            },
        )
        .unwrap();
        assert!(!off.in_use && !list(&dir, "p1").unwrap()[0].in_use);
        assert_eq!(off.revisions, v1.revisions, "nothing else changes");
        let on = update(
            &dir,
            "p1",
            &v1.id,
            UpdateRequest {
                base: off.taken_at.clone(),
                name: None,
                current_revision: None,
                in_use: Some(true),
            },
        )
        .unwrap();
        assert!(on.in_use);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_projects_do_not_see_each_others_blueprints() {
        let dir = temp_dir("scope");
        let a = create(&dir, "p1", create_req("A", 1)).unwrap();
        assert!(list(&dir, "p2").unwrap().is_empty());
        assert!(matches!(load(&dir, "p2", &a.id), Err(BlueprintError::NotFound(_))));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_path_unsafe_ids_are_refused() {
        let dir = temp_dir("unsafe");
        assert!(matches!(load(&dir, "p1", "../x"), Err(BlueprintError::Invalid(_))));
        assert!(matches!(list(&dir, "a/b"), Err(BlueprintError::Invalid(_))));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_validation_refuses_what_could_not_be_resolved() {
        let ok = content(2);
        assert!(validate_content(&ok).is_ok());

        let mut m = ok.clone();
        m.items[0].placement.edge = 99;
        assert!(validate_content(&m).is_err(), "an edge the outline does not have");

        let mut m = ok.clone();
        m.items[1].id = m.items[0].id.clone();
        assert!(validate_content(&m).is_err(), "a repeated item id");

        let mut m = ok.clone();
        m.items[0].placement.along_mm = f64::NAN;
        assert!(validate_content(&m).is_err(), "not a number");

        let mut m = ok.clone();
        m.items[0].placement.edge_len_mm = 0.0;
        assert!(validate_content(&m).is_err(), "a percentage of nothing");

        let mut m = ok.clone();
        m.items[0].follows =
            Some(crate::service::blueprint_model::Follow { item: "gone".into(), gap_mm: 0.0, inside: false });
        assert!(validate_content(&m).is_err(), "follows an item that is not there");
        m.items[0].follows =
            Some(crate::service::blueprint_model::Follow { item: m.items[0].id.clone(), gap_mm: 0.0, inside: false });
        assert!(validate_content(&m).is_err(), "follows itself");

        let mut m = ok;
        m.outline.coarse.truncate(2);
        assert!(validate_content(&m).is_err(), "fewer than three corners");
    }

    /// The point of decision 10: a pushed revision changes nothing about what is current.
    #[test]
    fn test_a_push_is_pending_and_changes_nothing_that_is_current() {
        let dir = temp_dir("push");
        let v1 = create(&dir, "p1", create_req("A", 2)).unwrap();
        let pushed = push(
            &dir,
            "p1",
            &v1.id,
            PushRequest { origin: Some("project Q".into()), note: None, bundle: None, content: content(5) },
        )
        .unwrap();
        assert_eq!(pushed.current_revision, 1);
        assert_eq!(pushed.revisions, v1.revisions, "the history is untouched");
        assert_eq!(pushed.pending.len(), 1);
        assert_eq!(pushed.pending[0].origin.as_deref(), Some("project Q"));
        assert_eq!(pushed.pending[0].content.items.len(), 5);
        assert_eq!(list(&dir, "p1").unwrap()[0].pending, 1);
        assert_eq!(list(&dir, "p1").unwrap()[0].items, 2, "the summary still describes the current revision");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_accepting_a_pending_revision_makes_it_the_next_and_current() {
        let dir = temp_dir("accept");
        let v1 = create(&dir, "p1", create_req("A", 2)).unwrap();
        let pushed = push(
            &dir,
            "p1",
            &v1.id,
            PushRequest { origin: None, note: Some("from Q".into()), bundle: None, content: content(5) },
        )
        .unwrap();
        let accepted = accept(&dir, "p1", &v1.id, 1, DecideRequest { base: pushed.taken_at.clone() }).unwrap();
        assert_eq!((accepted.current_revision, accepted.revisions.len(), accepted.pending.len()), (2, 2, 0));
        assert_eq!(accepted.revision(2).unwrap().content.items.len(), 5);
        assert_eq!(accepted.revision(2).unwrap().note.as_deref(), Some("from Q"));
        assert_eq!(accepted.revision(1).unwrap(), v1.revision(1).unwrap());
        let next = push(
            &dir,
            "p1",
            &v1.id,
            PushRequest { origin: None, note: None, bundle: None, content: content(1) },
        )
        .unwrap();
        assert_eq!(next.pending[0].pending_id, 2, "an accepted revision's id is not handed out again");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_rejecting_a_pending_revision_keeps_it_and_leaves_the_history_alone() {
        let dir = temp_dir("reject");
        let v1 = create(&dir, "p1", create_req("A", 2)).unwrap();
        let pushed = push(
            &dir,
            "p1",
            &v1.id,
            PushRequest { origin: None, note: None, bundle: None, content: content(5) },
        )
        .unwrap();
        let rejected = reject(&dir, "p1", &v1.id, 1, DecideRequest { base: pushed.taken_at.clone() }).unwrap();
        assert_eq!((rejected.current_revision, rejected.revisions.len(), rejected.pending.len()), (1, 1, 0));
        assert_eq!(rejected.rejected.len(), 1, "kept, not deleted");
        assert_eq!(rejected.rejected[0].revision.content.items.len(), 5);
        // A later push does not reuse the rejected one's id, nor an accepted one's.
        let again = push(
            &dir,
            "p1",
            &v1.id,
            PushRequest { origin: None, note: None, bundle: None, content: content(1) },
        )
        .unwrap();
        assert_eq!(again.pending[0].pending_id, 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_deciding_needs_the_current_version_and_a_pending_revision_that_exists() {
        let dir = temp_dir("decide");
        let v1 = create(&dir, "p1", create_req("A", 1)).unwrap();
        let pushed = push(
            &dir,
            "p1",
            &v1.id,
            PushRequest { origin: None, note: None, bundle: None, content: content(2) },
        )
        .unwrap();
        let stale = accept(&dir, "p1", &v1.id, 1, DecideRequest { base: v1.taken_at.clone() });
        assert!(matches!(stale, Err(BlueprintError::Conflict(_))), "read before the push");
        assert!(accept(&dir, "p1", &v1.id, 9, DecideRequest { base: pushed.taken_at.clone() }).is_err());
        assert!(reject(&dir, "p1", &v1.id, 9, DecideRequest { base: pushed.taken_at.clone() }).is_err());
        assert_eq!(load(&dir, "p1", &v1.id).unwrap(), pushed, "a refused decision wrote nothing");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_pushed_revision_is_validated_like_any_other() {
        let dir = temp_dir("push-bad");
        let v1 = create(&dir, "p1", create_req("A", 1)).unwrap();
        let mut bad_content = content(1);
        bad_content.items[0].placement.edge = 42;
        assert!(push(
            &dir,
            "p1",
            &v1.id,
            PushRequest { origin: None, note: None, bundle: None, content: bad_content }
        )
        .is_err());
        assert!(matches!(
            push(
                &dir,
                "p1",
                "nope",
                PushRequest { origin: None, note: None, bundle: None, content: content(1) }
            ),
            Err(BlueprintError::NotFound(_))
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_blank_name_is_refused_and_a_name_is_trimmed() {
        let dir = temp_dir("name");
        assert!(create(&dir, "p1", create_req("   ", 1)).is_err());
        assert_eq!(create(&dir, "p1", create_req("  Ward bay  ", 1)).unwrap().name, "Ward bay");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The JSON a client sees: revision fields sit beside the content, not under a wrapper.
    #[test]
    fn test_a_revision_is_flat_on_the_wire() {
        let dir = temp_dir("wire");
        let made = create(&dir, "p1", create_req("A", 1)).unwrap();
        let json = serde_json::to_value(&made).unwrap();
        let rev = &json["revisions"][0];
        assert!(rev.get("outline").is_some() && rev.get("items").is_some() && rev.get("number").is_some());
        assert!(rev.get("content").is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_door_on_an_edge_the_outline_does_not_have_is_refused() {
        let mut c = content(1);
        c.doors.push(crate::service::blueprint_model::BlueprintDoor {
            id: "d".to_string(),
            edge: 9,
            along_mm: 100.0,
            edge_len_mm: 1000.0,
            width_mm: 900.0,
        });
        assert!(matches!(validate_content(&c), Err(BlueprintError::Invalid(_))));
        c.doors[0].edge = 0;
        assert!(validate_content(&c).is_ok());
    }
}
