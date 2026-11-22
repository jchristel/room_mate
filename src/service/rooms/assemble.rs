//! Assembling rooms from the stored snapshots: each room resolved (label, reference data, classification), the levels deduplicated by
//! name and elevation across linked models, and the revision that vouches for the answer.

use super::*;

/// Resolve one room's label fields from the configured, ordered name list.
/// `"$name"` / `"$id"` are intrinsic tokens for `Room`'s own fields (not
/// reachable via `lookup_property`, which only reads `room.properties`); a
/// `<source>.<label>` name reads that joined reference record's field; anything
/// else is a canonical property name resolved the same way
/// dRofus/classification already are, so a second source (or a differently-
/// named property) needs no change here.
///
/// `known` is the project's *configured* source vocabulary, deliberately not
/// the subset that actually joined onto this room. The two differ only on a
/// room with no record for a named source, and there the configured set is what
/// keeps the rule uniform: the name still parses as a namespace and resolves to
/// nothing ("an unmatched key is a signal, not an error"). Handing in the joined
/// subset instead would make the very same label fall through to a bogus
/// `room.properties["<source>.<label>"]` lookup on exactly the rooms that failed
/// to match — one rule for matched rooms and another for the rest.
///
/// A blank reference cell contributes nothing, matching `lookup_property`, which
/// already collapses "property missing" and "property blank" together — a label
/// list should not sprout an empty chip because one CSV cell was empty.
pub(in crate::service) fn resolve_label_fields(
    room: &Room,
    fields: &[String],
    source: &str,
    builtin_defs: &[BuiltinPropertyDef],
    reference: &BTreeMap<String, ReferenceRecord>,
    known: &BTreeSet<String>,
) -> Vec<String> {
    fields
        .iter()
        .filter_map(|name| match name.as_str() {
            "$name" => Some(room.name.clone()).filter(|s| !s.is_empty()),
            "$id" => Some(room.id.clone()),
            other => match split_namespace(other, known) {
                NamespaceSplit::Joined { source: ns, property } => reference
                    .get(&ns)
                    .and_then(|record| record.fields.get(property))
                    .filter(|v| !v.trim().is_empty())
                    .cloned(),
                // An unknown namespace has no error channel here — a label that
                // does not resolve contributes nothing, same as an absent
                // property. `validate_namespaced_field` is what refuses the
                // typo, at load, where there IS somewhere to put the message.
                NamespaceSplit::UnknownSource(_) => None,
                NamespaceSplit::Unqualified(canonical) => lookup_property(room, canonical, source, builtin_defs),
            },
        })
        .collect()
}

/// Assemble one room's response: raw room + every reference-source join +
/// classification. Pulled out so the single- and multi-model paths derive
/// rooms identically — the join/classify logic lives in exactly one place.
///
/// `bundle` is the owning payload's project's settings (see
/// `AppState::settings_for`) — every field that used to come off `AppState`
/// directly now comes off this per-project bundle instead. `source` comes
/// from the owning model's `Model.source` (e.g. "revit") — it picks which
/// `BuiltinPropertyDef.by_source` entry `lookup_property` uses to resolve a
/// canonical name to this room's actual raw property name.
///
/// `effective_reference` (source name → its data) is passed in explicitly
/// rather than read off `bundle.reference`, so a milestone view can join
/// *pinned* snapshots instead of each source's current data — see
/// `assemble_scoped_rooms`, which resolves it once per project and passes
/// the same map into every room `assemble_room` sees for that project.
///
/// `known` is `effective_reference`'s key set, hoisted to the caller because it
/// is per-payload and this runs per room — see `resolve_label_fields` for why
/// the vocabulary is the configured sources rather than the joined ones.
pub(in crate::service) fn assemble_room(
    bundle: &ProjectSettings,
    effective_reference: &BTreeMap<String, &ReferenceData>,
    known: &BTreeSet<String>,
    room: &Room,
    source: &str,
    key: &ModelKey,
) -> RoomResponse {
    // One join per configured source: read its link property off the room,
    // look up the record. A source with no match for this room contributes
    // no entry — an unmatched key is a signal, not an error.
    let reference: BTreeMap<String, ReferenceRecord> = effective_reference
        .iter()
        .filter_map(|(name, data)| {
            let record = lookup_property(room, &data.link_property, source, &bundle.builtin_properties)
                .and_then(|key| data.by_id.get(&key).cloned())?;
            Some((name.clone(), record))
        })
        .collect();

    // Classification resolved fresh — see staleness note on classify_room.
    let classification = classify_room(room, &bundle.hierarchy, source, &bundle.builtin_properties);

    let label = resolve_label_fields(room, &bundle.room_label, source, &bundle.builtin_properties, &reference, known);

    RoomResponse {
        room: room.clone(),
        reference,
        classification,
        label,
        source: source.to_string(),
        project_id: key.project_id.clone(),
        model_id: key.model_id.clone(),
    }
}

