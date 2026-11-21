//! Project settings: the types a project's TOML file parses into, and their
//! validation.
//!
//! Config a human hand-edits lives here (hence TOML, with comments); the *data*
//! it points at stays JSON. Everything is resolved once at startup and fails
//! fast on bad config — better a loud startup error than a surprise on the first
//! request. See `settings-infrastructure-handoff.md`.
//!
//! `ReferenceOrigin` lives here (not in `drofus`) because it's part of the
//! settings contract; the `#[serde(tag = "type")]` enum is the seam that makes
//! the future file→API swap a loader-only change. `HierarchyTier` lives here
//! too, as the classification *definition*; `classify` consumes it but
//! doesn't own its shape.
//!
//! **In `roommate-shared` rather than the server**, so a consumer outside the
//! server can use the types — see the crate header. The loaders stayed behind in
//! `roommate::settings`, which glob re-exports this module, so the paths the
//! server uses (`crate::settings::Settings`, `::validate_reference_fields`, …)
//! never moved. Split by concern:
//! - **this file** — the config/domain types and their inherent `validate()`
//!   methods (part of each type's own API);
//! - **`validate`** — the standalone validation *functions* over those types.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

mod validate;

pub use validate::{validate_colour_plans, validate_reference_field_shapes, validate_reference_fields};

mod appearance;
mod colour;
mod milestone;
mod policy;
mod reference;
pub use appearance::*;
pub use colour::*;
pub use milestone::*;
pub use policy::*;
pub use reference::*;

