//! Settings file paths -> a running `Shared` state. The one place that knows
//! how to turn a server-settings file plus a directory of per-project
//! settings files into a live `AppState`: load each project's settings, load
//! each of its configured reference sources' data, validate their declared
//! fields against it, register it under its project id, pick the storage
//! backend, and seed dev/test data.
//! Shared verbatim by both binaries (`main.rs`'s HTTP server and
//! `bin/mcp.rs`'s MCP server) so they can't drift on this wiring -- a change
//! to how the store backend is chosen, for instance, only has one call site
//! to update.
//!
//! Settings are one-per-project rather than one-per-process, while `[storage]`/`[test_data]` (server-wide, not tied
//! to any one project) stayed behind in their own `ServerConfig` file.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context;

use crate::reference::{load_reference_from_bytes, load_reference_from_path};
use crate::service::rooms::validate_comparison_field;
use crate::settings::{
    load_server_config, load_settings, validate_reference_field_shapes, validate_reference_fields,
    ReferenceOrigin, ServerConfig,
};
use crate::state::{seed_if_test, AppState, ProjectReferenceSource, ProjectSettings, Shared};
use crate::storage::{FsStore, MemStore, SnapshotStore};

/// Load and fully validate ONE project settings file into its runtime
/// bundle: parse TOML, load each configured reference source's data (a
/// `file` source reads its CSV path; an `upload` source hydrates the latest
/// stored CSV from the snapshot store — which is why the store is a
/// parameter), validate each source's declared fields against it. This is
/// the single validation pipeline for a project file — startup
/// (`load_project_settings_dir`) and the settings API's save both run
/// exactly this, so a file the UI accepts can never fail the next boot.
pub fn load_project_bundle(path: &Path, store: &dyn SnapshotStore) -> anyhow::Result<(String, bool, ProjectSettings)> {
    let settings = load_settings(path).with_context(|| format!("bad settings file: {}", path.display()))?;

    // Comparison fields: the namespace half is checkable right here, and a bad
    // one left unchecked yields an empty milestone diff indistinguishable from
    // "no changes" — the silent no-op this loud failure replaces. Lives here
    // rather than in `load_settings` because the vocabulary belongs to
    // `service::rooms` (settings must not depend on service); running inside
    // this function is also what gives the settings-save path the same
    // rejection for free. Unqualified names stay unvalidated — free-text room
    // properties may legitimately match nothing yet.
    //
    // Validated against THIS project's own configured source names, not a
    // global cross-project union: a comparison is always scoped to one
    // project (`compare_milestones(state, project, ..)`), so a
    // `comparison_key` naming a source only some OTHER project configures
    // could never resolve here anyway — the tighter, project-local check is
    // both simpler (no chicken-and-egg with the rest of the directory still
    // loading) and the semantically correct one. Contrast the live `/rooms`
    // filter (`handlers::get_rooms`), which spans projects unscoped and so
    // must use the registry-wide union instead (see
    // `SettingsRegistry::known_reference_sources`).
    let known_here: std::collections::BTreeSet<String> = settings.sources.reference.keys().cloned().collect();
    for (which, field) in settings
        .comparison_key
        .iter()
        .map(|f| ("comparison_key", f))
        .chain(settings.comparison_properties.iter().map(|f| ("comparison_properties", f)))
    {
        validate_comparison_field(field, &known_here)
            .map_err(|msg| anyhow::anyhow!("bad {which} entry {field:?} in {}: {msg}", path.display()))?;
    }

    // A source name that collides with a room's own wire field would
    // silently overwrite it: `RoomResponse.reference` (service::rooms) is
    // `#[serde(flatten)]`ed onto the response precisely so today's one
    // source ("drofus") costs no wire change, but that means a source
    // literally named "id" or "properties" would shadow the room's real
    // field on every response instead of adding one. Checked here, not in
    // `load_settings`, because the reserved vocabulary is `service::rooms`'s
    // to define and settings must not depend on service.
    const RESERVED_REFERENCE_NAMES: &[&str] =
        &["id", "name", "level_id", "loops", "properties", "classification", "label"];
    for name in settings.sources.reference.keys() {
        if RESERVED_REFERENCE_NAMES.contains(&name.as_str()) {
            anyhow::bail!(
                "{}: [sources.reference.{name}] uses a reserved name — it would shadow the room's own '{name}' field on every /rooms response",
                path.display()
            );
        }
    }

    // Every configured reference source is loaded and validated the same way:
    // a `file` source reads its CSV path; an `upload` source hydrates the
    // latest stored CSV for that (project, source) pair from the snapshot
    // store (which is why the store is a parameter) — absent upload data is a
    // legitimate "not configured yet" state, not a startup error. Each
    // source's `fields` list lives on its own settings entry
    // (`[sources.reference.<name>].fields`), so there is no way to declare
    // fields with no source to attach them to.
    let mut reference = std::collections::BTreeMap::new();
    for (name, source_cfg) in &settings.sources.reference {
        let data = match &source_cfg.origin {
            ReferenceOrigin::File { path: csv_path } => {
                let data = load_reference_from_path(csv_path)
                    .with_context(|| format!("bad '{name}' reference source in {}", path.display()))?;

                // Can't validate this inside `load_settings`: the CSV (and its
                // label set) isn't loaded until the line above, one step later.
                validate_reference_fields(&source_cfg.fields, &data.all_labels)
                    .with_context(|| format!("bad '{name}' fields in {}", path.display()))?;
                Some(data)
            }
            ReferenceOrigin::Upload => match store.get_latest_reference(&settings.project_id, name)? {
                Some((taken_at, bytes)) => {
                    // A stored CSV that fails to parse fails the boot loudly —
                    // same discipline as a rotted `file` CSV. The upload endpoint
                    // validates before storing, so this is only reachable by
                    // hand-editing the store.
                    let data = load_reference_from_bytes(&bytes).with_context(|| {
                        format!(
                            "bad stored '{name}' upload {} for project '{}' (referenced by {})",
                            taken_at,
                            settings.project_id,
                            path.display()
                        )
                    })?;
                    validate_reference_fields(&source_cfg.fields, &data.all_labels)
                        .with_context(|| format!("bad '{name}' fields in {}", path.display()))?;
                    Some(data)
                }
                // No upload yet: a legitimate "not configured yet" state, not an
                // error. The label set is unknowable, so only the label-free half
                // of the field validation can run.
                None => {
                    validate_reference_field_shapes(&source_cfg.fields)
                        .with_context(|| format!("bad '{name}' fields in {}", path.display()))?;
                    None
                }
            },
        };
        reference.insert(name.clone(), ProjectReferenceSource { data, fields: source_cfg.fields.clone() });
    }

    let bundle = ProjectSettings {
        reference,
        hierarchy: settings.hierarchy,
        builtin_properties: settings.builtin_properties,
        room_label: settings.room_label,
        milestones: settings.milestones,
        comparison_key: settings.comparison_key,
        comparison_properties: settings.comparison_properties,
        areas: settings.areas,
        hierarchy_exclusions: settings.hierarchy_exclusions,
    };
    Ok((settings.project_id, settings.is_default, bundle))
}

