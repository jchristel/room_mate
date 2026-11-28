//! The settings the server runs on, per project: `ProjectSettings` (what one project resolves to) and `SettingsRegistry` (all of them, swapped
//! whole when settings change).

use super::*;

/// One reference source's resolved runtime state for a project: its loaded
/// data (`None` when configured but not yet uploaded — the `Upload`-sourced,
/// no-CSV-yet state) and its declared per-column type/QA fields, carried
/// together so a request-time consumer (comparison, validation) never needs a
/// second lookup back into `Settings` for the field configs that go with the
/// data it just resolved.
#[derive(Clone)]
pub struct ProjectReferenceSource {
    /// Which primary entity this source joins onto — carried through from
    /// `ReferenceSourceConfig::entity` so every read path can filter to its own
    /// entity without reaching back into the raw settings.
    pub entity: ReferenceEntity,
    pub data: Option<ReferenceData>,
    pub fields: Vec<ReferenceFieldConfig>,
}

/// One project's classification/join inputs — everything that used to be a
/// flat field on `AppState`, bundled so it can be registered per project
/// instead of applied globally.
#[derive(Clone)]
pub struct ProjectSettings {
    /// Resolved reference sources for this project, keyed by source name —
    /// the same name a `/rooms` filter or `comparison_key` writes before the
    /// dot (`drofus.NetArea`). Loaded once at startup from
    /// `Settings.sources.reference`. Joined onto rooms at response assembly —
    /// a stored snapshot is never mutated by the join.
    ///
    /// **Every source in this map is live on every read path**: the join and
    /// filter in `service::rooms`, the QA report in `service::validation`
    /// (one section per source), `service::comparison`, the graph nodes in
    /// `service::adjacency`, and each milestone's per-source snapshot pin. No
    /// source is privileged — "drofus" is just the name most projects happen
    /// to configure.
    pub reference: BTreeMap<String, ProjectReferenceSource>,

    /// Classification tiers loaded from this project's settings. Resolved
    /// per-room inside `/rooms` assembly; not cached (see classify_room).
    pub hierarchy: Vec<HierarchyTier>,

    /// Canonical → per-source raw property name mappings loaded from this
    /// project's settings. Passed to `lookup_property` alongside each room's
    /// source so dRofus join and classification resolve names consistently
    /// regardless of which producer the room came from.
    pub builtin_properties: Vec<BuiltinPropertyDef>,

    /// Ordered property names shown on a room's label in the viewer. Resolved
    /// per-room inside `/rooms` assembly, same as `hierarchy`.
    pub room_label: Vec<String>,

    /// User-defined milestones (named dates with explicit snapshot pins)
    /// loaded from this project's settings. Read by the milestones listing
    /// and by `assemble_rooms`' milestone filter.
    pub milestones: Vec<Milestone>,

    /// The model whose coordinate space this project's plans are drawn in, or
    /// `None` to derive it. See `Settings::anchor_model`; read by
    /// `service::placement::from_index`.
    pub anchor_model: Option<String>,

    /// The FF&E type parameter whose value keys a tag rule (see `Settings::tag_type_code`),
    /// or `None` when the project names none. Read by `service::task_build`.
    pub tag_type_code: Option<String>,

    /// The user-chosen room property that identifies "the same room" across
    /// milestones, or `None` when unset (see `Settings::comparison_key`). Read
    /// by `service::comparison`; its own concept, independent of the dRofus
    /// `link_property`.
    pub comparison_key: Option<String>,

    /// Ordered room property names compared across milestones (see
    /// `Settings::comparison_properties`). Read by `service::comparison`.
    pub comparison_properties: Vec<String>,

    /// This project's area-measurement policy (see `settings::AreaPolicy`):
    /// the declared standard, the wall-thickness ceiling, and the boundary-
    /// regime fallback for models whose extractor predates the envelope field.
    /// Server-used like `hierarchy_exclusions`, not client-only like
    /// `colour_plans`, so it belongs in the resolved bundle — `service::areas`
    /// sizes its wall zone from it and `service::adjacency` takes its default
    /// gap tolerance from the same value.
    pub areas: crate::settings::AreaPolicy,

