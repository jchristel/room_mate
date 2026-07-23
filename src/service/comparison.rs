//! Milestone comparison: a *star* diff of N milestones against one chosen
//! baseline (baseline-vs-each-other, never all-pairs). For each compared
//! milestone it reports the rooms added and removed relative to the baseline,
//! and, on rooms present in both, the differences over a user-defined,
//! persisted set of comparable properties.
//!
//! This is deliberately a **new consumer** of machinery that already exists,
//! not a reimplementation:
//! - `rooms::assemble_rooms(.., Some(milestone))` already resolves a
//!   milestone's pinned per-model snapshots into fully-joined rooms — called
//!   once per milestone (baseline + each other), it gives every milestone's
//!   room set with no new resolution logic. Because the (optional) dRofus
//!   join, pinned per milestone, happens inside it, a `drofus.`-qualified
//!   comparison field diffs the *pinned* dRofus values with no
//!   comparison-side plumbing.
//! - `rooms::resolve_presence` resolves each comparable field in the same
//!   `source.property` vocabulary the `/rooms` filter uses — `Area` reads the
//!   room, `drofus.NetArea` the joined record — so a name means the same
//!   thing filtered and compared, and gives the Absent/Empty/Present
//!   distinction: `Absent` on the compared side of a property the baseline
//!   has is the "missing property" signal.
//! - `contract::numeric_match` is the numeric-adaptive comparator the dRofus
//!   QA path uses; property equality reuses it rather than growing a second one.
//!
//! The room-matching key is its own concept: a **user-chosen** room property
//! (`ProjectSettings::comparison_key`, possibly `drofus.`-qualified),
//! deliberately NOT the dRofus `link_property`. dRofus data is *comparable*
//! here, but never *required*: when no key is configured this returns an
//! explicit "not configured" result rather than silently falling back to
//! dRofus or to room `id` — that no-fallback rule predates joined-source
//! comparison and survives it unchanged.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::contract::{numeric_match, PropertyPresence};
use crate::settings::BuiltinPropertyDef;
use crate::state::AppState;

use super::rooms::{assemble_rooms, resolve_presence, source_joined, RoomResponse, RoomScope};
use super::ServiceError;

/// One comparable property whose value differs between the baseline room and
/// the same room in a compared milestone. Values are the raw room-property
/// strings; equality is the numeric-adaptive comparison reused from the dRofus
/// QA path (see `values_agree`).
#[derive(Serialize)]
pub struct PropertyDifference {
    pub property: String,
    pub baseline_value: String,
    pub other_value: String,
}

/// A comparable property the baseline room has but the compared milestone's
/// matched room does not — a distinct state from a value difference (the
/// property was never extracted for that room on the other side), so it is
/// reported separately rather than as a difference against an empty value.
#[derive(Serialize)]
pub struct MissingProperty {
    pub property: String,
    pub baseline_value: String,
}

/// One room present in both the baseline and a compared milestone (matched by
/// the user-defined key) that has at least one property difference or missing
/// property. `key` is the shared comparison-key value; `room_id` is the
/// baseline room's own id, for display.
#[derive(Serialize)]
pub struct ChangedRoom {
    pub key: String,
    pub room_id: String,
    pub differences: Vec<PropertyDifference>,
    pub missing_properties: Vec<MissingProperty>,
    /// Joined sources (e.g. `"drofus"`) named by the comparable-property set
    /// whose record is missing *entirely* on the compared side, while the
    /// baseline side has values to compare. One entry per source, replacing
    /// what would otherwise be N identical `missing_properties` rows — a
    /// failed join is one per-room fact, not a fact about each configured
    /// field. A room whose only change is a lost join still appears here:
    /// losing the join IS the change.
    pub unjoined_sources: Vec<String>,
}

/// One comparison-key value shared by more than one room on a single side —
/// ambiguous, so it can't be matched across milestones and is excluded from
/// the added/removed/changed logic and reported here instead. This matters
/// more than the dRofus duplicate guard it mirrors (`DuplicateLinkValue`),
/// because the key is arbitrary user config with no uniqueness guarantee.
#[derive(Serialize)]
pub struct DuplicateKeyValue {
    pub value: String,
    pub room_ids: Vec<String>,
}

