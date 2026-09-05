//! `/ffe` read assembly: merge every stored model's latest FF&E snapshot into
//! one flat payload, scoped by project, milestone and property filter.
//!
//! **Deliberately thin, and the thinness is the claim.** Everything this shares
//! with `service::openings` — scoping, milestone pinning, the revision hash, the
//! phase report, the candidate build, the building lookup, the filter grammar,
//! the reference join — is not written here at all; it is
//! `service::entity_scope`, generic over `contract::SnapshotEnvelope`, which
//! knows nothing about either entity. What is left in this file is the part that
//! genuinely differs from an opening, and it is two things:
//!
//! - **one room, not two sides.** An item names the room it is in, so there is
//!   no attribution policy to apply and `owner_rooms` is a list of zero or one.
//! - **containment, not a probe off a wall.** `locate_within` tests the point
//!   where the item stands; stepping would move it *out* of the right answer.
//!
//! That is the whole per-entity cost of a read, and it is what
//! [Entities](../../docs/STRATEGY-ENTITIES.md) predicted when it said a category
//! with one side "needs the glue, not the geometry".
//!
//! **`OpeningKind` has no `Ffe` variant and must not grow one.** That enum is
//! the per-*opening* lookup table: it answers which storage kind, which
//! reference entity, which `OpeningPolicy` and which pin map, and two of those
//! four have no correct answer for an item — there is no `OpeningPolicy` for
//! FF&E, because `room_attribution` would be a field choosing between sides an
//! item does not have. The lookups it would have provided are three lines at the
//! two call sites below instead.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::contract::{Item, ItemEnvelope, PropertyPresence};
use crate::reference::{ReferenceData, ReferenceRecord};
use crate::settings::{BuiltinPropertyDef, NestedComponents, ReferenceEntity, RoomResolution};
use crate::state::{AppState, ModelKey};
use crate::storage::SnapshotKind;

use super::entity_scope::{self, Probe, SideOrigin};
use super::room_locator::{self, RoomRef, Unresolved};
use super::rooms::{FilterTarget, RoomFilter};
use super::ServiceError;

/// An FF&E instance as sent to a consumer: the stored item plus the identity of
/// the model it came from.
#[derive(Serialize)]
pub struct ItemResponse {
    #[serde(flatten)]
    pub item: Item,

    /// The project this item's model belongs to.
    pub project_id: String,
    /// The model this item came from — the scope its room reference resolves in.
    ///
    /// On the wire for the reason `OpeningResponse`'s is: a room id is unique
    /// only within a model, so a consumer resolving an item to its room needs to
    /// know which model's rooms to look in.
    pub model_id: String,

    /// The room this item is attributed to, in a list of zero or one.
    ///
    /// **A list for an entity that can only ever have one owner**, which looks
    /// like over-engineering and is not. It keeps "empty means homeless"
    /// identical across every entity, so `?building=` and every consumer that
    /// already handles an unowned door handles an unowned item with no special
    /// case. An `Option` here would be a second spelling of the same state and
    /// would fork exactly the code that ought to be shared.
    ///
    /// Derived at read time from the stored reference, never stored — so
    /// changing `[ffe] nested_components` or `room_resolution` changes every
    /// answer immediately and rewrites nothing.
    pub owner_rooms: Vec<String>,

    /// The same owner, **model-qualified**, and the only field that can name a
    /// room in another model. See `OpeningResponse::owner_rooms_qualified`,
    /// which carries the full argument: once geometry can reach a room in a
    /// linked model, a bare id would resolve against the wrong model's rooms —
    /// a wrong answer that looks right.
    pub owner_rooms_qualified: Vec<RoomRef>,

    /// Where the room reference came from — stated by the model, derived from
    /// geometry, or unresolved with a reason. One side, because an item has one.
    pub room_origin: SideOrigin,

    /// Whether this item is a **component of another instance**, and the policy
    /// therefore let it through rather than excluding it.
    ///
    /// On the wire because the exclusion is a policy: a consumer reading a
    /// project configured `nested_components = "include"` needs to be able to
    /// tell a joinery handle from the joinery. `false` for every item under the
    /// default, since those are not returned at all.
    pub is_component: bool,