    /// This project's door policy (see `settings::OpeningPolicy`): the comparison
    /// key and comparable property set for doors. Server-used like `areas`, so
    /// it belongs in the resolved bundle rather than being re-read from
    /// `Settings` at request time.
    pub doors: crate::settings::OpeningPolicy,

    /// The same, for windows. A second field rather than a map keyed on the
    /// entity: the two are read from different call sites and a map would put
    /// an `unwrap_or_default` at each, which is how a project silently gets the
    /// wrong policy.
    pub windows: crate::settings::OpeningPolicy,

    /// This project's FF&E policy (see `settings::FfePolicy`). Its own type
    /// rather than a third `OpeningPolicy`: an item has one room, so there is
    /// no attribution to choose, and it has components, which no opening does.
    pub ffe: crate::settings::FfePolicy,

    /// This project's spaces policy: the space-to-room key and the properties
    /// compared over it. See `crate::settings::SpacePolicy`.
    pub spaces: crate::settings::SpacePolicy,

    /// Footprint exclusions for the hierarchy-areas feature. Unlike
    /// `colour_plans` (client-only, never in this bundle), exclusions are used by
    /// the SERVER when it computes footprints in `service::areas`, so they belong
    /// here in the resolved bundle alongside `hierarchy` — resolved via
    /// `settings_for` like every other classification input.
    pub hierarchy_exclusions: Vec<HierarchyExclusion>,

    /// Where a route's size checks read their data from. Read by `service::connectivity`.
    pub routing: roommate_shared::settings::RoutingPolicy,

    /// How the room-state audit judges place, nearness and turn. Read by `service::blueprint_audit`.
    pub blueprint_audit: roommate_shared::settings::BlueprintAuditPolicy,
}

/// One immutable snapshot of every project's settings. Swapped wholesale
/// behind `AppState`'s lock when the settings UI saves (see `settings_api`) —
/// a request takes ONE snapshot up front and works off it, so a mid-request
/// swap can never produce a torn read (half old hierarchy, half new dRofus).
pub struct SettingsRegistry {
    /// Per-project settings bundles, keyed by project id. Storage stays one
    /// tree keyed by `(project_id, model_id)` independently of this registry
    /// — a project can have stored snapshots with no registered settings (see
    /// `settings_for`'s fallback/skip semantics at each call site).
    pub by_project: HashMap<String, ProjectSettings>,

    /// Explicit fallback bundle for a project with no dedicated settings
    /// file, if the operator configured one (one project file marked
    /// `is_default = true`). When absent, an unregistered project is skipped
    /// on read and rejected on ingest rather than silently falling back to
    /// any bundle.
    pub default: Option<ProjectSettings>,

    /// What this registry's *content* hashes to — mixed into every read's
    /// ETag cursor (`service::scope_cursor`) so a settings change invalidates
    /// a cached read the way a push does.
    ///
    /// **Without it a policy change is invisible to a polling client.** The
    /// cursor hashes which snapshot each model would serve; flipping
    /// `room_attribution` or a space key rewrites every row of `/doors`,
    /// `/spaces` or a report while serving no new snapshot, so the tag stayed
    /// byte-identical and the client was answered 304 over the old body until
    /// the next push. A settings save hot-swaps the registry mid-session, so
    /// that window is reachable in an ordinary afternoon, not only across a
    /// restart.
    ///
    /// **A content hash, not a counter.** Identical settings must produce an
    /// identical tag: a restart, or a save that changes nothing, would
    /// otherwise throw away every client's cache, and a counter reset by a
    /// restart would *match* a pre-change tag when the files were edited by
    /// hand while the server was down — which is the one direction a cursor
    /// may never be wrong in (see `scope_cursor`'s asymmetry).
    ///
    /// Built by [`SettingsRegistry::new`]; see there for what feeds it.
    pub revision: u64,
}

