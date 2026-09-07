//! `/spaces` read assembly: merge every stored model's latest spaces snapshot
//! into one flat payload, scoped by project, model, milestone and property
//! filter.
//!
//! **Thinner than `service::items`, and thinner than `service::rooms`.** The
//! scoping, the milestone pin, the revision hash and the phase report are
//! `service::entity_scope`, generic over `contract::SnapshotEnvelope` and
//! knowing nothing about spaces. The record is `contract::Room`, so the
//! reference join and the filter grammar are the ones rooms already use. What is
//! left here is one struct, one `FilterTarget` impl and a loop.
//!
//! **Two things a rooms read does that this deliberately does not**, both
//! recorded in `docs/Superseded/PLAN-spaces.md` rather than merely omitted:
//!
//! - **No `?building=`.** A room's building comes from its own hierarchy
//!   properties. A space's would come either from properties nobody has checked
//!   exist on the services side, or from the room it matches -- and that is a
//!   join, not a scope. Scoping one side of a two-sided comparison by a value
//!   derived from the other is how a set difference turns into a wrong answer
//!   rather than a smaller one.
//! - **No geometric room resolution.** A space names no room at all, so there is
//!   nothing for geometry to fill in. Matching a space to a room is a
//!   *value* match across models, and it lives in the QA report where its
//!   disagreements can be reported rather than silently applied.
//!
//! `?model=` is here and is not on the rooms read, because this entity's first
//! question is per model: "does this services model have spaces at all".
//!
//! **This module also holds the space-to-room reconciliation, and is past the
//! ~500-line split trigger with that in it.** Judged and kept whole: the read
//! and the report are the entity's only two answers and they share one
//! vocabulary -- `value_of`'s tiered lookup resolves a filter's property, the
//! match key and a compared property with the same call, against the same
//! `builtin_properties`. Splitting them would put that helper across a module
//! boundary and leave two files that have to agree about what a property name
//! means. If the viewer layer adds a third concern, the seam is read versus
//! report.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::contract::{Enclosure, PropertyPresence, Room};
use crate::reference::{ReferenceData, ReferenceRecord};
use crate::settings::{BuiltinPropertyDef, CompareMode, ReferenceEntity, SpaceFieldConfig, SpacePolicy};
use crate::state::{AppState, ModelKey};
use crate::storage::SnapshotKind;

use super::entity_scope;
use super::rooms::{FilterTarget, RoomFilter};
use super::ServiceError;

/// A space as sent to a consumer: the stored record plus the identity of the
/// model it came from.
#[derive(Serialize)]
pub struct SpaceResponse {
    #[serde(flatten)]
    pub space: Room,

    /// Joined reference data, keyed by source name — the same sub-object a room
    /// carries, from sources declaring `entity = "spaces"`.
    #[serde(flatten)]
    pub reference: BTreeMap<String, ReferenceRecord>,

    /// The project this space's model belongs to.
    pub project_id: String,
    /// **The model this space came from, and it is load-bearing here in a way it
    /// is not for a room.** One services file per service means the same room
    /// number legitimately names a space in each of them, so a consumer holding
    /// two spaces with one key is looking at two disciplines rather than a
    /// duplicate. Measured on RHH, where pooling them made 3,046 keys look
    /// duplicated.
    pub model_id: String,

    #[serde(skip)]
    pub source: String,
}

impl FilterTarget for SpaceResponse {
    fn presence(&self, source: Option<&str>, property: &str, builtin_defs: &[BuiltinPropertyDef]) -> PropertyPresence {
        /// A space's own struct fields always exist, so blank collapses to
        /// `Empty`, never `Absent`.
        fn intrinsic(value: Option<&str>) -> PropertyPresence {
            match value {
                None => PropertyPresence::Absent,
                Some("") => PropertyPresence::Empty,
                Some(v) => PropertyPresence::Present(v.to_string()),
            }
        }

        match source {
            Some(name) => match self.reference.get(name) {
                None => PropertyPresence::Absent,
                Some(record) => match record.fields.get(property) {
                    None => PropertyPresence::Absent,
                    Some(v) if v.is_empty() => PropertyPresence::Empty,
                    Some(v) => PropertyPresence::Present(v.clone()),
                },
            },
            None => match property {
                "$id" => intrinsic(Some(&self.space.id)),
                "$name" => intrinsic(Some(&self.space.name)),
                "$level_id" => intrinsic(Some(&self.space.level_id)),
                "$model_id" => intrinsic(Some(&self.model_id)),
                // **The intrinsic this entity exists for.**
                // `?filter=$enclosure!=enclosed` is the whole "which spaces are
                // broken" query, answered without a QA endpoint. Absent rather
                // than empty when unset, because a space always states it and an
                // absent value means the snapshot predates the field.
                "$enclosure" => intrinsic(self.space.enclosure.map(enclosure_str)),
                _ => crate::contract::property_presence(&self.space, property, &self.source, builtin_defs),
            },
        }
    }
}

/// The wire spelling of an enclosure state, matching its serde rename so a
/// filter value and a response value are the same string.
fn enclosure_str(enclosure: Enclosure) -> &'static str {
    match enclosure {
        Enclosure::Enclosed => "enclosed",
        Enclosure::Unenclosed => "unenclosed",
        Enclosure::Unmeasured => "unmeasured",
    }
}

/// Resolve one `source.property` name against a space — the spaces counterpart
/// of `items::resolve_presence`, on identical terms.
pub fn resolve_presence(
    space: &SpaceResponse,
    field: &str,
    known: &BTreeSet<String>,
    builtin: &[BuiltinPropertyDef],
) -> PropertyPresence {
    match super::rooms::split_namespace(field, known) {
        super::rooms::NamespaceSplit::Joined { source, property } => space.presence(Some(&source), property, builtin),
        super::rooms::NamespaceSplit::Unqualified(name) => space.presence(None, name, builtin),
        super::rooms::NamespaceSplit::UnknownSource(_) => PropertyPresence::Absent,
    }
}

