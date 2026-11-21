//! Learning rules: an observation's tags become samples (`samples_of`), a rule's answer is the median of its samples (`reduce`) and a push
//! replaces its own sheet's samples (`observe`).

use super::*;

impl ObserveContext<'_> {
    /// The blueprint a sheet's tags belong to and its items: the push's own blueprint if it names one,
    /// else the one the sheet is already bound to. `None`: the tags are generic.
    fn scope_of(&self, sheet: &SourceSheet) -> Option<(&str, &std::collections::HashSet<String>)> {
        if let Some((id, items)) = &self.blueprint {
            return Some((id, items));
        }
        let id = self.sheet_blueprints.get(&(sheet.document_title.clone(), sheet.revit_sheet_id.clone()))?;
        Some((id, self.blueprint_items.get(id)?))
    }
}

/// What the server knows that the observations do not say (plan decision 27). The store has none of
/// it, so the API layer fills it in; the default is "no blueprint, no rooms on hand".
#[derive(Default)]
pub struct ObserveContext<'a> {
    /// The room blueprint the push is for, with the ids of the items it holds: a tag on one of those
    /// is bound to it (and to that item), a tag on any other item is generic.
    pub blueprint: Option<(String, std::collections::HashSet<String>)>,
    /// For a push that names no blueprint (the Sheets button): the blueprint each SHEET is already bound to,
    /// by `(document title, Revit sheet id)`, and the items of each such blueprint. A sheet bound to a
    /// blueprint keeps its tags bound whichever button pushed them; without this the second push would
    /// move the sheet's tag samples out of the blueprint's rules into the generic ones while the sheet
    /// stayed bound. An explicit `blueprint` wins over this.
    pub sheet_blueprints: std::collections::HashMap<(String, String), String>,
    pub blueprint_items: std::collections::HashMap<String, std::collections::HashSet<String>>,
    /// A room tag head written against its room's boundary: `(model id, room id, head in the model's
    /// frame, mm, bound, view scale)`. `None`: the room is not on the server, and the sample keeps
    /// only its offset in the plan's axes.
    #[allow(clippy::type_complexity)]
    pub boundary: Option<&'a dyn Fn(&str, i64, P3, bool, f64) -> Option<Boundary>>,
}

