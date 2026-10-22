//! Room tasks: work the server holds for a Revit session to do (`docs/PLAN-blueprints.md`,
//! decisions 21 to 23 and 26).
//!
//! **A task is a record, unlike an assignment.** An assignment is a request worked out again on
//! every read; a task holds what Revit is told, as it was when the task was made -- the resolver's
//! placements (in the target MODEL's own frame, mm), the sheet layouts and the findings -- so
//! what a Revit session writes cannot change underneath it, and a later look says what was asked.
//! The plugin validates and writes; it never resolves (decision 2).
//!
//! **Three kinds.** `populate_room` places a blueprint's items in a room (and, if asked, its
//! sheets); `create_rls` makes room layout sheets from a library sheet and places nothing;
//! `update_room` brings a room populated earlier back in line with its blueprint (decision 21:
//! items, or only their locations, after a room-size or design change). An update is a DRY RUN
//! unless the person turns that off (decision 13): the session reports the diff and writes nothing.
//!
//! **States, and who moves them.** A task is `queued` when made or resent. A Revit session asks
//! for a model's tasks (`claim`) and every queued one becomes `in_progress`, so another session
//! is not handed it. The session then reports each as `completed`, `completed_with_warnings`,
//! `failed` or `aborted` (an abort found by validating the whole list is reported before any
//! work). A person can `dismiss` a task, or `resend` one (back to `queued`), which is also the
//! way out of a session that died holding tasks `in_progress`. Nothing is ever deleted: a
//! task's attempts are kept, each with what the session said.
//!
//! One JSON file per task under `<projects_dir>/tasks/<project>/<id>.json`, written the way
//! assignments are (temp, then rename, a copy of the file replaced under `.backups/`, one lock).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::service::blueprint_model::{BlueprintSheet, ItemClass, Property};
use crate::state::is_path_safe_component;
use crate::tag_rules::RuleSummary;

pub const SCHEMA_VERSION: u32 = 1;
const MAX_TASKS_PER_PROJECT: usize = 50_000;
const MAX_MESSAGES: usize = 200;
const MAX_MESSAGE_CHARS: usize = 2_000;

static SAVE_LOCK: Mutex<()> = Mutex::new(());
static COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// Place a blueprint's items in a room, and its sheets if `create_rls`.
    PopulateRoom,
    /// Make room layout sheets from a library sheet; nothing is placed.
    CreateRls,
    /// Update a room populated earlier from its blueprint: the task holds the layout as the
    /// resolver now places it, and the session matches it to what is in the room by the
    /// marker each placed item carries (blueprint id, revision, item id).
    UpdateRoom,
}

/// What an update changes in a room that already holds the blueprint's items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    /// Items: add the ones the layout has and the room lacks, remove the ones it no longer has,
    /// change type and properties, and move.
    Items,
    /// Locations: move what is there; add and remove nothing.
    Locations,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    InProgress,
    Completed,
    CompletedWithWarnings,
    Failed,
    Aborted,
    Dismissed,
}

impl TaskState {
    /// What a Revit session may report a task as.
    pub fn is_report(self) -> bool {
        matches!(self, Self::Completed | Self::CompletedWithWarnings | Self::Failed | Self::Aborted)
    }
}

/// The room, as the task names it. The id is the server's reference; the number is what the Revit
/// side finds the room by (decision 22), and is absent when the room has none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRoom {
    pub room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    pub name: String,
}

/// Which blueprint revision a populate task was made from: a record, not a reference that must resolve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlueprintRef {
    pub id: String,
    pub name: String,
    pub revision: u32,
}

/// One family instance to place, in the target model's own frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskItem {
    /// The blueprint item it came from.
    pub item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family_name: Option<String>,
    pub type_name: String,
    pub category: String,
    /// `free`, or `wall_device` (placed by its wall and height, no footprint).
    pub placement_type: ItemClass,
    /// The instance's `Host` property as exported (for example `Level : LEVEL 1`), when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Centre in the target model's frame, millimetres.
    pub x_mm: f64,
    pub y_mm: f64,
    /// Unit vector of the item's facing, in the same frame.
    pub facing_x: f64,
    pub facing_y: f64,
    /// Footprint size in plan: along the facing, then across it.
    pub size_mm: [f64; 2],
    /// Instance and type properties as exported ("related data": elevation, mounting height ...).
    pub properties: Vec<Property>,
    pub type_properties: Vec<Property>,
}

