//! The sheet library's files: one JSON per sheet under one lock, written atomically, with the lists and loads the rest of the server
//! reads (`load_in_use`, `bound_to`, `unbind_missing`). The record's types and what identity means are in `mod.rs`.

use super::*;

pub(super) static SAVE_LOCK: Mutex<()> = Mutex::new(());

pub(super) static COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) fn bad<T>(msg: String) -> Result<T, SheetError> {
    Err(SheetError::Invalid(msg))
}

pub(super) fn internal<E: Into<anyhow::Error>>(e: E) -> SheetError {
    SheetError::Internal(e.into())
}

pub(super) fn project_dir(projects_dir: &Path, project: &str) -> Result<PathBuf, SheetError> {
    if !is_path_safe_component(project) {
        return bad(format!("project id {project:?} cannot be a file name"));
    }
    Ok(projects_dir.join("sheets").join(project))
}

pub(super) fn sheet_path(projects_dir: &Path, project: &str, id: &str) -> Result<PathBuf, SheetError> {
    if !is_path_safe_component(id) {
        return bad(format!("sheet id {id:?} cannot be a file name"));
    }
    Ok(project_dir(projects_dir, project)?.join(format!("{id}.json")))
}

pub(super) fn read(path: &Path) -> Result<SheetRecord, SheetError> {
    let bytes = std::fs::read(path).map_err(internal)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| SheetError::Internal(anyhow::anyhow!("{} is not a sheet record: {e}", path.display())))
}

/// [`load_all`] without the sheets bound to a blueprint that is NOT IN USE: what a person is offered when choosing a
/// sheet for a room (suggest, solve, the set preview and the set task). One function, so the HTTP routes and the MCP
/// tools offer the same library. A blueprint list that cannot be read hides nothing.
pub fn load_in_use(projects_dir: &Path, project: &str) -> Result<Vec<SheetRecord>, SheetError> {
    let mut records = load_all(projects_dir, project)?;
    let retired: std::collections::HashSet<String> = crate::blueprints::list(projects_dir, project)
        .unwrap_or_default()
        .into_iter()
        .filter(|b| !b.in_use)
        .map(|b| b.id)
        .collect();
    records.retain(|r| r.blueprint.as_deref().is_none_or(|b| !retired.contains(b)));
    Ok(records)
}

/// Every record of the project, by sheet number then document. A file that cannot be read is
/// an error, not a gap: skipping it would make a corrupt sheet look like a deleted one.
pub fn load_all(projects_dir: &Path, project: &str) -> Result<Vec<SheetRecord>, SheetError> {
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
    out.sort_by(|a, b| {
        (a.sheet_number.to_lowercase(), &a.document_title, &a.id).cmp(&(
            b.sheet_number.to_lowercase(),
            &b.document_title,
            &b.id,
        ))
    });
    Ok(out)
}

/// Unbind the sheets bound to `blueprint` that a re-push of it no longer carries: they stay in the
/// library as generic sheets (and get their `room_key`), nothing is deleted. `keep` is the identity
/// (Revit sheet id, document title) of every sheet the push carried. Returns how many were unbound.
pub fn unbind_missing(
    projects_dir: &Path,
    project: &str,
    blueprint: &str,
    keep: &[(String, String)],
) -> Result<usize, SheetError> {
    let _guard = SAVE_LOCK.lock().unwrap();
    let mut changed = 0;
    for mut record in load_all(projects_dir, project)? {
        let carried = keep.iter().any(|(id, doc)| *id == record.revit_sheet_id && *doc == record.document_title);
        if record.blueprint.as_deref() == Some(blueprint) && !carried {
            record.blueprint = None;
            settle_room_key(&mut record);
            record.taken_at = next_stamp(&record.taken_at);
            write(projects_dir, project, &record)?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// The sheets bound to a room blueprint (plan decision 27), in number order.
pub fn bound_to(projects_dir: &Path, project: &str, blueprint: &str) -> Result<Vec<SheetRecord>, SheetError> {
    let mut out: Vec<SheetRecord> = load_all(projects_dir, project)?
        .into_iter()
        .filter(|r| r.blueprint.as_deref() == Some(blueprint))
        .collect();
    out.sort_by(|a, b| a.sheet_number.cmp(&b.sheet_number));
    Ok(out)
}

pub fn list(projects_dir: &Path, project: &str) -> Result<Vec<SheetSummary>, SheetError> {
    Ok(load_all(projects_dir, project)?.iter().map(SheetSummary::from).collect())
}

pub fn load(projects_dir: &Path, project: &str, id: &str) -> Result<SheetRecord, SheetError> {
    let path = sheet_path(projects_dir, project, id)?;
    match std::fs::read(&path) {
        Ok(_) => read(&path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(SheetError::NotFound(format!("no sheet {id:?} in project {project:?}")))
        }
        Err(e) => Err(internal(e)),
    }
}

pub(super) fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

pub(super) fn next_stamp(previous: &str) -> String {
    let stamp = now();
    if stamp.as_str() <= previous {
        format!("{previous}~")
    } else {
        stamp
    }
}

pub(super) fn new_id(dir: &Path) -> String {
    loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let id = format!("sh-{:x}-{n:x}", chrono::Utc::now().timestamp_micros());
        if !dir.join(format!("{id}.json")).exists() {
            return id;
        }
    }
}

pub(super) fn conflict(base: &str, current: &str) -> SheetError {
    SheetError::Conflict(format!(
        "the sheet changed since you read it (yours is from {base:?}, the server has {current:?}); reload and try again"
    ))
}

/// Same order as `blueprints::write`: everything that can refuse runs before this, the old file
/// is copied aside while it is still in place, and the new one is installed by rename.
pub(super) fn write(projects_dir: &Path, project: &str, record: &SheetRecord) -> Result<(), SheetError> {
    let path = sheet_path(projects_dir, project, &record.id)?;
    let dir = path.parent().expect("a sheet path always has a parent");
    std::fs::create_dir_all(dir).map_err(internal)?;
    let json = serde_json::to_vec_pretty(record).map_err(internal)?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &json).map_err(internal)?;
    let stem = format!("sheet-{project}-{}", record.id);
    if path.exists()
        && let Err(e) = crate::backups::back_up(projects_dir, &stem, "json", &path)
    {
        std::fs::remove_file(&temp).ok();
        return Err(SheetError::Internal(e));
    }
    std::fs::rename(&temp, &path).map_err(internal)?;
    crate::backups::prune(projects_dir, &stem, "json");
    Ok(())
}