/// The diff of one compared milestone against the baseline.
#[derive(Serialize)]
pub struct MilestoneComparison {
    /// The compared ("other") milestone's name.
    pub milestone: String,
    /// Comparison-key values present in this milestone but not the baseline.
    pub rooms_added: Vec<String>,
    /// Comparison-key values present in the baseline but not this milestone.
    pub rooms_removed: Vec<String>,
    /// Rooms present in both (by key) with at least one property difference or
    /// missing property. A room in both that agrees on everything is omitted.
    pub changed_rooms: Vec<ChangedRoom>,
    /// This milestone's own ambiguous keys (a key value shared by more than one
    /// of its rooms), excluded from the diff above.
    pub duplicate_key_values: Vec<DuplicateKeyValue>,
}

/// The whole comparison report for one project.
#[derive(Serialize)]
pub struct ComparisonResponse {
    /// False when the project has no `comparison_key` configured (or has no
    /// registered settings at all) — every list below is then empty. A real,
    /// reachable state, since dRofus is optional here and the key is its own
    /// separate setting: surfaced plainly, not treated as an error.
    pub comparison_key_configured: bool,
    /// The property matched on, echoed for the client. `None` iff
    /// `comparison_key_configured` is false.
    pub comparison_key: Option<String>,
    /// The baseline milestone every other is compared against.
    pub baseline: String,
    /// The comparable property set (from settings), echoed for the client.
    pub compared_properties: Vec<String>,
    /// The baseline side's own ambiguous keys, computed once (they are the same
    /// for every compared milestone, so they live here rather than repeated in
    /// each `MilestoneComparison`).
    pub baseline_duplicate_key_values: Vec<DuplicateKeyValue>,
    /// One entry per compared milestone, in the order given.
    pub comparisons: Vec<MilestoneComparison>,
}

impl ComparisonResponse {
    /// The "no comparison key configured" result — an unregistered project or
    /// one whose settings leave `comparison_key` unset. Not an error: the
    /// feature simply has no way to match rooms across milestones, and the
    /// client renders that state plainly.
    fn not_configured(baseline: &str) -> Self {
        Self {
            comparison_key_configured: false,
            comparison_key: None,
            baseline: baseline.to_string(),
            compared_properties: vec![],
            baseline_duplicate_key_values: vec![],
            comparisons: vec![],
        }
    }
}

/// A comparison-key value → the single room that resolved it. Rooms whose key
/// value is shared (ambiguous) are excluded from this map and surfaced as
/// `DuplicateKeyValue`s instead; rooms that resolve no key value at all can't
/// be matched and are dropped. Borrows out of the assembled room set.
type KeyIndex<'a> = BTreeMap<String, &'a RoomResponse>;

/// Index one milestone's rooms by their resolved comparison-key value, pulling
/// out any value shared by more than one room as a duplicate (mirrors how
/// `validation::compute_validation` guards ambiguous dRofus link values). The
/// key resolves through `resolve_presence` — the same namespace vocabulary as
/// the comparable properties — so a `drofus.`-qualified key (matching rooms
/// across milestones by their dRofus identity) works exactly like a room
/// property does.
fn index_by_key<'a>(
    rooms: &'a [RoomResponse],
    key_prop: &str,
    builtin: &[BuiltinPropertyDef],
) -> (KeyIndex<'a>, Vec<DuplicateKeyValue>) {
    let mut groups: BTreeMap<String, Vec<&RoomResponse>> = BTreeMap::new();
    for rr in rooms {
        // Only `Present` yields a usable key: a room with no value for it
        // (absent and empty collapse together here) can't be matched across
        // milestones — dropped, there is nothing to diff it against.
        if let (_, PropertyPresence::Present(value)) = resolve_presence(rr, key_prop, builtin) {
            groups.entry(value).or_default().push(rr);
        }
    }

    let mut index = KeyIndex::new();
    let mut duplicates = Vec::new();
    for (value, matched) in groups {
        if matched.len() > 1 {
            duplicates.push(DuplicateKeyValue {
                value,
                room_ids: matched.iter().map(|r| r.room.id.clone()).collect(),
            });
        } else {
            index.insert(value, matched[0]);
        }
    }
    (index, duplicates)
}