/// One go at a task by a Revit session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub schema_version: u32,
    pub id: String,
    pub kind: TaskKind,
    pub state: TaskState,
    /// The model the task is for: what a Revit session asks for tasks by.
    pub model_id: String,
    pub room: TaskRoom,
    pub phase: String,
    #[serde(default)]
    pub overwrite_layout: bool,
    #[serde(default)]
    pub create_rls: bool,
    #[serde(default)]
    pub overwrite_rls: bool,
    /// `update_room` only: what the update changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_mode: Option<UpdateMode>,
    /// `update_room` only: report what would change and write nothing. Defaults to true.
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blueprint: Option<BlueprintRef>,
    /// `create_rls`: the library sheet the layout was taken from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_sheet_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<TaskItem>,
    /// The sheet layouts to make (no tags: the Revit side applies the tag rules, decision 24).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sheets: Vec<BlueprintSheet>,
    /// How to tag what the sheets show, as the rules were when the task was made: one per key
    /// (`tag_rules`), for the viewport types of the sheets and, for a populate task, only the
    /// families and types it places. The plug-in applies them to the items it places or finds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_rules: Vec<RuleSummary>,
    /// What the resolver said the room changes, and any other warning from making the task.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Value>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts: Vec<Attempt>,
}

/// A task without its items and sheets: what a list shows.
#[derive(Debug, Clone, Serialize)]
pub struct TaskSummary {
    pub id: String,
    pub kind: TaskKind,
    pub state: TaskState,
    pub model_id: String,
    pub room: TaskRoom,
    pub phase: String,
    pub overwrite_layout: bool,
    pub create_rls: bool,
    pub overwrite_rls: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_mode: Option<UpdateMode>,
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blueprint: Option<BlueprintRef>,
    pub items: usize,
    pub sheets: usize,
    pub tag_rules: usize,
    pub findings: usize,
    pub created_at: String,
    pub updated_at: String,
    pub attempts: usize,
    /// What the last attempt said, for a glance at why a task failed or aborted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub last_messages: Vec<String>,
}

impl From<&Task> for TaskSummary {
    fn from(t: &Task) -> Self {
        TaskSummary {
            id: t.id.clone(),
            kind: t.kind,
            state: t.state,
            model_id: t.model_id.clone(),
            room: t.room.clone(),
            phase: t.phase.clone(),
            overwrite_layout: t.overwrite_layout,
            create_rls: t.create_rls,
            overwrite_rls: t.overwrite_rls,
            update_mode: t.update_mode,
            dry_run: t.dry_run,
            blueprint: t.blueprint.clone(),
            items: t.items.len(),
            sheets: t.sheets.len(),
            tag_rules: t.tag_rules.len(),
            findings: t.findings.len(),
            created_at: t.created_at.clone(),
            updated_at: t.updated_at.clone(),
            attempts: t.attempts.len(),
            last_messages: t.attempts.last().map(|a| a.messages.clone()).unwrap_or_default(),
        }
    }
}

/// What a person's request makes of a task, before it has an id or a state.
#[derive(Debug, Clone)]
pub struct NewTask {
    pub kind: TaskKind,
    pub model_id: String,
    pub room: TaskRoom,
    pub phase: String,
    pub overwrite_layout: bool,
    pub create_rls: bool,
    pub overwrite_rls: bool,
    pub update_mode: Option<UpdateMode>,
    pub dry_run: bool,
    pub blueprint: Option<BlueprintRef>,
    pub library_sheet_id: Option<String>,
    pub items: Vec<TaskItem>,
    pub sheets: Vec<BlueprintSheet>,
    pub tag_rules: Vec<RuleSummary>,
    pub findings: Vec<Value>,
}

/// One report from a Revit session about a task it was handed.
#[derive(Debug, Clone, Deserialize)]
pub struct Report {
    pub task_id: String,
    pub status: TaskState,
    #[serde(default)]
    pub messages: Vec<String>,
}

