//! Copies of the files a hostile or mistaken save would otherwise overwrite.
//!
//! Shared by every mutable document that lives beside the project settings
//! (the settings themselves and the authored connections), so there is one rule
//! for where a prior version goes and how many are kept. See
//! `docs/STRATEGY-SECURITY.md`, "Settings backups".

use std::path::{Path, PathBuf};

/// Directory under the projects dir holding prior versions. A subdirectory, so
/// the settings loader (which reads `*.toml` directly inside the projects dir)
/// can never mistake a backup for a live project.
pub const BACKUP_DIR: &str = ".backups";

/// Backups kept per document; older ones are pruned after a successful install.
pub const BACKUPS_KEPT: usize = 20;

/// Length of the timestamp in a backup name, `20260101T000000.000000Z`.
const STAMP_LEN: usize = 23;

fn name_of(path: &Path) -> String {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string()
}

/// Copy `target` to `.backups/<stem>.<utc-stamp>.<ext>`. A copy, never a move:
/// the live file stays until the caller's rename installs its replacement. The
/// stamp is UTC and sorts lexically like a snapshot id, but is not RFC 3339 — a
/// colon is not a legal file-name character on Windows. Never overwrites: a name
/// collision is an error rather than a silent loss of the older backup.
pub fn back_up(projects_dir: &Path, stem: &str, ext: &str, target: &Path) -> anyhow::Result<()> {
    let dir = projects_dir.join(BACKUP_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| anyhow::anyhow!("could not create {}: {e}", dir.display()))?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.6fZ");
    let backup = dir.join(format!("{stem}.{stamp}.{ext}"));
    if backup.exists() {
        anyhow::bail!("backup {} already exists", backup.display());
    }
    std::fs::copy(target, &backup).map_err(|e| anyhow::anyhow!("could not back up {}: {e}", name_of(target)))?;
    Ok(())
}

/// Keep the newest `BACKUPS_KEPT` backups of one document. Best effort and run
/// after the install: failing to prune costs disk, never a save. A name only
/// counts as this document's when what follows `<stem>.` is exactly a stamp, so
/// project `a` never prunes project `a.b`'s history.
pub fn prune(projects_dir: &Path, stem: &str, ext: &str) {
    let dir = projects_dir.join(BACKUP_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let prefix = format!("{stem}.");
    let suffix = format!(".{ext}");
    let mut mine: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let name = name_of(p);
            name.strip_prefix(&prefix)
                .and_then(|rest| rest.strip_suffix(suffix.as_str()))
                .is_some_and(|stamp| stamp.len() == STAMP_LEN && stamp.ends_with('Z'))
        })
        .collect();
    mine.sort();
    let excess = mine.len().saturating_sub(BACKUPS_KEPT);
    for old in mine.into_iter().take(excess) {
        if let Err(e) = std::fs::remove_file(&old) {
            tracing::warn!("could not prune backup {}: {e}", old.display());
        }
    }
}
