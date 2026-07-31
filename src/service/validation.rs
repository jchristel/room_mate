//! Reference reconciliation QA: does each room's Revit data agree with every
//! reference source the project configures.
//!
//! **One report per source, not one report.** Each source declares its own
//! link property, so "which rooms resolved no link value" and "what is this
//! room's link value" are different questions for each of them and cannot
//! share a list. `compute_validation` reconciles one source and knows nothing
//! about the others; `compute_project_validation` runs it once per loaded
//! source and sums the tallies for the panel's collapsed header. A source
//! configured but not yet uploaded is skipped, not failed — "declared,
//! nothing uploaded yet" is a normal state, the same "signal, not error"
//! policy the unmatched-key checks themselves follow.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::contract::{
    date_match, lookup_property, numeric_match, property_presence, PropertyPresence, Room, RoomPayload,
};
use crate::reference::ReferenceData;
use crate::settings::{BuiltinPropertyDef, CompareMode, FieldType, ReferenceFieldConfig};
use crate::state::{AppState, ModelKey};

use super::ServiceError;

/// Resolved link value → every `(room, source)` that resolved to it. A value
/// with more than one entry is an ambiguous (duplicate) link value, excluded
/// from the unmatched/mismatch checks. Borrows the rooms out of the stored
/// payloads, hence the lifetime.
type LinkValueIndex<'a> = BTreeMap<String, Vec<(&'a Room, &'a str)>>;

/// One link-property value shared by more than one room — ambiguous, so it's
/// excluded from the unmatched/mismatch checks below rather than guessing
/// which room a dRofus record actually describes.
#[derive(Serialize)]
pub struct DuplicateLinkValue {
    pub value: String,
    pub room_ids: Vec<String>,
}

/// One property where a uniquely-matched room and its dRofus record disagree.
#[derive(Serialize)]
pub struct PropertyMismatch {
    pub room_id: String,
    pub reference_id: String,
    /// The dRofus field label (row 1) — the same key `reconciliation` and
    /// `ReferenceRecord.fields` use.
    pub field: String,
    pub room_value: String,
    pub reference_value: String,
}

/// One reconciled field where dRofus has a real value but the matched room's
/// corresponding Revit property doesn't (see `PropertyPresence`). Kept as two
/// separate response lists rather than one, because the two cases mean
/// different things: landing here via `Absent` means the property was never
/// extracted from Revit for this room at all -- a mapping typo or a
/// parameter the extractor never wired up, worth flagging loudly; via
/// `Empty` it just means nobody has filled the value in yet, an ordinary
/// per-room gap.
#[derive(Serialize)]
pub struct MissingInRevit {
    pub room_id: String,
    pub reference_id: String,
    pub field: String,
}

/// Whether one dRofus CSV field (row 1) is actually checked by this QA pass,
/// and if so, which Revit property it's checked against. A field overridden
/// `Ignore` in settings is left out of this list entirely -- that's a
/// deliberate exclusion (e.g. a sync timestamp that will legitimately always
/// differ), not a coverage gap someone needs to notice and fix.
#[derive(Serialize)]
pub struct FieldCoverage {
    pub label: String,
    pub checked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revit_property: Option<String>,
}

/// Per-room detail for a room that appears in some discrepancy list — the
/// human-friendly fields the CSV export shows beyond the bare `room_id`. Keyed
/// by `room_id` in `ValidationResponse::error_rooms`. Every field defaults to
/// `""` when the underlying property doesn't resolve (an absent room Number, or
/// a room that resolved no link value at all), so a consumer never has to
/// distinguish "absent" from "empty" here — the discrepancy lists already carry
/// that distinction where it matters.
#[derive(Serialize)]
pub struct ErrorRoomInfo {
    /// The room's Revit "Number" parameter value (resolved via `lookup_property`).
    pub number: String,
    /// The room's Revit "Name" parameter value (resolved via `lookup_property`).
    pub name: String,
    /// The room's dRofus link value — its value for the link property. `""` when
    /// the room resolved none (i.e. it's in `rooms_missing_link_value`).
    pub link_value: String,
}

/// Discrepancy tallies so a consumer (MCP `get_validation`, the browser panel)
/// can answer "how many discrepancies?" without re-summing the six lists.
/// Category counts are the **list lengths** — `duplicate_link_values` counts
/// duplicate-value *groups*, not the rooms in them — matching the panel's
/// existing issue count. `total` is their sum.
#[derive(Serialize, Default)]
pub struct DiscrepancyCounts {
    pub total: usize,
    pub rooms_missing_link_value: usize,
    pub duplicate_link_values: usize,
    pub rooms_unmatched: usize,
    pub property_mismatches: usize,
    pub fields_absent_in_revit: usize,
    pub fields_empty_in_revit: usize,
}

impl DiscrepancyCounts {
    /// Accumulate another source's tallies into this one — how the response's
    /// cross-source total is built, so the QA header can show one number
    /// without the client re-summing every section.
    fn add(&mut self, other: &DiscrepancyCounts) {
        self.total += other.total;
        self.rooms_missing_link_value += other.rooms_missing_link_value;
        self.duplicate_link_values += other.duplicate_link_values;
        self.rooms_unmatched += other.rooms_unmatched;
        self.property_mismatches += other.property_mismatches;
        self.fields_absent_in_revit += other.fields_absent_in_revit;
        self.fields_empty_in_revit += other.fields_empty_in_revit;
    }
}