/// What reporting did with one task.
#[derive(Debug, Clone, Serialize)]
pub struct ReportOutcome {
    pub task_id: String,
    pub recorded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug)]
pub enum TaskError {
    NotFileBacked,
    Invalid(String),
    NotFound(String),
    /// The task is not in a state that allows this (for example resending one already queued).
    Conflict(String),
    Internal(anyhow::Error),
}

impl std::fmt::Display for TaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFileBacked => write!(f, "this server was not started from a settings directory"),
            Self::Invalid(m) | Self::NotFound(m) | Self::Conflict(m) => write!(f, "{m}"),
            Self::Internal(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TaskError {}

fn bad<T>(msg: String) -> Result<T, TaskError> {
    Err(TaskError::Invalid(msg))
}

fn internal<E: Into<anyhow::Error>>(e: E) -> TaskError {
    TaskError::Internal(e.into())
}

fn project_dir(projects_dir: &Path, project: &str) -> Result<PathBuf, TaskError> {
    if !is_path_safe_component(project) {
        return bad(format!("project id {project:?} cannot be a file name"));
    }
    Ok(projects_dir.join("tasks").join(project))
}

fn path_of(projects_dir: &Path, project: &str, id: &str) -> Result<PathBuf, TaskError> {
    if !is_path_safe_component(id) {
        return bad(format!("task id {id:?} cannot be a file name"));
    }
    Ok(project_dir(projects_dir, project)?.join(format!("{id}.json")))
}

fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

fn read(path: &Path) -> Result<Task, TaskError> {
    let bytes = std::fs::read(path).map_err(internal)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| TaskError::Internal(anyhow::anyhow!("{} is not a task: {e}", path.display())))
}

/// Every task of the project, oldest first. A file that cannot be read is an error and not a gap.
pub fn list_all(projects_dir: &Path, project: &str) -> Result<Vec<Task>, TaskError> {
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
        out.push(read(&path)?);
    }
    out.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
    Ok(out)
}

/// Summaries, optionally narrowed to one model and/or one state.
pub fn list(
    projects_dir: &Path,
    project: &str,
    model_id: Option<&str>,
    state: Option<TaskState>,
) -> Result<Vec<TaskSummary>, TaskError> {
    Ok(list_all(projects_dir, project)?
        .iter()
        .filter(|t| model_id.is_none_or(|m| t.model_id == m) && state.is_none_or(|s| t.state == s))
        .map(TaskSummary::from)
        .collect())
}

pub fn load(projects_dir: &Path, project: &str, id: &str) -> Result<Task, TaskError> {
    let path = path_of(projects_dir, project, id)?;
    if !path.exists() {
        return Err(TaskError::NotFound(format!("no task {id:?} in project {project:?}")));
    }
    read(&path)
}

fn write(projects_dir: &Path, project: &str, task: &Task) -> Result<(), TaskError> {
    let path = path_of(projects_dir, project, &task.id)?;
    let dir = path.parent().expect("a task path always has a parent");
    std::fs::create_dir_all(dir).map_err(internal)?;
    let json = serde_json::to_vec_pretty(task).map_err(internal)?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &json).map_err(internal)?;
    let stem = format!("task-{project}-{}", task.id);
    if path.exists()
        && let Err(e) = crate::backups::back_up(projects_dir, &stem, "json", &path)
    {
        std::fs::remove_file(&temp).ok();
        return Err(TaskError::Internal(e));
    }
    std::fs::rename(&temp, &path).map_err(internal)?;
    crate::backups::prune(projects_dir, &stem, "json");
    Ok(())
}

fn new_id(dir: &Path) -> String {
    loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let stamp = chrono::Utc::now().timestamp_micros();
        let id = format!("t{stamp:x}{n:x}");
        if !dir.join(format!("{id}.json")).exists() {
            return id;
        }
    }
}

/// Whether a task of this kind for this room is still waiting or being worked on: a second one
/// would be handed to Revit alongside the first.
fn is_live(t: &Task) -> bool {
    matches!(t.state, TaskState::Queued | TaskState::InProgress)
}

