//! The door half of a milestone comparison: doors matched by a key between two milestones, the properties that differ, and the
//! report that says plainly when the project names no door key (`not_configured`).

use super::*;

/// One door present in both the baseline and a compared milestone (matched by
/// the configured door key) that has at least one property difference or
/// missing property.
///
/// No `unjoined_sources`, unlike `ChangedRoom`: doors carry no joined reference
/// sources yet (R4), so there is no join for a door to lose.
#[derive(Serialize)]
pub struct ChangedDoor {
    pub key: String,
    /// The baseline door's own id, for display.
    pub door_id: String,
    /// **And the model it belongs to.** A door id is unique only within one
    /// model, so an id alone would not identify the element a reader is being
    /// asked to go and look at — the same reason the QA report carries it.
    pub model_id: String,
    pub differences: Vec<PropertyDifference>,
    pub missing_properties: Vec<MissingProperty>,
}

/// The diff of one compared milestone's doors against the baseline's.
#[derive(Serialize)]
pub struct DoorMilestoneComparison {
    pub milestone: String,
    /// Door-key values present in this milestone but not the baseline.
    pub doors_added: Vec<String>,
    /// Door-key values present in the baseline but not this milestone.
    pub doors_removed: Vec<String>,
    pub changed_doors: Vec<ChangedDoor>,
    /// This milestone's own ambiguous door keys, excluded from the diff above.
    pub duplicate_key_values: Vec<DuplicateKeyValue>,
}

/// The doors half of a project's milestone comparison.
///
/// **Configured separately from rooms**, via `[doors] comparison_key` and
/// `[doors] comparison_properties`. Reusing the room settings would have been
/// smaller and wrong: `Mark` on a door and `Mark` on a room are different
/// properties that happen to share a spelling, so one shared key would silently
/// mean two things.
///
/// A project that compares rooms but not doors gets
/// `comparison_key_configured: false` here and a full room diff beside it —
/// a normal state, not an error, and the reason the two flags are separate.
#[derive(Serialize)]
pub struct DoorComparisonReport {
    pub comparison_key_configured: bool,
    /// The door property matched on. `None` iff `comparison_key_configured` is
    /// false.
    pub comparison_key: Option<String>,
    /// The comparable door property set (from settings), echoed for the client.
    pub compared_properties: Vec<String>,
    /// The baseline side's own ambiguous door keys, computed once.
    pub baseline_duplicate_key_values: Vec<DuplicateKeyValue>,
    /// One entry per compared milestone, in the order given.
    pub comparisons: Vec<DoorMilestoneComparison>,
}

impl DoorComparisonReport {
    /// The "no door comparison key configured" result. Distinct from the room
    /// one so a project can have exactly one of the two configured.
    pub(super) fn not_configured() -> Self {
        Self {
            comparison_key_configured: false,
            comparison_key: None,
            compared_properties: vec![],
            baseline_duplicate_key_values: vec![],
            comparisons: vec![],
        }
    }
}

/// A door-key value → the single door that resolved it. Doors whose key value
/// is shared are excluded and surfaced as `DuplicateKeyValue`s instead; doors
/// resolving no key value at all can't be matched and are dropped.
pub(super) type DoorKeyIndex<'a> = BTreeMap<String, &'a OpeningResponse>;

/// Index one milestone's doors by their resolved key value, pulling out any
/// value shared by more than one door.
///
/// **The duplicate guard matters more here than for rooms**, because the
/// obvious door key (`$id`) is unique per model but *not* across models: a
/// project merging two linked models can legitimately have the same door
/// ElementId twice, and matching those across milestones would diff two
/// unrelated leaves against each other. Reported rather than guessed.
pub(super) fn index_doors_by_key<'a>(
    doors: &'a [OpeningResponse],
    key_prop: &str,
    known: &std::collections::BTreeSet<String>,
    builtin: &[BuiltinPropertyDef],
) -> (DoorKeyIndex<'a>, Vec<DuplicateKeyValue>) {
    let mut groups: BTreeMap<String, Vec<&OpeningResponse>> = BTreeMap::new();
    for door in doors {
        if let PropertyPresence::Present(value) = openings_service::resolve_presence(door, key_prop, known, builtin) {
            groups.entry(value).or_default().push(door);
        }
    }

    let mut index = DoorKeyIndex::new();
    let mut duplicates = Vec::new();
    for (value, matched) in groups {
        if matched.len() > 1 {
            duplicates.push(DuplicateKeyValue { value, room_ids: matched.iter().map(|d| d.door.id.clone()).collect() });
        } else {
            index.insert(value, matched[0]);
        }
    }
    (index, duplicates)
}

