//! Does each property a setting names still exist in the project's data?
//!
//! Colour plans, the room label, the hover and the element tags all name
//! properties by hand, and nothing stops a name being typed wrong or a model being
//! re-exported without it. A name nothing carries is not an error -- it resolves to
//! nothing, and a plan paints "no data" grey -- which is exactly why it goes
//! unnoticed. This reports each one, with the section and setting it came from, so
//! the settings page can mark them. A finding is a signal and never a refusal.
//!
//! **What "exists" means.** A name is available when it is in the dictionary of the
//! latest snapshot of that entity in any of the project's models (instance and type
//! tier alike; read from the snapshot's last line only, see `state::property_names_of`).
//! Three things are deliberately NOT findings, because the answer is "cannot tell"
//! rather than "missing": an entity with nothing stored yet, an entity whose latest
//! snapshot predates the dictionary and is too large to parse for this (rooms are
//! parsed), and a `$`-prefixed token, which is an element's own field and not a property.
//!
//! Rooms also answer to a builtin's canonical name and to `<source>.<label>` of a
//! joined reference source, the two vocabularies `lookup_property` resolves.

use std::collections::BTreeSet;

use roommate_shared::settings_api::SettingFinding;

use crate::settings::{ColourMode, Settings};
use crate::state::{property_names_of, AppState};
use crate::storage::SnapshotKind;

/// Every property reference in `settings`, with where it was written.
struct Reference {
    section: &'static str,
    setting: String,
    entity: SnapshotKind,
    property: String,
}

fn references(settings: &Settings) -> Vec<Reference> {
    let mut out = Vec::new();
    let mut add = |section, setting: String, entity, property: &str| {
        // `$name` and friends are an element's own fields, resolved without the
        // property tiers; there is nothing in the data to be missing.
        if !property.trim().is_empty() && !property.starts_with('$') {
            out.push(Reference { section, setting, entity, property: property.to_string() });
        }
    };
    for field in &settings.room_label {
        add("room_label", "room_label".to_string(), SnapshotKind::Rooms, field);
    }
    for plan in &settings.colour_plans {
        let setting = format!("colour_plans.{}", plan.name);
        match &plan.mode {
            ColourMode::PropertyCompare { property_a, property_b, .. } => {
                add("colour_plans", setting.clone(), SnapshotKind::Rooms, property_a);
                add("colour_plans", setting, SnapshotKind::Rooms, property_b);
            }
            ColourMode::DateRange { property, .. } => add("colour_plans", setting, SnapshotKind::Rooms, property),
            ColourMode::Hierarchy { .. } => {}
        }
    }
    let h = &settings.hover;
    for (key, kind, name) in [
        ("rooms", SnapshotKind::Rooms, &h.rooms),
        ("doors", SnapshotKind::Doors, &h.doors),
        ("windows", SnapshotKind::Windows, &h.windows),
        ("ffe", SnapshotKind::Ffe, &h.ffe),
        ("spaces", SnapshotKind::Spaces, &h.spaces),
        ("ceilings", SnapshotKind::Ceilings, &h.ceilings),
        ("floors", SnapshotKind::Floors, &h.floors),
    ] {
        if let Some(name) = name {
            add("hover", format!("hover.{key}"), kind, name);
        }
    }
    let t = &settings.tags;
    for (key, kind, tag) in [
        ("ffe", SnapshotKind::Ffe, &t.ffe),
        ("spaces", SnapshotKind::Spaces, &t.spaces),
        ("ceilings", SnapshotKind::Ceilings, &t.ceilings),
        ("floors", SnapshotKind::Floors, &t.floors),
    ] {
        for field in &tag.fields {
            add("tags", format!("tags.{key}"), kind, field);
        }
    }
    out
}

fn entity_name(kind: SnapshotKind) -> &'static str {
    kind.label()
}

/// The names `kind`'s latest snapshots carry, or `None` when that cannot be said.
fn available(state: &AppState, project_id: &str, kind: SnapshotKind) -> Option<BTreeSet<String>> {
    let raw = match state.store().all_latest_raw(kind, Some(project_id)) {
        Ok(raw) => raw,
        Err(e) => {
            tracing::warn!("settings check: could not read {} for {project_id}: {e:#}", kind.label());
            return None;
        }
    };
    if raw.is_empty() {
        return None;
    }
    let mut names = BTreeSet::new();
    for (_, bytes) in &raw {
        match property_names_of(bytes) {
            Some(found) => names.extend(found),
            // A snapshot written before the dictionary. Rooms are small enough to
            // parse (it is what `/rooms` does on every read); a legacy FF&E snapshot
            // costs seconds, so there "cannot tell" stands.
            None if kind == SnapshotKind::Rooms => {
                let payloads = state.all_snapshots(Some(project_id)).ok()?;
                return Some(
                    payloads
                        .iter()
                        .flat_map(|(_, p)| p.rooms.iter().flat_map(|r| r.properties.keys().map(|k| k.to_string())))
                        .collect(),
                );
            }
            None => return None,
        }
    }
    Some(names)
}