/// Queue a task. **Refused when the same kind of task for the same room is already queued or in
/// progress**: two would be handed out together, and the second would find the room populated.
pub fn create(projects_dir: &Path, project: &str, new: NewTask) -> Result<Task, TaskError> {
    if new.phase.trim().is_empty() {
        return bad("a task needs the name of the phase to work in".to_string());
    }
    let _guard = SAVE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let existing = list_all(projects_dir, project)?;
    if existing.len() >= MAX_TASKS_PER_PROJECT {
        return bad(format!("a project holds at most {MAX_TASKS_PER_PROJECT} tasks"));
    }
    if let Some(t) = existing
        .iter()
        .find(|t| is_live(t) && t.kind == new.kind && t.model_id == new.model_id && t.room.room_id == new.room.room_id)
    {
        return Err(TaskError::Conflict(format!(
            "room {:?} already has a {} task ({}); dismiss it or wait for it first",
            new.room.name,
            serde_json::to_value(t.state)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default(),
            t.id
        )));
    }
    let stamp = now();
    let task = Task {
        schema_version: SCHEMA_VERSION,
        id: new_id(&project_dir(projects_dir, project)?),
        kind: new.kind,
        state: TaskState::Queued,
        model_id: new.model_id,
        room: new.room,
        phase: new.phase.trim().to_string(),
        overwrite_layout: new.overwrite_layout,
        create_rls: new.create_rls,
        overwrite_rls: new.overwrite_rls,
        update_mode: new.update_mode,
        dry_run: new.dry_run,
        blueprint: new.blueprint,
        library_sheet_id: new.library_sheet_id,
        items: new.items,
        sheets: new.sheets,
        tag_rules: new.tag_rules,
        findings: new.findings,
        created_at: stamp.clone(),
        updated_at: stamp,
        attempts: Vec::new(),
    };
    write(projects_dir, project, &task)?;
    tracing::info!(
        "queued {:?} task {} for room {:?} of model {:?}",
        task.kind,
        task.id,
        task.room.name,
        task.model_id
    );
    Ok(task)
}

/// Hand a Revit session every queued task of `model_id`, oldest first, marking each `in_progress`
/// so another session is not given it. The only call that moves tasks to `in_progress`.
pub fn claim(projects_dir: &Path, project: &str, model_id: &str) -> Result<Vec<Task>, TaskError> {
    if model_id.trim().is_empty() {
        return bad("name the model the tasks are for".to_string());
    }
    let _guard = SAVE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut claimed = Vec::new();
    for mut task in list_all(projects_dir, project)? {
        if task.state != TaskState::Queued || task.model_id != model_id {
            continue;
        }
        let stamp = now();
        task.state = TaskState::InProgress;
        task.updated_at = stamp.clone();
        task.attempts
            .push(Attempt { started_at: stamp, finished_at: None, status: None, messages: Vec::new() });
        write(projects_dir, project, &task)?;
        claimed.push(task);
    }
    Ok(claimed)
}

fn clean_messages(messages: Vec<String>) -> Vec<String> {
    messages
        .into_iter()
        .take(MAX_MESSAGES)
        .map(|m| m.chars().take(MAX_MESSAGE_CHARS).collect())
        .collect()
}

/// Record what a Revit session says happened to the tasks it was handed. Each report stands on
/// its own: one that cannot be recorded (a task that is not in progress, a status that is not a
/// result) is a row saying why, and the rest are still recorded.
pub fn report(projects_dir: &Path, project: &str, reports: Vec<Report>) -> Result<Vec<ReportOutcome>, TaskError> {
    let _guard = SAVE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut out = Vec::new();
    for r in reports {
        let result = record_one(projects_dir, project, &r);
        out.push(ReportOutcome {
            task_id: r.task_id,
            recorded: result.is_ok(),
            error: result.err().map(|e| e.to_string()),
        });
    }
    Ok(out)
}

