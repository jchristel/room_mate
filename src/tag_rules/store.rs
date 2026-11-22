//! The rule files: one JSON per rule under one lock, written atomically, with the lists, loads and removals the rest of the server uses.

use super::*;

pub(crate) static SAVE_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn bad<T>(msg: String) -> Result<T, TagRuleError> {
    Err(TagRuleError::Invalid(msg))
}

pub(super) fn internal<E: Into<anyhow::Error>>(e: E) -> TagRuleError {
    TagRuleError::Internal(e.into())
}

pub(super) fn project_dir(projects_dir: &Path, project: &str) -> Result<PathBuf, TagRuleError> {
    if !is_path_safe_component(project) {
        return bad(format!("project id {project:?} cannot be a file name"));
    }
    Ok(projects_dir.join("tagrules").join(project))
}

pub(super) fn rule_path(projects_dir: &Path, project: &str, id: &str) -> Result<PathBuf, TagRuleError> {
    if !is_path_safe_component(id) {
        return bad(format!("rule id {id:?} cannot be a file name"));
    }
    Ok(project_dir(projects_dir, project)?.join(format!("{id}.json")))
}

pub(super) fn read(path: &Path) -> Result<TagRule, TagRuleError> {
    let bytes = std::fs::read(path).map_err(internal)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| TagRuleError::Internal(anyhow::anyhow!("{} is not a tag rule: {e}", path.display())))
}

/// Every rule of the project, by key. A file that cannot be read is an error, not a gap; a file of
/// an older schema is left on disk and not loaded, and said once in the log.
pub fn load_all(projects_dir: &Path, project: &str) -> Result<Vec<TagRule>, TagRuleError> {
    let dir = project_dir(projects_dir, project)?;
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(internal(e)),
    };
    let mut out = Vec::new();
    let mut old = 0usize;
    for entry in entries {
        let path = entry.map_err(internal)?.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let rule = read(&path)?;
        if rule.schema_version < SCHEMA_VERSION {
            old += 1;
        } else {
            out.push(rule);
        }
    }
    if old > 0 {
        tracing::warn!("{old} tag rule(s) of project {project:?} are keyed by family and type names and are not used; push the sheets again to learn them by type code");
    }
    out.sort_by_key(|r| r.key.canonical());
    Ok(out)
}

pub fn list(projects_dir: &Path, project: &str) -> Result<Vec<RuleSummary>, TagRuleError> {
    Ok(load_all(projects_dir, project)?.iter().map(RuleSummary::from).collect())
}

pub fn load(projects_dir: &Path, project: &str, id: &str) -> Result<TagRule, TagRuleError> {
    let path = rule_path(projects_dir, project, id)?;
    match std::fs::read(&path) {
        Ok(_) => read(&path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(TagRuleError::NotFound(format!("no tag rule {id:?} in project {project:?}")))
        }
        Err(e) => Err(internal(e)),
    }
}

pub(crate) fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

pub(crate) fn write(projects_dir: &Path, project: &str, rule: &TagRule) -> Result<(), TagRuleError> {
    let path = rule_path(projects_dir, project, &rule.id)?;
    let dir = path.parent().expect("a rule path always has a parent");
    std::fs::create_dir_all(dir).map_err(internal)?;
    let json = serde_json::to_vec_pretty(rule).map_err(internal)?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &json).map_err(internal)?;
    let stem = format!("tagrule-{project}-{}", rule.id);
    if path.exists()
        && let Err(e) = crate::backups::back_up(projects_dir, &stem, "json", &path)
    {
        std::fs::remove_file(&temp).ok();
        return Err(TagRuleError::Internal(e));
    }
    std::fs::rename(&temp, &path).map_err(internal)?;
    crate::backups::prune(projects_dir, &stem, "json");
    Ok(())
}

pub fn delete(projects_dir: &Path, project: &str, id: &str) -> Result<(), TagRuleError> {
    let _guard = SAVE_LOCK.lock().unwrap();
    remove(projects_dir, project, id)
}

/// `delete` for a caller already holding the lock.
pub(crate) fn remove(projects_dir: &Path, project: &str, id: &str) -> Result<(), TagRuleError> {
    let path = rule_path(projects_dir, project, id)?;
    if !path.exists() {
        return Err(TagRuleError::NotFound(format!("no tag rule {id:?} in project {project:?}")));
    }
    let stem = format!("tagrule-{project}-{id}");
    crate::backups::back_up(projects_dir, &stem, "json", &path).map_err(TagRuleError::Internal)?;
    std::fs::remove_file(&path).map_err(internal)
}

// ---- from an observation to samples -----------------------------------------------------------