/// One project's settings, parsed once at startup from its own TOML file (one
/// of N files in the `--project-settings` directory). Server-wide config
/// (`[storage]`, `[test_data]`) lives separately in `ServerConfig`, loaded
/// once from `--server-settings` independent of this per-project loop.
///
/// Also derives `Serialize` (as do all the types it contains): the settings
/// API serves this exact shape as JSON and writes it back as TOML, so the
/// wire shape and the config-file shape can never drift.
#[derive(Debug, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct Settings {
    /// This bundle's project id — matched against `RoomPayload.project.id` to
    /// select which bundle applies to a given model. Must be non-empty
    /// (validated at load).
    pub project_id: String,

    /// Human-readable project name, for display only — never matched against
    /// anything, so it stays freely editable in a way `project_id` (a storage
    /// path key) can't be. Producers read it from `/api/settings/projects` and
    /// send it as `RoomPayload.project.name`, which is what the store's
    /// `project.toml` manifest and the viewer's project picker then show; this
    /// file is where that name is *authored*.
    ///
    /// Optional, and absence is a normal state, not a defect: a project that
    /// never sets one is displayed under its id (every consumer falls back
    /// that way), which is exactly the behaviour before this field existed.
    /// Must be non-empty *when present* (validated at load) — a blank name is
    /// a mistake, and silently displaying an empty label is worse than saying
    /// so at startup.
    ///
    /// Declared here, adjacent to the other scalars and above `sources`,
    /// because TOML requires scalar keys to precede any table — serde emits
    /// fields in declaration order, so moving this below `sources` would write
    /// files that don't round-trip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// When true, this bundle is also the explicit fallback for any project
    /// with no dedicated settings file (`AppState::settings_for`). At most
    /// one project file may set this — validated across the whole directory
    /// at load time, not here (a single file can't see its siblings).
    #[serde(default)]
    pub is_default: bool,

    /// The room property whose value identifies "the same room" when comparing
    /// milestones (see `service::comparison`). Its own concept, deliberately
    /// **not** the dRofus `link_property`: milestone comparison stands entirely
    /// on its own — a project may compare milestones with no dRofus configured
    /// at all — so the id key it matches rooms on is user-chosen and lives
    /// here, separate from anything dRofus. `None` (the default, and every
    /// project file predating this feature) is a real, reachable state: the
    /// comparison then has no way to match rooms across milestones and reports
    /// a "no comparison key configured" result rather than silently falling
    /// back to dRofus or to room `id`. Resolved per-room the same canonical/
    /// source way as every other property name, so a rename or a second source
    /// needs no change here.
    ///
    /// A scalar declared before any table field so the TOML serializer emits it
    /// ahead of `[sources]` etc. — the ordering footgun documented in
    /// CODING-CONVENTIONS.md.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison_key: Option<String>,

    /// Ordered room property names compared across milestones, on rooms present
    /// in both a compared milestone and the baseline. Persisted here (not passed
    /// per request) with the same lifecycle as `room_label`/`milestones`, so it
    /// survives and rides the settings save pipeline. Enumeration is off the
    /// *baseline's* rooms at compare time (only properties on the baseline are
    /// comparable); a name that doesn't resolve on the other side is reported as
    /// a distinct "missing property" state, not a value difference. No startup
    /// validation — an unresolvable name simply contributes nothing, the house
    /// "absence is fine" discipline. Empty (the default) means no properties are
    /// compared, only room add/remove. A value array, declared before any table
    /// field for the same TOML-ordering reason as `comparison_key`.
    #[serde(default)]
    pub comparison_properties: Vec<String>,

    /// Reference sources joined onto this project's rooms, keyed by name (the
    /// join namespace — see `Sources`). Defaulted so a project with no
    /// external sources at all is legal config — a project not using dRofus
    /// is normal, and the validation endpoint reports it as an empty `sources`
    /// map rather than an error.
    #[serde(default)]
    pub sources: Sources,

    /// Area-measurement **policy** for this project (see `AreaPolicy`).
    /// Defaulted, so a project file predating it behaves exactly as before.
    /// `skip_serializing_if` keeps an all-default policy out of the written
    /// file — nothing is gained by stamping the defaults into every project.
    #[serde(default)]
    pub areas: AreaPolicy,

    /// Door policy for this project (see `OpeningPolicy`). Defaulted and skipped
    /// when empty, so a project file predating doors is unchanged on disk and
    /// unchanged in meaning. A table, so it is declared after the scalars for
    /// the TOML ordering reason `comparison_key` documents.
    #[serde(default)]
    pub doors: OpeningPolicy,

    /// Window policy for this project — the same `OpeningPolicy` under a second
    /// key, defaulted and skipped when empty so a project file predating windows
    /// is unchanged on disk and unchanged in meaning.
    ///
    /// **Its own section rather than sharing `[doors]`, and the reason is the
    /// one that made `[doors]` separate from the top-level room settings.** The
    /// values are per-entity vocabulary: `comparison_key` names a *window*
    /// property, and `room_reference_property` names the parameter a window
    /// family carries, which is a different family-and-office convention from
    /// the door one. One shared table would silently mean two things.
    ///
    /// **`room_attribution` keeps the same default and the argument is NOT
    /// inherited.** `to_room_then_from_room` reads as "the room it opens into",
    /// which a window does not do — it is glazing in a wall, not a passage. The
    /// chain is still right, for a different reason: it takes whichever side
    /// names a room and reports nothing when neither does, and for a window that
    /// is exactly "the room this is glazed into, else the one it faces from".
    /// The default survives because the mechanism fits, not because doors had it.
    ///
    /// **`room_resolution` matters far more here than it does for doors**, and
    /// the measurement that decided it is worth carrying: a facade file held 158
    /// windows and 191 doors and not one room, because it links its interiors
    /// rather than containing them. `FromRoom[phase]` cannot see into a link, so
    /// every opening in such a model is unattributable from authored data alone.
    /// Left `Off`, a windows read of that project returns 158 homeless windows
    /// and no way to tell that from a modelling failure.
    #[serde(default)]
    pub windows: OpeningPolicy,

    /// FF&E policy for this project (see `FfePolicy`). Defaulted and skipped
    /// when empty, so a project file predating FF&E is unchanged on disk and
    /// unchanged in meaning.
    ///
    /// **Its own type rather than a third `OpeningPolicy`, and the reason is
    /// the one that made `Item` a sibling of `Opening` rather than a widening
    /// of it.** `OpeningPolicy` carries `room_attribution`, which chooses
    /// between a `from_room` and a `to_room`. An item sits IN one room -- there
    /// is no choice to make, and a field offering five ways to make it would be
    /// right in shape and meaningless in fact. What FF&E adds instead is
    /// `nested_components`, which no opening needs because a door leaf is never
    /// a door.
    ///
    /// What the two DO share -- `comparison_key`, `room_resolution`,
    /// `room_reference_property`, `comparison_properties` -- is shared by
    /// spelling the fields the same way and documenting them against the same
    /// rules, not by sharing a struct that would then have to carry both
    /// entities' exceptions.
    #[serde(default)]
    pub ffe: FfePolicy,

    /// One project's SPACES policy: how a space is matched to a room, and which
    /// properties are compared once it is.
    ///
    /// **Its own section for the reason every entity's is, plus one that is
    /// this entity's alone.** The others compare an entity against *itself*
    /// across time, so one property name suffices. This compares two different
    /// vocabularies -- a space's and a room's -- which is why
    /// `compared_properties` holds pairs rather than names, and why it could
    /// not have reused `comparison_properties` however similar it looks.
    #[serde(default)]
    pub spaces: SpacePolicy,

    /// Where a route's size checks read their data from (see `RoutingPolicy`).
    /// Defaulted and skipped when untouched, so a project file predating it is
    /// unchanged on disk and unchanged in meaning.
    #[serde(default)]
    pub routing: RoutingPolicy,

    /// Ordered classification tiers, outermost first. Empty if the section is
    /// omitted (a project with no classification defined).
    #[serde(default)]
    pub hierarchy: Vec<HierarchyTier>,

    /// Canonical property names, each resolved to a source-specific raw
    /// property name. Lets a project retarget which raw property backs a
    /// canonical concept (e.g. "Area") without a Rust code change — the seam
    /// that matters once a second data source (e.g. IFC) can produce rooms
    /// alongside Revit, since the same canonical concept lives under a
    /// different raw name per source. Empty if the section is omitted, in
    /// which case `lookup_property` matches names verbatim (today's
    /// single-source behaviour).
    #[serde(default)]
    pub builtin_properties: Vec<BuiltinPropertyDef>,

    /// Ordered list of property names shown on a room's label in the viewer.
    /// `"$name"` / `"$id"` are intrinsic tokens referring to the room's own
    /// `name`/`id` fields (not resolvable via `lookup_property`, which only
    /// reads `room.properties`); anything else is a canonical property name
    /// resolved the same way dRofus/classification already are. Defaults to
    /// `["$name", "$id"]` — today's label — so omitting this section changes
    /// nothing. No startup validation: an unresolvable name just contributes
    /// nothing to that room's label, same "absence is fine" discipline as
    /// everywhere else here.
    #[serde(default = "default_room_label")]
    pub room_label: Vec<String>,

    /// User-defined milestones: named dates with data snapshots explicitly
    /// pinned to them, so the viewer can show the project as captured at a
    /// milestone instead of each model's latest push. Lives in settings (not
    /// storage) deliberately: milestones are per-project user-authored
    /// metadata with the same lifecycle as hierarchy/room_label, and riding
    /// this file buys the whole save pipeline — validation, atomic install,
    /// hot-reload — for free. Empty if omitted.
    #[serde(default)]
    pub milestones: Vec<Milestone>,

    /// User-authored colour plans for the viewer: named, persisted colouring
    /// configs the user switches between. Lives in settings (not storage) for
    /// the same reason `milestones` does — per-project user metadata with the
    /// same lifecycle as hierarchy/room_label, riding this file's save
    /// pipeline (validation, atomic install, hot-reload) for free.
    ///
    /// The server treats this as **opaque**: it stores and serves it verbatim
    /// and computes no colours. ALL colour math is client-side, where room
    /// property values already live — the same "keep axum a pure JSON API"
    /// decision that kept CSV export and QA rendering out of the server
    /// (see STRATEGY-BROWSER.md). A `Vec` (not a single plan) so a project can
    /// keep a library of plans; `ColourPlan.active` marks the one the viewer's
    /// colour picker defaults to (the picker's "None (flat)" always overrides,
    /// so `active` is a default, not a forced application). Empty if omitted —
    /// no plans, today's flat fill. The `#[serde(default)]` is the back-compat
    /// net: every already-saved project file (which has no `colour_plans` key)
    /// still deserializes to an empty `Vec`.
    #[serde(default)]
    pub colour_plans: Vec<ColourPlan>,

    /// Footprint exclusions for the hierarchy-areas feature (`service::areas`):
    /// rooms or whole groups withheld from the aggregated footprints. Empty (the
    /// default, and every file predating this feature) means nothing is excluded.
    /// `skip_serializing_if` so an empty list emits nothing — a trailing
    /// `hierarchy_exclusions = []` after the colour-plan tables would trip the
    /// TOML "value-after-table" ordering footgun (CODING-CONVENTIONS.md).
    #[serde(default)]
    pub hierarchy_exclusions: Vec<HierarchyExclusion>,

    /// Which model's coordinate space this project's plans are drawn in — the
    /// anchor every linked model is placed against (`service::placement`).
    ///
    /// **An override, not a requirement.** Absent (the default, and every file
    /// predating it) the anchor is derived: the lexicographically smallest model
    /// id that declares a placement. That derivation is deterministic and
    /// request-independent, which is all *correctness* needs — every model is
    /// mapped through the same rigid `anchor⁻¹`, so which one is chosen cannot
    /// change how the models sit relative to each other. It only sets where the
    /// whole plan's origin and north are, and the viewer refits either way.
    ///
    /// It exists because two things do leak out of that choice: **exported
    /// coordinates** (SVG, `/areas`) come out in the anchor's frame, and "the
    /// car park, because C sorts first" is a poor frame to compare against
    /// Revit in; and the derived anchor **moves** if a model with a
    /// lower-sorting id is ever pushed, which silently shifts every exported
    /// coordinate. Naming the model pins both.
    ///
    /// A name that matches no model with a placement is a **warning, not an
    /// error** — the derived anchor answers instead, on the "signal, not error"
    /// rule. A project is drawable with a stale anchor name; refusing to serve
    /// it would not make the name any more correct.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_model: Option<String>,

    /// The name of the FF&E TYPE parameter whose value identifies a kind of item for tag
    /// blueprints (plan decision 27): every instance of a type shares it, so a tag learned from
    /// one room's item is found for the same type in another, whatever the family and type are
    /// called. **Absent means no FF&E tag can be learned or applied**: the extractor refuses to
    /// send tag observations and an RLS task carries no FF&E tag rules, each saying why. A rule
    /// records the parameter it was keyed by, so changing this leaves the old rules unused
    /// rather than wrong.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_type_code: Option<String>,

    /// How this project's plan is COLOURED, per entity (see `Appearance`).
    /// Defaulted and skipped when empty, so a project file predating it is
    /// unchanged on disk and unchanged in meaning. A table, so it is declared
    /// after every scalar for the TOML ordering reason `comparison_key`
    /// documents.
    #[serde(default)]
    pub appearance: Appearance,

    /// What a hover over the plan shows, per entity (see `HoverProperties`).
    /// A table, so it is declared after every scalar for the TOML ordering
    /// reason `comparison_key` documents.
    #[serde(default)]
    pub hover: HoverProperties,

    /// What the text written ON an element says, and in which colours (see
    /// `ElementTags`). A table, so it is declared after every scalar for the
    /// TOML ordering reason `comparison_key` documents.
    #[serde(default)]
    pub tags: ElementTags,
}