fn record_one(projects_dir: &Path, project: &str, r: &Report) -> Result<(), TaskError> {
    if !r.status.is_report() {
        return bad(format!("{:?} is not a result a session can report", r.status));
    }
    let mut task = load(projects_dir, project, &r.task_id)?;
    if task.state != TaskState::InProgress {
        return Err(TaskError::Conflict(
            "the task is not in progress (it may have been dismissed or resent); this report was not recorded"
                .to_string(),
        ));
    }
    let stamp = now();
    task.state = r.status;
    task.updated_at = stamp.clone();
    if let Some(a) = task.attempts.last_mut() {
        a.finished_at = Some(stamp);
        a.status = Some(r.status);
        a.messages = clean_messages(r.messages.clone());
    }
    write(projects_dir, project, &task)
}

/// A person drops a task. Allowed from any state but already dismissed, including `in_progress`
/// (a session that will never report); a late report for it is then refused.
pub fn dismiss(projects_dir: &Path, project: &str, id: &str) -> Result<Task, TaskError> {
    let _guard = SAVE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut task = load(projects_dir, project, id)?;
    if task.state == TaskState::Dismissed {
        return Err(TaskError::Conflict("the task is already dismissed".to_string()));
    }
    task.state = TaskState::Dismissed;
    task.updated_at = now();
    write(projects_dir, project, &task)?;
    Ok(task)
}