/// One reference source's reconciliation report. Every list here is scoped to
/// that source: each source declares its own link property, so "which rooms
/// failed to resolve a link value" and "what is this room's link value" are
/// different questions per source and cannot share a list.
#[derive(Serialize)]
pub struct SourceValidation {
    pub link_property: String,
    pub rooms_missing_link_value: Vec<String>,
    pub duplicate_link_values: Vec<DuplicateLinkValue>,
    pub rooms_unmatched: Vec<String>,
    pub property_mismatches: Vec<PropertyMismatch>,
    pub fields_absent_in_revit: Vec<MissingInRevit>,
    pub fields_empty_in_revit: Vec<MissingInRevit>,
    pub field_coverage: Vec<FieldCoverage>,
    /// Discrepancy tallies (total + per-category) — see `DiscrepancyCounts`.
    pub discrepancies: DiscrepancyCounts,
    /// `room_id` → its `ErrorRoomInfo`, populated only for rooms that appear in
    /// some discrepancy list above. What the CSV export reads to fill its
    /// room_number/room_name/link-value columns. Per source because
    /// `link_value` is resolved through *this* source's link property.
    pub error_rooms: BTreeMap<String, ErrorRoomInfo>,
}

/// Data-quality report for one project's rooms against every reference source
/// it configures, for the header's validation panel. An on-demand aggregate
/// over the whole snapshot, not a per-room render concern — see
/// STRATEGY-SOURCES.md.
#[derive(Serialize)]
pub struct ValidationResponse {
    /// One report per configured, loaded reference source, keyed by source
    /// name. **Empty is the normal "nothing to reconcile" answer**, not an
    /// error: a project may configure no reference source, have one declared
    /// but not yet uploaded, or have no registered settings at all. The old
    /// `drofus_configured: false` said exactly this for the single-source
    /// world; an empty map says it for N.
    pub sources: BTreeMap<String, SourceValidation>,
    /// Rooms examined. A project-level fact, identical for every source, so it
    /// sits here rather than being repeated per section.
    pub total_rooms: usize,
    /// Tallies summed across every source — what the collapsed QA header
    /// counts. Each source's own breakdown is on its `SourceValidation`.
    pub discrepancies: DiscrepancyCounts,
}

impl ValidationResponse {
    /// The "nothing configured" answer: no sources, no rooms counted, no
    /// discrepancies. Callers return this rather than an error — see
    /// `compute_project_validation`.
    fn nothing_to_reconcile() -> Self {
        Self {
            sources: BTreeMap::new(),
            total_rooms: 0,
            discrepancies: DiscrepancyCounts::default(),
        }
    }
}

/// The declaration for one dRofus field label, if the settings carry one.
fn field_config<'a>(drofus_fields: &'a [ReferenceFieldConfig], label: &str) -> Option<&'a ReferenceFieldConfig> {
    drofus_fields.iter().find(|f| f.label == label)
}

/// The configured QA override for one dRofus field label, or `None` when the
/// column has no declaration, or a declaration with no `qa` set (both mean
/// the default: numeric-adaptive if both sides parse as a number, else exact
/// string match).
fn compare_mode(drofus_fields: &[ReferenceFieldConfig], label: &str) -> Option<CompareMode> {
    field_config(drofus_fields, label).and_then(|f| f.qa)
}

/// A copy of `s` with every non-ASCII character replaced by `?`, mirroring
/// duHast's `encode_ascii` step (Python's `str.encode("ascii", "replace")`,
/// see `Objects/base.py`'s `to_json_utf`) that every room value already went
/// through before it reached this service. Used to re-check a string-compare
/// mismatch: if narrowing the dRofus side the same lossy way makes it equal
/// to the room value, the two sides agree and the mismatch was purely an
/// artefact of that export step, not a real disagreement.
fn ascii_narrowed(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii() { c } else { '?' }).collect()
}

/// Phase 1 — resolve every room's link-property value. Returns the room
/// count, the ids of rooms that resolved no value at all
/// (`rooms_missing_link_value`), and a map of resolved link value → every
/// `(room, source)` that resolved to it (so the caller can detect a value
/// shared by more than one room). Borrows the rooms out of `stored`.
fn resolve_link_values<'a>(
    project_id: &str,
    stored: &'a [(ModelKey, RoomPayload)],
    drofus: &ReferenceData,
    builtin_defs: &[BuiltinPropertyDef],
) -> (usize, Vec<String>, LinkValueIndex<'a>) {
    let mut total_rooms = 0;
    let mut rooms_missing_link_value = Vec::new();
    let mut by_value: LinkValueIndex = BTreeMap::new();

    for (_key, payload) in stored {
        if payload.project.id != project_id {
            continue;
        }
        for room in &payload.rooms {
            total_rooms += 1;
            match lookup_property(room, &drofus.link_property, &payload.model.source, builtin_defs) {
                Some(value) => by_value.entry(value).or_default().push((room, &payload.model.source)),
                None => rooms_missing_link_value.push(room.id.clone()),
            }
        }
    }

    (total_rooms, rooms_missing_link_value, by_value)
}

