//! Tag rules and room blueprints: what binding a rule to a blueprint, and taking it away, does to the
//! rule store (plan decision 27). Split from `tag_rules`, which keeps the rule, its key, how a tag
//! becomes a sample and how samples become a rule; this is the part that moves rules and samples
//! between the generic and the bound scope.
//!
//! - `take_moved_samples_out`: a sample lives in exactly one rule, so a push that puts it under a new
//!   key takes it out of the old one.
//! - `bound_to`, `import_bound`: a blueprint's rules as they are exported and imported (the bundle).
//! - `unlink_blueprint`: the rules a re-push no longer writes become generic.
//! - `import_generic`: rules from a file (the library page's Import), always taken in as generic.
//!
//! The store's private pieces (the lock, the file write) are `pub(crate)` in `tag_rules`: this module
//! is the one other user, and it writes through the same lock and the same files.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::tag_rules::{
    bad, clean, load_all, now, reduce, remove, write, RuleKey, Sample, SampleSource, TagRule, TagRuleError,
    MAX_RULES_PER_PROJECT, MAX_SAMPLES_PER_RULE, SAVE_LOCK, SCHEMA_VERSION,
};

/// The rules bound to a room blueprint, samples and all (the export of a blueprint's bundle).
pub fn bound_to(projects_dir: &Path, project: &str, blueprint: &str) -> Result<Vec<TagRule>, TagRuleError> {
    Ok(load_all(projects_dir, project)?
        .into_iter()
        .filter(|r| r.key.blueprint == blueprint)
        .collect())
}

/// Take rules in as bound to `blueprint` (the import of a blueprint's bundle): the key is made over
/// with this blueprint's id, the answer is worked out again from the samples, and an existing rule
/// of the same key is replaced. Returns the ids written. Rules that arrive generic stay generic.
pub fn import_bound(
    projects_dir: &Path,
    project: &str,
    blueprint: &str,
    rules: Vec<TagRule>,
) -> Result<Vec<String>, TagRuleError> {
    if rules.len() > MAX_RULES_PER_PROJECT {
        return bad(format!("a project holds at most {MAX_RULES_PER_PROJECT} tag rules"));
    }
    let _guard = SAVE_LOCK.lock().unwrap();
    let mut ids = Vec::new();
    for mut rule in rules {
        if rule.samples.is_empty() {
            continue;
        }
        if !rule.key.blueprint.is_empty() {
            rule.key.blueprint = blueprint.to_string();
        }
        rule.id = rule.key.id();
        rule.schema_version = SCHEMA_VERSION;
        rule.samples.truncate(MAX_SAMPLES_PER_RULE);
        rule.placement = reduce(&rule.samples);
        rule.sample_count = rule.samples.len();
        rule.updated_at = now();
        write(projects_dir, project, &rule)?;
        ids.push(rule.id);
    }
    Ok(ids)
}