/// The findings for one project's settings, against the data stored now.
pub fn check(state: &AppState, settings: &Settings) -> Vec<SettingFinding> {
    let refs = references(settings);
    if refs.is_empty() {
        return Vec::new();
    }
    // Each entity is read once, and only if something names one of its properties.
    let mut kinds: Vec<SnapshotKind> = refs.iter().map(|r| r.entity).collect();
    kinds.sort_by_key(|k| k.label());
    kinds.dedup();
    let sets: Vec<(SnapshotKind, Option<BTreeSet<String>>)> =
        kinds.into_iter().map(|k| (k, available(state, &settings.project_id, k))).collect();

    // The vocabularies rooms add to their raw keys.
    let registry = state.settings();
    let bundle = registry.settings_for(&settings.project_id);
    let canonical: BTreeSet<&str> = settings.builtin_properties.iter().map(|b| b.canonical.as_str()).collect();

    let mut findings = Vec::new();
    for r in refs {
        let Some(Some(names)) = sets.iter().find(|(k, _)| *k == r.entity).map(|(_, n)| n) else {
            continue;
        };
        let mut found = names.contains(&r.property);
        if r.entity == SnapshotKind::Rooms && !found {
            found = canonical.contains(r.property.as_str());
            if let (false, Some((source, label))) = (found, r.property.split_once('.')) {
                // A joined source's column. Unknown labels are known only once the
                // source has data; until then it cannot be called missing.
                found = match bundle.and_then(|b| b.reference.get(source)) {
                    Some(src) => src.data.as_ref().is_none_or(|d| d.all_labels.iter().any(|l| l == label)),
                    None => false,
                };
            }
        }
        if !found {
            findings.push(SettingFinding {
                section: r.section.to_string(),
                setting: r.setting,
                message: format!(
                    "'{}' is not a property of any {} in this project's latest data.",
                    r.property,
                    entity_name(r.entity)
                ),
                entity: entity_name(r.entity).to_string(),
                property: r.property,
            });
        }
    }
    findings.sort_by(|a, b| (&a.section, &a.setting, &a.property).cmp(&(&b.section, &b.setting, &b.property)));
    findings.dedup();
    findings
}

/// Log every project's findings once, at startup. The same check the settings page
/// runs, so a server's log and the page agree.
pub fn log_startup(state: &AppState, projects_dir: &std::path::Path) {
    let Ok(files) = crate::settings_api::list_project_files(projects_dir) else {
        return;
    };
    for file in files {
        let Some(project_id) = file.project_id else { continue };
        let Ok((_, settings)) = crate::settings_api::get_project_file(projects_dir, &project_id) else {
            continue;
        };
        for f in check(state, &settings) {
            tracing::warn!("settings for {}: {} -- {}", settings.project_id, f.setting, f.message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{CustomValue, Level, Model, Project, Room, RoomPayload, Snapshot, SUPPORTED_SCHEMA};
    use crate::storage::MemStore;
    use std::sync::Arc;

    fn state_with_room_property(name: &str) -> AppState {
        let state = AppState::new(Box::new(MemStore::new()), Default::default(), None);
        let mut properties = std::collections::BTreeMap::new();
        properties.insert(Arc::from(name), CustomValue { value: "x".to_string(), storage_type: None });
        state
            .set_snapshot(RoomPayload {
                schema_version: SUPPORTED_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
                phase: None,
                model_to_shared: None,
                room_boundary: None,
                levels: vec![Level { id: "l1".to_string(), name: "L".to_string(), elevation: 0.0 }],
                rooms: vec![Room {
                    enclosure: None,
                    id: "r1".to_string(),
                    name: "R".to_string(),
                    level_id: "l1".to_string(),
                    loops: vec![],
                    properties,
                }],
            })
            .unwrap();
        state
    }

    fn settings(toml: &str) -> Settings {
        toml::from_str(&format!("project_id = \"p1\"\n{toml}")).unwrap()
    }

    #[test]
    fn test_a_room_label_property_the_data_lacks_is_a_finding() {
        let state = state_with_room_property("Department");
        let found = check(&state, &settings("room_label = [\"$name\", \"Department\", \"Gone\"]"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].section, "room_label");
        assert_eq!(found[0].property, "Gone");
    }

    #[test]
    fn test_intrinsics_and_unstored_entities_are_never_findings() {
        // `$name` is an element's own field; nothing is stored for FF&E, so a tag
        // naming any property there cannot be called missing.
        let state = state_with_room_property("Department");
        let s = settings("[tags.ffe]\nfields = [\"$type_name\", \"ItemCode\"]\n");
        assert!(check(&state, &s).is_empty());
    }

    #[test]
    fn test_hover_is_checked_against_its_own_entity() {
        let state = state_with_room_property("Department");
        assert!(check(&state, &settings("[hover]\nrooms = \"Department\"\n")).is_empty());
        let found = check(&state, &settings("[hover]\nrooms = \"Typo\"\n"));
        assert_eq!((found[0].section.as_str(), found[0].setting.as_str()), ("hover", "hover.rooms"));
    }
}