fn default_room_label() -> Vec<String> {
    vec!["$name".to_string(), "$id".to_string()]
}

impl Settings {
    /// Every setting that has a default, set to it where the file leaves it unset, so
    /// a written project file lists everything it is running on.
    ///
    /// **Written, not just read.** An absent key means "the default" only for as long
    /// as the code's default never changes, and a person reading the file cannot tell
    /// what that is. The cost is knowing it: a project written by this holds the
    /// default of the day, and a later change to a default does not reach it. Settings
    /// whose default is *none* (a comparison key, the anchor, a hover property) stay
    /// absent, because absent is their value -- TOML has no null to write.
    pub fn fill_defaults(&mut self) {
        let r = &mut self.routing;
        r.room_height_property
            .get_or_insert_with(|| RoutingPolicy::DEFAULT_ROOM_HEIGHT_PROPERTY.to_string());
        r.door_frame_allowance_mm.get_or_insert(RoutingPolicy::DEFAULT_DOOR_FRAME_ALLOWANCE_MM);
        if r.door_height_properties.is_empty() {
            r.door_height_properties =
                RoutingPolicy::DEFAULT_DOOR_HEIGHT_PROPERTIES.iter().map(|p| p.to_string()).collect();
        }
    }
}

/// Server-wide settings, parsed once at startup from the `--server-settings`
/// file — separate from per-project `Settings` because storage and dev
/// seeding are properties of the running server, not of any one project.
#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    /// Where model snapshots are persisted on disk. When present, pushes are
    /// written under this root (project-guid/model-guid/snapshot.json) and
    /// survive restarts. When absent, storage stays purely in-memory (dev/test).
    #[serde(default)]
    pub storage: Option<Storage>,

    /// Dev-only: when present, seeds the server with a snapshot from disk at
    /// startup so no manual POST is needed. Omit in prod.
    #[serde(default)]
    pub test_data: Option<TestData>,
}

