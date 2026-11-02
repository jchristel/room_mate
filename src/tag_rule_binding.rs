//! Tag rules and room blueprints: what binding a rule to a blueprint, and taking it away, does to the
//! rule store (plan decision 27). Split from `tag_rules`, which keeps the rule, its key, how a tag
//! becomes a sample and how samples become a rule; this is the part that moves rules and samples
//! between the generic and the bound scope.
//!
//! - `take_moved_samples_out`: a sample lives in exactly one rule, so a push that puts it under a new
//!   key takes it out of the old one.
//! - `bound_to`, `import_bound`: a blueprint's rules as they are exported and imported (the bundle).
//! - `unlink_blueprint`: the rules a re-push no longer writes become generic.
//!
//! The store's private pieces (the lock, the file write) are `pub(crate)` in `tag_rules`: this module
//! is the one other user, and it writes through the same lock and the same files.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::tag_rules::{
    bad, load_all, now, reduce, remove, write, Sample, SampleSource, TagRule, TagRuleError, MAX_RULES_PER_PROJECT,
    MAX_SAMPLES_PER_RULE, SAVE_LOCK, SCHEMA_VERSION,
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