/// The typed comparison ladder for one reconciled field, each rung falling
/// through to the next on `None`: a `Date`-declared field is compared as
/// parsed instants first (two renderings of one moment agree); then
/// `numeric_match` when both sides parse as numbers; finally string equality
/// (with the ASCII-narrowing re-check that forgives duHast's lossy
/// `encode_ascii` export step — see `ascii_narrowed`). `Exact` mode skips both
/// typed rungs and forces the string comparison.
fn field_values_agree(reference_value: &str, room_value: &str, field_cfg: Option<&ReferenceFieldConfig>) -> bool {
    let exact_mode = field_cfg.and_then(|f| f.qa) == Some(CompareMode::Exact);
    let date = if exact_mode {
        None
    } else {
        field_cfg.filter(|f| f.field_type == FieldType::Date).and_then(|f| {
            let fmt = f.format.as_deref()?; // always Some on Date (validated at startup)
            let revit_fmt = f.revit_format.as_deref().unwrap_or(fmt);
            date_match(reference_value, room_value, fmt, revit_fmt)
        })
    };
    let numeric = if exact_mode || date.is_some() {
        None
    } else {
        numeric_match(reference_value, room_value)
    };
    match (date, numeric) {
        (Some(date_matches), _) => date_matches,
        (None, Some(numeric_matches)) => numeric_matches,
        (None, None) => {
            reference_value.trim() == room_value.trim() || ascii_narrowed(reference_value.trim()) == room_value.trim()
        }
    }
}

/// Phase 3 — which dRofus fields this pass actually checks: every row-1 label
/// except those overridden `Ignore` (a deliberate exclusion, hidden from this
/// report entirely rather than shown as "not checked"), each flagged with
/// whether row 2 mapped it to a Revit property.
fn compute_field_coverage(drofus: &ReferenceData, drofus_fields: &[ReferenceFieldConfig]) -> Vec<FieldCoverage> {
    let ignored: BTreeSet<&str> = drofus_fields
        .iter()
        .filter(|f| f.qa == Some(CompareMode::Ignore))
        .map(|f| f.label.as_str())
        .collect();
    drofus
        .all_labels
        .iter()
        .filter(|label| !ignored.contains(label.as_str()))
        .map(|label| FieldCoverage {
            label: label.clone(),
            checked: drofus.reconciliation.contains_key(label),
            revit_property: drofus.reconciliation.get(label).cloned(),
        })
        .collect()
}

/// Resolve the human-friendly detail (`ErrorRoomInfo`) for every room whose id
/// is in `error_ids`, in a single pass over the project's rooms. Number, name
/// and link value all go through `lookup_property` the same way
/// `resolve_link_values` resolves the link value — so canonical→raw resolution
/// (and the source dimension) stays consistent with the rest of the pass, and a
/// property that doesn't resolve degrades to `""` (the CSV shows a blank cell).
///
/// Keyed by `room_id`, which is only unique within a model — the same
/// pre-existing caveat the discrepancy lists already carry (a colliding id from
/// a second linked model resolves to whichever room is seen last). This is a
/// detail lookup for display, not an identity the checks depend on.
fn collect_error_rooms(
    project_id: &str,
    stored: &[(ModelKey, RoomPayload)],
    drofus: &ReferenceData,
    builtin_defs: &[BuiltinPropertyDef],
    error_ids: &BTreeSet<String>,
) -> BTreeMap<String, ErrorRoomInfo> {
    let mut error_rooms = BTreeMap::new();
    for (_key, payload) in stored {
        if payload.project.id != project_id {
            continue;
        }
        let source = &payload.model.source;
        for room in &payload.rooms {
            if !error_ids.contains(&room.id) {
                continue;
            }
            error_rooms.insert(
                room.id.clone(),
                ErrorRoomInfo {
                    number: lookup_property(room, "Number", source, builtin_defs).unwrap_or_default(),
                    name: lookup_property(room, "Name", source, builtin_defs).unwrap_or_default(),
                    link_value: lookup_property(room, &drofus.link_property, source, builtin_defs).unwrap_or_default(),
                },
            );
        }
    }
    error_rooms
}