/// `(key, kind, sample)` for each tagged element of the observation that can be used, and a
/// reason for the ones that cannot.
pub(super) fn samples_of(
    obs: &Observation,
    parameter: &str,
    ctx: &ObserveContext,
) -> Vec<Result<(RuleKey, String, Sample), String>> {
    let tag = &obs.tag;
    let Some(viewport_type) = obs.viewport_type.as_deref().map(clean).filter(|v| !v.is_empty()) else {
        return vec![Err("the observation names no viewport type".to_string())];
    };
    let Some(scale) = obs.view_scale.filter(|s| *s > 0.0) else {
        return vec![Err("the view's scale was not read".to_string())];
    };
    if tag.tag_type.family_name.trim().is_empty() {
        return vec![Err("the tag's family name was not read".to_string())];
    }
    if tag.tagged_elements.is_empty() {
        return vec![Err("the tag refers to no element".to_string())];
    }
    let spatial = is_spatial(&tag.tag_kind);
    tag.tagged_elements
        .iter()
        .map(|el| {
            if spatial {
                if el.names.category.trim().is_empty() {
                    return Err(format!("tagged element {}: its category was not read", el.host_element_id));
                }
            } else if parameter.is_empty() {
                return Err(format!(
                    "tagged element {}: the push names no type-code parameter (set `tag_type_code` in the project's settings)",
                    el.host_element_id
                ));
            } else if el.type_code.as_deref().map(clean).is_none_or(|c| c.is_empty()) {
                return Err(format!(
                    "tagged element {}: its type has no value for the type-code parameter {parameter:?}",
                    el.host_element_id
                ));
            }
            let loc = el
                .location_point
                .as_ref()
                .ok_or_else(|| format!("tagged element {}: it has no point location", el.host_element_id))?;
            // An item in an elevation is measured along the view (right, up) and keyed by how it is seen.
            let elevation = !spatial && is_elevation(&viewport_type);
            let looks = if elevation {
                let d = obs.view_direction.unwrap_or_default();
                Some(plan_direction(d.x, d.y).ok_or_else(|| {
                    format!("tagged element {}: the elevation's view direction was not read", el.host_element_id)
                })?)
            } else {
                None
            };
            // A room has no orientation of its own: its tag is placed in the plan's axes.
            let facing = if spatial {
                (1.0, 0.0)
            } else {
                facing_of(loc).ok_or_else(|| format!("tagged element {}: it has no plan facing", el.host_element_id))?
            };
            // An elevation is measured from the item's middle when the extractor read it, because the insertion point
            // need not be at the item's height (a plan keeps the point: its offsets turn with the item's facing).
            let centre = el.centre.filter(|_| looks.is_some());
            let from = centre.unwrap_or(loc.translation_coord);
            let element_id = if el.link_element_id > 0 { el.link_element_id } else { el.host_element_id };
            // Bound to the push's blueprint: a room tag always, an item's tag only when the item is one of
            // the blueprint's (its id is the Revit element id, which the blueprint kept).
            let (blueprint, instance) = match ctx.scope_of(&obs.sheet) {
                Some((id, _)) if spatial => (id.to_string(), String::new()),
                Some((id, items)) if items.contains(&element_id.to_string()) => (id.to_string(), element_id.to_string()),
                _ => (String::new(), String::new()),
            };
            let key = RuleKey {
                tagged_category: clean(&el.names.category),
                code_parameter: if spatial { String::new() } else { clean(parameter) },
                type_code: if spatial { String::new() } else { el.type_code.as_deref().map(clean).unwrap_or_default() },
                tag_category: clean(&tag.tag_type.category),
                tag_family: clean(&tag.tag_type.family_name),
                tag_type: clean(&tag.tag_type.type_name),
                viewport_type: viewport_type.clone(),
                blueprint: blueprint.clone(),
                instance,
                view_side: looks.map(|l| view_side(facing, l).to_string()).unwrap_or_default(),
            };
            let offset = |p: P3| match looks {
                Some(l) => to_view_offset(p, from, l, scale),
                None => to_offset(p, from, facing, scale),
            };
            let boundary = if spatial {
                ctx.boundary.and_then(|f| f(&obs.sheet.document_title, element_id, tag.point, !blueprint.is_empty(), scale))
            } else {
                None
            };
            let sample = Sample {
                source: SampleSource { sheet: obs.sheet.clone(), tag_id: tag.id, element_id },
                view_scale: scale,
                head: offset(tag.point),
                // in an elevation the tag's turn is about the view's normal, which the item's facing says nothing of
                rotation: if elevation { wrap(tag.rotation) } else { wrap(tag.rotation - facing.1.atan2(facing.0)) },
                orientation: clean(&tag.orientation),
                has_leader: tag.has_leader,
                leader_end_condition: clean(&tag.leader_end_condition),
                elbow: el.elbow_location.filter(|_| tag.has_leader).map(offset),
                end: el.leader_end.filter(|_| tag.has_leader).map(offset),
                boundary,
                from_centre: centre.is_some(),
            };
            let kind = if tag.tag_kind.trim().is_empty() {
                "independent".to_string()
            } else {
                clean(&tag.tag_kind)
            };
            Ok((key, kind, sample))
        })
        .collect()
}

// ---- from samples to the one rule -------------------------------------------------------------

pub(super) fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

pub(super) fn median_offset(it: impl Iterator<Item = Offset>) -> Option<Offset> {
    let all: Vec<Offset> = it.collect();
    (!all.is_empty()).then(|| Offset {
        x: median(all.iter().map(|o| o.x).collect()),
        y: median(all.iter().map(|o| o.y).collect()),
    })
}

/// The most common value, the first seen winning a tie, so the answer does not move with order.
pub(super) fn mode(values: impl Iterator<Item = String>) -> String {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for v in values {
        match counts.iter_mut().find(|(s, _)| *s == v) {
            Some(c) => c.1 += 1,
            None => counts.push((v, 1)),
        }
    }
    let best = counts.iter().map(|c| c.1).max().unwrap_or(0);
    counts.into_iter().find(|c| c.1 == best).map(|c| c.0).unwrap_or_default()
}

/// The single placement a rule's samples agree on. Median per component; a rotation is the
/// median of angles taken around the first (so a spread across +-pi does not average to nothing).
/// Samples measured from an item's centre and from its insertion point are offsets from two different places, so
/// they are never mixed: when any sample is from the centre only those count, and the rule is from the centre. A
/// rule whose samples are all older stays from the point until its sheets are pushed again.
pub fn reduce(samples: &[Sample]) -> Placement {
    let centred: Vec<Sample> = samples.iter().filter(|s| s.from_centre).cloned().collect();
    let from_centre = !centred.is_empty();
    let samples = if from_centre { &centred[..] } else { samples };
    let anchor = samples.first().map_or(0.0, |s| s.rotation);
    let rotation = wrap(anchor + median(samples.iter().map(|s| wrap(s.rotation - anchor)).collect()));
    let leaders = samples.iter().filter(|s| s.has_leader).count();
    let has_leader = leaders * 2 > samples.len();
    let with_leader = || samples.iter().filter(|s| s.has_leader);
    Placement {
        head: median_offset(samples.iter().map(|s| s.head)).unwrap_or(Offset { x: 0.0, y: 0.0 }),
        rotation,
        orientation: mode(samples.iter().map(|s| s.orientation.clone())),
        has_leader,
        leader_end_condition: if has_leader {
            mode(with_leader().map(|s| s.leader_end_condition.clone()))
        } else {
            String::new()
        },
        elbow: if has_leader {
            median_offset(with_leader().filter_map(|s| s.elbow))
        } else {
            None
        },
        end: if has_leader {
            median_offset(with_leader().filter_map(|s| s.end))
        } else {
            None
        },
        boundary: reduce_boundary(samples),
        from_centre,
    }
}