/// Load and validate every `*.toml` file directly inside `projects_dir` (not
/// recursive) into a project-id-keyed registry, plus the explicit default
/// bundle if exactly one file sets `is_default = true`. Fails the whole
/// startup on: a malformed file, a duplicate `project_id` across files, or
/// more than one file claiming `is_default` -- same "loud startup error over
/// a silent no-op" discipline `load_settings` already uses for hierarchy
/// tiers and builtin properties. Also re-run by the settings API after a
/// save, to build the registry it hot-swaps in.
pub fn load_project_settings_dir(
    projects_dir: &Path,
    store: &dyn SnapshotStore,
) -> anyhow::Result<(HashMap<String, ProjectSettings>, Option<ProjectSettings>)> {
    let mut registry = HashMap::new();
    let mut default_bundle: Option<(String, ProjectSettings)> = None;

    let entries = std::fs::read_dir(projects_dir)
        .with_context(|| format!("could not read project settings directory: {}", projects_dir.display()))?;

    for entry in entries {
        let entry = entry.with_context(|| format!("could not read entry in {}", projects_dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue; // not a settings file (e.g. a stray drofus.csv sitting alongside)
        }

        let (project_id, is_default, bundle) = load_project_bundle(&path, store)?;
        tracing::info!("project settings loaded from {} (project_id = {})", path.display(), project_id);

        if is_default {
            if let Some((other_id, _)) = &default_bundle {
                anyhow::bail!(
                    "more than one project settings file sets is_default = true: '{}' and '{}'",
                    other_id,
                    project_id
                );
            }
            default_bundle = Some((project_id.clone(), bundle.clone()));
        }

        if registry.insert(project_id.clone(), bundle).is_some() {
            anyhow::bail!("duplicate project_id across settings files: '{}'", project_id);
        }
    }

    Ok((registry, default_bundle.map(|(_, b)| b)))
}

pub fn build_state(server_settings: &Path, projects_dir: &Path) -> anyhow::Result<Shared> {
    let ServerConfig { storage, test_data } = load_server_config(server_settings)
        .with_context(|| format!("bad server settings file: {}", server_settings.display()))?;
    tracing::info!("server settings loaded from {}", server_settings.display());

    // Pick the backend from config: a `[storage]` root → persistent FsStore,
    // otherwise the volatile MemStore (dev/test). Both satisfy SnapshotStore, so
    // this is the only line that knows which one is running. Constructed BEFORE
    // the project bundles load, because an `upload`-sourced project hydrates
    // its dRofus data from this store.
    let store: Box<dyn SnapshotStore> = match storage {
        Some(cfg) => {
            tracing::info!("persistent storage at {}", cfg.root.display());
            Box::new(FsStore::new(cfg.root)?)
        }
        None => {
            tracing::info!("no [storage] configured — using in-memory store");
            Box::new(MemStore::new())
        }
    };

    let (project_settings, default_settings) = load_project_settings_dir(projects_dir, store.as_ref())
        .with_context(|| format!("bad project settings directory: {}", projects_dir.display()))?;

    if project_settings.is_empty() && default_settings.is_none() {
        tracing::warn!("no project settings files found in {} -- every read/ingest will be rejected/skipped until one is added", projects_dir.display());
    }

    let state: Shared = Arc::new(
        AppState::new(store, project_settings, default_settings).with_projects_dir(projects_dir.to_path_buf()),
    );

    seed_if_test(&state, test_data.as_ref())?;

    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_projects_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("roommate-bootstrap-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A project with no `[sources]` at all registers with an empty
    /// `reference` map — the state `compute_project_validation` reports as
    /// `drofus_configured: false`.
    #[test]
    fn test_project_without_sources_registers_with_no_drofus() {
        let dir = temp_projects_dir("no-sources");
        std::fs::write(dir.join("p1.toml"), "project_id = \"p1\"\n").unwrap();

        let (registry, _default) = load_project_settings_dir(&dir, &MemStore::new()).unwrap();
        assert!(registry.get("p1").unwrap().reference.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    // A `drofus_fields`-without-a-source config mistake (the old
    // `test_drofus_fields_without_source_fails_startup`) is now structurally
    // unrepresentable: a source's fields live under its own
    // `[sources.reference.<name>]` entry, so there is no longer a way to
    // declare fields with no source to attach them to.

    /// An `upload`-sourced project with no upload yet registers with
    /// `drofus: None` — a legitimate not-configured-yet state — and its
    /// fields are accepted on their label-free shape checks alone.
    #[test]
    fn test_upload_source_with_empty_store_registers_without_drofus() {
        let dir = temp_projects_dir("upload-empty");
        std::fs::write(
            dir.join("p1.toml"),
            "project_id = \"p1\"\n\n[sources.reference.drofus]\ntype = \"upload\"\n\n[[sources.reference.drofus.fields]]\nlabel = \"NetArea\"\nqa = \"exact\"\n",
        )
        .unwrap();

        let (registry, _default) = load_project_settings_dir(&dir, &MemStore::new()).unwrap();
        let bundle = registry.get("p1").unwrap();
        let drofus_source = &bundle.reference["drofus"];
        assert!(drofus_source.data.is_none());
        assert_eq!(drofus_source.fields.len(), 1, "field declarations carried along for later");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// An `upload`-sourced project's shape checks still run with no data: a
    /// `date` field without a `format` is rejectable without knowing labels.
    #[test]
    fn test_upload_source_shape_validation_runs_without_data() {
        let dir = temp_projects_dir("upload-shape");
        std::fs::write(
            dir.join("p1.toml"),
            "project_id = \"p1\"\n\n[sources.reference.drofus]\ntype = \"upload\"\n\n[[sources.reference.drofus.fields]]\nlabel = \"Updated\"\ntype = \"date\"\n",
        )
        .unwrap();

        let msg = match load_project_settings_dir(&dir, &MemStore::new()) {
            Err(err) => format!("{err:#}"),
            Ok(_) => panic!("expected failure: date field with no format"),
        };
        assert!(msg.contains("format"), "message names the problem: {msg}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// An `upload`-sourced project with a stored CSV hydrates it as its
    /// dRofus data, and its fields' labels are validated against it.
    #[test]
    fn test_upload_source_hydrates_latest_stored_csv() {
        let dir = temp_projects_dir("upload-hydrate");
        std::fs::write(
            dir.join("p1.toml"),
            "project_id = \"p1\"\n\n[sources.reference.drofus]\ntype = \"upload\"\n\n[[sources.reference.drofus.fields]]\nlabel = \"NetArea\"\nqa = \"exact\"\n",
        )
        .unwrap();

        let store = MemStore::new();
        store
            .put_reference("p1", "drofus", "2026-01-01T10:00:00Z", b"DrofusRoomId,NetArea\nNumber,Area\n1,25.5\n")
            .unwrap();

        let (registry, _default) = load_project_settings_dir(&dir, &store).unwrap();
        let drofus = registry.get("p1").unwrap().reference["drofus"].data.as_ref().expect("hydrated");
        assert_eq!(drofus.link_property, "Number");
        assert_eq!(drofus.by_id["1"].fields.get("NetArea"), Some(&"25.5".to_string()));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A bad namespace in a comparison field fails the boot with a message
    /// naming the file, the field, and the known sources — replacing the
    /// silent empty-diff no-op it used to become at read time. The unqualified
    /// entry alongside it proves free-text names stay unvalidated.
    #[test]
    fn test_bad_comparison_namespace_fails_startup() {
        let dir = temp_projects_dir("cmp-ns");
        std::fs::write(
            dir.join("p1.toml"),
            "project_id = \"p1\"\ncomparison_key = \"Number\"\ncomparison_properties = [\"Area\", \"drofuss.NetArea\"]\n",
        )
        .unwrap();

        let msg = match load_project_settings_dir(&dir, &MemStore::new()) {
            Err(err) => format!("{err:#}"),
            Ok(_) => panic!("expected startup failure for an unknown comparison namespace"),
        };
        assert!(msg.contains("unknown data source"), "names the problem: {msg}");
        assert!(msg.contains("drofus"), "names the known sources: {msg}");
        assert!(msg.contains("comparison_properties"), "names the setting: {msg}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The valid comparison shapes all boot: a known namespace, unqualified
    /// free-text (which may match nothing yet), and no key at all.
    #[test]
    fn test_valid_comparison_fields_boot() {
        let dir = temp_projects_dir("cmp-ok");
        std::fs::write(
            dir.join("p1.toml"),
            // `drofus.`-qualified fields validate only against sources THIS
            // project actually configures (see the comment on the
            // `known_here` computation in `load_project_bundle`), so the
            // fixture must declare the source, not just reference it.
            "project_id = \"p1\"\ncomparison_key = \"drofus.RoomId\"\ncomparison_properties = [\"Area\", \"drofus.NetArea\", \"No Such Property\"]\n\n[sources.reference.drofus]\ntype = \"upload\"\n",
        )
        .unwrap();

        let (registry, _default) = load_project_settings_dir(&dir, &MemStore::new()).unwrap();
        assert_eq!(registry.get("p1").unwrap().comparison_key.as_deref(), Some("drofus.RoomId"));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A stored CSV whose labels don't cover the declared fields fails the
    /// load loudly — same discipline as a `file` source.
    #[test]
    fn test_upload_source_label_mismatch_fails_loudly() {
        let dir = temp_projects_dir("upload-mismatch");
        std::fs::write(
            dir.join("p1.toml"),
            "project_id = \"p1\"\n\n[sources.reference.drofus]\ntype = \"upload\"\n\n[[sources.reference.drofus.fields]]\nlabel = \"NoSuchColumn\"\nqa = \"exact\"\n",
        )
        .unwrap();

        let store = MemStore::new();
        store
            .put_reference("p1", "drofus", "2026-01-01T10:00:00Z", b"DrofusRoomId,NetArea\nNumber,Area\n1,25.5\n")
            .unwrap();

        let msg = match load_project_settings_dir(&dir, &store) {
            Err(err) => format!("{err:#}"),
            Ok(_) => panic!("expected failure: field label not in stored CSV"),
        };
        assert!(msg.contains("NoSuchColumn"), "message names the label: {msg}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A reference source named after one of `RoomResponse`'s own wire fields
    /// fails the boot loudly rather than silently shadowing that field once
    /// `#[serde(flatten)]` spreads it onto every `/rooms` response.
    #[test]
    fn test_reserved_reference_source_name_fails_startup() {
        let dir = temp_projects_dir("reserved-name");
        std::fs::write(
            dir.join("p1.toml"),
            "project_id = \"p1\"\n\n[sources.reference.properties]\ntype = \"upload\"\n",
        )
        .unwrap();

        let msg = match load_project_settings_dir(&dir, &MemStore::new()) {
            Err(err) => format!("{err:#}"),
            Ok(_) => panic!("expected startup failure for a reserved source name"),
        };
        assert!(msg.contains("reserved") && msg.contains("properties"), "names the problem: {msg}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Two configured reference sources on one project both load into
    /// `ProjectSettings.reference`, independently of each other — the
    /// end-to-end proof that `load_project_bundle` no longer special-cases
    /// "drofus" as the only loadable source name.
    #[test]
    fn test_two_reference_sources_both_load_independently() {
        let dir = temp_projects_dir("two-sources");
        std::fs::write(
            dir.join("p1.toml"),
            "project_id = \"p1\"\n\n\
             [sources.reference.drofus]\ntype = \"upload\"\n\n\
             [[sources.reference.drofus.fields]]\nlabel = \"NetArea\"\ntype = \"numeric\"\n\n\
             [sources.reference.doors]\ntype = \"upload\"\n\n\
             [[sources.reference.doors.fields]]\nlabel = \"Mark\"\n",
        )
        .unwrap();

        let store = MemStore::new();
        store.put_reference("p1", "drofus", "2026-01-01T10:00:00Z", b"DrofusRoomId,NetArea\nNumber,Area\n1,25.5\n").unwrap();
        store.put_reference("p1", "doors", "2026-01-01T10:00:00Z", b"DoorId,Mark\nMark,Mark\nD1,101A\n").unwrap();

        let (registry, _default) = load_project_settings_dir(&dir, &store).unwrap();
        let bundle = registry.get("p1").unwrap();

        assert_eq!(bundle.reference.len(), 2, "both sources registered, not just \"drofus\"");
        let drofus = bundle.reference["drofus"].data.as_ref().expect("drofus hydrated");
        assert_eq!(drofus.by_id["1"].fields.get("NetArea"), Some(&"25.5".to_string()));
        let doors = bundle.reference["doors"].data.as_ref().expect("doors hydrated");
        assert_eq!(doors.by_id["D1"].fields.get("Mark"), Some(&"101A".to_string()));
        assert_eq!(bundle.reference["doors"].fields[0].label, "Mark");

        std::fs::remove_dir_all(&dir).ok();
    }
}
