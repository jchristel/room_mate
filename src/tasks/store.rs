//! The task store: one JSON file per task, listed, loaded, written and created under one lock, and the states a task moves
//! through (`claim`, `report`, `dismiss`, `resend`). The types and the states' meaning are in `mod.rs`.

use super::*;

pub(super) fn bad<T>(msg: String) -> Result<T, TaskError> {
    Err(TaskError::Invalid(msg))
}

pub(super) fn internal<E: Into<anyhow::Error>>(e: E) -> TaskError {
    TaskError::Internal(e.into())
}

pub(super) fn project_dir(projects_dir: &Path, project: &str) -> Result<PathBuf, TaskError> {
    if !is_path_safe_component(project) {
        return bad(format!("project id {project:?} cannot be a file name"));
    }
    Ok(projects_dir.join("tasks").join(project))
}

pub(super) fn path_of(projects_dir: &Path, project: &str, id: &str) -> Result<PathBuf, TaskError> {
    if !is_path_safe_component(id) {
        return bad(format!("task id {id:?} cannot be a file name"));
    }
    Ok(project_dir(projects_dir, project)?.join(format!("{id}.json")))
}

pub(super) fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

pub(super) fn read(path: &Path) -> Result<Task, TaskError> {
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

pub(super) fn write(projects_dir: &Path, project: &str, task: &Task) -> Result<(), TaskError> {
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

pub(super) fn new_id(dir: &Path) -> String {
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
pub(super) fn is_live(t: &Task) -> bool {
    matches!(t.state, TaskState::Queued | TaskState::InProgress)
}

/// The rooms a task works on: a set's every room, else its one room.
pub(super) fn rooms_of<'a>(room: &'a TaskRoom, set: Option<&'a RlsSetPlan>) -> Vec<&'a str> {
    match set {
        Some(s) if !s.rooms.is_empty() => s.rooms.iter().map(|r| r.room_id.as_str()).collect(),
        _ => vec![room.room_id.as_str()],
    }
}

/// Whether two tasks of one kind and model would work on a room in common. For sets that is any shared room: two
/// sets holding one room would each make its elevations.
pub(super) fn overlaps(t: &Task, kind: TaskKind, model_id: &str, rooms: &[&str]) -> bool {
    t.kind == kind && t.model_id == model_id && rooms_of(&t.room, t.rls_set.as_ref()).iter().any(|r| rooms.contains(r))
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
    let rooms = rooms_of(&new.room, new.rls_set.as_ref());
    if let Some(t) = existing.iter().find(|t| is_live(t) && overlaps(t, new.kind, &new.model_id, &rooms)) {
        // name the room the two have in common, which for a set need not be either one's lead
        let held = rooms_of(&t.room, t.rls_set.as_ref());
        let shared = rooms.iter().find(|r| held.contains(r)).copied().unwrap_or(new.room.room_id.as_str());
        let name = new
            .rls_set
            .as_ref()
            .and_then(|s| s.rooms.iter().find(|r| r.room_id == shared))
            .map_or(new.room.name.as_str(), |r| r.name.as_str());
        return Err(TaskError::Conflict(format!(
            "room {:?} already has a {} task ({}); dismiss it or wait for it first",
            name,
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
        rls: new.rls,
        rls_set: new.rls_set,
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
///
/// `kinds` narrows what is handed over: the RLS script asks only for `create_rls`, so a populate task
/// queued for the same model is left for the button that does that. Empty means every kind.
pub fn claim(projects_dir: &Path, project: &str, model_id: &str, kinds: &[TaskKind]) -> Result<Vec<Task>, TaskError> {
    claim_chosen(projects_dir, project, model_id, kinds, &[])
}

/// `claim`, narrowed further to the tasks a person ticked: `ids` empty means all of them, so a session
/// that does not choose is unchanged. An id that is not queued for this model is simply not handed over
/// (another session may have taken it, or it was dismissed since the list was read).
pub fn claim_chosen(
    projects_dir: &Path,
    project: &str,
    model_id: &str,
    kinds: &[TaskKind],
    ids: &[String],
) -> Result<Vec<Task>, TaskError> {
    if model_id.trim().is_empty() {
        return bad("name the model the tasks are for".to_string());
    }
    let _guard = SAVE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut claimed = Vec::new();
    for mut task in list_all(projects_dir, project)? {
        if task.state != TaskState::Queued
            || task.model_id != model_id
            || !(kinds.is_empty() || kinds.contains(&task.kind))
            || !(ids.is_empty() || ids.contains(&task.id))
        {
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

pub(super) fn clean_messages(messages: Vec<String>) -> Vec<String> {
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

pub(super) fn record_one(projects_dir: &Path, project: &str, r: &Report) -> Result<(), TaskError> {
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
    let rooms = rooms_of(&task.room, task.rls_set.as_ref());
    let clash = list_all(projects_dir, project)?
        .into_iter()
        .any(|t| t.id != task.id && is_live(&t) && overlaps(&t, task.kind, &task.model_id, &rooms));
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