/// The boundary recipe the samples agree on: the more common kind, then the more common edge of
/// that kind, then the median of how far along and how far in. `None` when no sample had one.
pub(super) fn reduce_boundary(samples: &[Sample]) -> Option<Boundary> {
    let all: Vec<Boundary> = samples.iter().filter_map(|s| s.boundary).collect();
    let walls = all.iter().filter(|b| b.of == BoundaryOf::Wall).count();
    let of = if walls * 2 >= all.len() && walls > 0 {
        BoundaryOf::Wall
    } else {
        BoundaryOf::Rectangle
    };
    let same: Vec<Boundary> = all.into_iter().filter(|b| b.of == of).collect();
    if same.is_empty() {
        return None;
    }
    let edge = mode(same.iter().map(|b| b.edge.to_string())).parse().unwrap_or(0);
    let on: Vec<&Boundary> = same.iter().filter(|b| b.edge == edge).collect();
    Some(Boundary {
        of,
        edge,
        along: median(on.iter().map(|b| b.along).collect()),
        off: median(on.iter().map(|b| b.off).collect()),
    })
}

/// Takes a push of observations in. Each sample replaces the one read from the same place.
pub fn observe(
    projects_dir: &Path,
    project: &str,
    request: ObservationsRequest,
    ctx: &ObserveContext,
) -> Result<ObservationOutcome, TagRuleError> {
    if request.observations.len() > MAX_OBSERVATIONS_PER_PUSH {
        return bad(format!("at most {MAX_OBSERVATIONS_PER_PUSH} observations in one push"));
    }
    let parameter = clean(&request.type_code_parameter);
    let mut outcome = ObservationOutcome::default();
    let mut grouped: HashMap<String, (RuleKey, String, Vec<Sample>)> = HashMap::new();
    for obs in &request.observations {
        for result in samples_of(obs, &parameter, ctx) {
            match result {
                Ok((key, kind, sample)) => {
                    outcome.samples += 1;
                    grouped.entry(key.id()).or_insert_with(|| (key, kind, Vec::new())).2.push(sample);
                }
                Err(reason) => outcome.skipped.push(Skipped { tag_id: obs.tag.id, reason }),
            }
        }
    }
    let _guard = SAVE_LOCK.lock().unwrap();
    let existing = load_all(projects_dir, project)?;
    let by_id: HashMap<String, TagRule> = existing.into_iter().map(|r| (r.id.clone(), r)).collect();
    let mut ids: Vec<String> = grouped.keys().cloned().collect();
    ids.sort();
    let touched: std::collections::HashSet<String> = ids.iter().cloned().collect();
    let sources: Vec<SampleSource> = grouped.values().flat_map(|g| g.2.iter().map(|s| s.source.clone())).collect();
    for id in ids {
        let (key, kind, incoming) = grouped.remove(&id).expect("id came from the map");
        if !by_id.contains_key(&id) && by_id.len() >= MAX_RULES_PER_PROJECT {
            return bad(format!("a project holds at most {MAX_RULES_PER_PROJECT} tag rules"));
        }
        let mut samples = by_id.get(&id).map(|r| r.samples.clone()).unwrap_or_default();
        for s in incoming {
            samples.retain(|e| e.source != s.source);
            samples.push(s);
        }
        if samples.len() > MAX_SAMPLES_PER_RULE {
            samples.drain(..samples.len() - MAX_SAMPLES_PER_RULE);
        }
        let rule = TagRule {
            schema_version: SCHEMA_VERSION,
            id: id.clone(),
            key,
            tag_kind: kind,
            placement: reduce(&samples),
            sample_count: samples.len(),
            updated_at: now(),
            samples,
        };
        write(projects_dir, project, &rule)?;
        outcome.rules_changed += 1;
        outcome.rule_ids.push(rule.id.clone());
    }
    outcome.rules_changed +=
        crate::tag_rule_binding::take_moved_samples_out(projects_dir, project, &by_id, &touched, &sources)?;
    Ok(outcome)
}