    /// Joined reference-source records, keyed by source name — the same shape
    /// and the same flattening `RoomResponse` and `OpeningResponse` use, so
    /// `schedule.AssetNumber` reads identically on any entity.
    ///
    /// Only sources declaring `entity = "ffe"` land here. A source with no match
    /// for this item contributes no entry: an unmatched key is a signal, not an
    /// error.
    #[serde(flatten)]
    pub reference: BTreeMap<String, ReferenceRecord>,

    /// The owning model's `Model.source`, for canonical property resolution. Not
    /// wire shape, same as `RoomResponse::source`.
    #[serde(skip)]
    pub source: String,
}

/// An item resolves its own two property tiers plus a small set of intrinsics.
///
/// The same three-way answer `RoomResponse` and `OpeningResponse` give, so a
/// predicate written against one entity means the same thing against another.
/// `$room` and `$category` are the two intrinsics an item adds and an opening
/// has no equivalent for.
impl FilterTarget for ItemResponse {
    fn presence(&self, source: Option<&str>, property: &str, builtin_defs: &[BuiltinPropertyDef]) -> PropertyPresence {
        /// An item's own struct fields always exist, so blank collapses to
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
                "$id" => intrinsic(Some(&self.item.id)),
                "$type_name" => intrinsic(Some(&self.item.type_name)),
                "$type_id" => intrinsic(Some(&self.item.type_id)),
                "$level_id" => intrinsic(Some(&self.item.level_id)),
                // The two an opening has no equivalent for. `$category=` is the
                // first question anyone asks of FF&E, which is why `category`
                // is on the record at all even though the export does not carry
                // it.
                "$category" => intrinsic(Some(&self.item.category)),
                // `$room=` with no match is how a caller asks "what is in this
                // room". An item outside every room is `Absent`, so it fails
                // every operator rather than matching a negative one -- the
                // same rule `$from_room` follows for an external door.
                "$room" => intrinsic(self.item.room.as_deref()),
                canonical => crate::contract::property_presence(&self.item, canonical, &self.source, builtin_defs),
            },
        }
    }
}

/// Resolve one comparable/filterable field name against an assembled item, in
/// the same `source.property` vocabulary `/ffe`'s filter parses.
///
/// The item counterpart of `openings::resolve_presence`, and it exists for the
/// identical reason: "what can I write before the dot" must have one answer
/// across filtering and comparison, or a name that filters correctly would
/// silently diff as nothing.
pub fn resolve_presence(
    item: &ItemResponse,
    field: &str,
    known: &std::collections::BTreeSet<String>,
    builtin: &[BuiltinPropertyDef],
) -> PropertyPresence {
    match super::rooms::split_namespace(field, known) {
        super::rooms::NamespaceSplit::Joined { source, property } => item.presence(Some(&source), property, builtin),
        super::rooms::NamespaceSplit::Unqualified(name) => item.presence(None, name, builtin),
        super::rooms::NamespaceSplit::UnknownSource(_) => PropertyPresence::Absent,
    }
}

/// Everything that narrows an FF&E read.
#[derive(Default)]
pub struct ItemScope<'a> {
    pub project: Option<&'a str>,
    /// Restrict to items whose owning room is in this building. A **homeless**
    /// item matches no building and drops out, exactly as a homeless door does
    /// and for the same reason: nothing attributes it.
    pub building: Option<&'a str>,
    pub milestone: Option<&'a str>,
    pub filter: Option<&'a RoomFilter>,
}

/// The merged FF&E payload, as `/ffe` answers it.
#[derive(Serialize)]
pub struct FfeResult {
    pub schema_version: u32,
    /// Stable content revision over the contributing `(model, snapshot)` pairs,
    /// same role and same construction as `RoomsResult::revision`.
    pub revision: String,
    pub ffe: Vec<ItemResponse>,
    /// The Revit phase each contributing model's items were filtered to, keyed
    /// by project id then model id. Read off each snapshot, never off the
    /// lineage's current phase.
    pub phase_by_model: BTreeMap<String, BTreeMap<String, Option<String>>>,
    /// How many items this read **excluded** as components of another instance,
    /// under `[ffe] nested_components`.
    ///
    /// **On the wire so an exclusion is a number rather than a silence**, which
    /// is the whole argument for the policy living here instead of in the
    /// extractor. 2236 of 4134 exported "doors" on one job were hardware, and
    /// nobody could see it because the producer had dropped them before anyone
    /// could count. On House A this reads 179 of 647.
    pub excluded_components: usize,
}