/// On-disk snapshot storage config. Its own section (not under `[sources]`):
/// a source *supplies* join data, storage *persists* the snapshots themselves —
/// different kind of thing. Kept as an `Option` on `ServerConfig` so omitting
/// it is a clean fallback to the in-memory store, no other change.
#[derive(Debug, Deserialize)]
pub struct Storage {
    /// Root directory holding one sub-dir per project (named by project GUID).
    /// Created on first push if missing; must be writable.
    pub root: PathBuf,
}

/// External reference sources joined onto the Revit snapshot, keyed by name —
/// the key IS the join namespace a `/rooms` filter or `comparison_key` writes
/// before the dot (`drofus.NetArea`; see `service::rooms::split_namespace`).
/// Every source is optional: an empty map is legal config (a project not
/// using any reference data is normal), and an absent source degrades to "not
/// configured" downstream (e.g. `ValidationResponse.drofus_configured:
/// false`), never an error.
///
/// **Entity-scoped since R4.** Each source declares which primary entity it
/// joins onto via `ReferenceSourceConfig::entity`, defaulting to `rooms` so
/// every settings file predating the field is unchanged and still means what it
/// meant. Before that, `[sources.reference.<name>]` meant "for rooms" with
/// nothing saying so, and a source configured for anything else parsed, loaded
/// and joined nowhere.
#[derive(Debug, Default, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct Sources {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reference: BTreeMap<String, ReferenceSourceConfig>,
}