/// What a `/spaces` read is scoped to.
pub struct SpaceScope<'a> {
    pub project: Option<&'a str>,
    /// Restrict to one model — see the module header for why this exists here
    /// and not on the rooms read.
    pub model: Option<&'a str>,
    pub milestone: Option<&'a str>,
    pub filter: Option<&'a RoomFilter>,
}

/// One assembled `/spaces` response.
#[derive(Serialize)]
pub struct SpacesResult {
    pub schema_version: u32,
    /// Stable content revision over the contributing `(model, snapshot)` pairs,
    /// same role and construction as `RoomsResult::revision`.
    pub revision: String,
    pub spaces: Vec<SpaceResponse>,
    /// The Revit phase each contributing model's spaces were filtered to, keyed
    /// by project id then model id. Read off each snapshot, never off the
    /// lineage's current phase.
    ///
    /// **Worth reading on this entity above all others.** The push phase is one
    /// per run and the disciplines do not agree on the name: on RHH the
    /// mechanical model's spaces are in `Future` while its siblings are in `New
    /// Construction`. Two models reporting different phases here is ordinary and
    /// correct; it is also the only place a consumer can see it.
    pub phase_by_model: BTreeMap<String, BTreeMap<String, Option<String>>>,

    /// Each contributing model's levels, keyed by model id.
    ///
    /// **Per model, never merged**, which is the opposite of what `/rooms` does
    /// with its flat `levels` — and the difference is the whole reason this
    /// field exists. A level id is per document, so a space's `level_id` means
    /// something only alongside the model it came from; merging the lists would
    /// produce a set in which two different storeys can share an id.
    ///
    /// What a consumer does with it is match on **elevation**, not on id: that
    /// is what crosses documents (`LEVEL_EPS_MM` is the tolerance the geometric
    /// resolver already uses for "same storey"). Without this the viewer cannot
    /// place a space on a storey at all, and drew every level at once — which
    /// looked like a design choice and was really missing data.
    ///
    /// **Empty for a model that pushed no levels**, which is legal (they are
    /// optional on the payload) and which a consumer must handle rather than
    /// treat as "no levels match".
    pub levels_by_model: BTreeMap<String, Vec<crate::contract::Level>>,
}

/// Merge every scoped model's latest spaces snapshot into one payload.
///
/// `None` when nothing has ever been pushed, which the handler turns into a 204
/// — the same "no data at all" answer every other entity's read gives, and
/// distinct from an empty `spaces` list, which means the scope matched models
/// that hold none. **That distinction is the entity's whole first requirement**,
/// so it must survive the read as well as the push.
pub fn assemble_spaces(state: &AppState, scope: &SpaceScope<'_>) -> Result<Option<SpacesResult>, ServiceError> {
    if !state.has_any_snapshot(SnapshotKind::Spaces).map_err(ServiceError::Internal)? {
        return Ok(None);
    }
    let registry = state.settings();

    let mut scoped: Vec<(ModelKey, crate::contract::SpacePayload)> =
        entity_scope::scope_snapshots(state, SnapshotKind::Spaces, scope.project, scope.milestone, |ms| {
            &ms.space_attachments
        })?;
    if let Some(wanted) = scope.model {
        scoped.retain(|(key, _)| key.model_id == wanted);
    }

    let revision = entity_scope::revision(&scoped);
    let phase_by_model = entity_scope::phase_by_model(&scoped);
    let levels_by_model = entity_scope::levels_by_model(&scoped);
    // From the manifest index, so the frame matches `/rooms` -- see
    // `service::placement::from_index`. THE entity this exists for: RHH's five
    // services models are exported from an origin ~250 ft from the
    // architectural one, so their spaces drew beside the plan instead of over
    // it.
    let placement = super::placement::from_index(state)?;
    let mut spaces: Vec<SpaceResponse> = Vec::new();

    for (_key, payload) in &scoped {
        let bundle = registry.settings_for(&payload.project.id);
        let builtin: &[BuiltinPropertyDef] = bundle.map(|b| b.builtin_properties.as_slice()).unwrap_or_default();

        let sources: BTreeMap<&str, &ReferenceData> = bundle
            .map(|b| {
                b.reference
                    .iter()
                    .filter(|(_, cfg)| cfg.entity == ReferenceEntity::Spaces)
                    .filter_map(|(name, cfg)| Some((name.as_str(), cfg.data.as_ref()?)))
                    .collect()
            })
            .unwrap_or_default();

        // This model's step into the project frame, resolved once per model.
        let model_frame = placement.for_model(&payload.project.id, &payload.model.id);

        for space in &payload.spaces {
            let reference: BTreeMap<String, ReferenceRecord> = sources
                .iter()
                .filter_map(|(name, data)| {
                    let record =
                        crate::contract::lookup_property(space, &data.link_property, &payload.model.source, builtin)
                            .and_then(|key| data.by_id.get(&key).cloned())?;
                    Some(((*name).to_string(), record))
                })
                .collect();

            let mut response = SpaceResponse {
                space: space.clone(),
                reference,
                project_id: payload.project.id.clone(),
                model_id: payload.model.id.clone(),
                source: payload.model.source.clone(),
            };
            // Into the project frame. A space carries loops and nothing else
            // directional, so this is the whole placement for the entity.
            if let Some(transform) = model_frame {
                super::placement::place_loops(transform, &mut response.space.loops);
            }

            // The filter runs *after* assembly so a predicate sees the same
            // resolved vocabulary a consumer does — the rule every entity's read
            // follows.
            if scope.filter.is_none_or(|f| f.matches(&response, builtin)) {
                spaces.push(response);
            }
        }
    }

    Ok(Some(SpacesResult {
        schema_version: crate::contract::SUPPORTED_SPACE_SCHEMA,
        revision,
        spaces,
        phase_by_model,
        levels_by_model,
    }))
}