/// Merge every stored model's FF&E into one payload, scoped by `ItemScope`.
///
/// Returns `Ok(None)` when nothing has ever been pushed — the adapter's 204
/// case, same contract as `assemble_rooms` and `assemble_openings`. A filter or
/// scope that merely matches nothing is `Ok(Some)` with an empty list: the store
/// has data, the question just has an empty answer.
///
/// Eight lines over the discovery threshold and kept whole, on the same terms
/// `assemble_openings` is: two sequential phases over one scoped set — scope,
/// then derive — and the second reads the first. Splitting them would move the
/// order into call sites without removing it, and the order is the part a reader
/// has to see. The per-item derivation is the only candidate for a helper and it
/// would take eight arguments, which is the shape that hides an order rather
/// than one that states it.
///
/// Worth noting it is *shorter* than `assemble_openings` despite doing the same
/// job for a fourth entity, and that difference is `entity_scope`: the scoping,
/// pinning, revision, phase report and candidate build that used to be written
/// out here are four calls.
#[allow(clippy::too_many_lines)]
pub fn assemble_items(state: &AppState, scope: &ItemScope<'_>) -> Result<Option<FfeResult>, ServiceError> {
    if !state.has_any_snapshot(SnapshotKind::Ffe).map_err(ServiceError::Internal)? {
        return Ok(None);
    }
    let registry = state.settings();

    // Phase 1 -- scope, through the same function every entity uses. The pin map
    // is the only per-entity argument, and it arrives as a closure.
    let scoped: Vec<(ModelKey, crate::contract::FfePayload)> =
        entity_scope::scope_snapshots(state, SnapshotKind::Ffe, scope.project, scope.milestone, |ms| {
            &ms.ffe_attachments
        })?;

    let revision = entity_scope::revision(&scoped);
    let phase_by_model = entity_scope::phase_by_model(&scoped);
    let mut items: Vec<ItemResponse> = Vec::new();
    let mut excluded_components = 0usize;

    // An item's building is its room's building, resolved only when a building
    // filter is actually given -- a second storage read plus a classification
    // pass, and the common read does not need it.
    let building_of_room = match scope.building {
        Some(_) => entity_scope::building_by_room(state, scope.project, scope.milestone)?,
        None => BTreeMap::new(),
    };

    // Geometric resolution, when a project asks for it. Off by default and
    // costing nothing then -- and Off is the right default here where it was
    // arguable for windows, because FF&E lives in the same document as its
    // rooms and authored references populate.
    let mut candidates_by_project: BTreeMap<String, entity_scope::Candidates> = BTreeMap::new();
    for (_, payload) in &scoped {
        let mode = registry
            .settings_for(&payload.project.id)
            .map(|b| b.ffe.room_resolution)
            .unwrap_or_default();
        if mode == RoomResolution::Off || candidates_by_project.contains_key(&payload.project.id) {
            continue;
        }
        candidates_by_project.insert(
            payload.project.id.clone(),
            entity_scope::build_candidates(state, Some(&payload.project.id), scope.milestone, mode, &scoped)?,
        );
    }

    // Phase 2 -- derive the response items, applying the property filter *after*
    // assembly so a predicate sees the same resolved vocabulary a consumer does.
    for (_key, payload) in &scoped {
        let bundle = registry.settings_for(&payload.project.id);
        let builtin: &[BuiltinPropertyDef] = bundle.map(|b| b.builtin_properties.as_slice()).unwrap_or_default();
        let nested = bundle.map(|b| b.ffe.nested_components).unwrap_or_default();

        let sources: BTreeMap<&str, &ReferenceData> = bundle
            .map(|b| {
                b.reference
                    .iter()
                    .filter(|(_, cfg)| cfg.entity == ReferenceEntity::Ffe)
                    .filter_map(|(name, cfg)| Some((name.as_str(), cfg.data.as_ref()?)))
                    .collect()
            })
            .unwrap_or_default();

        for item in payload.items() {
            let is_component = item.super_component_id.is_some();
            // Counted before it is dropped, so `excluded_components` is a count
            // of what the policy removed rather than of what happened to be in
            // the snapshot -- which is the number that tells a reader whether a
            // homeless-looking model is a data artifact.
            if is_component && nested == NestedComponents::Exclude {
                excluded_components += 1;
                continue;
            }

            let reference: BTreeMap<String, ReferenceRecord> = sources
                .iter()
                .filter_map(|(name, data)| {
                    let record =
                        crate::contract::lookup_property(item, &data.link_property, &payload.model.source, builtin)
                            .and_then(|key| data.by_id.get(&key).cloned())?;
                    Some(((*name).to_string(), record))
                })
                .collect();

            // Authored first, geometry only where the model said nothing -- and
            // the probe is skipped entirely for an item that named a room, since
            // nothing it could find would be used.
            let derived = match candidates_by_project.get(&payload.project.id) {
                Some(candidates) if item.room.is_none() => candidates.locate_within(
                    &Probe {
                        insertion_point: item.insertion_point,
                        loops: &item.loops,
                        level_id: &item.level_id,
                        // No wall to step off. See `locate_within`.
                        normal: None,
                    },
                    &payload.model.id,
                ),
                _ => room_locator::Located::Unresolved(Unresolved::NoCandidate),
            };
            let room_origin = entity_scope::side_origin(item.room.as_deref(), &payload.model.id, &derived);
            let owner_rooms_qualified: Vec<RoomRef> = room_origin.room().cloned().into_iter().collect();

            let response = ItemResponse {
                owner_rooms: owner_rooms_qualified
                    .iter()
                    .filter(|r| r.model_id == payload.model.id)
                    .map(|r| r.room_id.clone())
                    .collect(),
                owner_rooms_qualified,
                room_origin,
                is_component,
                item: item.clone(),
                project_id: payload.project.id.clone(),
                model_id: payload.model.id.clone(),
                reference,
                source: payload.model.source.clone(),
            };

            if let Some(wanted) = scope.building
                && !response.owner_rooms_qualified.iter().any(|room| {
                    building_of_room
                        .get(&(room.model_id.clone(), room.room_id.clone()))
                        .is_some_and(|b| b == wanted)
                })
            {
                continue;
            }
            if scope.filter.is_none_or(|f| f.matches(&response, builtin)) {
                items.push(response);
            }
        }
    }

    Ok(Some(FfeResult {
        schema_version: crate::contract::SUPPORTED_FFE_SCHEMA,
        revision,
        ffe: items,
        phase_by_model,
        excluded_components,
    }))
}