/// A re-push of a blueprint was accepted and these rules, bound to it, are not among the ones it
/// wrote (`keep`): their item or room tag no longer exists in it. They are **unlinked** (plan
/// decision 27): each sample moves to the generic rule with the same key (blueprint and instance
/// cleared), merged into it by the rule's own reduction, and the bound rule is removed. A sample's
/// boundary recipe was written against the blueprint's room, so it is dropped on the way: the
/// generic rule keeps the boundary its other samples give it. Returns how many rules were unlinked.
pub fn unlink_blueprint(
    projects_dir: &Path,
    project: &str,
    blueprint: &str,
    keep: &HashSet<String>,
) -> Result<usize, TagRuleError> {
    let _guard = SAVE_LOCK.lock().unwrap();
    let all = load_all(projects_dir, project)?;
    let mut generic: HashMap<String, TagRule> = all
        .iter()
        .filter(|r| r.key.blueprint.is_empty())
        .map(|r| (r.id.clone(), r.clone()))
        .collect();
    let mut changed: Vec<String> = Vec::new();
    let mut orphans: Vec<&TagRule> =
        all.iter().filter(|r| r.key.blueprint == blueprint && !keep.contains(&r.id)).collect();
    orphans.sort_by(|a, b| a.id.cmp(&b.id));
    for orphan in &orphans {
        let mut key = orphan.key.clone();
        key.blueprint.clear();
        key.instance.clear();
        let id = key.id();
        let target = generic.entry(id.clone()).or_insert_with(|| TagRule {
            schema_version: SCHEMA_VERSION,
            id: id.clone(),
            key: key.clone(),
            tag_kind: orphan.tag_kind.clone(),
            placement: orphan.placement.clone(),
            sample_count: 0,
            updated_at: now(),
            samples: Vec::new(),
        });
        for sample in &orphan.samples {
            let mut moved = sample.clone();
            moved.boundary = None;
            target.samples.retain(|e| e.source != moved.source);
            target.samples.push(moved);
        }
        target.samples.truncate(MAX_SAMPLES_PER_RULE);
        target.placement = reduce(&target.samples);
        target.sample_count = target.samples.len();
        target.updated_at = now();
        if !changed.contains(&id) {
            changed.push(id);
        }
    }
    for id in &changed {
        write(projects_dir, project, &generic[id])?;
    }
    for orphan in &orphans {
        remove(projects_dir, project, &orphan.id)?;
    }
    Ok(orphans.len())
}