/// Dev-only seed data. Kept separate from `drofus` so removing this test seam
/// later is a one-section deletion with no other changes.
#[derive(Debug, Deserialize)]
pub struct TestData {
    /// Path to a pre-exported snapshot (same JSON shape a POST sends).
    pub snapshot_path: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A HierarchyTier with neither property fails validation.
    #[test]
    fn test_unkeyable_tier_fails_validation() {
        let tier = HierarchyTier { name: "Ghost".to_string(), code_property: None, name_property: None };
        assert!(tier.validate().is_err());
    }

    fn milestone(name: &str, date: &str) -> Milestone {
        Milestone {
            space_attachments: BTreeMap::new(),
            ceiling_attachments: BTreeMap::new(),
            floor_attachments: BTreeMap::new(),
            name: name.to_string(),
            date: date.to_string(),
            reference_snapshots: Default::default(),
            attachments: Default::default(),
            door_attachments: Default::default(),
            window_attachments: Default::default(),
            ffe_attachments: Default::default(),
        }
    }

    /// A milestone's own checks: empty name, unparseable date, and an
    /// attachment whose snapshot id isn't an RFC3339 UTC date-time all fail;
    /// both accepted date shapes pass.
    #[test]
    fn test_milestone_validate() {
        assert!(milestone("Design Freeze", "2026-06-30").validate().is_ok());
        assert!(milestone("Design Freeze", "2026-06-30T10:00:00Z").validate().is_ok());

        assert!(milestone("  ", "2026-06-30").validate().is_err(), "empty name");
        assert!(milestone("M", "sometime in June").validate().is_err(), "bad date");

        let mut bad_pin = milestone("M", "2026-06-30");
        bad_pin.attachments.insert("model-1".to_string(), "not-a-snapshot-id".to_string());
        assert!(bad_pin.validate().is_err(), "attachment id must be a valid snapshot id");

        let mut good_pin = milestone("M", "2026-06-30");
        good_pin
            .attachments
            .insert("model-1".to_string(), "2026-06-29T10:00:00.123456Z".to_string());
        assert!(good_pin.validate().is_ok());

        // A reference-source pin follows the same snapshot-id rule as an attachment.
        let mut bad_drofus = milestone("M", "2026-06-30");
        bad_drofus
            .reference_snapshots
            .insert("drofus".to_string(), "not-a-snapshot-id".to_string());
        assert!(bad_drofus.validate().is_err(), "reference_snapshots entry must be a valid snapshot id");

        let mut good_drofus = milestone("M", "2026-06-30");
        good_drofus
            .reference_snapshots
            .insert("drofus".to_string(), "2026-06-29T17:00:00Z".to_string());
        assert!(good_drofus.validate().is_ok());
    }

    /// The default policy is exactly today's behaviour: the 1.5 ft ceiling both
    /// geometry services previously hardcoded, no declared standard, and no
    /// regime fallback — so an un-migrated project file changes nothing.
    #[test]
    fn test_area_policy_default_preserves_todays_behaviour() {
        let policy = AreaPolicy::default();
        assert_eq!(policy.max_wall_thickness, AreaPolicy::DEFAULT_MAX_WALL_THICKNESS_FT);
        assert_eq!(policy.measurement_standard, None);
        assert_eq!(policy.boundary_location, None);
        assert!(policy.validate().is_ok());
        assert!(policy.is_default(), "an untouched policy writes no [areas] block");
    }

