//! Writing a project's settings file: the TOML rendered from the settings with the stored file's comments carried over, and a save merged over
//! what is stored so a page that does not know a setting cannot silently empty it (`merge_over_stored`).

use super::*;

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
pub(super) fn render_settings(settings: &Settings, previous: Option<&str>) -> Result<String, SettingsError> {
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
pub(super) fn carry_comments(previous: &toml_edit::Table, next: &mut toml_edit::Table) {
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

pub(super) fn carry_table_comments(previous: &toml_edit::Table, next: &mut toml_edit::Table) {
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
pub(super) fn matching_entry<'a>(
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
/// **This exists because a save is a whole-file write and a page may edit only
/// part of a project.** The hand-written `static/settings.html` this API was
/// built for had no controls for `[doors]`, `[areas]` or `hierarchy_exclusions`
/// and never sent them, and
/// all three are `#[serde(default, skip_serializing_if = ...)]` — so an unsent
/// section did not stay unedited, it was **deleted**, silently, by editing
/// something unrelated. A real project lost `[doors] room_resolution = "same_model"` that
/// way and put every door back to homeless; nothing anywhere said so.
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
pub(super) fn merge_over_stored(
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