impl SettingsRegistry {
    /// Build a registry, hashing `files_digest` together with the reference
    /// data the bundles resolved to.
    ///
    /// **Two inputs, because neither sees the other's changes.**
    /// `files_digest` is over the settings files' bytes (see
    /// `bootstrap::load_project_settings_dir`), which is complete by
    /// construction — a field added to `Settings` needs nothing added here,
    /// which a hand-listed digest of policy fields would (and would then
    /// silently stop covering the day someone forgot). But an uploaded
    /// reference CSV changes what every joined column answers *without*
    /// touching a settings file, so the resolved `ReferenceData` is hashed
    /// too: source name, link property and every record, since a re-upload
    /// with the same row count and different values is exactly the case a
    /// cheaper digest would miss.
    ///
    /// Deliberately NOT hashed: anything that cannot change a served row.
    /// The bundles hold no client-only settings today (`colour_plans` never
    /// reaches one), so this is a rule for the next field rather than a
    /// current exclusion list — and the files digest covers those anyway.
    pub fn new(
        by_project: HashMap<String, ProjectSettings>,
        default: Option<ProjectSettings>,
        files_digest: u64,
    ) -> Self {
        use std::hash::{Hash, Hasher};

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        files_digest.hash(&mut hasher);

        // Sorted, for the reason `scoped_revision` sorts: a HashMap's
        // iteration order is an artefact, not content, and a revision that
        // changed on rehash would invalidate every cache for nothing.
        let mut projects: Vec<&String> = by_project.keys().collect();
        projects.sort_unstable();
        for id in projects {
            id.hash(&mut hasher);
            for (name, source) in &by_project[id].reference {
                name.hash(&mut hasher);
                match &source.data {
                    // `BTreeMap` throughout, so iteration is key order and the
                    // hash is stable across runs.
                    Some(data) => {
                        data.link_property.hash(&mut hasher);
                        for (key, record) in &data.by_id {
                            key.hash(&mut hasher);
                            for (field, value) in &record.fields {
                                field.hash(&mut hasher);
                                value.hash(&mut hasher);
                            }
                        }
                    }
                    // "No upload yet" is a state a later upload moves off, so
                    // it has to hash differently from any dataset.
                    None => "<no data>".hash(&mut hasher),
                }
            }
        }

        Self { by_project, default, revision: hasher.finish() }
    }

    /// Build a registry from bundles alone — in-memory construction, where
    /// there are no settings files to hash. The reference half of the digest
    /// still applies. Production always goes through
    /// `bootstrap::load_project_settings_dir`, which has the bytes.
    pub fn from_bundles(by_project: HashMap<String, ProjectSettings>, default: Option<ProjectSettings>) -> Self {
        Self::new(by_project, default, 0)
    }

    /// Resolve the settings bundle for one project: its own registered
    /// settings if present, else the explicit default bundle if one is
    /// configured, else `None` (unregistered, no fallback).
    pub fn settings_for(&self, project_id: &str) -> Option<&ProjectSettings> {
        self.by_project.get(project_id).or(self.default.as_ref())
    }

    /// The union of every registered project's reference-source names —
    /// what a `/rooms` filter or `comparison_key` may write before the dot
    /// (`service::rooms::split_namespace`'s vocabulary). Computed fresh from
    /// the live registry rather than cached: a `/rooms` filter can span
    /// several projects unscoped, so its namespace vocabulary can't be
    /// resolved from any one project's config alone, and recomputing this
    /// small union (a handful of projects, a source or two each) per request
    /// is cheap — cheaper than a second piece of state to keep in sync across
    /// a settings hot-swap.
    pub fn known_reference_sources(&self) -> std::collections::BTreeSet<String> {
        self.by_project
            .values()
            .chain(self.default.iter())
            .flat_map(|p| p.reference.keys())
            .cloned()
            .collect()
    }
}