/// Two property values agree iff they match numerically (numeric-adaptive, the
/// same tolerance the dRofus QA path uses) or, when either side isn't a number,
/// as trimmed strings. Deliberately does NOT pull in the dRofus QA path's date
/// and ASCII-narrowing rungs: for an unqualified field both sides came through
/// the same Revit export, so any such artefact is symmetric and cancels.
///
/// TODO(HANDOVER-comparison-sources.md step 4): that reasoning is weaker for a
/// `drofus.`-qualified field — two dRofus exports can render the same date
/// differently, which this comparator reports as a difference. Fixing it means
/// a source-aware ladder keyed on `resolve_presence`'s returned namespace.
/// `validation::field_values_agree` must NOT be reused as-is for that: it is
/// asymmetric (narrows only its dRofus side, comparing dRofus *against*
/// Revit), and a milestone diff of a dRofus field is dRofus-vs-dRofus.
fn values_agree(a: &str, b: &str) -> bool {
    match numeric_match(a, b) {
        Some(equal) => equal,
        None => a.trim() == b.trim(),
    }
}

/// Compare one common room (present in both baseline and other, by key) over
/// the comparable property set. Enumerated from the *baseline*: only a property
/// the baseline room actually has (`Present`) is comparable. On the other side,
/// `Absent` is a missing property (reported distinctly); `Empty` or a
/// disagreeing `Present` is a value difference (baseline value vs the other's,
/// empty for `Empty`) — except when the whole joined source is missing on the
/// other side, which collapses to one `unjoined_sources` entry (see below).
fn diff_room(
    key: &str,
    baseline: &RoomResponse,
    other: &RoomResponse,
    properties: &[String],
    builtin: &[BuiltinPropertyDef],
) -> Option<ChangedRoom> {
    let mut differences = Vec::new();
    let mut missing_properties = Vec::new();
    let mut unjoined_sources: Vec<String> = Vec::new();

    for property in properties {
        // "Only properties that exist on the baseline may be compared." That
        // rule is deliberately asymmetric and covers joined sources too: a
        // baseline room with no dRofus record has nothing to compare, so a
        // join *gained* on the other side goes unreported — exactly as a
        // property gained on the other side always has.
        let (_, base) = resolve_presence(baseline, property, builtin);
        let PropertyPresence::Present(baseline_value) = base else {
            continue;
        };

        match resolve_presence(other, property, builtin) {
            // The whole source is unmatched on the other side: one per-room
            // fact, recorded once — N identical missing-property rows would
            // bury the actual signal (the room lost its join).
            (Some(ns), PropertyPresence::Absent) if !source_joined(other, ns) => {
                if !unjoined_sources.iter().any(|s| s == ns) {
                    unjoined_sources.push(ns.to_string());
                }
            }
            (_, PropertyPresence::Absent) => missing_properties.push(MissingProperty {
                property: property.clone(),
                baseline_value,
            }),
            (_, PropertyPresence::Empty) => differences.push(PropertyDifference {
                property: property.clone(),
                baseline_value,
                other_value: String::new(),
            }),
            (_, PropertyPresence::Present(other_value)) => {
                if !values_agree(&baseline_value, &other_value) {
                    differences.push(PropertyDifference {
                        property: property.clone(),
                        baseline_value,
                        other_value,
                    });
                }
            }
        }
    }

    // `unjoined_sources` alone keeps the room in the report: an otherwise
    // unchanged room that lost its dRofus join has changed — that loss is the
    // reportable fact, not a reason to filter the room out.
    if differences.is_empty() && missing_properties.is_empty() && unjoined_sources.is_empty() {
        return None; // unchanged room — omitted from the report
    }
    Some(ChangedRoom {
        key: key.to_string(),
        room_id: baseline.room.id.clone(),
        differences,
        missing_properties,
        unjoined_sources,
    })
}

/// One milestone's full room set for a project: no building narrowing and no
/// property filter, because a comparison is only meaningful over the whole
/// scope on both sides of the diff.
fn scope<'a>(project: &'a str, milestone: &'a str) -> RoomScope<'a> {
    RoomScope { project: Some(project), milestone: Some(milestone), ..Default::default() }
}