/// Compare one common door over the comparable property set, enumerated from
/// the *baseline* — the same asymmetry `diff_room` applies, and for the same
/// reason: only a property the baseline door actually has is comparable.
///
/// Values are compared with `values_agree` under an unqualified source: a door
/// carries no joined reference data, so there is no per-source date rung to
/// select and both sides came through the same Revit export at different times.
pub(super) fn diff_door(
    key: &str,
    baseline: &OpeningResponse,
    other: &OpeningResponse,
    properties: &[String],
    known: &std::collections::BTreeSet<String>,
    builtin: &[BuiltinPropertyDef],
) -> Option<ChangedDoor> {
    let mut differences = Vec::new();
    let mut missing_properties = Vec::new();

    for property in properties {
        let PropertyPresence::Present(baseline_value) =
            openings_service::resolve_presence(baseline, property, known, builtin)
        else {
            continue;
        };
        match openings_service::resolve_presence(other, property, known, builtin) {
            PropertyPresence::Absent => {
                missing_properties.push(MissingProperty { property: property.clone(), baseline_value })
            }
            PropertyPresence::Empty => differences.push(PropertyDifference {
                property: property.clone(),
                baseline_value,
                other_value: String::new(),
            }),
            PropertyPresence::Present(other_value) => {
                if !values_agree(&baseline_value, &other_value, None, None) {
                    differences.push(PropertyDifference { property: property.clone(), baseline_value, other_value });
                }
            }
        }
    }

    if differences.is_empty() && missing_properties.is_empty() {
        return None; // unchanged door — omitted from the report
    }
    Some(ChangedDoor {
        key: key.to_string(),
        door_id: baseline.door.id.clone(),
        model_id: baseline.model_id.clone(),
        differences,
        missing_properties,
    })
}

/// The doors half of `compare_milestones`, run over the same baseline and
/// `others` as the rooms half.
///
/// Doors come from `assemble_doors`, so a milestone's *pinned doors snapshot*
/// (`Milestone::door_attachments`) is what gets diffed — the same "a new
/// consumer of machinery that already exists" discipline the rooms comparison
/// follows, rather than a second resolution path.
pub(super) fn compare_doors(
    state: &AppState,
    bundle: &crate::state::ProjectSettings,
    project: &str,
    baseline: &str,
    others: &[String],
) -> Result<DoorComparisonReport, ServiceError> {
    let Some(key_prop) = bundle.doors.comparison_key.clone() else {
        return Ok(DoorComparisonReport::not_configured());
    };
    let properties = bundle.doors.comparison_properties.clone();
    let builtin = &bundle.builtin_properties;
    let known: std::collections::BTreeSet<String> = bundle.reference.keys().cloned().collect();

    /// One milestone's full door set for a project: no property filter, because
    /// a comparison is only meaningful over the whole scope on both sides.
    /// `service::openings`' counterpart of `scope` below.
    fn door_scope<'a>(project: &'a str, milestone: &'a str) -> OpeningScope<'a> {
        OpeningScope { project: Some(project), milestone: Some(milestone), ..Default::default() }
    }

    let baseline_doors = openings_service::assemble_openings::<crate::contract::DoorPayload>(
        state,
        OpeningKind::Doors,
        &door_scope(project, baseline),
    )?
    .map(|r| r.openings)
    .unwrap_or_default();
    let (baseline_index, baseline_duplicates) = index_doors_by_key(&baseline_doors, &key_prop, &known, builtin);

    let mut comparisons = Vec::new();
    for other in others {
        if other == baseline {
            continue;
        }
        let other_doors = openings_service::assemble_openings::<crate::contract::DoorPayload>(
            state,
            OpeningKind::Doors,
            &door_scope(project, other),
        )?
        .map(|r| r.openings)
        .unwrap_or_default();
        let (other_index, other_duplicates) = index_doors_by_key(&other_doors, &key_prop, &known, builtin);

        let doors_added: Vec<String> =
            other_index.keys().filter(|k| !baseline_index.contains_key(*k)).cloned().collect();
        let doors_removed: Vec<String> =
            baseline_index.keys().filter(|k| !other_index.contains_key(*k)).cloned().collect();

        let changed_doors: Vec<ChangedDoor> = baseline_index
            .iter()
            .filter_map(|(key, base)| {
                let other_door = other_index.get(key)?;
                diff_door(key, base, other_door, &properties, &known, builtin)
            })
            .collect();

        comparisons.push(DoorMilestoneComparison {
            milestone: other.clone(),
            doors_added,
            doors_removed,
            changed_doors,
            duplicate_key_values: other_duplicates,
        });
    }

    Ok(DoorComparisonReport {
        comparison_key_configured: true,
        comparison_key: Some(key_prop),
        compared_properties: properties,
        baseline_duplicate_key_values: baseline_duplicates,
        comparisons,
    })
}