/// Pure computation behind `compute_project_validation` — pulled out so it's
/// testable without a full `AppState`, same shape as `resolve_label_fields`.
///
/// Four checks, in order: (1) does every room resolve a value for the link
/// property (`resolve_link_values`); (2) among those that do, is the value
/// actually unique per room (a shared value is ambiguous — recorded, then
/// excluded from the rest); (3) does each remaining room's value find a dRofus
/// record; (4) for rooms that do, does every reconciled, non-`Ignore`d
/// property agree between the two sides (`field_values_agree`). Also reports
/// `field_coverage` (`compute_field_coverage`): which dRofus fields this pass
/// actually checks at all, for the panel's "what's being QA'd" reference.
pub fn compute_validation(
    project_id: &str,
    stored: &[(ModelKey, RoomPayload)],
    reference: &ReferenceData,
    builtin_defs: &[BuiltinPropertyDef],
    fields: &[ReferenceFieldConfig],
) -> SourceValidation {
    let (_total_rooms, rooms_missing_link_value, by_value) =
        resolve_link_values(project_id, stored, reference, builtin_defs);

    let mut duplicate_link_values = Vec::new();
    let mut rooms_unmatched = Vec::new();
    let mut property_mismatches = Vec::new();
    let mut fields_absent_in_revit = Vec::new();
    let mut fields_empty_in_revit = Vec::new();

    for (value, rooms) in &by_value {
        if rooms.len() > 1 {
            duplicate_link_values.push(DuplicateLinkValue {
                value: value.clone(),
                room_ids: rooms.iter().map(|(r, _)| r.id.clone()).collect(),
            });
            continue; // ambiguous -- can't uniquely match, so no further checks
        }
        let (room, source) = rooms[0];
        let Some(record) = reference.by_id.get(value) else {
            rooms_unmatched.push(room.id.clone());
            continue;
        };
        for (label, revit_property) in &reference.reconciliation {
            if compare_mode(fields, label) == Some(CompareMode::Ignore) {
                continue;
            }
            // Normalize the dRofus side the same way `lookup_property`
            // already does for the Revit side: a blank cell is "no value
            // here", not a real empty-string value to compare against. A
            // dRofus-side absence isn't tracked further -- only Revit-side
            // absence is (see `MissingInRevit`'s doc comment for why).
            let Some(reference_value) = record.fields.get(label).filter(|s| !s.is_empty()) else {
                continue;
            };
            match property_presence(room, revit_property, source, builtin_defs) {
                PropertyPresence::Absent => fields_absent_in_revit.push(MissingInRevit {
                    room_id: room.id.clone(),
                    reference_id: value.clone(),
                    field: label.clone(),
                }),
                PropertyPresence::Empty => fields_empty_in_revit.push(MissingInRevit {
                    room_id: room.id.clone(),
                    reference_id: value.clone(),
                    field: label.clone(),
                }),
                PropertyPresence::Present(room_value) => {
                    if !field_values_agree(reference_value, &room_value, field_config(fields, label)) {
                        property_mismatches.push(PropertyMismatch {
                            room_id: room.id.clone(),
                            reference_id: value.clone(),
                            field: label.clone(),
                            room_value,
                            reference_value: reference_value.clone(),
                        });
                    }
                }
            }
        }
    }

    // Per-category counts (list lengths — duplicate counts as groups, matching
    // the panel's issue count) and their total, so a consumer needn't re-sum.
    let discrepancies = DiscrepancyCounts {
        total: rooms_missing_link_value.len()
            + duplicate_link_values.len()
            + rooms_unmatched.len()
            + property_mismatches.len()
            + fields_absent_in_revit.len()
            + fields_empty_in_revit.len(),
        rooms_missing_link_value: rooms_missing_link_value.len(),
        duplicate_link_values: duplicate_link_values.len(),
        rooms_unmatched: rooms_unmatched.len(),
        property_mismatches: property_mismatches.len(),
        fields_absent_in_revit: fields_absent_in_revit.len(),
        fields_empty_in_revit: fields_empty_in_revit.len(),
    };

    // Every room id that appears in any discrepancy list — the set the CSV
    // export needs number/name/link-value for.
    let mut error_ids: BTreeSet<String> = BTreeSet::new();
    error_ids.extend(rooms_missing_link_value.iter().cloned());
    error_ids.extend(duplicate_link_values.iter().flat_map(|d| d.room_ids.iter().cloned()));
    error_ids.extend(rooms_unmatched.iter().cloned());
    error_ids.extend(property_mismatches.iter().map(|m| m.room_id.clone()));
    error_ids.extend(fields_absent_in_revit.iter().map(|m| m.room_id.clone()));
    error_ids.extend(fields_empty_in_revit.iter().map(|m| m.room_id.clone()));
    let error_rooms = collect_error_rooms(project_id, stored, reference, builtin_defs, &error_ids);

    SourceValidation {
        link_property: reference.link_property.clone(),
        rooms_missing_link_value,
        duplicate_link_values,
        rooms_unmatched,
        property_mismatches,
        fields_absent_in_revit,
        fields_empty_in_revit,
        field_coverage: compute_field_coverage(reference, fields),
        discrepancies,
        error_rooms,
    }
}

/// Rooms in one project across every stored model. Counted here rather than
/// taken from `compute_validation`, because it is a project-level fact that
/// every source would otherwise report identically — see
/// `ValidationResponse::total_rooms`.
fn count_project_rooms(project_id: &str, stored: &[(ModelKey, RoomPayload)]) -> usize {
    stored
        .iter()
        .filter(|(_, payload)| payload.project.id == project_id)
        .map(|(_, payload)| payload.rooms.len())
        .sum()
}