    /// A thickness that cannot describe a wall fails the boot with a message
    /// naming the problem. Zero gets its own wording: it is the mistake a reader
    /// is most likely to make (reaching for it to mean "centreline"), and the
    /// message has to say where that actually gets declared.
    #[test]
    fn test_area_policy_rejects_unusable_thickness() {
        let with = |t: f64| AreaPolicy { max_wall_thickness: t, ..Default::default() };

        let msg = format!("{:#}", with(0.0).validate().unwrap_err());
        assert!(msg.contains("positive"), "{msg}");
        assert!(msg.contains("boundary regime"), "points at the right knob: {msg}");

        assert!(with(-1.0).validate().is_err(), "negative");
        assert!(with(f64::NAN).validate().is_err(), "non-finite");
        assert!(with(f64::INFINITY).validate().is_err(), "non-finite");

        let msg = format!("{:#}", with(6.0).validate().unwrap_err());
        assert!(msg.contains("limit"), "{msg}");

        // The band is closed at the limit itself, and an ordinary value passes.
        assert!(with(AreaPolicy::MAX_WALL_THICKNESS_LIMIT_FT).validate().is_ok());
        assert!(with(0.5).validate().is_ok());
    }

    /// Regime resolution is three levels, most authoritative first: the model's
    /// own declaration beats the project fallback, which beats finish face. The
    /// first of those is the load-bearing one — a project fallback that could
    /// override a model that states its own regime would defeat the declaration.
    #[test]
    fn test_area_policy_resolve_boundary_precedence() {
        use crate::contract::RoomBoundary::{Centreline, FinishFace};

        let no_fallback = AreaPolicy::default();
        assert_eq!(no_fallback.resolve_boundary(None), FinishFace, "conservative default");
        assert_eq!(no_fallback.resolve_boundary(Some(Centreline)), Centreline);

        let fallback_centreline = AreaPolicy { boundary_location: Some(Centreline), ..Default::default() };
        assert_eq!(fallback_centreline.resolve_boundary(None), Centreline, "fallback applies when undeclared");
        assert_eq!(
            fallback_centreline.resolve_boundary(Some(FinishFace)),
            FinishFace,
            "the model always wins over the project's guess"
        );
    }

    /// The gap a regime implies: exactly zero for centreline (nothing to
    /// bridge — this is what deletes the artifact class, so it must not be a
    /// small number), the declared thickness for finish face.
    #[test]
    fn test_area_policy_wall_gap_per_regime() {
        use crate::contract::RoomBoundary::{Centreline, FinishFace};
        let policy = AreaPolicy { max_wall_thickness: 0.4, ..Default::default() };
        assert_eq!(policy.wall_gap_ft(Centreline), 0.0);
        assert_eq!(policy.wall_gap_ft(FinishFace), 0.4);
    }

    /// The `[areas]` block round-trips through TOML in a position TOML accepts
    /// — the serialize-side footgun (CODING-CONVENTIONS): a table emitted in
    /// the wrong place swallows the scalars that follow it.
    #[test]
    fn test_area_policy_round_trips_through_toml() {
        let toml_in = r#"
project_id = "p1"

[areas]
measurement_standard = "IPMS3"
max_wall_thickness = 0.5
boundary_location = "finish_face"
"#;
        let settings: Settings = toml::from_str(toml_in).unwrap();
        assert_eq!(settings.areas.measurement_standard, Some(MeasurementStandard::Ipms3));
        assert_eq!(settings.areas.max_wall_thickness, 0.5);
        assert_eq!(settings.areas.boundary_location, Some(crate::contract::RoomBoundary::FinishFace));

        let written = toml::to_string_pretty(&settings).unwrap();
        let reparsed: Settings = toml::from_str(&written).unwrap();
        assert_eq!(reparsed.areas, settings.areas, "survives a write→read cycle");
        assert_eq!(reparsed.project_id, "p1", "the table did not swallow a scalar");
    }