// --------------------------------------------------------------------------
// The space-to-room reconciliation.
//
// Lives here rather than in `service::validation` for two reasons. It is a
// different question from everything there -- that file reconciles a room
// against data from *outside* the model, keyed by a link property; this
// reconciles two sets of model elements against each other, keyed by a value
// somebody chose. And `validation.rs` is already the largest module in the
// service layer, so a fourth report shape in it would be the split nobody got
// round to. `compute_project_validation` calls in.
// --------------------------------------------------------------------------

/// One space named in a finding.
#[derive(Debug, Clone, Serialize)]
pub struct SpaceRef {
    pub model_id: String,
    pub space_id: String,
    pub name: String,
    /// The key value, when it has one. Absent means the space carries no value
    /// for the configured key at all, which is a different finding from a value
    /// that matched nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// One room named in a finding.
#[derive(Debug, Clone, Serialize)]
pub struct RoomKeyRef {
    pub model_id: String,
    pub room_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// A key value carried by more than one element on one side.
///
/// **Reported, never resolved.** An ambiguous key cannot be matched without
/// guessing which of the candidates was meant, and a guess here is a wrong
/// answer that looks like a right one.
#[derive(Debug, Clone, Serialize)]
pub struct AmbiguousKey {
    pub key: String,
    pub count: usize,
}

/// How one services model's spaces matched.
#[derive(Debug, Clone, Serialize)]
pub struct SpaceModelMatch {
    pub model_id: String,
    pub total: usize,
    /// Spaces carrying no value for the configured key. A different state from
    /// "matched nothing": there is nothing to match with.
    pub without_key: Vec<SpaceRef>,
    pub matched: usize,
    /// Spaces whose key names no room in the room set.
    pub unmatched: Vec<SpaceRef>,
    /// Keys duplicated **inside this one model**.
    ///
    /// Only within, never across. One services file per service means the same
    /// room number legitimately names a space in each of them, so cross-model
    /// duplication is the design rather than a finding -- pooled, RHH reported
    /// 3,046 duplicate keys where the real number was 31.
    pub ambiguous_keys: Vec<AmbiguousKey>,
}

/// One compared property disagreeing on a matched pair.
#[derive(Debug, Clone, Serialize)]
pub struct SpacePropertyMismatch {
    pub model_id: String,
    pub space_id: String,
    pub key: String,
    pub property: String,
    /// Absent when the space does not carry the property at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_value: Option<String>,
    /// The relative difference against the room, for a numeric field with a
    /// tolerance. **On the wire rather than a bare boolean**, because a
    /// threshold nobody can see the distribution behind cannot be tuned, and
    /// tuning it is the whole point of calling this a sanity check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta_pct: Option<f64>,
}

/// A pair that could not be compared numerically because the room's value is
/// zero.
///
/// **Neither a pass nor a mismatch**, and guaranteed to occur rather than
/// hypothetical: an unenclosed space has an area of zero, so the states this
/// entity invents on one side meet the arithmetic on the other. A relative
/// difference against zero is undefined, and reporting it as a 100% mismatch
/// would be the enclosure finding arriving a second time wearing an area
/// costume.
#[derive(Debug, Clone, Serialize)]
pub struct IncomparablePair {
    pub model_id: String,
    pub space_id: String,
    pub key: String,
    pub property: String,
    pub space_value: String,
}

/// How the enclosure states divide this project's spaces.
#[derive(Debug, Clone, Default, Serialize)]
pub struct EnclosureCounts {
    pub enclosed: usize,
    pub unenclosed: usize,
    pub unmeasured: usize,
    /// Spaces on snapshots written before `enclosure` existed. Not a finding.
    pub unstated: usize,
}

/// The top-line tallies, for a collapsed panel header.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SpaceDiscrepancyCounts {
    pub models_not_audited: usize,
    pub models_without_spaces: usize,
    pub spaces_without_key: usize,
    pub spaces_unmatched: usize,
    pub rooms_without_space: usize,
    pub ambiguous_keys: usize,
    pub property_mismatches: usize,
    pub not_enclosed: usize,
}

/// One project's space-to-room reconciliation.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SpaceReport {
    /// Models the project knows that have **no spaces snapshot at all**.
    ///
    /// "Not audited", and distinct from `models_without_spaces` below. That
    /// distinction is the entity's first requirement, and it is the reason an
    /// empty spaces push is accepted rather than refused.
    ///
    /// **It has a hole that no amount of care closes, and it is stated rather
    /// than papered over**: a model that has never pushed *anything* is not in
    /// this list, because the server has never heard of it. The complete answer
    /// needs the producer's own run summary as its other half.
    pub models_not_audited: Vec<String>,

    /// Models whose spaces snapshot holds **zero spaces** -- audited, and
    /// genuinely empty. This is the finding.
    pub models_without_spaces: Vec<String>,

    pub total_spaces: usize,
    pub enclosure: EnclosureCounts,

    /// The configured key, or `None` when the project has not set one -- in
    /// which case every match field below is empty because nothing was
    /// attempted, not because nothing disagreed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparison_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_key: Option<String>,

    /// Which models supplied the rooms, after `[spaces] room_models`. On the
    /// wire because a reader has to be able to tell a small room set from a
    /// misconfigured scope.
    pub room_models: Vec<String>,
    pub total_rooms: usize,

    /// One row per services model.
    pub by_model: Vec<SpaceModelMatch>,

    /// Rooms no space anywhere names.
    ///
    /// **A full list, in both directions**, which the first draft of this design
    /// got wrong. The room side was going to be counts only, on the assumption
    /// that a project is partially serviced and the list would be noise. These
    /// projects expect 1:1, so an unmatched room is a finding of exactly the
    /// same standing as an unmatched space and suppressing it would hide half
    /// the answer.
    pub rooms_without_space: Vec<RoomKeyRef>,