/// Resolve every item in one project against its rooms, for the QA report.
///
/// **Latest-based, with no milestone scope**, matching `/validation` as a whole:
/// the report is about the data being served now, and the drift it exists to
/// catch is precisely a rooms snapshot moving on without its FF&E.
///
/// Returns an empty map when resolution is off, so the caller needs no branch
/// beyond the one that decides whether to ask.
pub fn locate_project_items(
    state: &AppState,
    project_id: &str,
    mode: RoomResolution,
    stored: &[(ModelKey, crate::contract::FfePayload)],
) -> Result<BTreeMap<(String, String), room_locator::Located>, ServiceError> {
    let mut out = BTreeMap::new();
    if mode == RoomResolution::Off {
        return Ok(out);
    }
    let candidates = entity_scope::build_candidates(state, Some(project_id), None, mode, stored)?;
    for (key, payload) in stored.iter().filter(|(_, p)| p.project.id == project_id) {
        for item in payload.items() {
            out.insert(
                (key.model_id.clone(), item.id.clone()),
                candidates.locate_within(
                    &Probe {
                        insertion_point: item.insertion_point,
                        loops: &item.loops,
                        level_id: &item.level_id,
                        normal: None,
                    },
                    &payload.model.id,
                ),
            );
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{CustomValue, FfePayload, Model, Project, Snapshot, SUPPORTED_FFE_SCHEMA};
    use crate::settings::FfePolicy;
    use crate::state::ProjectSettings;
    use crate::storage::MemStore;
    use std::collections::{BTreeSet, HashMap};

    fn make_item(id: &str, category: &str, room: Option<&str>, parent: Option<&str>) -> Item {
        Item {
            id: id.to_string(),
            level_id: "1".to_string(),
            category: category.to_string(),
            room: room.map(str::to_string),
            // These tests exercise attribution, policy and filtering, never
            // placement, so `None` is the honest input rather than a stub: an
            // item with no measured position is a state the contract carries.
            insertion_point: None,
            facing: None,
            loops: vec![],
            super_component_id: parent.map(str::to_string),
            type_id: "t1".to_string(),
            type_name: "Desk 1600x800".to_string(),
            properties: BTreeMap::from([(
                "Mark".to_string(),
                CustomValue { value: format!("F-{id}"), storage_type: None },
            )]),
            type_properties: BTreeMap::new(),
        }
    }

    fn bundle(ffe: FfePolicy) -> ProjectSettings {
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
            ffe,
            hierarchy_exclusions: vec![],
        }
    }

    fn state_with(items: Vec<Item>, policy: FfePolicy) -> AppState {
        let mut projects = HashMap::new();
        projects.insert("p1".to_string(), bundle(policy));
        let state = AppState::new(Box::new(MemStore::new()), projects, None);
        state
            .set_element_snapshot(
                SnapshotKind::Ffe,
                &FfePayload {
                    schema_version: SUPPORTED_FFE_SCHEMA,
                    project: Project { id: "p1".to_string(), name: "P".to_string() },
                    model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                    snapshot: Snapshot { taken_at: "2026-09-05T00:00:00Z".to_string() },
                    phase: Some("New Construction".to_string()),
                    model_to_shared: None,
                    levels: vec![],
                    ffe: items,
                },
            )
            .unwrap();
        state
    }

    fn filter(expr: &str) -> RoomFilter {
        RoomFilter::parse_query(expr, &BTreeSet::new()).expect("parses")
    }

    /// Nothing pushed at all is `None` -- the adapter's 204, distinct from a
    /// scope that matched nothing.
    #[test]
    fn test_no_ffe_pushed_is_none() {
        let state = AppState::new(Box::new(MemStore::new()), HashMap::new(), None);
        assert!(assemble_items(&state, &ItemScope::default()).unwrap().is_none());
    }

    /// A scope that matches nothing is an empty list, not `None`: the store has
    /// data, the question just has an empty answer.
    #[test]
    fn test_a_scope_matching_nothing_is_an_empty_list() {
        let state = state_with(vec![make_item("f1", "OST_Furniture", Some("r1"), None)], FfePolicy::default());
        let result = assemble_items(&state, &ItemScope { project: Some("nope"), ..Default::default() })
            .unwrap()
            .expect("the store has data");
        assert!(result.ffe.is_empty());
    }

    /// **One room, so `owner_rooms` is a list of exactly zero or one.** The list
    /// shape is what keeps "empty means homeless" identical across entities;
    /// there is no attribution policy to apply, because there are no sides to
    /// choose between.
    #[test]
    fn test_an_item_is_attributed_to_the_one_room_it_names() {
        let state = state_with(
            vec![
                make_item("f1", "OST_Furniture", Some("r1"), None),
                make_item("f2", "OST_GenericModel", None, None),
            ],
            FfePolicy::default(),
        );
        let result = assemble_items(&state, &ItemScope::default()).unwrap().expect("data");

        let owned = result.ffe.iter().find(|i| i.item.id == "f1").expect("f1");
        assert_eq!(owned.owner_rooms, vec!["r1".to_string()]);
        assert!(matches!(owned.room_origin, SideOrigin::Authored(_)), "the model stated it");

        let homeless = result.ffe.iter().find(|i| i.item.id == "f2").expect("f2");
        assert!(homeless.owner_rooms.is_empty(), "empty means homeless, exactly as for an opening");
    }

    /// **Components are excluded by default and the count says how many**,
    /// which is the whole argument for the policy living on the read rather than
    /// in the extractor. 2236 of 4134 exported "doors" on one job were hardware
    /// and nobody could see it, because the producer had dropped them before
    /// anyone could count.
    #[test]
    fn test_components_are_excluded_and_counted() {
        let state = state_with(
            vec![
                make_item("f1", "OST_Furniture", Some("r1"), None),
                make_item("f2", "OST_Furniture", Some("r1"), Some("3729614")),
                make_item("f3", "OST_Furniture", Some("r1"), Some("3729614")),
            ],
            FfePolicy::default(),
        );
        let result = assemble_items(&state, &ItemScope::default()).unwrap().expect("data");

        assert_eq!(result.ffe.len(), 1, "only the parent survives");
        assert_eq!(result.ffe[0].item.id, "f1");
        assert_eq!(result.excluded_components, 2, "the exclusion is a number, not a silence");
        assert!(!result.ffe[0].is_component);
    }

    /// `nested_components = "include"` changes every answer and rewrites
    /// nothing -- the same read-time property `room_attribution` has. The
    /// components come back flagged, so a consumer can still tell a handle from
    /// the joinery.
    #[test]
    fn test_including_components_returns_them_flagged() {
        let policy = FfePolicy { nested_components: NestedComponents::Include, ..Default::default() };
        let state = state_with(
            vec![
                make_item("f1", "OST_Furniture", Some("r1"), None),
                make_item("f2", "OST_Furniture", Some("r1"), Some("3729614")),
            ],
            policy,
        );
        let result = assemble_items(&state, &ItemScope::default()).unwrap().expect("data");

        assert_eq!(result.ffe.len(), 2);
        assert_eq!(result.excluded_components, 0);
        let component = result.ffe.iter().find(|i| i.item.id == "f2").expect("f2");
        assert!(component.is_component, "a reader must still be able to tell it apart");
    }

    /// A nested item **names a room**, which is why "no room reference" cannot
    /// be the filter. Measured on House A: 97.8% of components name one against
    /// 84.8% of top-level items. Asserted here so nobody re-derives the doors
    /// discriminator from first principles and finds it plausible.
    #[test]
    fn test_a_component_still_names_its_room() {
        let policy = FfePolicy { nested_components: NestedComponents::Include, ..Default::default() };
        let state = state_with(vec![make_item("f2", "OST_Furniture", Some("r1"), Some("3729614"))], policy);
        let result = assemble_items(&state, &ItemScope::default()).unwrap().expect("data");
        assert_eq!(result.ffe[0].owner_rooms, vec!["r1".to_string()]);
    }

    /// `$category` is the intrinsic an opening has no equivalent for, and the
    /// first question anyone asks of FF&E. It filters through the shared
    /// grammar, so the predicate means here what it means everywhere.
    #[test]
    fn test_category_filters_through_the_shared_grammar() {
        let state = state_with(
            vec![
                make_item("f1", "OST_Furniture", Some("r1"), None),
                make_item("f2", "OST_GenericModel", Some("r1"), None),
            ],
            FfePolicy::default(),
        );
        let f = filter("$category=OST_Furniture");
        let result = assemble_items(&state, &ItemScope { filter: Some(&f), ..Default::default() })
            .unwrap()
            .expect("data");
        assert_eq!(result.ffe.len(), 1);
        assert_eq!(result.ffe[0].item.id, "f1");
    }

    /// `$room=` is how a caller asks "what is in this room". An item outside
    /// every room is `Absent` and fails every operator rather than matching a
    /// negative one -- the rule `$from_room` follows for an external door.
    #[test]
    fn test_room_filters_and_a_homeless_item_matches_nothing() {
        let state = state_with(
            vec![
                make_item("f1", "OST_Furniture", Some("r1"), None),
                make_item("f2", "OST_Furniture", Some("r2"), None),
                make_item("f3", "OST_Furniture", None, None),
            ],
            FfePolicy::default(),
        );
        let f = filter("$room=r1");
        let result = assemble_items(&state, &ItemScope { filter: Some(&f), ..Default::default() })
            .unwrap()
            .expect("data");
        let ids: BTreeSet<&str> = result.ffe.iter().map(|i| i.item.id.as_str()).collect();
        assert_eq!(ids, BTreeSet::from(["f1"]), "the homeless item is not swept in");
    }

    /// The property tiers resolve through the same `lookup_property` every
    /// entity uses, so a filter on a Revit parameter works on an item with no
    /// item-specific machinery at all.
    #[test]
    fn test_a_property_filter_reads_the_item_tiers() {
        let state = state_with(
            vec![
                make_item("f1", "OST_Furniture", Some("r1"), None),
                make_item("f2", "OST_Furniture", Some("r1"), None),
            ],
            FfePolicy::default(),
        );
        let f = filter("Mark=F-f2");
        let result = assemble_items(&state, &ItemScope { filter: Some(&f), ..Default::default() })
            .unwrap()
            .expect("data");
        assert_eq!(result.ffe.len(), 1);
        assert_eq!(result.ffe[0].item.id, "f2");
    }

    /// The phase rides the response per model, read off the snapshot rather
    /// than off the lineage -- PLAN-phasing D8, which every entity obeys.
    #[test]
    fn test_the_phase_is_reported_per_model() {
        let state = state_with(vec![make_item("f1", "OST_Furniture", Some("r1"), None)], FfePolicy::default());
        let result = assemble_items(&state, &ItemScope::default()).unwrap().expect("data");
        assert_eq!(result.phase_by_model["p1"]["m1"].as_deref(), Some("New Construction"));
        assert_eq!(result.schema_version, SUPPORTED_FFE_SCHEMA);
    }
}