/// Data-quality report for the header's validation panel — see
/// `ValidationResponse`/`compute_validation`. `drofus_configured: false` is a
/// normal, non-error result — covers both "no dRofus source configured for
/// this project" and "this project has no registered settings at all" (the
/// latter has no separate signal here, same as `list_buildings`) — and is
/// returned as `Ok`; a storage read failure is a real internal error and
/// surfaces as `ServiceError::Internal`, so the HTTP adapter can still map it
/// to 500 exactly as it does today.
pub fn compute_project_validation(state: &AppState, project_id: &str) -> Result<ValidationResponse, ServiceError> {
    let registry = state.settings();
    let Some(bundle) = registry.settings_for(project_id) else {
        return Ok(ValidationResponse::nothing_to_reconcile());
    };

    // Reconcile against every source that has data. A configured-but-not-yet-
    // uploaded source (`data: None`) is skipped rather than reported as a
    // failure — "declared, nothing uploaded yet" is a normal state, and the
    // same "signal, not error" policy the unmatched-key checks below follow.
    let loaded: Vec<(&String, &ReferenceData, &[ReferenceFieldConfig])> = bundle
        .reference
        .iter()
        .filter_map(|(name, src)| src.data.as_ref().map(|data| (name, data, src.fields.as_slice())))
        .collect();
    if loaded.is_empty() {
        return Ok(ValidationResponse::nothing_to_reconcile());
    }

    // One storage read for all of them: `all_snapshots` is the expensive call
    // here, and every source reconciles against the same room set.
    let stored = state.all_snapshots().map_err(ServiceError::Internal)?;

    let mut response = ValidationResponse::nothing_to_reconcile();
    response.total_rooms = count_project_rooms(project_id, &stored);
    for (name, data, fields) in loaded {
        let report = compute_validation(project_id, &stored, data, &bundle.builtin_properties, fields);
        response.discrepancies.add(&report.discrepancies);
        response.sources.insert(name.clone(), report);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{CustomValue, Model, Project, Snapshot};
    use crate::reference::ReferenceRecord;

    fn make_room(id: &str, name: &str, props: &[(&str, &str)]) -> Room {
        let mut properties = BTreeMap::new();
        for (k, v) in props {
            properties.insert(k.to_string(), CustomValue { value: v.to_string(), storage_type: None });
        }
        Room {
            id: id.to_string(),
            name: name.to_string(),
            level_id: "1".to_string(),
            loops: vec![],
            properties,
        }
    }

    fn make_payload(project_id: &str, rooms: Vec<Room>) -> (ModelKey, RoomPayload) {
        let key = ModelKey { project_id: project_id.to_string(), model_id: "m1".to_string() };
        let payload = RoomPayload {
            schema_version: 5,
            project: Project { id: project_id.to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
            model_to_shared: None,
            room_boundary: None,
            levels: vec![],
            rooms,
        };
        (key, payload)
    }

    fn make_drofus(
        link_property: &str,
        records: &[(&str, &[(&str, &str)])],
        reconciliation: &[(&str, &str)],
    ) -> ReferenceData {
        let mut by_id = BTreeMap::new();
        // `all_labels` mirrors the real loader's row-1 label set: the union
        // of every reconciled label and every field label that shows up in
        // any record (the real CSV always has a row-1 label for a column
        // regardless of whether row 2 mapped it).
        let mut all_labels: BTreeSet<String> = BTreeSet::new();
        for (id, fields) in records {
            let mut f = BTreeMap::new();
            for (k, v) in *fields {
                f.insert(k.to_string(), v.to_string());
                all_labels.insert(k.to_string());
            }
            by_id.insert(id.to_string(), ReferenceRecord { fields: f });
        }
        let mut reconciliation_map = BTreeMap::new();
        for (k, v) in reconciliation {
            reconciliation_map.insert(k.to_string(), v.to_string());
            all_labels.insert(k.to_string());
        }
        ReferenceData {
            link_property: link_property.to_string(),
            by_id,
            reconciliation: reconciliation_map,
            all_labels: all_labels.into_iter().collect(),
        }
    }

    /// A room with no value for the link property is reported, not silently
    /// dropped.
    #[test]
    fn test_compute_validation_missing_link_value() {
        let room = make_room("1", "Room", &[]); // no "Number" property
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[], &[]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert_eq!(count_project_rooms("p1", &stored), 1);
        assert_eq!(result.rooms_missing_link_value, vec!["1".to_string()]);
        assert!(result.duplicate_link_values.is_empty());
    }

    /// Two rooms sharing one link value are ambiguous: reported as a
    /// duplicate, and excluded from the unmatched/mismatch checks (neither
    /// can be uniquely said to be the room a dRofus record describes).
    #[test]
    fn test_compute_validation_duplicate_excluded_from_other_checks() {
        let rooms = vec![
            make_room("1", "Room A", &[("Number", "101")]),
            make_room("2", "Room B", &[("Number", "101")]),
        ];
        let (key, payload) = make_payload("p1", rooms);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("101", &[])], &[]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert_eq!(result.duplicate_link_values.len(), 1);
        let dup = &result.duplicate_link_values[0];
        assert_eq!(dup.value, "101");
        assert_eq!(dup.room_ids, vec!["1".to_string(), "2".to_string()]);
        assert!(result.rooms_unmatched.is_empty());
        assert!(result.property_mismatches.is_empty());
    }

    /// A room whose (unique) link value isn't in the dRofus map is reported
    /// as unmatched.
    #[test]
    fn test_compute_validation_unmatched_in_drofus() {
        let room = make_room("1", "Room", &[("Number", "999")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("1", &[])], &[]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert_eq!(result.rooms_unmatched, vec!["1".to_string()]);
    }

    /// A uniquely-matched room: an agreeing reconciled field produces no
    /// mismatch, a disagreeing one does.
    #[test]
    fn test_compute_validation_property_mismatch_and_agreement() {
        let room = make_room("1", "Room", &[("Number", "1"), ("Area", "25.5"), ("Department", "Cardiology")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus(
            "Number",
            &[("1", &[("NetArea", "30.0"), ("Dept", "Cardiology")])],
            &[("NetArea", "Area"), ("Dept", "Department")],
        );

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert!(result.rooms_unmatched.is_empty());
        assert_eq!(result.property_mismatches.len(), 1);
        let mismatch = &result.property_mismatches[0];
        assert_eq!(mismatch.field, "NetArea");
        assert_eq!(mismatch.room_value, "25.5");
        assert_eq!(mismatch.reference_value, "30.0");
    }

    /// A discrepant room carries its number/name/link-value in `error_rooms`
    /// (what the CSV export shows beyond the id), and the discrepancy counts
    /// tally the lists.
    #[test]
    fn test_compute_validation_error_rooms_and_counts() {
        let room = make_room("r1", "Office 101", &[("Number", "101"), ("Name", "Office"), ("Area", "25.5")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("101", &[("NetArea", "30.0")])], &[("NetArea", "Area")]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        // One mismatch (Area 25.5 vs NetArea 30.0), and the counts reflect it.
        assert_eq!(result.property_mismatches.len(), 1);
        assert_eq!(result.discrepancies.property_mismatches, 1);
        assert_eq!(result.discrepancies.total, 1);

        // The mismatched room's detail: Revit Number/Name params + link value.
        let info = result.error_rooms.get("r1").expect("mismatched room has detail");
        assert_eq!(info.number, "101");
        assert_eq!(info.name, "Office");
        assert_eq!(info.link_value, "101");
    }

    /// A room missing its link value appears in `error_rooms` with an empty
    /// `link_value` (there is none to resolve), while its Name still resolves;
    /// the counts tally the missing-link category and the total.
    #[test]
    fn test_compute_validation_error_rooms_missing_link_value_blank() {
        let room = make_room("r1", "Office", &[("Name", "Office")]); // no "Number"
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[], &[]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert_eq!(result.rooms_missing_link_value, vec!["r1".to_string()]);
        assert_eq!(result.discrepancies.rooms_missing_link_value, 1);
        assert_eq!(result.discrepancies.total, 1);

        let info = result.error_rooms.get("r1").expect("missing-link room has detail");
        assert_eq!(info.link_value, "", "no link value resolved → blank");
        assert_eq!(info.number, "", "no Number param → blank");
        assert_eq!(info.name, "Office");
    }

    /// The reported bug: the Revit export's ASCII-narrowing step replaces any
    /// non-ASCII character with `?` before the value reaches this service, so
    /// a room value that legitimately started with an en dash arrives as
    /// `?`. That must not be flagged once the dRofus side is narrowed the
    /// same lossy way and the two agree.
    #[test]
    fn test_compute_validation_ascii_narrowing_no_false_mismatch() {
        let room = make_room("1", "Room", &[("Number", "1"), ("Department", "Loading Dock ? Option 2")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus(
            "Number",
            &[("1", &[("Dept", "Loading Dock \u{2013} Option 2")])],
            &[("Dept", "Department")],
        );

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert!(result.property_mismatches.is_empty());
    }

    /// A genuine content mismatch that merely happens to contain a literal
    /// `?` on the dRofus side must still be reported -- narrowing only
    /// rescues a mismatch when it's the *sole* difference, not any mismatch
    /// touching a `?` character.
    #[test]
    fn test_compute_validation_ascii_narrowing_does_not_mask_genuine_mismatch() {
        let room = make_room("1", "Room", &[("Number", "1"), ("Department", "MECH")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("1", &[("Dept", "SM.EX?")])], &[("Dept", "Department")]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert_eq!(result.property_mismatches.len(), 1);
    }

    /// The reported bug: a unit-conversion float artifact (Revit's
    /// `"1.49999935417"` vs dRofus's `"1.5"`) must not be flagged once both
    /// are rounded to the lesser stated precision.
    #[test]
    fn test_compute_validation_numeric_tolerance_no_false_mismatch() {
        let room = make_room("1", "Room", &[("Number", "1"), ("Area", "1.49999935417")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("1", &[("NetArea", "1.5")])], &[("NetArea", "Area")]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert!(result.property_mismatches.is_empty());
    }

    /// A blank dRofus cell must be treated as "no value here", not compared
    /// against Revit's real value -- previously this produced a false
    /// `""` vs `"25.5"` mismatch.
    #[test]
    fn test_compute_validation_empty_drofus_value_not_flagged() {
        let room = make_room("1", "Room", &[("Number", "1"), ("Area", "25.5")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("1", &[("NetArea", "")])], &[("NetArea", "Area")]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert!(result.property_mismatches.is_empty());
        assert!(result.fields_absent_in_revit.is_empty());
        assert!(result.fields_empty_in_revit.is_empty());
    }

    /// dRofus has a real value but the room has no such Revit property at
    /// all -- the serious case (mapping/model-setup problem), reported
    /// separately from a merely-blank value.
    #[test]
    fn test_compute_validation_field_absent_in_revit() {
        let room = make_room("1", "Room", &[("Number", "1")]); // no "Area" property at all
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("1", &[("NetArea", "30.0")])], &[("NetArea", "Area")]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert!(result.property_mismatches.is_empty());
        assert!(result.fields_empty_in_revit.is_empty());
        assert_eq!(result.fields_absent_in_revit.len(), 1);
        assert_eq!(result.fields_absent_in_revit[0].field, "NetArea");
    }

    /// dRofus has a real value, the room's Revit property exists but is
    /// blank -- an ordinary per-room gap, reported separately from `Absent`.
    #[test]
    fn test_compute_validation_field_empty_in_revit() {
        let room = make_room("1", "Room", &[("Number", "1"), ("Area", "")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("1", &[("NetArea", "30.0")])], &[("NetArea", "Area")]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert!(result.property_mismatches.is_empty());
        assert!(result.fields_absent_in_revit.is_empty());
        assert_eq!(result.fields_empty_in_revit.len(), 1);
        assert_eq!(result.fields_empty_in_revit[0].field, "NetArea");
    }

    /// A field overridden `Ignore` is skipped entirely: no mismatch, no
    /// absent/empty entry, and no row in the coverage report.
    #[test]
    fn test_compute_validation_ignore_override_skips_field_entirely() {
        let room = make_room("1", "Room", &[("Number", "1"), ("SyncTime", "2026-07-02")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("1", &[("LastSync", "2026-06-29")])], &[("LastSync", "SyncTime")]);
        // Also declares the field's type -- proves `qa: Ignore` and `type:
        // Date` coexist: QA still skips it, independent of what a future
        // date-consuming feature would do with the same declaration.
        let drofus_fields = vec![crate::settings::ReferenceFieldConfig {
            label: "LastSync".to_string(),
            field_type: crate::settings::FieldType::Date,
            format: Some("%Y-%m-%d".to_string()),
            revit_format: None,
            qa: Some(CompareMode::Ignore),
        }];

        let result = compute_validation("p1", &stored, &drofus, &[], &drofus_fields);

        assert!(result.property_mismatches.is_empty());
        assert!(result.fields_absent_in_revit.is_empty());
        assert!(result.fields_empty_in_revit.is_empty());
        assert!(result.field_coverage.iter().all(|c| c.label != "LastSync"));
    }

    /// A `Date` field declaration for tests: the shipped dRofus pattern,
    /// optionally a distinct Revit-side pattern, optionally a QA override.
    fn date_field(label: &str, revit_format: Option<&str>, qa: Option<CompareMode>) -> ReferenceFieldConfig {
        ReferenceFieldConfig {
            label: label.to_string(),
            field_type: FieldType::Date,
            format: Some("%-m/%-d/%Y %-I:%M:%S %p %z".to_string()),
            revit_format: revit_format.map(|s| s.to_string()),
            qa,
        }
    }

    const DROFUS_DATE_FMT: &str = "%-m/%-d/%Y %-I:%M:%S %p %z";

    /// `date_match` with the shipped dRofus pattern: two renderings of the
    /// same instant agree, different instants disagree, and an unparseable
    /// side yields `None` (fall back to string comparison).
    #[test]
    fn test_date_match_same_instant_different_rendering() {
        // Same instant: 5:01:01 PM +10:00 == 7:01:01 AM +00:00.
        assert_eq!(
            date_match(
                "6/29/2026 5:01:01 PM +10:00",
                "6/29/2026 7:01:01 AM +00:00",
                DROFUS_DATE_FMT,
                DROFUS_DATE_FMT,
            ),
            Some(true)
        );
        assert_eq!(
            date_match(
                "6/29/2026 5:01:01 PM +10:00",
                "6/29/2026 5:01:02 PM +10:00",
                DROFUS_DATE_FMT,
                DROFUS_DATE_FMT,
            ),
            Some(false)
        );
        assert_eq!(
            date_match("not a date", "6/29/2026 5:01:01 PM +10:00", DROFUS_DATE_FMT, DROFUS_DATE_FMT),
            None
        );
    }

    /// A distinct `revit_format` parses the room side with its own pattern; a
    /// zoned dRofus side against a naive Revit side compares the zoned side's
    /// local wall-clock reading.
    #[test]
    fn test_date_match_revit_format_and_mixed_offset() {
        assert_eq!(
            date_match("6/29/2026 5:01:01 PM +10:00", "2026-06-29 17:01:01", DROFUS_DATE_FMT, "%Y-%m-%d %H:%M:%S",),
            Some(true)
        );
        assert_eq!(
            date_match("6/29/2026 5:01:01 PM +10:00", "2026-06-29 07:01:01", DROFUS_DATE_FMT, "%Y-%m-%d %H:%M:%S",),
            Some(false),
            "a naive side is a wall-clock reading, not a UTC instant"
        );
    }

    /// A `Date`-declared field where the two sides differ textually but
    /// denote the same instant produces no mismatch; `qa = "exact"` on the
    /// same field forces the textual comparison and reports it.
    #[test]
    fn test_compute_validation_date_field_same_instant_not_flagged() {
        let room = make_room("1", "Room", &[("Number", "1"), ("SyncTime", "6/29/2026 7:01:01 AM +00:00")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus(
            "Number",
            &[("1", &[("LastSync", "6/29/2026 5:01:01 PM +10:00")])],
            &[("LastSync", "SyncTime")],
        );

        let typed = vec![date_field("LastSync", None, None)];
        let result = compute_validation("p1", &stored, &drofus, &[], &typed);
        assert!(result.property_mismatches.is_empty(), "same instant, different rendering: no mismatch");

        let exact = vec![date_field("LastSync", None, Some(CompareMode::Exact))];
        let result = compute_validation("p1", &stored, &drofus, &[], &exact);
        assert_eq!(result.property_mismatches.len(), 1, "exact mode forces the textual comparison");
    }

    /// A `Date` declaration whose values don't actually parse falls back to
    /// the string path -- the declaration is a hint, not truth, so a
    /// free-text value in a date-labeled column still compares as a string.
    #[test]
    fn test_compute_validation_date_field_unparseable_falls_back_to_string() {
        let room = make_room("1", "Room", &[("Number", "1"), ("SyncTime", "pending")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("1", &[("LastSync", "pending")])], &[("LastSync", "SyncTime")]);

        let typed = vec![date_field("LastSync", None, None)];
        let result = compute_validation("p1", &stored, &drofus, &[], &typed);
        assert!(result.property_mismatches.is_empty(), "equal strings agree on the fallback path");
    }

    /// The coverage report shows every dRofus field: a reconciled one as
    /// checked (with its mapped Revit property), an unmapped one as
    /// unchecked.
    #[test]
    fn test_compute_validation_field_coverage() {
        let room = make_room("1", "Room", &[("Number", "1"), ("Area", "25.5")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus(
            "Number",
            &[("1", &[("NetArea", "25.5"), ("Notes", "not mapped")])],
            &[("NetArea", "Area")],
        );

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        let net_area = result.field_coverage.iter().find(|c| c.label == "NetArea").unwrap();
        assert!(net_area.checked);
        assert_eq!(net_area.revit_property.as_deref(), Some("Area"));

        let notes = result.field_coverage.iter().find(|c| c.label == "Notes").unwrap();
        assert!(!notes.checked);
        assert!(notes.revit_property.is_none());
    }

    /// Register a project whose settings carry `sources`, each a
    /// (name, data, field configs) triple, and store one payload for it.
    fn state_with_sources(rooms: Vec<Room>, sources: Vec<(&str, ReferenceData)>) -> AppState {
        let (_key, payload) = make_payload("p1", rooms);
        let reference = sources
            .into_iter()
            .map(|(name, data)| {
                (
                    name.to_string(),
                    crate::state::ProjectReferenceSource { data: Some(data), fields: vec![] },
                )
            })
            .collect();
        let bundle = crate::state::ProjectSettings {
            reference,
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec![],
            milestones: vec![],
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            hierarchy_exclusions: vec![],
        };
        let state = AppState::new(
            Box::new(crate::storage::MemStore::new()),
            std::collections::HashMap::from([("p1".to_string(), bundle)]),
            None,
        );
        state.set_snapshot(payload).unwrap();
        state
    }

    /// **The point of the generalization.** Two configured sources produce two
    /// reports, each reconciled against its OWN link property — not one report
    /// about whichever source happened to be called "drofus". `drofus` keys on
    /// Number and agrees; `ffe` keys on Code and disagrees on Finish, and each
    /// discrepancy lands in its own section.
    #[test]
    fn test_every_configured_source_gets_its_own_report() {
        let room = make_room("1", "Room", &[("Number", "1"), ("Code", "C1"), ("Area", "25.5"), ("Finish", "Vinyl")]);
        let drofus = make_drofus("Number", &[("1", &[("NetArea", "25.5")])], &[("NetArea", "Area")]);
        let ffe = make_drofus("Code", &[("C1", &[("FinishSpec", "Carpet")])], &[("FinishSpec", "Finish")]);
        let state = state_with_sources(vec![room], vec![("drofus", drofus), ("ffe", ffe)]);

        let result = compute_project_validation(&state, "p1").unwrap();

        assert_eq!(result.sources.keys().collect::<Vec<_>>(), vec!["drofus", "ffe"]);
        assert_eq!(result.total_rooms, 1, "counted once for the project, not once per source");

        let d = &result.sources["drofus"];
        assert_eq!(d.link_property, "Number");
        assert!(d.property_mismatches.is_empty(), "drofus agrees");

        let f = &result.sources["ffe"];
        assert_eq!(f.link_property, "Code", "each source reconciles on its own link property");
        assert_eq!(f.property_mismatches.len(), 1);
        assert_eq!(f.property_mismatches[0].reference_value, "Carpet");
        assert_eq!(f.property_mismatches[0].room_value, "Vinyl");

        // The header's one number is the sum, so a second source's problems
        // cannot hide behind a clean first one.
        assert_eq!(result.discrepancies.total, 1);
        assert_eq!(result.discrepancies.property_mismatches, 1);
    }

    /// A project with no reference source at all answers an empty map, not an
    /// error — the shape that replaced `drofus_configured: false`.
    #[test]
    fn test_no_configured_source_is_an_empty_map_not_an_error() {
        let state = state_with_sources(vec![make_room("1", "Room", &[])], vec![]);
        let result = compute_project_validation(&state, "p1").unwrap();
        assert!(result.sources.is_empty());
        assert_eq!(result.discrepancies.total, 0);

        // Same answer for a project that has no registered settings at all.
        let unknown = compute_project_validation(&state, "ghost").unwrap();
        assert!(unknown.sources.is_empty());
    }

    /// A source declared in settings but never uploaded (`data: None`) is
    /// skipped, not reported as a failure — "declared, nothing uploaded yet"
    /// is a normal state on the `Upload` origin.
    #[test]
    fn test_configured_but_unloaded_source_is_skipped() {
        let (_key, payload) = make_payload("p1", vec![make_room("1", "Room", &[("Number", "1")])]);
        let reference = BTreeMap::from([
            (
                "drofus".to_string(),
                crate::state::ProjectReferenceSource {
                    data: Some(make_drofus("Number", &[("1", &[])], &[])),
                    fields: vec![],
                },
            ),
            ("pending".to_string(), crate::state::ProjectReferenceSource { data: None, fields: vec![] }),
        ]);
        let bundle = crate::state::ProjectSettings {
            reference,
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec![],
            milestones: vec![],
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            hierarchy_exclusions: vec![],
        };
        let state = AppState::new(
            Box::new(crate::storage::MemStore::new()),
            std::collections::HashMap::from([("p1".to_string(), bundle)]),
            None,
        );
        state.set_snapshot(payload).unwrap();

        let result = compute_project_validation(&state, "p1").unwrap();
        assert_eq!(
            result.sources.keys().collect::<Vec<_>>(),
            vec!["drofus"],
            "the unloaded source contributes nothing"
        );
    }
}