    /// Keys duplicated across the room set.
    pub ambiguous_room_keys: Vec<AmbiguousKey>,

    pub property_mismatches: Vec<SpacePropertyMismatch>,
    pub incomparable: Vec<IncomparablePair>,

    pub discrepancies: SpaceDiscrepancyCounts,
}

/// Whether a space and a room agree on one configured property.
///
/// Returns `Some(delta_pct)` when they disagree under a tolerance, `None` when
/// they agree. The tolerance path is the only comparison in the codebase that
/// is not exact-or-stated-precision, and it exists because two independently
/// measured areas never agree at stated precision: they are not two renderings
/// of one number, they are two numbers.
fn values_disagree(space: &str, room: &str, cfg: &SpaceFieldConfig) -> Option<Option<f64>> {
    if cfg.qa == Some(CompareMode::Exact) {
        return (space != room).then_some(None);
    }

    if let (Some(pct), Ok(s), Ok(r)) = (cfg.tolerance_pct, space.trim().parse::<f64>(), room.trim().parse::<f64>()) {
        if r == 0.0 {
            // Handled by the caller as `incomparable`; never a mismatch.
            return None;
        }
        let delta = (s - r).abs();
        let delta_pct = delta / r.abs() * 100.0;
        // BOTH thresholds, which is what stops a percentage flagging every
        // small room. `tolerance_min` defaults to 0, so the simple case is a
        // plain percentage.
        if delta_pct > pct && delta > cfg.tolerance_min.unwrap_or(0.0) {
            return Some(Some((delta_pct * 10.0).round() / 10.0));
        }
        return None;
    }

    match crate::contract::numeric_match(space, room) {
        Some(true) => None,
        Some(false) => Some(None),
        // Not numbers on both sides: fall back to exact string comparison, the
        // same contract `numeric_match` has everywhere else.
        None => (space != room).then_some(None),
    }
}

/// Read one property off a spatial element, through the same tiered lookup a
/// filter uses, so a name means the same thing here as it does in `?filter=`.
fn value_of(room: &Room, property: &str, source: &str, builtin: &[BuiltinPropertyDef]) -> Option<String> {
    crate::contract::lookup_property(room, property, source, builtin)
}

/// One matched space/room pair, plus where each side's properties are read
/// from. A struct rather than six arguments because the two `source` strings
/// are easy to swap and the compiler would not notice.
struct Pair<'a> {
    model_id: &'a str,
    key: &'a str,
    space: &'a Room,
    space_source: &'a str,
    room: &'a Room,
    room_source: &'a str,
}

/// Compare one matched pair over every configured property, appending to the
/// report.
///
/// Split out of `space_report` because it is the one part that is about *values*
/// rather than about matching, and because it is where every tolerance decision
/// lands -- the zero-room case, the two thresholds, and the one-sided-absence
/// case each need saying once, in one place.
fn compare_properties(report: &mut SpaceReport, policy: &SpacePolicy, builtin: &[BuiltinPropertyDef], pair: &Pair<'_>) {
    for cfg in &policy.compared_properties {
        if cfg.qa == Some(CompareMode::Ignore) {
            continue;
        }
        let space_value = value_of(pair.space, &cfg.space, pair.space_source, builtin);
        let room_value = value_of(pair.room, cfg.room_name(), pair.room_source, builtin);

        match (space_value, room_value) {
            (Some(s), Some(r)) => {
                // The zero-room case is its own state, checked before the
                // comparison so it can never be reported as a 100% difference.
                if cfg.tolerance_pct.is_some() && r.trim().parse::<f64>() == Ok(0.0) {
                    report.incomparable.push(IncomparablePair {
                        model_id: pair.model_id.to_string(),
                        space_id: pair.space.id.clone(),
                        key: pair.key.to_string(),
                        property: cfg.space.clone(),
                        space_value: s,
                    });
                    continue;
                }
                if let Some(delta_pct) = values_disagree(&s, &r, cfg) {
                    report.property_mismatches.push(SpacePropertyMismatch {
                        model_id: pair.model_id.to_string(),
                        space_id: pair.space.id.clone(),
                        key: pair.key.to_string(),
                        property: cfg.space.clone(),
                        space_value: Some(s),
                        room_value: Some(r),
                        delta_pct,
                    });
                }
            }
            // One side carrying the property and the other not is a mismatch
            // with an absent value, never silence: a property that stopped
            // being extracted looks exactly like one that agrees, if absence
            // is skipped.
            (s, r) if s.is_some() != r.is_some() => {
                report.property_mismatches.push(SpacePropertyMismatch {
                    model_id: pair.model_id.to_string(),
                    space_id: pair.space.id.clone(),
                    key: pair.key.to_string(),
                    property: cfg.space.clone(),
                    space_value: s,
                    room_value: r,
                    delta_pct: None,
                });
            }
            _ => {}
        }
    }
}

/// The presence half of the report: which models were audited, which hold none,
/// and how the enclosure states divide the population.
///
/// Split out because it is the half that needs **no configuration to be true**.
/// A project with no key set still gets all of this, and separating it makes
/// that impossible to break by accident.
fn record_presence(
    report: &mut SpaceReport,
    stored_spaces: &[(ModelKey, crate::contract::SpacePayload)],
    known_models: &[String],
) {
    let audited: BTreeSet<&str> = stored_spaces.iter().map(|(k, _)| k.model_id.as_str()).collect();
    for model in known_models {
        if !audited.contains(model.as_str()) {
            report.models_not_audited.push(model.clone());
        }
    }
    for (key, payload) in stored_spaces {
        if payload.spaces.is_empty() {
            report.models_without_spaces.push(key.model_id.clone());
        }
        report.total_spaces += payload.spaces.len();
        for space in &payload.spaces {
            match space.enclosure {
                Some(Enclosure::Enclosed) => report.enclosure.enclosed += 1,
                Some(Enclosure::Unenclosed) => report.enclosure.unenclosed += 1,
                Some(Enclosure::Unmeasured) => report.enclosure.unmeasured += 1,
                None => report.enclosure.unstated += 1,
            }
        }
    }
}