/// A person puts a task back in the queue. Allowed from any state but `queued`. Refused when the
/// same kind of task for the room is live already, for the reason `create` is.
pub fn resend(projects_dir: &Path, project: &str, id: &str) -> Result<Task, TaskError> {
    let _guard = SAVE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut task = load(projects_dir, project, id)?;
    if task.state == TaskState::Queued {
        return Err(TaskError::Conflict("the task is already queued".to_string()));
    }
    let clash = list_all(projects_dir, project)?.into_iter().any(|t| {
        t.id != task.id
            && is_live(&t)
            && t.kind == task.kind
            && t.model_id == task.model_id
            && t.room.room_id == task.room.room_id
    });
    if clash {
        return Err(TaskError::Conflict(
            "another task of this kind for the room is queued or in progress".to_string(),
        ));
    }
    task.state = TaskState::Queued;
    task.updated_at = now();
    write(projects_dir, project, &task)?;
    Ok(task)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roommate-tasks-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn new_task(model: &str, room: &str) -> NewTask {
        NewTask {
            kind: TaskKind::PopulateRoom,
            model_id: model.to_string(),
            room: TaskRoom {
                room_id: room.to_string(),
                number: Some("1.01".to_string()),
                name: format!("Room {room}"),
            },
            phase: "New Construction".to_string(),
            overwrite_layout: false,
            create_rls: false,
            overwrite_rls: false,
            update_mode: None,
            dry_run: false,
            blueprint: None,
            library_sheet_id: None,
            items: vec![],
            sheets: vec![],
            tag_rules: vec![],
            findings: vec![],
        }
    }

    fn report_of(id: &str, status: TaskState) -> Report {
        Report { task_id: id.to_string(), status, messages: vec!["done".to_string()] }
    }

    #[test]
    fn test_a_session_is_handed_its_models_queued_tasks_once() {
        let dir = temp_dir("claim");
        let a = create(&dir, "p", new_task("m1", "r1")).unwrap();
        create(&dir, "p", new_task("m2", "r2")).unwrap();
        let got = claim(&dir, "p", "m1").unwrap();
        assert_eq!(got.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec![a.id.as_str()]);
        assert_eq!(got[0].state, TaskState::InProgress);
        assert!(claim(&dir, "p", "m1").unwrap().is_empty(), "a second session is not handed it again");
        assert_eq!(claim(&dir, "p", "m2").unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_report_records_the_result_and_only_for_a_task_in_progress() {
        let dir = temp_dir("report");
        let t = create(&dir, "p", new_task("m1", "r1")).unwrap();
        let early = report(&dir, "p", vec![report_of(&t.id, TaskState::Completed)]).unwrap();
        assert!(!early[0].recorded, "not handed out yet");
        claim(&dir, "p", "m1").unwrap();
        let bad_status = report(&dir, "p", vec![report_of(&t.id, TaskState::Queued)]).unwrap();
        assert!(!bad_status[0].recorded);
        let ok = report(
            &dir,
            "p",
            vec![
                report_of(&t.id, TaskState::CompletedWithWarnings),
                report_of("nope", TaskState::Failed),
            ],
        )
        .unwrap();
        assert!(ok[0].recorded && !ok[1].recorded, "one bad report does not cost the others");
        let after = load(&dir, "p", &t.id).unwrap();
        assert_eq!(after.state, TaskState::CompletedWithWarnings);
        assert_eq!(after.attempts.len(), 1);
        assert_eq!(after.attempts[0].messages, vec!["done".to_string()]);
        assert!(after.attempts[0].finished_at.is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_dismiss_and_resend_move_a_task_and_keep_its_attempts() {
        let dir = temp_dir("resend");
        let t = create(&dir, "p", new_task("m1", "r1")).unwrap();
        claim(&dir, "p", "m1").unwrap();
        // A session that died holding it: a person puts it back.
        assert_eq!(resend(&dir, "p", &t.id).unwrap().state, TaskState::Queued);
        assert!(matches!(resend(&dir, "p", &t.id), Err(TaskError::Conflict(_))), "already queued");
        claim(&dir, "p", "m1").unwrap();
        report(&dir, "p", vec![report_of(&t.id, TaskState::Failed)]).unwrap();
        assert_eq!(dismiss(&dir, "p", &t.id).unwrap().state, TaskState::Dismissed);
        assert!(matches!(dismiss(&dir, "p", &t.id), Err(TaskError::Conflict(_))));
        assert_eq!(load(&dir, "p", &t.id).unwrap().attempts.len(), 2, "attempts are kept");
        // A late report for a dismissed task is refused.
        assert!(!report(&dir, "p", vec![report_of(&t.id, TaskState::Completed)]).unwrap()[0].recorded);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_a_second_live_task_for_the_same_room_is_refused() {
        let dir = temp_dir("clash");
        let first = create(&dir, "p", new_task("m1", "r1")).unwrap();
        assert!(matches!(create(&dir, "p", new_task("m1", "r1")), Err(TaskError::Conflict(_))));
        create(&dir, "p", new_task("m1", "r2")).unwrap();
        dismiss(&dir, "p", &first.id).unwrap();
        let again = create(&dir, "p", new_task("m1", "r1")).unwrap();
        // The dismissed one cannot be resent while a live one exists for the room.
        assert!(matches!(resend(&dir, "p", &first.id), Err(TaskError::Conflict(_))));
        assert_ne!(first.id, again.id);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_the_list_narrows_by_model_and_state_and_a_task_needs_a_phase() {
        let dir = temp_dir("list");
        create(&dir, "p", new_task("m1", "r1")).unwrap();
        create(&dir, "p", new_task("m2", "r2")).unwrap();
        claim(&dir, "p", "m2").unwrap();
        assert_eq!(list(&dir, "p", Some("m1"), None).unwrap().len(), 1);
        assert_eq!(list(&dir, "p", None, Some(TaskState::InProgress)).unwrap().len(), 1);
        assert_eq!(list(&dir, "p", None, None).unwrap().len(), 2);
        let mut blank = new_task("m1", "r3");
        blank.phase = "  ".to_string();
        assert!(matches!(create(&dir, "p", blank), Err(TaskError::Invalid(_))));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_an_update_task_keeps_its_mode_and_is_a_separate_live_task_from_a_populate() {
        let dir = temp_dir("update");
        create(&dir, "p", new_task("m1", "r1")).unwrap();
        let mut update = new_task("m1", "r1");
        update.kind = TaskKind::UpdateRoom;
        update.update_mode = Some(UpdateMode::Locations);
        update.dry_run = true;
        let made = create(&dir, "p", update.clone()).expect("a different kind for the same room is allowed");
        assert_eq!(made.update_mode, Some(UpdateMode::Locations));
        assert!(made.dry_run);
        let again = create(&dir, "p", update);
        assert!(matches!(again, Err(TaskError::Conflict(_))), "{again:?}");
        let read = load(&dir, "p", &made.id).unwrap();
        assert_eq!(read.update_mode, Some(UpdateMode::Locations));
        assert_eq!(TaskSummary::from(&read).update_mode, Some(UpdateMode::Locations));
    }
}
