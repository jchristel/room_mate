//! Which rules a sheet or a task applies: the generic rules and those bound to a blueprint (`rules_for`), and the one effective rule per
//! type code a create-RLS task carries (`rls_item_rules`).

use super::*;

/// The rules a task carries: those for these viewport types and, when the items are known, only
/// for these type codes (a room, space or area tag rule is about the room, so it is carried
/// whatever the items are). `None` items means any element (a task that finds the items already
/// in the room). A rule keyed by another parameter than `parameter` is never carried: its codes
/// mean something else. **Generic rules, and those bound to `blueprint`** when the sheet is bound to
/// one (plan decision 27); a rule bound to another blueprint is not the sheet's. Compared
/// case-insensitively.
pub fn rules_for(
    projects_dir: &Path,
    project: &str,
    viewport_types: &[String],
    parameter: &str,
    codes: Option<&[String]>,
    blueprint: Option<&str>,
) -> Result<Vec<RuleSummary>, TagRuleError> {
    let wanted = |s: &str| s.trim().to_lowercase();
    let types: Vec<String> = viewport_types.iter().map(|v| wanted(v)).collect();
    let codes: Option<Vec<String>> = codes.map(|c| c.iter().map(|s| wanted(s)).collect());
    let parameter = wanted(parameter);
    Ok(load_all(projects_dir, project)?
        .iter()
        .filter(|r| types.contains(&wanted(&r.key.viewport_type)))
        .filter(|r| r.key.blueprint.is_empty() || blueprint.is_some_and(|b| r.key.blueprint == b))
        .filter(|r| {
            is_spatial(&r.tag_kind)
                || (!parameter.is_empty()
                    && wanted(&r.key.code_parameter) == parameter
                    && codes.as_ref().is_none_or(|c| c.contains(&wanted(&r.key.type_code))))
        })
        .map(RuleSummary::from)
        .collect())
}

/// The item rules an RLS creation task applies (plan decision 27, reworked 2026-11-05), worked out from `rules`.
///
/// **Per type code, viewport type and view side, ONE answer, whichever room blueprint it was learned in.**
/// The generic pool (rules from the Sheets button) is used when it has the key; a rule learned on one item of
/// any room blueprint is then ignored. Only a key the generic pool lacks falls back to the blueprint rules, and
/// those are available to every instance of the type code, assigned or not.
///
/// **A pool is pooled by SAMPLES, not by rule.** Several rules can hold one key's samples (one per blueprint
/// item, since a bound item rule holds one sample), and the median of their medians is not the median of the
/// samples; `reduce` is applied to all of them, as it is for one rule. Rules learned with different tags are
/// pooled too: the task places the tag the model's RLS settings name, never the one a rule was learned with.
///
/// The summary returned is the pool's most recently updated rule with the pooled placement and sample count, so
/// `rule_id` still names a rule the library lists. Room, space and area rules are not item rules and
/// are not returned (`rules_for` carries them).
pub fn rls_item_rules(rules: &[TagRule], viewport_types: &[String], parameter: &str) -> Vec<RuleSummary> {
    let wanted = |s: &str| s.trim().to_lowercase();
    let types: Vec<String> = viewport_types.iter().map(|v| wanted(v)).collect();
    let parameter = wanted(parameter);
    if parameter.is_empty() {
        return Vec::new();
    }
    // (type code, viewport type, view side) -> the rules for that key
    type Group<'a> = ((String, String, String), Vec<&'a TagRule>);
    let mut groups: Vec<Group> = Vec::new();
    for r in rules {
        if is_spatial(&r.tag_kind)
            || wanted(&r.key.code_parameter) != parameter
            || wanted(&r.key.type_code).is_empty()
            || !types.contains(&wanted(&r.key.viewport_type))
        {
            continue;
        }
        let key = (wanted(&r.key.type_code), wanted(&r.key.viewport_type), wanted(&r.key.view_side));
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(r),
            None => groups.push((key, vec![r])),
        }
    }
    let mut out = Vec::new();
    for (_, members) in groups {
        let generic: Vec<&TagRule> = members.iter().copied().filter(|r| r.key.blueprint.is_empty()).collect();
        let pool = if generic.is_empty() { members } else { generic };
        if let Some(rule) = pooled(&pool) {
            out.push(rule);
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// One pool of rules for a key, as one rule: every sample of every rule in it, reduced together. The rules may
/// have been learned with different tags (category, family, type), and that no longer divides them: the task places
/// the tag the model's RLS settings name, so each sample is just one more answer to where a tag of that item goes.
pub(super) fn pooled(pool: &[&TagRule]) -> Option<RuleSummary> {
    let weight = |r: &TagRule| r.samples.len().max(r.sample_count);
    let newest = pool
        .iter()
        .copied()
        .max_by(|a, b| a.updated_at.cmp(&b.updated_at).then_with(|| b.id.cmp(&a.id)))?;
    let samples: Vec<Sample> = pool.iter().flat_map(|r| r.samples.iter().cloned()).collect();
    let mut summary = RuleSummary::from(newest);
    if !samples.is_empty() {
        summary.placement = reduce(&samples);
    }
    summary.sample_count = pool.iter().map(|r| weight(r)).sum();
    Some(summary)
}