/// Key value -> the rooms carrying it, across every scoped room model.
///
/// Project-wide and hierarchy-blind, which is what the comparison is asked to
/// be. A room with no value for the key contributes nothing rather than an
/// empty-string bucket that would then match every space missing its own key.
fn index_rooms_by_key(
    room_models: &[&(ModelKey, crate::contract::RoomPayload)],
    room_key: &str,
    builtin: &[BuiltinPropertyDef],
) -> BTreeMap<String, Vec<RoomKeyRef>> {
    let mut rooms_by_key: BTreeMap<String, Vec<RoomKeyRef>> = BTreeMap::new();
    for (key, payload) in room_models {
        for room in &payload.rooms {
            let Some(value) = value_of(room, room_key, &payload.model.source, builtin) else {
                continue;
            };
            if value.trim().is_empty() {
                continue;
            }
            rooms_by_key.entry(value).or_default().push(RoomKeyRef {
                model_id: key.model_id.clone(),
                room_id: room.id.clone(),
                name: room.name.clone(),
                key: None,
            });
        }
    }
    rooms_by_key
}

/// Reconcile one project's spaces against its rooms.
///
/// A pure function of the two snapshot sets and the policy -- it never touches
/// the store, which is what lets its tests build a scenario as two vectors.
/// `known_models` is every model the project has pushed anything for, which is
/// the only thing here that a caller must read from storage.
pub fn space_report(
    stored_rooms: &[(ModelKey, crate::contract::RoomPayload)],
    stored_spaces: &[(ModelKey, crate::contract::SpacePayload)],
    known_models: &[String],
    policy: &SpacePolicy,
    builtin: &[BuiltinPropertyDef],
) -> SpaceReport {
    let mut report = SpaceReport {
        comparison_key: policy.comparison_key.clone(),
        room_key: policy.room_key_name().map(str::to_string),
        ..SpaceReport::default()
    };

    record_presence(&mut report, stored_spaces, known_models);

    // ---- the room set, scoped.
    let room_models: Vec<&(ModelKey, crate::contract::RoomPayload)> = stored_rooms
        .iter()
        .filter(|(key, _)| policy.room_models.is_empty() || policy.room_models.contains(&key.model_id))
        .collect();
    report.room_models = room_models.iter().map(|(k, _)| k.model_id.clone()).collect();
    report.total_rooms = room_models.iter().map(|(_, p)| p.rooms.len()).sum();

    let Some(space_key) = policy.comparison_key.as_deref() else {
        report.discrepancies = tally(&report);
        return report;
    };
    let room_key = policy.room_key_name().unwrap_or(space_key);

    let rooms_by_key = index_rooms_by_key(&room_models, room_key, builtin);
    for (value, rooms) in &rooms_by_key {
        if rooms.len() > 1 {
            report.ambiguous_room_keys.push(AmbiguousKey { key: value.clone(), count: rooms.len() });
        }
    }

    let mut keys_seen_by_any_space: BTreeSet<String> = BTreeSet::new();

    // ---- one row per services model.
    for (model_key, payload) in stored_spaces {
        let mut row = SpaceModelMatch {
            model_id: model_key.model_id.clone(),
            total: payload.spaces.len(),
            without_key: Vec::new(),
            matched: 0,
            unmatched: Vec::new(),
            ambiguous_keys: Vec::new(),
        };
        let mut within: BTreeMap<String, usize> = BTreeMap::new();

        for space in &payload.spaces {
            let value = value_of(space, space_key, &payload.model.source, builtin).filter(|v| !v.trim().is_empty());
            let Some(value) = value else {
                row.without_key.push(SpaceRef {
                    model_id: model_key.model_id.clone(),
                    space_id: space.id.clone(),
                    name: space.name.clone(),
                    key: None,
                });
                continue;
            };
            *within.entry(value.clone()).or_default() += 1;

            let Some(rooms) = rooms_by_key.get(&value) else {
                row.unmatched.push(SpaceRef {
                    model_id: model_key.model_id.clone(),
                    space_id: space.id.clone(),
                    name: space.name.clone(),
                    key: Some(value),
                });
                continue;
            };
            row.matched += 1;
            keys_seen_by_any_space.insert(value.clone());

            // An ambiguous room key is reported above and compared against the
            // first candidate rather than skipped: the reader already knows the
            // pairing is uncertain, and dropping the comparison would hide a
            // real disagreement behind a key problem.
            let Some((room, room_payload)) = rooms
                .first()
                .and_then(|r| room_models.iter().find(|(k, _)| k.model_id == r.model_id).map(|(_, p)| (r, p)))
            else {
                continue;
            };
            let Some(room_record) = room_payload.rooms.iter().find(|r| r.id == room.room_id) else {
                continue;
            };

            compare_properties(
                &mut report,
                policy,
                builtin,
                &Pair {
                    model_id: &model_key.model_id,
                    key: &value,
                    space,
                    space_source: &payload.model.source,
                    room: room_record,
                    room_source: &room_payload.model.source,
                },
            );
        }

        for (value, count) in within {
            if count > 1 {
                row.ambiguous_keys.push(AmbiguousKey { key: value, count });
            }
        }
        report.by_model.push(row);
    }

    // ---- the other direction, in full.
    for (value, rooms) in &rooms_by_key {
        if !keys_seen_by_any_space.contains(value) {
            for room in rooms {
                report.rooms_without_space.push(RoomKeyRef { key: Some(value.clone()), ..room.clone() });
            }
        }
    }

    report.discrepancies = tally(&report);
    report
}