/// Compare N milestones against a baseline for one project. Reads the
/// comparison key and the comparable-property list from the project's settings
/// bundle (both persisted config), not from arguments. Each milestone's rooms
/// come from `assemble_rooms`, so milestone resolution, level dedup, and the
/// (irrelevant-here-but-harmless) dRofus join all run exactly as they do for
/// the viewer. `others` is the set of milestones to compare; any entry equal to
/// the baseline is skipped (a milestone versus itself is empty by construction).
///
/// An unregistered project, or one with no `comparison_key`, yields a
/// `comparison_key_configured: false` result — not an error, the same
/// soft-signal discipline every other read path follows.
pub fn compare_milestones(
    state: &AppState,
    project: &str,
    baseline: &str,
    others: &[String],
) -> Result<ComparisonResponse, ServiceError> {
    let registry = state.settings();
    let Some(bundle) = registry.settings_for(project) else {
        return Ok(ComparisonResponse::not_configured(baseline));
    };
    let Some(key_prop) = bundle.comparison_key.clone() else {
        return Ok(ComparisonResponse::not_configured(baseline));
    };
    let properties = bundle.comparison_properties.clone();
    let builtin = &bundle.builtin_properties;

    // Baseline once; its index and duplicates are shared across every compared
    // milestone. `assemble_rooms` returns None only when the store is entirely
    // empty — an empty room set for comparison purposes.
    let baseline_rooms = assemble_rooms(state, &scope(project, baseline))?
        .map(|r| r.rooms)
        .unwrap_or_default();
    let (baseline_index, baseline_duplicates) = index_by_key(&baseline_rooms, &key_prop, builtin);

    let mut comparisons = Vec::new();
    for other in others {
        if other == baseline {
            continue; // a milestone compared against itself has nothing to show
        }

        let other_rooms = assemble_rooms(state, &scope(project, other))?
            .map(|r| r.rooms)
            .unwrap_or_default();
        let (other_index, other_duplicates) = index_by_key(&other_rooms, &key_prop, builtin);

        // Added = in other, not baseline. Removed = in baseline, not other.
        // Duplicated keys are absent from both indexes, so they're already
        // excluded from these diffs (the same exclusion validation applies).
        let rooms_added: Vec<String> = other_index
            .keys()
            .filter(|k| !baseline_index.contains_key(*k))
            .cloned()
            .collect();
        let rooms_removed: Vec<String> = baseline_index
            .keys()
            .filter(|k| !other_index.contains_key(*k))
            .cloned()
            .collect();

        // Rooms in both: compare their comparable properties.
        let changed_rooms: Vec<ChangedRoom> = baseline_index
            .iter()
            .filter_map(|(key, base_room)| {
                let other_room = other_index.get(key)?;
                diff_room(key, base_room, other_room, &properties, builtin)
            })
            .collect();

        comparisons.push(MilestoneComparison {
            milestone: other.clone(),
            rooms_added,
            rooms_removed,
            changed_rooms,
            duplicate_key_values: other_duplicates,
        });
    }

    Ok(ComparisonResponse {
        comparison_key_configured: true,
        comparison_key: Some(key_prop),
        baseline: baseline.to_string(),
        compared_properties: properties,
        baseline_duplicate_key_values: baseline_duplicates,
        comparisons,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{CustomValue, Level, Model, Project, Room, RoomPayload, Snapshot};
    use crate::settings::Milestone;
    use crate::state::ProjectSettings;
    use crate::storage::FsStore;
    use std::collections::BTreeMap;

    fn make_room(id: &str, props: &[(&str, &str)]) -> Room {
        let mut properties = BTreeMap::new();
        for (k, v) in props {
            properties.insert(k.to_string(), CustomValue { value: v.to_string(), storage_type: None });
        }
        Room { id: id.to_string(), name: id.to_string(), level_id: "1".to_string(), loops: vec![], properties }
    }

    fn milestone(name: &str, model_id: &str, taken_at: &str) -> Milestone {
        Milestone {
            name: name.to_string(),
            date: "2026-06-30".to_string(),
            drofus_snapshot: None,
            attachments: BTreeMap::from([(model_id.to_string(), taken_at.to_string())]),
        }
    }

    /// `milestone` plus a dRofus snapshot pin — the joined-source tests need
    /// each milestone to resolve its own dRofus data.
    fn milestone_with_drofus(name: &str, model_id: &str, taken_at: &str, drofus_ts: &str) -> Milestone {
        Milestone { drofus_snapshot: Some(drofus_ts.to_string()), ..milestone(name, model_id, taken_at) }
    }

    /// A *current* dRofus dataset: link property + one record per `(id,
    /// fields)` entry. Attached to a bundle by the joined-source tests —
    /// `make_bundle` itself stays dRofus-free, because comparison standing
    /// alone without dRofus is a design property under regression guard.
    fn drofus_data(link: &str, records: &[(&str, &[(&str, &str)])]) -> crate::drofus::DrofusData {
        crate::drofus::DrofusData {
            link_property: link.to_string(),
            by_id: records
                .iter()
                .map(|(id, fields)| {
                    (
                        id.to_string(),
                        crate::drofus::DrofusRecord {
                            fields: fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                        },
                    )
                })
                .collect(),
            reconciliation: BTreeMap::new(),
            all_labels: vec![],
        }
    }

    /// A bundle with the given comparison key/properties and milestones, no
    /// dRofus (proving milestone comparison stands alone without it).
    fn make_bundle(
        comparison_key: Option<&str>,
        comparison_properties: &[&str],
        milestones: Vec<Milestone>,
    ) -> ProjectSettings {
        ProjectSettings {
            drofus: None,
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec!["$name".to_string()],
            drofus_fields: vec![],
            milestones,
            comparison_key: comparison_key.map(|s| s.to_string()),
            comparison_properties: comparison_properties.iter().map(|s| s.to_string()).collect(),
            hierarchy_exclusions: vec![],
        }
    }

    fn payload_at(model_id: &str, taken_at: &str, rooms: Vec<Room>) -> RoomPayload {
        RoomPayload {
            schema_version: 5,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: model_id.to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: taken_at.to_string() },
            model_to_shared: None,
            levels: vec![Level { id: "1".to_string(), name: "Level 1".to_string(), elevation: 0.0 }],
            rooms,
        }
    }

    fn state_with(bundle: ProjectSettings, tag: &str) -> (AppState, std::path::PathBuf) {
        // FsStore because milestone pins address snapshot history, which
        // MemStore does not keep.
        let dir = std::env::temp_dir().join(format!("roommate-cmp-{}-{}", tag, std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let store = FsStore::new(dir.clone()).unwrap();
        let registry = std::collections::HashMap::from([("p1".to_string(), bundle)]);
        (AppState::new(Box::new(store), registry, None), dir)
    }

    /// No `comparison_key` configured → a plain "not configured" result, every
    /// list empty. The reachable dRofus-absent / key-unset state.
    #[test]
    fn test_compare_no_key_configured() {
        let bundle = make_bundle(None, &["Area"], vec![]);
        let (state, dir) = state_with(bundle, "no-key");

        let result = compare_milestones(&state, "p1", "Base", &["Later".to_string()]).unwrap();

        assert!(!result.comparison_key_configured);
        assert!(result.comparison_key.is_none());
        assert!(result.comparisons.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The core diff: added, removed, a value difference, and a missing
    /// property, all against one baseline. Two milestones pin the same model
    /// to two different snapshots.
    #[test]
    fn test_compare_added_removed_changed_missing() {
        let base_ts = "2026-06-01T00:00:00Z";
        let later_ts = "2026-07-01T00:00:00Z";
        let bundle = make_bundle(
            Some("Number"),
            &["Area", "Dept"],
            vec![milestone("Base", "m1", base_ts), milestone("Later", "m1", later_ts)],
        );
        let (state, dir) = state_with(bundle, "core");

        // Baseline (Number is the match key):
        //  R1: Area 10, Dept Cardio   R2: Area 20   (R2 only in baseline → removed)
        let base = payload_at("m1", base_ts, vec![
            make_room("r1", &[("Number", "101"), ("Area", "10"), ("Dept", "Cardio")]),
            make_room("r2", &[("Number", "102"), ("Area", "20")]),
        ]);
        // Later:
        //  R1: Area 15 (changed), Dept absent (missing)   R3: new (added)
        let later = payload_at("m1", later_ts, vec![
            make_room("r1b", &[("Number", "101"), ("Area", "15")]),
            make_room("r3", &[("Number", "103"), ("Area", "30")]),
        ]);
        state.set_snapshot(base).unwrap();
        state.set_snapshot(later).unwrap();

        let result = compare_milestones(&state, "p1", "Base", &["Later".to_string()]).unwrap();

        assert!(result.comparison_key_configured);
        assert_eq!(result.comparisons.len(), 1);
        let cmp = &result.comparisons[0];
        assert_eq!(cmp.milestone, "Later");
        assert_eq!(cmp.rooms_added, vec!["103".to_string()]);
        assert_eq!(cmp.rooms_removed, vec!["102".to_string()]);

        assert_eq!(cmp.changed_rooms.len(), 1);
        let changed = &cmp.changed_rooms[0];
        assert_eq!(changed.key, "101");
        assert_eq!(changed.room_id, "r1", "reports the baseline room's id");
        assert_eq!(changed.differences.len(), 1);
        assert_eq!(changed.differences[0].property, "Area");
        assert_eq!(changed.differences[0].baseline_value, "10");
        assert_eq!(changed.differences[0].other_value, "15");
        assert_eq!(changed.missing_properties.len(), 1);
        assert_eq!(changed.missing_properties[0].property, "Dept");
        assert_eq!(changed.missing_properties[0].baseline_value, "Cardio");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A numeric artifact (e.g. 10 vs 10.0) is not a difference — property
    /// equality reuses the numeric-adaptive comparator, not raw string compare.
    #[test]
    fn test_compare_numeric_values_agree() {
        let base_ts = "2026-06-01T00:00:00Z";
        let later_ts = "2026-07-01T00:00:00Z";
        let bundle = make_bundle(
            Some("Number"),
            &["Area"],
            vec![milestone("Base", "m1", base_ts), milestone("Later", "m1", later_ts)],
        );
        let (state, dir) = state_with(bundle, "numeric");

        state.set_snapshot(payload_at("m1", base_ts, vec![make_room("r1", &[("Number", "1"), ("Area", "10")])])).unwrap();
        state.set_snapshot(payload_at("m1", later_ts, vec![make_room("r1", &[("Number", "1"), ("Area", "10.0")])])).unwrap();

        let result = compare_milestones(&state, "p1", "Base", &["Later".to_string()]).unwrap();
        assert!(result.comparisons[0].changed_rooms.is_empty(), "10 and 10.0 agree numerically");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A key value shared by two rooms on one side is ambiguous: reported as a
    /// duplicate and excluded from the added/removed/changed logic.
    #[test]
    fn test_compare_duplicate_key_excluded() {
        let base_ts = "2026-06-01T00:00:00Z";
        let later_ts = "2026-07-01T00:00:00Z";
        let bundle = make_bundle(
            Some("Number"),
            &["Area"],
            vec![milestone("Base", "m1", base_ts), milestone("Later", "m1", later_ts)],
        );
        let (state, dir) = state_with(bundle, "dup");

        // Two baseline rooms share Number 101 → ambiguous.
        state.set_snapshot(payload_at("m1", base_ts, vec![
            make_room("r1", &[("Number", "101"), ("Area", "10")]),
            make_room("r2", &[("Number", "101"), ("Area", "20")]),
        ])).unwrap();
        state.set_snapshot(payload_at("m1", later_ts, vec![
            make_room("r1", &[("Number", "101"), ("Area", "99")]),
        ])).unwrap();

        let result = compare_milestones(&state, "p1", "Base", &["Later".to_string()]).unwrap();

        assert_eq!(result.baseline_duplicate_key_values.len(), 1);
        assert_eq!(result.baseline_duplicate_key_values[0].value, "101");
        let cmp = &result.comparisons[0];
        // 101 is ambiguous on the baseline, so it is neither "removed" nor
        // "changed"; the later 101 is "added" (it isn't in the baseline index).
        assert!(cmp.rooms_removed.is_empty());
        assert!(cmp.changed_rooms.is_empty());
        assert_eq!(cmp.rooms_added, vec!["101".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `drofus.`-qualified comparable property diffs the *pinned* dRofus
    /// snapshots, not the project's current dRofus — dRofus drift between
    /// milestones is exactly the diff this feature exists to surface, and the
    /// current dataset here carries a decoy value the report must not show.
    #[test]
    fn test_compare_drofus_property_uses_pinned_snapshots() {
        let base_ts = "2026-06-01T00:00:00Z";
        let later_ts = "2026-07-01T00:00:00Z";
        let d_base = "2026-06-01T09:00:00Z";
        let d_later = "2026-07-01T09:00:00Z";
        let mut bundle = make_bundle(
            Some("Number"),
            &["drofus.NetArea"],
            vec![
                milestone_with_drofus("Base", "m1", base_ts, d_base),
                milestone_with_drofus("Later", "m1", later_ts, d_later),
            ],
        );
        // The decoy: if the diff read the current dRofus it would see 99 on
        // both sides and report nothing.
        bundle.drofus = Some(drofus_data("Number", &[("101", &[("NetArea", "99")])]));
        let (state, dir) = state_with(bundle, "drofus-pin");

        state
            .set_snapshot(payload_at("m1", base_ts, vec![make_room("r1", &[("Number", "101")])]))
            .unwrap();
        state
            .set_snapshot(payload_at("m1", later_ts, vec![make_room("r1b", &[("Number", "101")])]))
            .unwrap();
        state.put_drofus("p1", d_base, b"DrofusRoomId,NetArea\nNumber,NetArea\n101,20\n").unwrap();
        state.put_drofus("p1", d_later, b"DrofusRoomId,NetArea\nNumber,NetArea\n101,25\n").unwrap();

        let result = compare_milestones(&state, "p1", "Base", &["Later".to_string()]).unwrap();

        let cmp = &result.comparisons[0];
        assert_eq!(cmp.changed_rooms.len(), 1);
        let changed = &cmp.changed_rooms[0];
        assert_eq!(changed.differences.len(), 1);
        assert_eq!(changed.differences[0].property, "drofus.NetArea");
        assert_eq!(changed.differences[0].baseline_value, "20", "the pinned value, not the current 99");
        assert_eq!(changed.differences[0].other_value, "25");
        assert!(changed.unjoined_sources.is_empty(), "both sides joined");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A room that loses its dRofus join between milestones reports ONE
    /// `unjoined_sources` entry — not one `MissingProperty` row per configured
    /// dRofus field — and still appears in `changed_rooms` even with no other
    /// difference: losing the join IS the change.
    #[test]
    fn test_compare_unjoined_source_collapses_to_one_entry() {
        let base_ts = "2026-06-01T00:00:00Z";
        let later_ts = "2026-07-01T00:00:00Z";
        let mut bundle = make_bundle(
            Some("Number"),
            &["drofus.NetArea", "drofus.Dept"],
            vec![milestone("Base", "m1", base_ts), milestone("Later", "m1", later_ts)],
        );
        // Link property (DKey) is distinct from the comparison key (Number):
        // the baseline room carries a link value, the later one lost it.
        bundle.drofus = Some(drofus_data("DKey", &[("d1", &[("NetArea", "20"), ("Dept", "Admin")])]));
        let (state, dir) = state_with(bundle, "unjoined");

        state
            .set_snapshot(payload_at(
                "m1",
                base_ts,
                vec![make_room("r1", &[("Number", "101"), ("DKey", "d1")])],
            ))
            .unwrap();
        state
            .set_snapshot(payload_at("m1", later_ts, vec![make_room("r1b", &[("Number", "101")])]))
            .unwrap();

        let result = compare_milestones(&state, "p1", "Base", &["Later".to_string()]).unwrap();

        let cmp = &result.comparisons[0];
        assert_eq!(cmp.changed_rooms.len(), 1, "the room appears despite no property difference");
        let changed = &cmp.changed_rooms[0];
        assert_eq!(changed.unjoined_sources, vec!["drofus".to_string()], "one entry, not one per field");
        assert!(changed.missing_properties.is_empty(), "no per-property noise for an unjoined source");
        assert!(changed.differences.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The baseline-enumeration rule covers joined sources: a baseline room
    /// with no dRofus record has nothing to compare, so a join *gained* on the
    /// other side goes unreported — the same deliberate asymmetry as a
    /// property gained on the other side.
    #[test]
    fn test_compare_join_gained_on_other_side_unreported() {
        let base_ts = "2026-06-01T00:00:00Z";
        let later_ts = "2026-07-01T00:00:00Z";
        let mut bundle = make_bundle(
            Some("Number"),
            &["drofus.NetArea"],
            vec![milestone("Base", "m1", base_ts), milestone("Later", "m1", later_ts)],
        );
        bundle.drofus = Some(drofus_data("DKey", &[("d1", &[("NetArea", "20")])]));
        let (state, dir) = state_with(bundle, "join-gained");

        // Baseline unjoined, later joined.
        state
            .set_snapshot(payload_at("m1", base_ts, vec![make_room("r1", &[("Number", "101")])]))
            .unwrap();
        state
            .set_snapshot(payload_at(
                "m1",
                later_ts,
                vec![make_room("r1b", &[("Number", "101"), ("DKey", "d1")])],
            ))
            .unwrap();

        let result = compare_milestones(&state, "p1", "Base", &["Later".to_string()]).unwrap();
        assert!(result.comparisons[0].changed_rooms.is_empty(), "nothing on the baseline to compare");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `drofus.`-qualified `comparison_key` matches rooms across milestones
    /// by their joined dRofus identity — same vocabulary as the properties.
    #[test]
    fn test_compare_drofus_qualified_key_matches_rooms() {
        let base_ts = "2026-06-01T00:00:00Z";
        let later_ts = "2026-07-01T00:00:00Z";
        let mut bundle = make_bundle(
            Some("drofus.Code"),
            &["Area"],
            vec![milestone("Base", "m1", base_ts), milestone("Later", "m1", later_ts)],
        );
        bundle.drofus = Some(drofus_data(
            "Number",
            &[("101", &[("Code", "A")]), ("102", &[("Code", "B")])],
        ));
        let (state, dir) = state_with(bundle, "drofus-key");

        state
            .set_snapshot(payload_at(
                "m1",
                base_ts,
                vec![
                    make_room("r1", &[("Number", "101"), ("Area", "10")]),
                    make_room("r2", &[("Number", "102"), ("Area", "20")]),
                ],
            ))
            .unwrap();
        state
            .set_snapshot(payload_at(
                "m1",
                later_ts,
                vec![
                    make_room("r1b", &[("Number", "101"), ("Area", "15")]),
                    make_room("r2b", &[("Number", "102"), ("Area", "20")]),
                ],
            ))
            .unwrap();

        let result = compare_milestones(&state, "p1", "Base", &["Later".to_string()]).unwrap();

        let cmp = &result.comparisons[0];
        assert!(cmp.rooms_added.is_empty() && cmp.rooms_removed.is_empty(), "both keys matched across");
        assert_eq!(cmp.changed_rooms.len(), 1);
        assert_eq!(cmp.changed_rooms[0].key, "A", "keyed by the joined dRofus field, not a room property");
        assert_eq!(cmp.changed_rooms[0].differences[0].property, "Area");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// TODO-anchor for the deferred source-aware comparator (see the
    /// `values_agree` doc comment): today the strict two-rung ladder applies
    /// to every field, so an ASCII-narrowing artefact on a dRofus-valued field
    /// IS reported as a difference. When step 4 of
    /// HANDOVER-comparison-sources.md lands, this assertion flips for
    /// `drofus.`-qualified fields only.
    #[test]
    fn test_values_agree_is_strict_regardless_of_source() {
        assert!(!values_agree("Room – A", "Room ? A"), "no ASCII-narrowing rung today");
        assert!(values_agree(" x ", "x"), "trimmed string equality");
        assert!(values_agree("10", "10.0"), "numeric-adaptive rung");
    }
}