/// A rule as an import file holds it: what the rule IS (its key, the kind of tag, the samples it was learned
/// from). The id, the answer and the counts are worked out again here, so a file need not, and cannot, fake them.
#[derive(Debug, Clone, Deserialize)]
pub struct ImportedRule {
    pub key: RuleKey,
    pub tag_kind: String,
    #[serde(default)]
    pub samples: Vec<Sample>,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct ImportOutcome {
    pub rules_created: usize,
    pub rules_updated: usize,
    /// Samples taken in (new, or replacing the same sample read from the same tag).
    pub samples: usize,
    /// Rules in the file that held no samples: nothing to learn from, so nothing was written.
    pub empty: usize,
    /// Rules written that this project does not use: their type-code parameter is not the one its settings name,
    /// and `rules_for` carries only the current parameter's.
    pub unused: usize,
}

const KINDS: [&str; 4] = ["independent", "room", "space", "area"];

/// The key as it is kept: text trimmed and capped as an observation's is, and scoped GENERIC.
fn generic_key(mut key: RuleKey) -> Result<RuleKey, TagRuleError> {
    for field in [
        &mut key.tagged_category,
        &mut key.code_parameter,
        &mut key.type_code,
        &mut key.tag_category,
        &mut key.tag_family,
        &mut key.tag_type,
        &mut key.viewport_type,
        &mut key.view_side,
    ] {
        *field = clean(field);
    }
    if key.tagged_category.is_empty() || key.tag_category.is_empty() {
        return bad("a rule needs the category of the tagged element and of the tag".to_string());
    }
    if !matches!(key.view_side.as_str(), "" | "front" | "side") {
        return bad(format!("a rule's view side is front or side, not {:?}", key.view_side));
    }
    key.blueprint.clear();
    key.instance.clear();
    Ok(key)
}

/// Take rules in from a file as **generic** rules (the library page's Import). A file is not tied to any
/// blueprint, so a rule that names one (or one of its items) is made generic and its samples lose their boundary
/// recipe, which was written against that blueprint's room (what `unlink_blueprint` does). A sample is identified by
/// where it was read: it replaces the same sample in an existing rule of the same key, **merged by the rule's own
/// reduction** rather than replacing the rule, and it is taken out of any other rule that holds it, so it lives in
/// exactly one. `parameter` is the project's own type-code parameter, only to count the rules it will not use.
pub fn import_generic(
    projects_dir: &Path,
    project: &str,
    rules: Vec<ImportedRule>,
    parameter: Option<&str>,
) -> Result<ImportOutcome, TagRuleError> {
    if rules.len() > MAX_RULES_PER_PROJECT {
        return bad(format!("a project holds at most {MAX_RULES_PER_PROJECT} tag rules"));
    }
    let _guard = SAVE_LOCK.lock().unwrap();
    let existing: HashMap<String, TagRule> =
        load_all(projects_dir, project)?.into_iter().map(|r| (r.id.clone(), r)).collect();
    let mut outcome = ImportOutcome::default();
    let mut targets: HashMap<String, TagRule> = HashMap::new();
    let mut sources: Vec<SampleSource> = Vec::new();
    for incoming in rules {
        if incoming.samples.is_empty() {
            outcome.empty += 1;
            continue;
        }
        let kind = incoming.tag_kind.trim().to_lowercase();
        if !KINDS.contains(&kind.as_str()) {
            return bad(format!(
                "a rule's tag kind is independent, room, space or area, not {:?}",
                incoming.tag_kind
            ));
        }
        let scoped = !incoming.key.blueprint.trim().is_empty() || !incoming.key.instance.trim().is_empty();
        let key = generic_key(incoming.key)?;
        let id = key.id();
        let target = targets.entry(id.clone()).or_insert_with(|| {
            existing.get(&id).cloned().unwrap_or_else(|| TagRule {
                schema_version: SCHEMA_VERSION,
                id: id.clone(),
                key: key.clone(),
                tag_kind: kind.clone(),
                placement: reduce(&incoming.samples),
                sample_count: 0,
                updated_at: now(),
                samples: Vec::new(),
            })
        });
        for mut sample in incoming.samples {
            if !sample.view_scale.is_finite() || sample.view_scale <= 0.0 {
                return bad("a sample's view scale must be a positive number".to_string());
            }
            if scoped {
                sample.boundary = None;
            }
            sources.push(sample.source.clone());
            target.samples.retain(|e| e.source != sample.source);
            target.samples.push(sample);
            outcome.samples += 1;
        }
    }
    let created = targets.keys().filter(|id| !existing.contains_key(*id)).count();
    if existing.len() + created > MAX_RULES_PER_PROJECT {
        return bad(format!("a project holds at most {MAX_RULES_PER_PROJECT} tag rules"));
    }
    outcome.rules_created = created;
    outcome.rules_updated = targets.len() - created;
    let mut ids: Vec<String> = targets.keys().cloned().collect();
    ids.sort();
    for id in &ids {
        let rule = targets.get_mut(id).expect("an id of the map");
        rule.samples.truncate(MAX_SAMPLES_PER_RULE);
        rule.placement = reduce(&rule.samples);
        rule.sample_count = rule.samples.len();
        rule.updated_at = now();
        if !rule.key.code_parameter.is_empty() && parameter != Some(rule.key.code_parameter.as_str()) {
            outcome.unused += 1;
        }
        write(projects_dir, project, rule)?;
    }
    take_moved_samples_out(projects_dir, project, &existing, &ids.iter().cloned().collect(), &sources)?;
    Ok(outcome)
}

/// A sample is identified by where it was read, wherever it now lives: a sheet pushed again under
/// another key (bound to a blueprint after being generic, or its type code changed) must not leave
/// its old sample behind. Every rule not written by this push loses the samples this push carried,
/// is written again from what is left, or is deleted when nothing is. Returns how many rules changed.
pub(crate) fn take_moved_samples_out(
    projects_dir: &Path,
    project: &str,
    existing: &HashMap<String, TagRule>,
    touched: &HashSet<String>,
    sources: &[SampleSource],
) -> Result<usize, TagRuleError> {
    let mut changed = 0;
    let mut others: Vec<&TagRule> = existing.values().filter(|r| !touched.contains(&r.id)).collect();
    others.sort_by(|a, b| a.id.cmp(&b.id));
    for rule in others {
        let samples: Vec<Sample> = rule.samples.iter().filter(|s| !sources.contains(&s.source)).cloned().collect();
        if samples.len() == rule.samples.len() {
            continue;
        }
        if samples.is_empty() {
            remove(projects_dir, project, &rule.id)?;
        } else {
            let rewritten = TagRule {
                placement: reduce(&samples),
                sample_count: samples.len(),
                updated_at: now(),
                samples,
                ..rule.clone()
            };
            write(projects_dir, project, &rewritten)?;
        }
        changed += 1;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    //! `import_generic`, through the real store. Fixtures are this module's own (the house rule is a helper per module).
    use super::*;
    use crate::tag_rules::{load, Boundary, BoundaryOf, Offset, SourceSheet};
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roommate-import-rules-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn key(code: &str) -> RuleKey {
        RuleKey {
            tagged_category: "Furniture".into(),
            code_parameter: "Assembly Code".into(),
            type_code: code.into(),
            tag_category: "Furniture Tags".into(),
            tag_family: "Tag".into(),
            tag_type: "Standard".into(),
            viewport_type: "plan".into(),
            blueprint: String::new(),
            instance: String::new(),
            view_side: String::new(),
        }
    }

    fn sample(tag_id: i64, x: f64) -> Sample {
        Sample {
            source: SampleSource {
                sheet: SourceSheet { document_title: "M".into(), revit_sheet_id: "1".into(), sheet_number: "A".into() },
                tag_id,
                element_id: tag_id,
            },
            view_scale: 50.0,
            head: Offset { x, y: 0.0 },
            rotation: 0.0,
            orientation: "Horizontal".into(),
            has_leader: false,
            leader_end_condition: String::new(),
            elbow: None,
            end: None,
            boundary: None,
            from_centre: false,
        }
    }

    fn imported(key: RuleKey, samples: Vec<Sample>) -> ImportedRule {
        ImportedRule { key, tag_kind: "independent".into(), samples }
    }

    fn stored(dir: &Path, k: &RuleKey) -> TagRule {
        load(dir, "p", &k.id()).unwrap()
    }

    #[test]
    fn a_new_rule_is_made_with_its_answer_worked_out_here() {
        let dir = temp_dir("new");
        let out = import_generic(
            &dir,
            "p",
            vec![imported(
                key("B1"),
                vec![sample(1, 10.0), sample(2, 20.0), sample(3, 30.0)],
            )],
            Some("Assembly Code"),
        )
        .unwrap();
        assert_eq!(ImportOutcome { rules_created: 1, rules_updated: 0, samples: 3, empty: 0, unused: 0 }, out);
        let rule = stored(&dir, &key("B1"));
        assert_eq!(3, rule.sample_count);
        assert_eq!(20.0, rule.placement.head.x, "the median of the samples");
    }

    #[test]
    fn samples_merge_into_a_rule_of_the_same_key_by_where_they_were_read() {
        let dir = temp_dir("merge");
        import_generic(&dir, "p", vec![imported(key("B1"), vec![sample(1, 10.0), sample(2, 20.0)])], None).unwrap();
        let out =
            import_generic(&dir, "p", vec![imported(key("B1"), vec![sample(2, 90.0), sample(3, 30.0)])], None).unwrap();
        assert_eq!(1, out.rules_updated);
        assert_eq!(0, out.rules_created);
        let rule = stored(&dir, &key("B1"));
        assert_eq!(3, rule.sample_count, "tag 2 was replaced, tag 3 added, tag 1 kept");
        assert_eq!(30.0, rule.placement.head.x);
    }

    #[test]
    fn a_rule_that_names_a_blueprint_arrives_generic_without_its_boundary() {
        let dir = temp_dir("bound");
        let mut k = key("B1");
        k.blueprint = "bp-1".into();
        k.instance = "42".into();
        let mut s = sample(1, 10.0);
        s.boundary = Some(Boundary { of: BoundaryOf::Wall, edge: 1, along: 0.5, off: 4.0 });
        import_generic(&dir, "p", vec![imported(k, vec![s])], None).unwrap();
        let rule = stored(&dir, &key("B1"));
        assert!(rule.key.blueprint.is_empty() && rule.key.instance.is_empty());
        assert!(rule.samples[0].boundary.is_none(), "it was written against a blueprint's room");
    }

    #[test]
    fn a_generic_rules_boundary_is_kept() {
        let dir = temp_dir("rect");
        let mut s = sample(1, 10.0);
        s.boundary = Some(Boundary { of: BoundaryOf::Rectangle, edge: 0, along: 0.5, off: 4.0 });
        import_generic(&dir, "p", vec![imported(key("B1"), vec![s])], None).unwrap();
        assert!(stored(&dir, &key("B1")).samples[0].boundary.is_some());
    }

    #[test]
    fn a_sample_lives_in_one_rule_so_it_leaves_the_one_that_held_it() {
        let dir = temp_dir("moved");
        import_generic(&dir, "p", vec![imported(key("OLD"), vec![sample(1, 10.0), sample(2, 20.0)])], None).unwrap();
        import_generic(&dir, "p", vec![imported(key("NEW"), vec![sample(1, 10.0)])], None).unwrap();
        assert_eq!(1, stored(&dir, &key("OLD")).sample_count, "tag 1 moved to the rule it is now filed under");
        assert_eq!(1, stored(&dir, &key("NEW")).sample_count);
        import_generic(&dir, "p", vec![imported(key("NEW"), vec![sample(2, 20.0)])], None).unwrap();
        assert!(load(&dir, "p", &key("OLD").id()).is_err(), "a rule left empty is removed");
    }

    #[test]
    fn a_rule_with_no_samples_is_counted_and_not_written() {
        let dir = temp_dir("empty");
        let out = import_generic(&dir, "p", vec![imported(key("B1"), vec![])], None).unwrap();
        assert_eq!(1, out.empty);
        assert_eq!(0, out.rules_created);
        assert!(load(&dir, "p", &key("B1").id()).is_err());
    }

    #[test]
    fn rules_of_another_type_code_parameter_are_said_to_be_unused() {
        let dir = temp_dir("unused");
        let mut other = key("B2");
        other.code_parameter = "Other Code".into();
        let out = import_generic(
            &dir,
            "p",
            vec![
                imported(key("B1"), vec![sample(1, 1.0)]),
                imported(other, vec![sample(2, 1.0)]),
            ],
            Some("Assembly Code"),
        )
        .unwrap();
        assert_eq!(1, out.unused);
        let none = import_generic(&dir, "p", vec![imported(key("B3"), vec![sample(3, 1.0)])], None).unwrap();
        assert_eq!(1, none.unused, "a project that names no parameter uses no item rule");
    }

    #[test]
    fn a_rule_that_cannot_be_kept_is_refused_and_nothing_is_written() {
        let dir = temp_dir("refused");
        let mut kind = imported(key("B1"), vec![sample(1, 1.0)]);
        kind.tag_kind = "balloon".into();
        let mut side = key("B1");
        side.view_side = "above".into();
        let mut nameless = key("B1");
        nameless.tag_category = " ".into();
        let mut flat = sample(1, 1.0);
        flat.view_scale = 0.0;
        for rule in [
            kind,
            imported(side, vec![sample(1, 1.0)]),
            imported(nameless, vec![sample(1, 1.0)]),
            imported(key("B1"), vec![flat]),
        ] {
            assert!(matches!(
                import_generic(&dir, "p", vec![imported(key("OK"), vec![sample(9, 1.0)]), rule], None),
                Err(TagRuleError::Invalid(_))
            ));
        }
        assert!(
            load(&dir, "p", &key("OK").id()).is_err(),
            "a refused import leaves the good rule unwritten too"
        );
    }
}