    /// An unrecognised standard is rejected at parse time, and the error names
    /// the accepted spellings — the whole point of declaring a standard is that
    /// the reader knows what the number means, so `"IMPS3"` must not be carried
    /// silently. No hand-rolled check: serde's own message is the specific one.
    #[test]
    fn test_unknown_measurement_standard_is_rejected() {
        let err = toml::from_str::<Settings>("project_id = \"p1\"\n\n[areas]\nmeasurement_standard = \"IMPS3\"\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("IPMS3"), "the message names the accepted spellings: {err}");
    }

    /// **A reference source with no `entity` still means rooms**, which is what
    /// makes R4 a non-event for every settings file written before it.
    #[test]
    fn test_reference_source_entity_defaults_to_rooms() {
        let settings: Settings =
            toml::from_str("project_id = \"p1\"\n\n[sources.reference.drofus]\ntype = \"upload\"\n").expect("parses");
        assert_eq!(settings.sources.reference["drofus"].entity, ReferenceEntity::Rooms);
    }

    #[test]
    fn test_reference_source_entity_accepts_doors() {
        let settings: Settings = toml::from_str(
            "project_id = \"p1\"\n\n[sources.reference.schedule]\ntype = \"upload\"\nentity = \"doors\"\n",
        )
        .expect("parses");
        assert_eq!(settings.sources.reference["schedule"].entity, ReferenceEntity::Doors);
    }

    /// **An unknown entity is a loud failure naming it**, not a silent fall back
    /// to rooms. That is the whole point of the field: before it,
    /// `[sources.reference.doors]` parsed, loaded and joined nowhere, and the
    /// only symptom was a source that never matched anything. Serde's own
    /// **`entity = "spaces"` is accepted**, which is the whole extension point
    /// R4 built: a fifth entity joins the reference model as one settings line,
    /// not a design. A services model's spaces can carry their own dRofus-style
    /// source, verified independently of the rooms one.
    #[test]
    fn test_reference_source_entity_accepts_spaces() {
        let settings: Settings = toml::from_str(
            "project_id = \"p1\"

[sources.reference.schedule]
type = \"upload\"
entity = \"spaces\"
",
        )
        .expect("spaces is an accepted entity");
        assert_eq!(settings.sources.reference["schedule"].entity, ReferenceEntity::Spaces);
    }

    /// message is the specific one, so there is no hand-rolled check.
    #[test]
    fn test_reference_source_entity_accepts_windows() {
        let settings: Settings = toml::from_str(
            "project_id = \"p1\"\n\n[sources.reference.schedule]\ntype = \"upload\"\nentity = \"windows\"\n",
        )
        .expect("parses");
        assert_eq!(settings.sources.reference["schedule"].entity, ReferenceEntity::Windows);
    }

    /// **An unknown entity is a loud failure naming it**, not a silent fall back
    /// to rooms. That is the whole point of the field: before it,
    /// `[sources.reference.doors]` parsed, loaded and joined nowhere, and the
    /// only symptom was a source that never matched anything. Serde's own
    /// message is the specific one, so there is no hand-rolled check.
    ///
    /// **The example has now been wrong twice, and the third choice is made on
    /// a different principle.** It proved its point with `entity = "windows"`
    /// until windows shipped, then with `entity = "ffe"` — chosen, in as many
    /// words, because "`ffe` is the next candidate entity and the same trap is
    /// set for whoever builds it". It was, and this test caught them: it failed
    /// the moment `ReferenceEntity::Ffe` landed, which is the behaviour wanted
    /// and the reason it is worth keeping.
    ///
    /// So the example is deliberately no longer a *plausible* entity. Picking
    /// the next candidate guarantees the test breaks again on the day that
    /// candidate ships, and a test that must be edited every time the thing it
    /// guards grows is one people learn to edit without reading. The point being
    /// made is about the mechanism — serde rejects an unknown variant and names
    /// the accepted ones — and the mechanism does not care whether the rejected
    /// spelling is a near miss.
    #[test]
    fn test_unknown_reference_entity_is_rejected() {
        let err = toml::from_str::<Settings>(
            "project_id = \"p1\"\n\n[sources.reference.s]\ntype = \"upload\"\nentity = \"sprockets\"\n",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("sprockets"), "names what was written: {err}");
        assert!(
            err.contains("rooms") && err.contains("doors") && err.contains("windows") && err.contains("ffe"),
            "names every accepted value, ffe included: {err}"
        );
    }

    /// The positive half, added because the negative test above stopped
    /// covering it. A source scoped to FF&E parses and lands as `Ffe` rather
    /// than falling back to the `Rooms` default — which is the failure this
    /// pair exists for: an entity-scoped source that silently joins the wrong
    /// entity looks configured and matches nothing.
    #[test]
    fn test_a_reference_source_can_be_scoped_to_ffe() {
        let settings: Settings = toml::from_str(
            "project_id = \"p1\"\n\n[sources.reference.schedule]\ntype = \"upload\"\nentity = \"ffe\"\n",
        )
        .expect("ffe is an accepted entity");
        assert_eq!(settings.sources.reference["schedule"].entity, ReferenceEntity::Ffe);
        assert_eq!(ReferenceEntity::Ffe.as_str(), "ffe");
    }

    /// `[ffe]` is defaulted on READ, so a project file written before FF&E existed
    /// still loads; and it is WRITTEN too, like every section, so a saved file
    /// lists what it runs on.
    #[test]
    fn test_an_untouched_ffe_section_is_still_written() {
        let settings: Settings = toml::from_str("project_id = \"p1\"\n").expect("parses with no [ffe]");
        assert!(settings.ffe.is_default());
        let text = toml::to_string_pretty(&settings).expect("serializes");
        assert!(text.contains("[ffe]"), "a default section is written: {text}");
    }

    /// A configured `[ffe]` round-trips, including the one field no opening has.
    #[test]
    fn test_ffe_policy_round_trips() {
        let settings: Settings = toml::from_str(
            "project_id = \"p1\"\n\n[ffe]\ncomparison_key = \"Mark\"\nnested_components = \"include\"\nroom_resolution = \"same_model\"\n",
        )
        .expect("parses");
        assert_eq!(settings.ffe.comparison_key.as_deref(), Some("Mark"));
        assert_eq!(settings.ffe.nested_components, NestedComponents::Include);
        assert_eq!(settings.ffe.room_resolution, RoomResolution::SameModel);
        assert!(!settings.ffe.is_default());
    }

    /// Excluding components is the DEFAULT, and it is a measurement rather than
    /// a preference — 179 of 647 House A instances had a super-component and
    /// every population in that set was hardware or sub-assembly. Asserted so a
    /// future edit has to argue with the number rather than with a taste.
    #[test]
    fn test_components_are_excluded_by_default() {
        assert_eq!(FfePolicy::default().nested_components, NestedComponents::Exclude);
    }

    #[test]
    fn test_an_untouched_routing_policy_is_default_and_fills_on_write() {
        let policy = RoutingPolicy::default();
        assert!(policy.is_default());
        assert_eq!(policy.room_height_property(), "Ceiling Height");
        assert_eq!(policy.door_height_properties(), vec!["Height".to_string(), "Door Height".to_string()]);
        assert_eq!(policy.frame_allowance_mm(), 150.0);
        assert!(
            policy.door_height_from_type_name,
            "on by default: nearly all real door types follow the convention"
        );
        assert!(
            policy.room_height_from_ceilings,
            "on by default: a ceiling is where the ceiling really is"
        );
        let settings: Settings = toml::from_str("project_id = \"p1\"\n").expect("parses");
        assert!(settings.routing.is_default());
        assert!(toml::to_string(&settings).unwrap().contains("[routing]"));
        let mut filled = settings;
        filled.fill_defaults();
        assert_eq!(filled.routing.room_height_property.as_deref(), Some("Ceiling Height"));
        assert_eq!(filled.routing.door_frame_allowance_mm, Some(150.0));
        assert_eq!(
            filled.routing.door_height_properties,
            vec!["Height".to_string(), "Door Height".to_string()]
        );
        assert_eq!(filled.routing.room_height_property(), "Ceiling Height", "filling changes no answer");
    }

    #[test]
    fn test_a_routing_policy_round_trips_through_toml() {
        let settings: Settings = toml::from_str(
            "project_id = \"p1\"\n\n[routing]\nroom_height_property = \"Room Height\"\ndoor_height_from_type_name = false\n\
             door_frame_allowance_mm = 120.0\ndoor_height_properties = [\"Rough Height\", \"Height\"]\n",
        )
        .expect("parses");
        let r = &settings.routing;
        assert_eq!(r.room_height_property(), "Room Height");
        assert!(!r.door_height_from_type_name);
        assert_eq!(r.frame_allowance_mm(), 120.0);
        assert_eq!(r.door_height_properties(), vec!["Rough Height".to_string(), "Height".to_string()]);
        let again: Settings = toml::from_str(&toml::to_string(&settings).unwrap()).expect("re-parses");
        assert_eq!(again.routing, settings.routing);
    }

    #[test]
    fn test_a_routing_policy_that_could_never_match_or_fit_fails_validation() {
        assert!(RoutingPolicy::default().validate().is_ok());
        let blank = RoutingPolicy { room_height_property: Some("  ".into()), ..Default::default() };
        assert!(blank.validate().is_err());
        let blank_door =
            RoutingPolicy { door_height_properties: vec!["Height".into(), "".into()], ..Default::default() };
        assert!(blank_door.validate().is_err());
        for bad in [-1.0, f64::NAN, 5_000.0] {
            let p = RoutingPolicy { door_frame_allowance_mm: Some(bad), ..Default::default() };
            assert!(p.validate().is_err(), "{bad}");
        }
        let ok = RoutingPolicy { door_frame_allowance_mm: Some(0.0), ..Default::default() };
        assert!(ok.validate().is_ok(), "no frame at all is a legitimate answer");
    }
}