/// A stable content revision for a `RoomsResult`, derived from the set of
/// contributing `(model, snapshot)` pairs. It changes only when a push replaces
/// a model's snapshot (a new `taken_at`) or when the set of contributing models
/// changes, and is byte-identical between two idle responses — which is exactly
/// the "has anything actually changed?" signal the viewer's poll needs.
///
/// It deliberately tracks snapshot *identity*, not derived data: a settings-only
/// change (a colour plan, a dRofus mapping) leaves the pushed geometry untouched
/// and does not move the revision. The set is sorted before hashing so
/// linked-model iteration order can't perturb the result. Milestone pins already
/// substituted their pinned payload upstream, so `snapshot.taken_at` here is the
/// snapshot actually rendered.
pub(in crate::service) fn scoped_revision(scoped: &[ScopedPayload]) -> String {
    use std::hash::{Hash, Hasher};

    let mut parts: Vec<(&str, &str, &str)> = scoped
        .iter()
        .map(|(key, payload, _)| (key.project_id.as_str(), key.model_id.as_str(), payload.snapshot.taken_at.as_str()))
        .collect();
    parts.sort_unstable();

    // DefaultHasher (SipHash with fixed keys) is deterministic across runs, so
    // the value is comparable even across a server restart — the client only
    // ever compares consecutive responses, but stability costs nothing here.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Each contributing model's phase, read off the payload actually in this
/// response — so under `?milestone=` it reports the *pinned* snapshot's phase,
/// not the model's current one. Reading it from the payload rather than the
/// manifest is what keeps that true (PLAN-phasing.md "D8").
pub(in crate::service) fn phases_of(
    scoped: &[ScopedPayload<'_>],
) -> BTreeMap<String, BTreeMap<String, Option<String>>> {
    let mut out: BTreeMap<String, BTreeMap<String, Option<String>>> = BTreeMap::new();
    for (key, payload, _) in scoped {
        out.entry(key.project_id.clone())
            .or_default()
            .insert(key.model_id.clone(), payload.phase.clone());
    }
    out
}

/// Load and parse a milestone's pinned CSV for one project's reference
/// source. A missing or unparseable pin resolves to `None` with a warning
/// (fall back to that source's current data — signal, not error, same stance
/// as a dangling model pin).
pub(in crate::service) fn resolve_pinned_reference(
    state: &AppState,
    milestone: &str,
    project_id: &str,
    source: &str,
    pin: &str,
) -> Result<Option<ReferenceData>, ServiceError> {
    match state.get_reference(project_id, source, pin).map_err(ServiceError::Internal)? {
        Some(bytes) => match crate::reference::load_reference_from_bytes(&bytes) {
            Ok(data) => Ok(Some(data)),
            Err(e) => {
                tracing::warn!(
                    "milestone '{}' pins '{}' snapshot {:?} for project {}, but it failed to parse ({e:#}) — falling back to current data",
                    milestone, source, pin, project_id
                );
                Ok(None)
            }
        },
        None => {
            tracing::warn!(
                "milestone '{}' pins '{}' snapshot {:?} for project {}, but no such snapshot exists — falling back to current data",
                milestone, source, pin, project_id
            );
            Ok(None)
        }
    }
}

/// Phase 2 — level dedup across linked models. A `Level.id` is only unique
/// *within* its own model (same caveat as room ids -- see `ModelKey`'s doc
/// comment), so two linked models that both define "the same" architectural
/// level produce two distinct `Level` rows with the same (name, elevation) but
/// different ids. Merge them: same name + same elevation (tolerant of
/// cross-file float drift via `elevation_match`, the same rounding discipline
/// used for dRofus property comparison) IS the same level. Returns the remap
/// `(project_id, model_id, level_id) -> canonical id`; first-seen id per group
/// wins as canonical, so the level picker and room filtering agree on one id
/// per real-world level.
///
/// Grouped *per project*: level identity is only meaningful within one project
/// (the dedup exists for linked models of one job), so two unrelated projects
/// that both have a "Level 1" @ 0.0 keep their own levels in an unscoped merge
/// instead of collapsing onto whichever project happened to be seen first.
pub(in crate::service) fn dedup_levels(scoped: &[ScopedPayload<'_>]) -> BTreeMap<(String, String, String), String> {
    let mut canonical_levels: BTreeMap<String, Vec<Level>> = BTreeMap::new(); // project_id -> levels
    let mut level_remap: BTreeMap<(String, String, String), String> = BTreeMap::new();
    for (key, payload, _bundle) in scoped {
        let project_levels = canonical_levels.entry(key.project_id.clone()).or_default();
        for level in &payload.levels {
            let canonical_id = match project_levels
                .iter()
                .find(|c| c.name == level.name && elevation_match(c.elevation, level.elevation))
            {
                Some(existing) => existing.id.clone(),
                None => {
                    project_levels.push(level.clone());
                    level.id.clone()
                }
            };
            level_remap.insert((key.project_id.clone(), key.model_id.clone(), level.id.clone()), canonical_id);
        }
    }
    level_remap
}

/// Phase 3 — derive the response levels and rooms from the scoped payloads.
/// Applies the optional building filter (a project with no "Building" tier
/// matches nothing under it, never everything), joins each room against its
/// effective dRofus (the milestone-pinned override when one resolved, else the
/// project's current dRofus — identical to pre-pinning behaviour), remaps room
/// `level_id`s to the canonical ids from phase 2, applies the optional property
/// filter, and emits each canonical level once per project.
///
/// The two filters sit on opposite sides of the join, and have to: the building
/// filter reads a raw `Room`'s classification, while a property predicate may
/// name a *joined* field (`drofus.NetArea`) that only exists once
/// `assemble_room` has run. Hence rooms are assembled before the
/// "contributed nothing" check, which now counts *post-filter* rooms — a model
/// whose rooms all fail the filter contributes no levels either, the same rule
/// the building filter already followed.
///
/// Also collects each project's dRofus label set (the third return value)
/// from the *same* effective "drofus" data the rooms are joined against —
/// carried out of this loop rather than re-resolved so a milestone view can
/// never show current column headers over pinned data. Collected *before*
/// the "contributed nothing" check: the labels describe the project's
/// dataset, not its rooms, so a project whose rooms all fail a filter still
/// reports its columns. Collected for EVERY configured source, keyed by
/// source name: two sources may both declare a label called `NetArea`, so the
/// source has to be part of the address.
pub(in crate::service) fn assemble_scoped_rooms(
    scoped: &[ScopedPayload<'_>],
    level_remap: &BTreeMap<(String, String, String), String>,
    milestone_reference: &MilestoneReference,
    scope: &RoomScope<'_>,
    placement: &crate::service::placement::Placement,
) -> AssembledRooms {
    let building = scope.building;
    let mut levels = Vec::new();
    // Keyed (project_id, canonical_id): canonical ids are model-local, so two
    // projects could in principle mint the same id -- a flat set would let one
    // project's level suppress another's.
    let mut emitted_level_ids: BTreeSet<(String, String)> = BTreeSet::new();
    let mut rooms: Vec<RoomResponse> = Vec::new();
    let mut reference_labels: BTreeMap<String, BTreeMap<String, ReferenceLabels>> = BTreeMap::new();
    let mut boundary_by_level: BTreeMap<String, RoomBoundary> = BTreeMap::new();

    for (key, payload, bundle) in scoped {
        // Building tier index is resolved from this payload's own project
        // bundle -- projects with different hierarchies coexist in one merge.
        let building_idx = building_tier_index(&bundle.hierarchy);
        // Either scope narrowing the room set arms the "a model that
        // contributed nothing contributes no levels either" rule below.
        let scope_filter_active = building.is_some() || scope.filter.is_some();

        let matching_rooms: Vec<&Room> = match (building, building_idx) {
            (Some(wanted), Some(idx)) => payload
                .rooms
                .iter()
                .filter(|room| {
                    let path =
                        classify_room(room, &bundle.hierarchy, &payload.model.source, &bundle.builtin_properties);
                    match path.get(idx) {
                        Some(tier) if tier.undefined => wanted == UNCLASSIFIED_BUILDING_KEY,
                        Some(tier) => building_key(&tier.code, &tier.name) == *wanted,
                        None => false,
                    }
                })
                .collect(),
            // A building filter was requested but this project has no
            // "Building" tier: it can't answer the question, so it matches
            // nothing -- contributing all its rooms instead would leak them
            // into a response the caller believes is filtered.
            (Some(_), None) => Vec::new(),
            (None, _) => payload.rooms.iter().collect(),
        };

        // A milestone-pinned override wins per source when it resolved;
        // otherwise (no milestone, no pin for that source, or a pin that fell
        // back) that source's current data — identical to pre-pinning
        // behaviour, generalized from one hardcoded source to every source
        // this project configures.
        let effective_reference: BTreeMap<String, &ReferenceData> = bundle
            .reference
            .iter()
            .filter_map(|(source, cfg)| {
                // Rooms join only sources declared for rooms. A door schedule
                // configured in the same project is not "a source with no match
                // for this room" -- it is not this entity's source at all, and
                // joining it would put a door's columns on a room whose link
                // property happened to collide.
                if cfg.entity != crate::settings::ReferenceEntity::Rooms {
                    return None;
                }
                let pinned = milestone_reference
                    .get(&(key.project_id.clone(), source.clone()))
                    .and_then(|o| o.as_ref());
                let data = pinned.or(cfg.data.as_ref())?;
                Some((source.clone(), data))
            })
            .collect();

        // The namespace vocabulary a `<source>.<label>` room label splits
        // against, built once per payload rather than per room.
        let known_sources: BTreeSet<String> = effective_reference.keys().cloned().collect();

        // First model of a project wins; every model of one project resolves
        // the same effective data, so this is dedup, not precedence. A
        // project with no reference source gets no entry — absent, not empty,
        // matching how `RoomResponse.reference` treats an unmatched room.
        if !effective_reference.is_empty() {
            let per_source = reference_labels.entry(key.project_id.clone()).or_default();
            for (source, data) in &effective_reference {
                per_source.entry(source.clone()).or_insert_with(|| ReferenceLabels {
                    all_labels: data.all_labels.clone(),
                    reconciliation: data.reconciliation.clone(),
                });
            }
        }

        // This model's step into the project frame, resolved once per model
        // rather than per room.
        let model_frame = placement.for_model(&key.project_id, &key.model_id);

        // Assemble first, filter second: a predicate may name a joined field,
        // which does not exist until `assemble_room` has run.
        let assembled: Vec<RoomResponse> = matching_rooms
            .into_iter()
            .map(|room| {
                let mut response =
                    assemble_room(bundle, &effective_reference, &known_sources, room, &payload.model.source, key);
                if let Some(canonical_id) =
                    level_remap.get(&(key.project_id.clone(), key.model_id.clone(), room.level_id.clone()))
                {
                    response.room.level_id = canonical_id.clone();
                }
                // Into the project frame, so a linked model exported from a
                // different origin lands ON the plan rather than beside it.
                // Derived at read time and never stored, like `label` and every
                // other resolved field -- see `service::placement`.
                if let Some(transform) = model_frame {
                    crate::service::placement::place_loops(transform, &mut response.room.loops);
                }
                response
            })
            .filter(|response| scope.filter.is_none_or(|f| f.matches(response, &bundle.builtin_properties)))
            .collect();

        if scope_filter_active && assembled.is_empty() {
            continue; // this model contributed nothing to the requested scope
        }

        // The regime this model was drawn to: what its envelope declared, else
        // the project's `[areas]` fallback, else finish face.
        let model_boundary = bundle.areas.resolve_boundary(payload.room_boundary);

        for level in &payload.levels {
            let canonical_id = level_remap
                .get(&(key.project_id.clone(), key.model_id.clone(), level.id.clone()))
                .cloned()
                .unwrap_or_else(|| level.id.clone());

            // Widen, never narrow: a level fed by two models keeps finish face
            // if either says so (see `RoomsResult::boundary_by_level`). Written
            // as a max over the two rather than a first-wins insert precisely
            // because model iteration order must not decide this.
            boundary_by_level
                .entry(canonical_id.clone())
                .and_modify(|b| *b = widest_boundary(*b, model_boundary))
                .or_insert(model_boundary);

            if emitted_level_ids.insert((key.project_id.clone(), canonical_id.clone())) {
                let mut level = level.clone();
                level.id = canonical_id;
                levels.push(level);
            }
        }
        rooms.extend(assembled);
    }

    AssembledRooms { levels, rooms, reference_labels, boundary_by_level }
}

/// The regime that needs the *more* work done of the two — finish face, whose
/// rooms are still separated by their walls. See
/// `RoomsResult::boundary_by_level` for why a disagreement resolves this way
/// and not the other.
pub(in crate::service) fn widest_boundary(a: RoomBoundary, b: RoomBoundary) -> RoomBoundary {
    match (a, b) {
        (RoomBoundary::Centreline, RoomBoundary::Centreline) => RoomBoundary::Centreline,
        _ => RoomBoundary::FinishFace,
    }
}

/// Phase 3's four outputs. A named struct rather than a 4-tuple: the two maps
/// are both `BTreeMap<String, _>` keyed on different things (project id vs
/// level id), and a positional tuple is one refactor away from swapping them
/// silently.
pub(in crate::service) struct AssembledRooms {
    pub(in crate::service) levels: Vec<Level>,
    pub(in crate::service) rooms: Vec<RoomResponse>,
    pub(in crate::service) reference_labels: BTreeMap<String, BTreeMap<String, ReferenceLabels>>,
    pub(in crate::service) boundary_by_level: BTreeMap<String, RoomBoundary>,
}