/// The collapsed-header tallies, derived rather than accumulated so they cannot
/// drift from the lists they count.
fn tally(report: &SpaceReport) -> SpaceDiscrepancyCounts {
    SpaceDiscrepancyCounts {
        models_not_audited: report.models_not_audited.len(),
        models_without_spaces: report.models_without_spaces.len(),
        spaces_without_key: report.by_model.iter().map(|m| m.without_key.len()).sum(),
        spaces_unmatched: report.by_model.iter().map(|m| m.unmatched.len()).sum(),
        rooms_without_space: report.rooms_without_space.len(),
        ambiguous_keys: report.by_model.iter().map(|m| m.ambiguous_keys.len()).sum::<usize>()
            + report.ambiguous_room_keys.len(),
        property_mismatches: report.property_mismatches.len(),
        not_enclosed: report.enclosure.unenclosed + report.enclosure.unmeasured,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{Model, Project, Snapshot, SpacePayload};
    use crate::settings::{SpaceFieldConfig, SpacePolicy};
    use crate::state::ProjectSettings;
    use crate::storage::MemStore;
    use std::collections::HashMap;

    fn space(id: &str, enclosure: Option<Enclosure>) -> Room {
        Room {
            id: id.to_string(),
            name: format!("Space {}", id),
            level_id: "l1".to_string(),
            loops: vec![],
            enclosure,
            properties: BTreeMap::new(),
        }
    }

    fn bundle() -> ProjectSettings {
        ProjectSettings {
            spaces: Default::default(),
            reference: BTreeMap::new(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec![],
            milestones: vec![],
            anchor_model: None,
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            windows: Default::default(),
            ffe: Default::default(),
            hierarchy_exclusions: vec![],
        }
    }

    fn state_with(model: &str, spaces: Vec<Room>) -> AppState {
        state_with_levels(
            model,
            spaces,
            vec![crate::contract::Level { id: "l1".to_string(), name: "Level 01".to_string(), elevation: 51000.0 }],
        )
    }

    fn state_with_levels(model: &str, spaces: Vec<Room>, levels: Vec<crate::contract::Level>) -> AppState {
        let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), bundle())]), None);
        let payload = SpacePayload {
            schema_version: crate::contract::SUPPORTED_SPACE_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: model.to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-09-06T00:00:00Z".to_string() },
            phase: Some("New Construction".to_string()),
            model_to_shared: None,
            room_boundary: None,
            levels,
            spaces,
        };
        state.set_element_snapshot(SnapshotKind::Spaces, &payload).unwrap();
        state
    }

    fn scope<'a>() -> SpaceScope<'a> {
        SpaceScope { project: None, model: None, milestone: None, filter: None }
    }

    /// **Nothing pushed and pushed-but-empty are different answers**, and the
    /// entity's first requirement rests on the difference surviving the read.
    /// `None` becomes a 204; an empty list means "these models were audited".
    #[test]
    fn test_no_snapshots_is_none_but_an_empty_model_is_an_empty_list() {
        let empty = AppState::new(Box::new(MemStore::new()), HashMap::new(), None);
        assert!(assemble_spaces(&empty, &scope()).unwrap().is_none(), "nothing pushed at all");

        let audited = state_with("av", vec![]);
        let result = assemble_spaces(&audited, &scope()).unwrap().expect("a push happened");
        assert!(result.spaces.is_empty());
        assert_eq!(result.phase_by_model["p1"]["av"].as_deref(), Some("New Construction"));
    }

    /// The `$enclosure` intrinsic is what makes "which spaces are broken" a
    /// filter rather than an endpoint.
    #[test]
    fn test_enclosure_is_filterable() {
        let state = state_with(
            "me",
            vec![
                space("s1", Some(Enclosure::Enclosed)),
                space("s2", Some(Enclosure::Unenclosed)),
                space("s3", Some(Enclosure::Unmeasured)),
            ],
        );
        let known = BTreeSet::new();
        let filter = RoomFilter::parse_query("$enclosure=unenclosed", &known).unwrap();
        let result = assemble_spaces(&state, &SpaceScope { filter: Some(&filter), ..scope() })
            .unwrap()
            .unwrap();

        assert_eq!(result.spaces.len(), 1);
        assert_eq!(result.spaces[0].space.id, "s2");
    }

    /// **A space's storey is findable only through its own model's levels**, so
    /// the read has to carry them per model. Merging them the way `/rooms` does
    /// would put two different storeys under one id, since level ids are per
    /// document.
    #[test]
    fn test_levels_ride_the_read_keyed_by_model() {
        let state = state_with("me", vec![space("s1", Some(Enclosure::Enclosed))]);
        let result = assemble_spaces(&state, &scope()).unwrap().unwrap();

        assert_eq!(result.levels_by_model["me"].len(), 1);
        assert_eq!(result.levels_by_model["me"][0].id, "l1");
        assert_eq!(result.levels_by_model["me"][0].elevation, 51000.0);
    }

    /// **A model that pushed no levels reports an empty list, not an absent
    /// key.** Levels are optional on a spaces payload, and a consumer has to be
    /// able to tell "this model declared none" from "this model is not here" --
    /// the first is the state every probe-loaded snapshot is in.
    #[test]
    fn test_a_model_with_no_levels_still_appears() {
        let state = state_with_levels("me", vec![space("s1", None)], vec![]);
        let result = assemble_spaces(&state, &scope()).unwrap().unwrap();

        assert!(result.levels_by_model.contains_key("me"), "the model is present");
        assert!(result.levels_by_model["me"].is_empty(), "and declares no levels");
    }

    /// `?model=` narrows to one services model, which is where this entity's
    /// first question is asked.
    #[test]
    fn test_model_scope_narrows_to_one_services_model() {
        let state = state_with("me", vec![space("s1", Some(Enclosure::Enclosed))]);
        let result = assemble_spaces(&state, &SpaceScope { model: Some("hy"), ..scope() }).unwrap().unwrap();
        assert!(result.spaces.is_empty(), "a model that contributed nothing to this scope");

        let result = assemble_spaces(&state, &SpaceScope { model: Some("me"), ..scope() }).unwrap().unwrap();
        assert_eq!(result.spaces.len(), 1);
    }

    /// Every space names its own model, and on this entity that is not
    /// redundant: one services file per service means one room number
    /// legitimately names a space in each.
    #[test]
    fn test_every_space_names_its_model() {
        let state = state_with("fr", vec![space("s1", Some(Enclosure::Enclosed))]);
        let result = assemble_spaces(&state, &scope()).unwrap().unwrap();
        assert_eq!(result.spaces[0].model_id, "fr");
        assert_eq!(result.spaces[0].project_id, "p1");
    }

    // ---------------- the space-to-room reconciliation ----------------

    fn keyed(id: &str, number: &str, extra: &[(&str, &str)]) -> Room {
        let mut properties = BTreeMap::from([(
            "Number".to_string(),
            crate::contract::CustomValue { value: number.to_string(), storage_type: None },
        )]);
        for (k, v) in extra {
            properties.insert(k.to_string(), crate::contract::CustomValue { value: v.to_string(), storage_type: None });
        }
        Room {
            id: id.to_string(),
            name: format!("Room {}", number),
            level_id: "l1".to_string(),
            loops: vec![],
            enclosure: None,
            properties,
        }
    }

    fn rooms_of(model: &str, rooms: Vec<Room>) -> (ModelKey, crate::contract::RoomPayload) {
        (
            ModelKey { project_id: "p1".to_string(), model_id: model.to_string() },
            crate::contract::RoomPayload {
                schema_version: crate::contract::SUPPORTED_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: model.to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-09-06T00:00:00Z".to_string() },
                phase: Some("New Construction".to_string()),
                model_to_shared: None,
                room_boundary: None,
                levels: vec![],
                rooms,
            },
        )
    }

    fn spaces_of(model: &str, spaces: Vec<Room>) -> (ModelKey, SpacePayload) {
        (
            ModelKey { project_id: "p1".to_string(), model_id: model.to_string() },
            SpacePayload {
                schema_version: crate::contract::SUPPORTED_SPACE_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: model.to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-09-06T00:00:00Z".to_string() },
                phase: Some("New Construction".to_string()),
                model_to_shared: None,
                room_boundary: None,
                levels: vec![],
                spaces,
            },
        )
    }

    fn keyed_policy() -> SpacePolicy {
        SpacePolicy { comparison_key: Some("Number".to_string()), ..SpacePolicy::default() }
    }

    /// **The presence question, which is the entity's whole first
    /// requirement.** Three states, and the two that look alike on a dashboard
    /// mean opposite things: `av` was audited and holds none, `st` was never
    /// pushed.
    #[test]
    fn test_not_audited_and_audited_but_empty_are_different_findings() {
        let spaces = vec![spaces_of("av", vec![]), spaces_of("me", vec![keyed("s1", "101", &[])])];
        let known = vec!["av".to_string(), "me".to_string(), "st".to_string()];

        let report = space_report(&[], &spaces, &known, &SpacePolicy::default(), &[]);

        assert_eq!(report.models_not_audited, vec!["st".to_string()], "never pushed anything");
        assert_eq!(report.models_without_spaces, vec!["av".to_string()], "pushed, and holds none");
        assert_eq!(report.total_spaces, 1);
    }

    /// **Duplication ACROSS services models is the design, not a finding.** One
    /// file per service means the same room number legitimately names a space in
    /// each. Pooled, RHH reported 3,046 duplicate keys where the real number was
    /// 31 -- this is that mistake, pinned.
    #[test]
    fn test_the_same_key_in_two_services_models_is_not_ambiguous() {
        let rooms = vec![rooms_of("arch", vec![keyed("r1", "101", &[])])];
        let spaces = vec![
            spaces_of("me", vec![keyed("s1", "101", &[])]),
            spaces_of("hy", vec![keyed("s2", "101", &[])]),
        ];

        let report = space_report(&rooms, &spaces, &[], &keyed_policy(), &[]);

        assert!(
            report.by_model.iter().all(|m| m.ambiguous_keys.is_empty()),
            "four disciplines, not a duplicate"
        );
        assert!(report.ambiguous_room_keys.is_empty());
        assert_eq!(report.by_model.iter().map(|m| m.matched).sum::<usize>(), 2);
        assert!(report.rooms_without_space.is_empty());
    }

    /// The same key twice **inside one** services model is the real ambiguity.
    #[test]
    fn test_a_key_repeated_within_one_model_is_ambiguous() {
        let rooms = vec![rooms_of("arch", vec![keyed("r1", "101", &[])])];
        let spaces = vec![spaces_of("me", vec![keyed("s1", "101", &[]), keyed("s2", "101", &[])])];

        let report = space_report(&rooms, &spaces, &[], &keyed_policy(), &[]);

        assert_eq!(report.by_model[0].ambiguous_keys.len(), 1);
        assert_eq!(report.by_model[0].ambiguous_keys[0].count, 2);
    }

    /// **`room_models` scopes the room side, and without it the report drowns.**
    /// RHH's electrical model holds 3,418 rooms of its own on a numbering
    /// nothing else shares; pooled in, they are 3,418 rooms that name nothing
    /// and they bury the handful that genuinely have no space.
    #[test]
    fn test_room_models_scopes_the_room_side() {
        let rooms = vec![
            rooms_of("arch", vec![keyed("r1", "101", &[])]),
            rooms_of("el", vec![keyed("r9", "1", &[]), keyed("r10", "2", &[])]),
        ];
        let spaces = vec![spaces_of("me", vec![keyed("s1", "101", &[])])];

        let unscoped = space_report(&rooms, &spaces, &[], &keyed_policy(), &[]);
        assert_eq!(
            unscoped.rooms_without_space.len(),
            2,
            "the electrical model's own rooms, named by nothing"
        );

        let scoped = space_report(
            &rooms,
            &spaces,
            &[],
            &SpacePolicy { room_models: vec!["arch".to_string()], ..keyed_policy() },
            &[],
        );
        assert!(scoped.rooms_without_space.is_empty());
        assert_eq!(scoped.room_models, vec!["arch".to_string()]);
        assert_eq!(scoped.total_rooms, 1);
    }

    /// Both directions are full lists. An unmatched room is a finding of the
    /// same standing as an unmatched space -- the reversal the first draft of
    /// this design got wrong.
    #[test]
    fn test_both_unmatched_directions_are_reported() {
        let rooms = vec![rooms_of("arch", vec![keyed("r1", "101", &[]), keyed("r2", "999", &[])])];
        let spaces = vec![spaces_of("me", vec![keyed("s1", "101", &[]), keyed("s2", "888", &[])])];

        let report = space_report(&rooms, &spaces, &[], &keyed_policy(), &[]);

        assert_eq!(report.by_model[0].unmatched.len(), 1);
        assert_eq!(report.by_model[0].unmatched[0].key.as_deref(), Some("888"));
        assert_eq!(report.rooms_without_space.len(), 1);
        assert_eq!(report.rooms_without_space[0].key.as_deref(), Some("999"));
    }

    /// **The threshold pair, which is the measurement `tolerance_min` exists
    /// for.** A 1.44 m2 room against a 1.96 m2 space is 36% -- over a flat 30%,
    /// and pure noise from the boundary regime. A 2 m2 floor removes it while
    /// leaving a genuine 20 m2 versus 30 m2 disagreement standing.
    #[test]
    fn test_a_percentage_alone_flags_small_rooms_and_a_floor_removes_them() {
        let rooms = vec![rooms_of(
            "arch",
            vec![
                keyed("r1", "101", &[("Area", "1.44")]),
                keyed("r2", "102", &[("Area", "20.0")]),
            ],
        )];
        let spaces = vec![spaces_of(
            "me",
            vec![
                keyed("s1", "101", &[("Area", "1.96")]),
                keyed("s2", "102", &[("Area", "30.0")]),
            ],
        )];

        let area = |min: Option<f64>| SpaceFieldConfig {
            space: "Area".to_string(),
            room: None,
            field_type: crate::settings::FieldType::Numeric,
            qa: None,
            tolerance_pct: Some(30.0),
            tolerance_min: min,
        };

        let loose = space_report(
            &rooms,
            &spaces,
            &[],
            &SpacePolicy { compared_properties: vec![area(None)], ..keyed_policy() },
            &[],
        );
        assert_eq!(loose.property_mismatches.len(), 2, "a flat 30% flags the riser too");

        let floored = space_report(
            &rooms,
            &spaces,
            &[],
            &SpacePolicy { compared_properties: vec![area(Some(2.0))], ..keyed_policy() },
            &[],
        );
        assert_eq!(floored.property_mismatches.len(), 1, "the small room drops out");
        assert_eq!(floored.property_mismatches[0].key, "102");
        assert_eq!(floored.property_mismatches[0].delta_pct, Some(50.0));
    }

    /// A zero room value is `incomparable`, never a 100% mismatch. Guaranteed
    /// to occur: an unenclosed space has an area of zero, so the states this
    /// entity invents meet the arithmetic.
    #[test]
    fn test_a_zero_room_value_is_incomparable_not_a_mismatch() {
        let rooms = vec![rooms_of("arch", vec![keyed("r1", "101", &[("Area", "0")])])];
        let spaces = vec![spaces_of("me", vec![keyed("s1", "101", &[("Area", "12.0")])])];

        let report = space_report(
            &rooms,
            &spaces,
            &[],
            &SpacePolicy {
                compared_properties: vec![SpaceFieldConfig {
                    space: "Area".to_string(),
                    room: None,
                    field_type: crate::settings::FieldType::Numeric,
                    qa: None,
                    tolerance_pct: Some(30.0),
                    tolerance_min: None,
                }],
                ..keyed_policy()
            },
            &[],
        );

        assert!(report.property_mismatches.is_empty());
        assert_eq!(report.incomparable.len(), 1);
        assert_eq!(report.incomparable[0].property, "Area");
    }

    /// No key configured means nothing was *attempted*, and the report says so
    /// by carrying no key rather than by reporting a clean match.
    #[test]
    fn test_an_unconfigured_project_still_reports_presence() {
        let spaces = vec![spaces_of("me", vec![keyed("s1", "101", &[])])];
        let report = space_report(&[], &spaces, &["me".to_string()], &SpacePolicy::default(), &[]);

        assert!(report.comparison_key.is_none());
        assert!(report.by_model.is_empty(), "no matching was attempted");
        assert_eq!(report.total_spaces, 1, "presence needs no configuration to be true");
    }

    /// Enclosure is tallied for the collapsed header, and `not_enclosed` sums
    /// the two defect states rather than reporting one of them.
    #[test]
    fn test_enclosure_is_tallied() {
        let mut unenclosed = keyed("s2", "102", &[]);
        unenclosed.enclosure = Some(Enclosure::Unenclosed);
        let mut unmeasured = keyed("s3", "103", &[]);
        unmeasured.enclosure = Some(Enclosure::Unmeasured);
        let mut enclosed = keyed("s1", "101", &[]);
        enclosed.enclosure = Some(Enclosure::Enclosed);

        let spaces = vec![spaces_of("me", vec![enclosed, unenclosed, unmeasured])];
        let report = space_report(&[], &spaces, &[], &SpacePolicy::default(), &[]);

        assert_eq!(report.enclosure.enclosed, 1);
        assert_eq!(report.enclosure.unenclosed, 1);
        assert_eq!(report.enclosure.unmeasured, 1);
        assert_eq!(report.discrepancies.not_enclosed, 2);
    }
}
