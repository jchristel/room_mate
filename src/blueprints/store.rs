//! A blueprint's file: one JSON per blueprint under one lock, written atomically (and the replaced copy kept under `.backups/`),
//! with the listing and the loads the rest of the server uses.

use super::*;

/// Serialises every write, so the read-compare-write below cannot interleave.
pub(super) static SAVE_LOCK: Mutex<()> = Mutex::new(());

pub(super) static COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) fn bad<T>(msg: String) -> Result<T, BlueprintError> {
    Err(BlueprintError::Invalid(msg))
}

pub(super) fn project_dir(projects_dir: &Path, project: &str) -> Result<PathBuf, BlueprintError> {
    if !is_path_safe_component(project) {
        return bad(format!("project id {project:?} cannot be a file name"));
    }
    Ok(projects_dir.join("blueprints").join(project))
}

pub(super) fn blueprint_path(projects_dir: &Path, project: &str, id: &str) -> Result<PathBuf, BlueprintError> {
    if !is_path_safe_component(id) {
        return bad(format!("blueprint id {id:?} cannot be a file name"));
    }
    Ok(project_dir(projects_dir, project)?.join(format!("{id}.json")))
}

pub(super) fn internal<E: Into<anyhow::Error>>(e: E) -> BlueprintError {
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

pub(super) fn read(path: &Path) -> Result<Blueprint, BlueprintError> {
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

pub(super) fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

/// A version stamp that sorts after the one it replaces, even on a clock that did not move.
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
pub(super) fn write(projects_dir: &Path, project: &str, blueprint: &Blueprint) -> Result<(), BlueprintError> {
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

pub(super) fn conflict(base: &str, current: &str) -> BlueprintError {
    BlueprintError::Conflict(format!(
        "the blueprint changed since you read it (yours is from {base:?}, the server has {current:?}); reload and try again"
    ))
}
