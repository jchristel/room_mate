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
//! recorded in `docs/PLAN-spaces.md` rather than merely omitted:
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

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::contract::{Enclosure, PropertyPresence, Room};
use crate::reference::{ReferenceData, ReferenceRecord};
use crate::settings::{BuiltinPropertyDef, ReferenceEntity};
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

            let response = SpaceResponse {
                space: space.clone(),
                reference,
                project_id: payload.project.id.clone(),
                model_id: payload.model.id.clone(),
                source: payload.model.source.clone(),
            };

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
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{Model, Project, Snapshot, SpacePayload};
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
            reference: BTreeMap::new(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec![],
            milestones: vec![],
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
        let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), bundle())]), None);
        let payload = SpacePayload {
            schema_version: crate::contract::SUPPORTED_SPACE_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: model.to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-09-06T00:00:00Z".to_string() },
            phase: Some("New Construction".to_string()),
            model_to_shared: None,
            room_boundary: None,
            levels: vec![],
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
}
