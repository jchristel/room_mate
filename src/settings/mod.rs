//! Startup configuration: the TOML settings file and everything parsed from it.
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
//! Split across three files by concern, re-exported here so the public paths
//! (`crate::settings::Settings`, `::load_settings`, `::validate_reference_fields`,
//! …) never move:
//! - **this file** — the config/domain types and their inherent `validate()`
//!   methods (part of each type's own API);
//! - **`validate`** — the standalone validation *functions* over those types;
//! - **`load`** — the TOML loaders and settings-file-relative path resolution.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

mod load;
mod validate;

pub use load::{load_server_config, load_settings};
pub use validate::{validate_colour_plans, validate_reference_field_shapes, validate_reference_fields};

/// One project's settings, parsed once at startup from its own TOML file (one
/// of N files in the `--project-settings` directory). Server-wide config
/// (`[storage]`, `[test_data]`) lives separately in `ServerConfig`, loaded
/// once from `--server-settings` independent of this per-project loop.
///
/// Also derives `Serialize` (as do all the types it contains): the settings
/// API serves this exact shape as JSON and writes it back as TOML, so the
/// wire shape and the config-file shape can never drift.
#[derive(Debug, Deserialize, Serialize)]
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
    #[serde(default, skip_serializing_if = "AreaPolicy::is_default")]
    pub areas: AreaPolicy,

    /// Door policy for this project (see `OpeningPolicy`). Defaulted and skipped
    /// when empty, so a project file predating doors is unchanged on disk and
    /// unchanged in meaning. A table, so it is declared after the scalars for
    /// the TOML ordering reason `comparison_key` documents.
    #[serde(default, skip_serializing_if = "OpeningPolicy::is_default")]
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
    #[serde(default, skip_serializing_if = "OpeningPolicy::is_default")]
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
    #[serde(default, skip_serializing_if = "FfePolicy::is_default")]
    pub ffe: FfePolicy,

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hierarchy_exclusions: Vec<HierarchyExclusion>,
}

fn default_room_label() -> Vec<String> {
    vec!["$name".to_string(), "$id".to_string()]
}

/// A recognised area-measurement standard. Closed enum on purpose: the whole
/// point of declaring the standard is that a reader knows what the number
/// means, and a free-text field that accepts `"IMPS3"` defeats that silently.
/// An unknown value therefore fails the boot at TOML parse time, with serde
/// naming the accepted spellings — the "loud startup over silent no-op" rule.
///
/// The server **computes nothing** from this: it is carried and echoed on
/// `/areas` so an area figure never travels without its definition, which is
/// precisely what measurement standards exist to prevent. Adding a standard is
/// one variant here; the list is short because these are the ones a UK/EU
/// healthcare job actually cites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum MeasurementStandard {
    /// IPMS 1 — the whole building envelope, external walls included.
    #[serde(rename = "IPMS1")]
    Ipms1,
    /// IPMS 2 — measured to the internal dominant face, by component.
    #[serde(rename = "IPMS2")]
    Ipms2,
    /// IPMS 3 — exclusive occupier area, to the internal dominant face.
    #[serde(rename = "IPMS3")]
    Ipms3,
    /// DIN 277 — the German gross/net floor-area standard.
    #[serde(rename = "DIN277")]
    Din277,
    /// SIA 416 — the Swiss equivalent.
    #[serde(rename = "SIA416")]
    Sia416,
    /// BOMA — the North American office standard.
    #[serde(rename = "BOMA")]
    Boma,
    /// RICS *Code of Measuring Practice* (GIA/NIA family).
    #[serde(rename = "RICS")]
    Rics,
}

/// Area-measurement **policy**: the two facts about area aggregation that Revit
/// does not know and should not be asked.
///
/// The split against `contract::RoomBoundary` is the load-bearing part. Which
/// boundary regime a model was drawn to is a *model fact* Revit already holds,
/// so it rides the upload envelope. Which measurement standard applies is
/// **contractual**, and the width above which a gap stops being a wall and
/// becomes a void is a **project judgement** — neither is discoverable from the
/// model, so both live here. What the resulting number may be *called* is
/// STRATEGY-AREA-CALCULATION.md, "Relationship to measurement standards".
///
/// `max_wall_thickness` is deliberately **one quantity with two consumers**:
/// `service::areas` sizes its wall zone by it and `service::adjacency` uses it
/// as the default gap tolerance. Those were previously two separate constants
/// (`areas::MAX_WALL_FT` and `adjacency::WALL_MAX_FT`) holding the same physical
/// number in two modules — a live drift risk this type removes.
/// One project's **door** policy: how doors are matched and compared across
/// milestones.
///
/// Its own section rather than reusing the top-level `comparison_key` /
/// `comparison_properties`, because those name *room* properties. A door's
/// vocabulary is a different one that merely overlaps in spelling — `Mark` on a
/// door and `Mark` on a room are different properties that happen to share a
/// name — so one shared setting would silently mean two things. That is the
/// "which canonical property names exist at all" item on
/// [Entities](../../docs/STRATEGY-ENTITIES.md)'s "what does not generalize" list.
///
/// The settings that depend on *which* of a door's two rooms owns it —
/// `room_attribution` and `room_reference_property` — waited for that question
/// to be answered rather than shipping with a default that would have settled
/// it by accident. Both now live on `OpeningPolicy`.
/// How far the server may go to work out which rooms an element is between,
/// when the model does not say.
///
/// **Off by default, and that is a deliberate choice rather than caution.**
/// Turning it on changes `owner_rooms` for elements the model left unresolved —
/// homeless becomes attributed — so it must be something a project opts into,
/// not something an upgrade does on its behalf. `room_attribution` has a default
/// because the chain only ever uses what the model already states; this fills in
/// what the model does not.
///
/// **A derived answer never overrides an authored one**, at any setting. A
/// door's `to_room` is the modeller's assignment — what the door *serves*, which
/// is not always what it opens into — and geometry replacing it would be exactly
/// the reconciliation `CLAUDE.md` forbids. What the geometry does is fill an
/// absent side, and disagree audibly with a present one
/// (`OpeningReport::room_geometry_mismatches`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomResolution {
    /// Never derive. Absent references stay absent, and the geometric drift
    /// check does not run.
    #[default]
    Off,
    /// Resolve against the element's **own model's** rooms only. The setting for
    /// a model that holds its own rooms and simply failed to populate a
    /// reference.
    SameModel,
    /// Resolve against **every model in the project**, placing each one through
    /// its `model_to_shared` transform first. The setting for a split model —
    /// doors or windows in one file, rooms in another.
    ///
    /// **This is the first thing in the server to depend on `model_to_shared`
    /// being numerically correct.** The transform has been carried on the
    /// envelope since before any consumer existed, and nothing has ever checked
    /// it against a real survey: a model whose shared coordinates were never set
    /// up emits an identity transform, and two such models will be silently
    /// stacked on top of each other. Verify a project's placement before
    /// trusting this mode, and prefer `same_model` where it suffices.
    Project,
}

/// Which of a door's two room references attributes it to a room, for area
/// rollups, hierarchy scoping and door schedules. The attribution rule itself
/// is in `CLAUDE.md`.
///
/// **The default is a precedence chain, not a single pick**, and that is the
/// decision rather than a convenience. Each single pick leaves external doors
/// either mis-attributed or unattributed; the chain attributes every door that
/// has any room at all and leaves the rest *named* rather than silently absent.
///
/// **Trust it exactly as far as the model was drawn consistently.** Revit's
/// `FromRoom`/`ToRoom` follow the door instance's *orientation*, not the leaf
/// swing — flipping a door swaps them. So "the room it opens into" is a
/// modelling convention, which is why this is project policy with an override
/// rather than a rule in code, the same stance `measurement_standard` takes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomAttribution {
    /// The room the door opens *into*, falling back to the one it opens *from*.
    /// A door with neither is **homeless** — a reported state, not an error.
    #[default]
    ToRoomThenFromRoom,
    /// Strictly the room it opens into. A door with only a `from_room` is
    /// homeless under this policy even though it has a reference.
    ToRoom,
    /// Strictly the room it opens from.
    FromRoom,
    /// Both, so a door between two rooms is attributed twice. The reason
    /// `owner_rooms` is a list rather than an `Option`.
    Both,
    /// Never attribute a door to a room — every door is homeless, by choice.
    None,
}

impl RoomAttribution {
    /// The rooms this policy attributes one door to, in order, skipping absent
    /// references. Empty means homeless.
    ///
    /// The single place the chain is expressed, so the read path, the QA report
    /// and any future rollup cannot disagree about what a door belongs to.
    ///
    /// Generic over what a reference *is*, because it is now two things: a bare
    /// room id where the model stated one, and a model-qualified reference where
    /// the geometry derived one (a probe can reach a room in a linked model, and
    /// a bare id would be resolved against the wrong model's rooms). The policy
    /// is about presence and order, never about the payload, so making it
    /// generic keeps one chain rather than a second copy that could drift.
    pub fn owners<'a, T: ?Sized>(&self, from_room: Option<&'a T>, to_room: Option<&'a T>) -> Vec<&'a T> {
        match self {
            RoomAttribution::ToRoomThenFromRoom => to_room.or(from_room).into_iter().collect(),
            RoomAttribution::ToRoom => to_room.into_iter().collect(),
            RoomAttribution::FromRoom => from_room.into_iter().collect(),
            RoomAttribution::Both => [to_room, from_room].into_iter().flatten().collect(),
            RoomAttribution::None => vec![],
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct OpeningPolicy {
    /// The door property whose value identifies "the same door" across
    /// milestones — the door counterpart of `Settings::comparison_key`, and
    /// `None` (the default) is a real, reachable state that reports "not
    /// configured" rather than falling back to anything.
    ///
    /// `"$id"` is usually the right answer, and is worth stating because it is
    /// *not* the right answer for rooms: a door's ElementId is stable across
    /// pushes of the same model, so it identifies the same physical leaf, while
    /// a room comparison keys on an authored Number precisely because room ids
    /// are not meaningful to the people reading the diff. There is still no
    /// default — a project that renumbers doors between milestones wants `Mark`,
    /// and guessing wrong produces a diff that looks authoritative and is wrong.
    ///
    /// A scalar declared before `comparison_properties` per the TOML ordering
    /// rule in CODING-CONVENTIONS.md.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison_key: Option<String>,

    /// Which room a door belongs to (see `RoomAttribution`).
    ///
    /// **This has a default, reversing what the design originally proposed.**
    /// That sketch argued for no default, on the grounds that an unset value
    /// meaning "do not attribute" is honest rather than arbitrary. The argument
    /// was sound for a *single pick* — choosing `to_room` on a project's behalf
    /// would silently mis-attribute every external door. It stops holding for
    /// the chain, which attributes only what the model actually states and names
    /// the rest; and the attribution is derived at read time, never stored, so a
    /// project that disagrees changes one line and loses nothing.
    #[serde(default)]
    pub room_attribution: RoomAttribution,

    /// How far the server may go to work out a door's rooms from its geometry
    /// (see `RoomResolution`). Off by default.
    ///
    /// Declared after `room_attribution` because it feeds it: a derived
    /// reference fills an absent side, and attribution then runs over the
    /// result. It never replaces a reference the model states.
    #[serde(default)]
    pub room_resolution: RoomResolution,

    /// The door property carrying an **authored** room reference (in the sample
    /// export, `"Door Room Reference"`), reconciled against the room
    /// `room_attribution` actually picked. Absent — the default — disables the
    /// check.
    ///
    /// **A property name rather than the `reconcile_room_reference = true` the
    /// design first sketched.** A bool would have needed the property name
    /// hard-coded, and which parameter carries this is a family-and-office
    /// convention, not a fact about doors. One field states both that the check
    /// is wanted and what it reads.
    ///
    /// It is worth turning on: on the House A sample the authored value
    /// disagrees with the attributed room on 4 of 26 doors, three of them where
    /// the geometry picks an exterior or circulation space over the served room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_reference_property: Option<String>,

    /// Ordered door property names compared across milestones, resolved in the
    /// same vocabulary the `/doors` filter uses — so `$to_room` and `$from_room`
    /// are comparable, which is how "this door moved between rooms" becomes a
    /// reported difference with no machinery of its own.
    #[serde(default)]
    pub comparison_properties: Vec<String>,
}

impl OpeningPolicy {
    /// Whether this policy is entirely unset — used to keep an untouched
    /// `[doors]` section out of a written settings file, same as `AreaPolicy`.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Whether a family instance that is a *component of another instance* counts
/// as an item.
///
/// **The one place FF&E deliberately does not follow doors**, and the argument
/// is about what kind of question it is. `nested_opening_ids` drops a door leaf
/// at the producer and is right to: "is this door leaf a door" has one answer,
/// always, everywhere. "Is this component an item" does not -- a joinery handle
/// is not, a chair nested in a workstation group might be -- so it is a project
/// convention, and conventions live here and are applied at read time.
///
/// That also makes the exclusion *visible*. 2236 of 4134 exported "doors" on one
/// job were hardware, and they were invisible because the producer had already
/// dropped them by the time anyone could count them. `ItemReport` states how
/// many this policy excluded, on every read, from the first push.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NestedComponents {
    /// A component is not an item. **The default, and it is a measurement
    /// rather than a preference**: on House A, 179 of 647 instances had a
    /// super-component and every population in that set was hardware or
    /// sub-assembly -- 87 joinery handles of one family, 70 generic models
    /// inside electrical fixtures, 3 generic models inside doors.
    #[default]
    Exclude,
    /// Every instance is an item, components included. For a project that nests
    /// genuinely schedulable FF&E -- a workstation group whose chair is a real
    /// line on the schedule.
    Include,
}

/// FF&E policy for one project.
///
/// Four of these fields are spelled exactly as `OpeningPolicy` spells them and
/// mean exactly what they mean there. They are not shared through a common
/// struct, because the two policies differ in both directions: an opening needs
/// `room_attribution` and an item has nothing to attribute, an item needs
/// `nested_components` and an opening has nothing to exclude. A shared struct
/// would carry both exceptions and force every reader to work out which half
/// applied.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct FfePolicy {
    /// The item property whose value identifies "the same item" across
    /// milestones, and `None` (the default) reports "not configured" rather
    /// than falling back to anything.
    ///
    /// **`"$id"` is a worse default here than it is for doors, which is why
    /// there is still no default at all.** A door's ElementId is stable across
    /// pushes and identifies the same physical leaf. FF&E is the thing that gets
    /// deleted and re-placed wholesale between issues -- a re-specified desk is
    /// a new ElementId and the same desk -- so a project comparing FF&E across
    /// milestones usually wants an authored `Mark` or an asset number, and
    /// guessing `$id` would produce a diff that looks authoritative and reports
    /// every replaced item as one deletion plus one addition.
    ///
    /// A scalar declared before `comparison_properties` per the TOML ordering
    /// rule in CODING-CONVENTIONS.md.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison_key: Option<String>,

    /// Whether a component of another instance counts as an item (see
    /// `NestedComponents`). Defaults to excluding them.
    #[serde(default)]
    pub nested_components: NestedComponents,

    /// How far the server may go to work out an item's room from its geometry
    /// (see `RoomResolution`). Off by default.
    ///
    /// **Off is right here where it was arguable for windows**, and the reason
    /// is the premise of the entity rather than a preference. A facade model
    /// holds windows and no rooms, so `room_resolution` is the only mechanism by
    /// which any of its openings is ever attributed. FF&E lives in the same
    /// document as the rooms it serves -- that is *why* RoomMate performs this
    /// join, since Revit will not schedule it -- so authored references
    /// populate: 572 of 647 on House A. Geometry is a fallback for the
    /// remainder, taken deliberately, not the primary path.
    #[serde(default)]
    pub room_resolution: RoomResolution,

    /// The item property carrying an **authored** room reference, reconciled
    /// against the room the item actually names. Absent -- the default --
    /// disables the check.
    ///
    /// A property name rather than a bool, for the reason `[doors]` gives: which
    /// parameter carries this is a family-and-office convention, not a fact
    /// about furniture, and one field states both that the check is wanted and
    /// what it reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_reference_property: Option<String>,

    /// Ordered item property names compared across milestones, resolved in the
    /// same vocabulary the `/ffe` filter uses -- so `$room` and `$category` are
    /// comparable, which is how "this item moved between rooms" becomes a
    /// reported difference with no machinery of its own.
    #[serde(default)]
    pub comparison_properties: Vec<String>,
}

impl FfePolicy {
    /// Whether this policy is entirely unset -- used to keep an untouched
    /// `[ffe]` section out of a written settings file, same as `OpeningPolicy`.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AreaPolicy {
    /// What the reported area figure *means*. `None` (the default) is an honest
    /// "undeclared" and is echoed as such — not silently presented as any
    /// particular standard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurement_standard: Option<MeasurementStandard>,

    /// Feet. The widest gap between finish-face rooms still counted as a wall;
    /// anything wider is a genuine void (a courtyard, atrium, lightwell) and
    /// stays open. Must be positive and finite — it describes a real partition,
    /// and a zero-thickness wall is not a thing.
    ///
    /// **Zero is not how you say "centreline".** That is the *regime*, declared
    /// per model on the envelope, and it collapses the effective gap to zero
    /// (see `wall_gap_ft`). Conflating the two would make a project-wide policy
    /// value override a per-model fact — the exact mistake declaring the regime
    /// per model exists to prevent. `adjacency`'s `?wall_max=0` request parameter is a different
    /// thing again: a caller asking one question, not a project declaring policy.
    #[serde(default = "AreaPolicy::default_max_wall_thickness")]
    pub max_wall_thickness: f64,

    /// **Fallback only** for models whose extractor predates
    /// `contract::RoomBoundary` on the envelope. A declared envelope value
    /// always wins — this can never override a model that states its own regime,
    /// because the model is the authority. `None` (the default) falls through to
    /// finish face, which is the conservative reading: a close still runs, which
    /// is exactly today's behaviour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_location: Option<crate::contract::RoomBoundary>,
}

impl Default for AreaPolicy {
    fn default() -> Self {
        Self {
            measurement_standard: None,
            max_wall_thickness: Self::DEFAULT_MAX_WALL_THICKNESS_FT,
            boundary_location: None,
        }
    }
}

impl AreaPolicy {
    /// ~1.5 ft (~450mm): clears a thick partition without reaching across the
    /// narrowest real void. The value `areas::MAX_WALL_FT` and
    /// `adjacency::WALL_MAX_FT` both independently carried before they became
    /// one declared quantity, so an un-migrated project behaves exactly as it
    /// did.
    pub const DEFAULT_MAX_WALL_THICKNESS_FT: f64 = 1.5;

    /// Hard ceiling on the declared thickness, and on a requested
    /// `?wall_max=`. Beyond ~5 ft a "wall" spans a corridor, and the answer
    /// stops being about walls at all. Rejected loudly rather than clamped.
    pub const MAX_WALL_THICKNESS_LIMIT_FT: f64 = 5.0;

    fn default_max_wall_thickness() -> f64 {
        Self::DEFAULT_MAX_WALL_THICKNESS_FT
    }

    /// Whether this policy is entirely defaults — the `skip_serializing_if`
    /// predicate, so an untouched project file gains no `[areas]` block.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Startup-loud checks. A thickness that is non-finite, non-positive or
    /// absurd can never describe a wall, and left unchecked it would silently
    /// produce either no wall zone at all or one that swallows courtyards.
    pub fn validate(&self) -> anyhow::Result<()> {
        if !self.max_wall_thickness.is_finite() {
            anyhow::bail!("[areas] max_wall_thickness must be a finite number");
        }
        if self.max_wall_thickness <= 0.0 {
            anyhow::bail!(
                "[areas] max_wall_thickness must be positive (got {}); a centreline model is \
                 declared by its boundary regime, not by a zero wall thickness",
                self.max_wall_thickness
            );
        }
        if self.max_wall_thickness > Self::MAX_WALL_THICKNESS_LIMIT_FT {
            anyhow::bail!(
                "[areas] max_wall_thickness {} ft exceeds the {} ft limit; a gap that wide \
                 bridges rooms, not walls",
                self.max_wall_thickness,
                Self::MAX_WALL_THICKNESS_LIMIT_FT
            );
        }
        Ok(())
    }

    /// The regime in force for one model: what the model declared, else this
    /// project's fallback, else finish face. Three levels, most authoritative
    /// first — the model knows, the project guesses, the server assumes the
    /// case that still needs work done.
    pub fn resolve_boundary(&self, declared: Option<crate::contract::RoomBoundary>) -> crate::contract::RoomBoundary {
        declared.or(self.boundary_location).unwrap_or(crate::contract::RoomBoundary::FinishFace)
    }

    /// The gap this regime implies, in feet — the one number `areas` closes by
    /// and `adjacency` defaults to. Centreline rooms already touch, so there is
    /// nothing to bridge and nothing to fill: zero, not a small number. That
    /// zero is the whole payoff of declaring the regime — at radius zero the morphological
    /// close is the identity, so bevels, chamfers, spikes and sibling overlaps
    /// cannot arise at all rather than merely being smaller.
    pub fn wall_gap_ft(&self, boundary: crate::contract::RoomBoundary) -> f64 {
        match boundary {
            crate::contract::RoomBoundary::Centreline => 0.0,
            crate::contract::RoomBoundary::FinishFace => self.max_wall_thickness,
        }
    }
}

/// One reference-source column's declared type/format, and optionally a QA
/// override. `label` matches row 1 of the source's CSV (the same key
/// `DrofusData::reconciliation`/`all_labels` use) — one declaration per
/// column, not two separate lists, so "what is this column" is answered in
/// one place. `type` is read by any consumer that needs to know a column's
/// shape: QA's date comparison parses a `Date`-declared column's values with
/// the declared `format` and compares the parsed instants, so two renderings
/// of the same moment no longer count as a mismatch (numeric-adaptive
/// comparison still infers numeric-ness at compare time without needing a
/// declaration). `qa` is the QA-specific override: `Exact` forces string
/// comparison even when both sides parse as numbers or dates; `Ignore`
/// excludes the field from comparison *and* the coverage report entirely —
/// for a column that's mapped (present in the source's row 2) but expected to
/// always differ. Belongs to the `ReferenceSourceConfig` that declares it —
/// each source owns its own field list, since a second source's columns are
/// unrelated to the first's.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReferenceFieldConfig {
    pub label: String,

    /// What kind of data this column holds. Defaults to `String` (today's
    /// implicit treatment of every column) when omitted.
    #[serde(default, rename = "type")]
    pub field_type: FieldType,

    /// Required when `field_type` is `Date`: a chrono strftime-style pattern
    /// describing how this column's raw string is laid out -- dRofus dates
    /// arrive as formatted text (e.g. `"6/29/2026 5:01:01 PM +10:00"`), not a
    /// structured value, so a parser needs to be told the shape rather than
    /// guessing it. Meaningless for any other `field_type`. Dry-run-validated
    /// at startup (a typo like `%Q` fails loudly rather than silently never
    /// parsing anything at compare time).
    #[serde(default)]
    pub format: Option<String>,

    /// Optional second strftime pattern for the *Revit* side of a date
    /// comparison, when the room property renders dates differently from the
    /// dRofus column. Absent (the common case) means `format` is used for
    /// both sides. Only legal on a `Date` field, same as `format`. Exists
    /// because the two sources format independently -- no real snapshot with
    /// a date-bearing room property existed when this was added, so rather
    /// than guess Revit's shape, a project can declare it when it shows up.
    #[serde(default)]
    pub revit_format: Option<String>,

    /// Optional QA comparison override for this column. `None` (the default)
    /// keeps today's behavior: numeric-adaptive comparison if both sides
    /// parse as a number, else exact string match.
    #[serde(default)]
    pub qa: Option<CompareMode>,
}

/// The kind of data a reference-source column holds. Not a closed set forever
/// -- more variants join as consumers need them (e.g. a `Numeric { unit }`
/// case, once real unit conversion rather than adaptive rounding is needed).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    #[default]
    String,
    Numeric,
    Date,
}

/// How one dRofus field's value is compared against Revit's, when the
/// default (numeric-adaptive if both sides parse as a number, else exact
/// string match) needs overriding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CompareMode {
    /// Force exact string comparison even when both sides parse as numbers.
    Exact,
    /// Skip comparison and coverage reporting for this field entirely.
    Ignore,
}

// ---------- colour plans ----------
//
// Persisted, per-project room-colouring configs for the viewer. The server is
// deliberately *opaque* to all of this: it round-trips these types verbatim
// and never computes a colour (see `Settings::colour_plans`). The types live
// here only so they persist through the settings save pipeline; every field's
// *meaning* is a browser concern, resolved in `index.html`.

/// One named, persisted colouring configuration. `active` marks the plan the
/// viewer's colour picker defaults to — the picker also offers "None (flat)",
/// which always overrides, so `active` is a default selection, not a forced
/// application. `name` is user-facing only (the picker label). At most one plan
/// may be `active` (validated — see `validate_colour_plans`).
///
/// Scalar fields (`name`, `active`) are declared before `mode` (a sub-table)
/// so the TOML serializer emits them ahead of the `[colour_plans.mode]` table
/// — the same footgun `Milestone`'s field order (scalars before
/// `reference_snapshots`/`attachments`) guards against.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ColourPlan {
    /// User-facing label shown in the viewer's colour picker.
    pub name: String,
    /// Whether this plan is the viewer picker's default selection. `false`
    /// (the default) means the plan is in the library but not the default —
    /// the picker starts on "None (flat)" unless some plan sets this.
    #[serde(default)]
    pub active: bool,
    /// The colouring strategy. Internally tagged on `kind` so the wire shape is
    /// self-describing and the browser switches on one field — the same tagged
    /// representation `ReferenceOrigin` uses.
    pub mode: ColourMode,
}

/// The colouring strategies. Internally tagged on `kind` (like `ReferenceOrigin`'s
/// `type`), so the browser branches on `mode.kind`. Every variant is a *struct*
/// variant, not a newtype/tuple: internally-tagged serde enums can't carry a
/// newtype variant that wraps a sequence, and struct variants keep the JSON/TOML
/// shape flat and self-describing.
///
/// All three modes are wired end-to-end in the viewer (`colourForRoom` in
/// `index.html` — see STRATEGY-BROWSER.md); an authored plan whose values don't
/// resolve (an unparseable property, a ratio-by-zero, a value between bands, an
/// undefined tier) degrades to a "no data" grey rather than erroring.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ColourMode {
    /// Categorical hue per parent hierarchy tier, tint/shade per child tier.
    /// `tiers` names which hierarchy tiers participate, parent first (the
    /// browser reads each room's server-resolved `classification` path and
    /// matches by tier name); `scheme` names a bundled qualitative palette for
    /// the parent hues (child tint/shade is derived by lightening, no second
    /// scheme). One tier → hue only; a room whose parent tier is `undefined`
    /// renders "no data" grey.
    Hierarchy { tiers: Vec<String>, scheme: String },

    /// Colour by proximity of a date-typed `property` to `near_date`: nearest
    /// green, furthest red, a date after `near_date` blue. `property` is a
    /// canonical/room property name resolved browser-side the same way labels
    /// are; `scheme` names a bundled diverging palette. `format` is the
    /// strftime pattern the room's date strings are in — the *same* pattern the
    /// dRofus date column uses (Revit room dates originate from dRofus), so an
    /// author reuses that reference field's `format` rather than inventing one;
    /// omitted means the browser falls back to native ISO-8601 parsing, and an
    /// unparseable value just renders "no data" grey. Validated as a real
    /// strftime pattern at load when present (see `validate_colour_plans`).
    DateRange {
        property: String,
        near_date: String,
        scheme: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<String>,
    },

    /// Compare two room properties. `op` derives one number per room
    /// (difference or ratio of A and B); `colouring` maps that number to a
    /// colour. The two steps are kept deliberately separate: the number
    /// derivation is what a *future* `MilestoneCompare` mode (same property
    /// across two snapshots — current vs a `/rooms?milestone=`-pinned one)
    /// would swap out, reusing `Colouring` untouched. Property names that don't
    /// resolve on a given room aren't an error — that room just renders "no
    /// data" grey (the `room_label` "absence is fine" discipline).
    PropertyCompare {
        property_a: String,
        property_b: String,
        op: CompareOp,
        colouring: Colouring,
    },
}

/// How `PropertyCompare` reduces two property values to one number.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CompareOp {
    /// `A − B`. The natural choice for match (`|A−B| ≤ tol`) and a
    /// zero-centred diverging ramp.
    Diff,
    /// `A / B`. For proportional comparisons; the browser guards division by
    /// zero (→ "no data" grey), so no server-side check is needed.
    Ratio,
}

/// The number→colour step, factored *out* of `PropertyCompare` on purpose: it's
/// the reusable half, so a future mode that derives a per-room number
/// differently (e.g. `MilestoneCompare`) reuses these three styles without
/// change. Internally tagged on `style`; struct variants only (see
/// `ColourMode` for why).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "style", rename_all = "lowercase")]
pub enum Colouring {
    /// Two colours: within `tolerance` of zero (a match) vs. not. This is the
    /// dRofus-vs-Revit QA case. Reuses the *philosophy* of `CompareMode`'s
    /// numeric-adaptive comparison (both sides parse as numbers → numeric
    /// compare with tolerance, else exact) rather than inventing a second one —
    /// the actual comparison runs browser-side against string property values.
    Match { tolerance: f64 },
    /// Map the number onto a diverging palette centred on zero, auto-scaled to
    /// the level's data extent (computed per-render in the browser). `scheme`
    /// names a bundled diverging palette.
    Diverging { scheme: String },
    /// Map the number through user-defined cutoff→colour `bands`. A struct
    /// variant (`{ bands }`), not a newtype `Bands(Vec<Band>)`, because
    /// internally-tagged enums can't wrap a sequence in a newtype variant.
    Bands { bands: Vec<Band> },
}

/// One band of a `Bands` colouring: the half-open interval `[lo, hi)` gets
/// `colour`. `lo`/`hi` are `Option` so the first/last band can be open-ended
/// (`None` = −∞ / +∞). Bands are validated at load to be sorted and disjoint
/// (see `validate_colour_plans`), which is what lets the browser do a simple
/// ordered first-match scan with no overlap-resolution logic. A value that
/// falls in a *gap* between bands (allowed) renders as "no data" grey — a
/// deliberate gap, not a bug.
///
/// `colour` is a CSS colour string (e.g. `"#b4541f"`); the server never parses
/// it — validating colour syntax is a browser concern, and an unparseable one
/// just renders as the browser's fallback.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Band {
    /// Inclusive lower bound; `None` = open (−∞).
    #[serde(default)]
    pub lo: Option<f64>,
    /// Exclusive upper bound; `None` = open (+∞).
    #[serde(default)]
    pub hi: Option<f64>,
    /// CSS colour string applied to rooms whose value lands in `[lo, hi)`.
    pub colour: String,
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
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Sources {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reference: BTreeMap<String, ReferenceSourceConfig>,
}

/// One reference source's full configuration: where its data comes from
/// (`origin`) and its per-column type/QA declarations (`fields`). `origin` is
/// `#[serde(flatten)]`ed so its `type` tag sits at the same TOML level as
/// `fields` — `[sources.reference.drofus] type = "upload"` — rather than
/// nesting under a second table.
#[derive(Debug, Deserialize, Serialize)]
pub struct ReferenceSourceConfig {
    #[serde(flatten)]
    pub origin: ReferenceOrigin,

    /// Which primary entity this source joins onto. Defaults to `rooms`, so
    /// every settings file written before entity-scoping existed still means
    /// exactly what it meant.
    ///
    /// **Declared before `fields`, and that is load-bearing.** A scalar emitted
    /// *after* a sequence lands inside it when the settings file is written back
    /// (see `test_toml_serializer_hoists_values_above_tables`), so `entity`
    /// would silently become a member of the last field entry. R3 measured that
    /// hazard and shipped the guard; this is the declaration order that guard
    /// exists to protect.
    ///
    /// An unrecognised value is a **startup failure naming it**, not a silent
    /// default — the whole point of the field is to retire the previous
    /// behaviour, where `[sources.reference.doors]` parsed, loaded and joined
    /// nowhere.
    #[serde(default)]
    pub entity: ReferenceEntity,

    /// Per-column declarations for this source's fields. See
    /// `ReferenceFieldConfig`. Empty if omitted, which is the default
    /// behavior for every column: treated as a string, numeric-adaptive
    /// comparison if both sides happen to parse as a number.
    #[serde(default)]
    pub fields: Vec<ReferenceFieldConfig>,
}

/// The primary entity a reference source joins onto.
///
/// **Source names stay unique across entities**, because the join namespace is
/// flat: `schedule.FireRating`, never `doors.schedule.FireRating`. That
/// namespace answers exactly one question — "what goes before the dot in a
/// filter" — and the entity is already known from the endpoint, so nesting it
/// would repeat what the URL just said and fork `split_namespace` into
/// variable-depth parsing. Revit property names contain dots; `a.b.c` would stop
/// having one reading.
///
/// Uniqueness costs nothing to enforce: the sources map is keyed by name, so
/// two entries claiming one name is a TOML duplicate-key error before this type
/// is ever constructed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReferenceEntity {
    #[default]
    Rooms,
    Doors,
    Windows,
    Ffe,
}

impl ReferenceEntity {
    /// The wire/TOML spelling, for error messages that have to name it.
    pub fn as_str(self) -> &'static str {
        match self {
            ReferenceEntity::Rooms => "rooms",
            ReferenceEntity::Doors => "doors",
            ReferenceEntity::Windows => "windows",
            ReferenceEntity::Ffe => "ffe",
        }
    }
}

/// A reference source's data origin. `#[serde(tag = "type")]` lets the TOML
/// `type` field pick the variant — adding an `Api` variant later is a
/// loader-only change; all consumers of `AppState` stay untouched.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ReferenceOrigin {
    /// **Removed. Retained only to fail an old settings file usefully.**
    ///
    /// `type = "file"` read a CSV from a path in the settings file, once at
    /// startup. It was the stand-in for uploads before uploads existed; now
    /// that they do, it is a second way to get the same data in, with a worse
    /// story on every axis — no history (an overwritten CSV is gone, while an
    /// upload is a dated snapshot a milestone can pin), no validation before
    /// the data goes live, and a path the server reads on the operator's
    /// behalf, which is what made `/api/settings/reference-check` an
    /// unauthenticated arbitrary-file read.
    ///
    /// Deserializing still works so `bootstrap::load_project_bundle` can
    /// reject it by name and say how to migrate, rather than leaving serde to
    /// answer with "unknown variant `file`". Nothing loads it — see that
    /// function's `File` arm, the only place this variant is now read.
    File { path: PathBuf },
    /// Data arrives via `POST /projects/{id}/reference/{source}` uploads,
    /// stored as timestamped snapshots in the `SnapshotStore`; the latest one
    /// is hydrated at startup (and hot-swapped in after each upload). A
    /// project with this source but no upload yet is legitimately "not
    /// configured yet" downstream — not a startup error. **The only origin.**
    Upload,
    // Future: Api { url: String, api_key: String },
}

/// Dev-only seed data. Kept separate from `drofus` so removing this test seam
/// later is a one-section deletion with no other changes.
#[derive(Debug, Deserialize)]
pub struct TestData {
    /// Path to a pre-exported snapshot (same JSON shape a POST sends).
    pub snapshot_path: PathBuf,
}

/// One user-defined milestone: a named date with data snapshots explicitly
/// pinned to it (`attachments`: model id → snapshot `taken_at`). The *name*
/// is the milestone's identity — unique per project, and what
/// `/rooms?milestone=` matches on; the date is display/ordering metadata.
/// A milestone can also pin reference-source snapshots (`reference_snapshots`,
/// source name → `taken_at`), so the milestone view joins each source's
/// reference data as it stood at the milestone rather than the project's
/// current data — dRofus is the first (and today, only) entry.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Milestone {
    /// Identity: unique per project, non-empty (validated at load).
    pub name: String,
    /// Display/order date: `YYYY-MM-DD` or a full RFC3339 date-time
    /// (validated at load).
    pub date: String,
    /// Optional reference-source snapshots pinned to this milestone: source
    /// name → the `taken_at` id of one uploaded CSV in the store, joined onto
    /// this milestone's rooms instead of that source's current data. A source
    /// with no entry (the common case, and every milestone authored before
    /// this field existed) keeps the pre-pinning behaviour — the milestone
    /// view joins that source's *current* data. Like an `attachments` pin,
    /// whether a pinned snapshot still *exists* is a read-time concern (skip +
    /// warn, fall back to current); only its *shape* (a valid RFC3339-UTC
    /// snapshot id) is validated here.
    #[serde(default)]
    pub reference_snapshots: BTreeMap<String, String>,
    /// Explicit pins: model id → snapshot id (`taken_at`). A model with no
    /// entry simply doesn't appear in this milestone's view. Whether a pinned
    /// snapshot still *exists* is a read-time concern (skip + warn), not
    /// validated here — settings can't see storage.
    #[serde(default)]
    pub attachments: BTreeMap<String, String>,
    /// The same, for this milestone's **doors** snapshots: model id → snapshot
    /// id. A separate map rather than a second use of `attachments`, because
    /// rooms and doors are pushed independently and their snapshot ids do not
    /// correspond — pinning a milestone to a rooms snapshot says nothing about
    /// which doors snapshot was current at the time, and guessing "the nearest"
    /// would silently pair data that never coexisted.
    ///
    /// A model with no entry contributes no doors to this milestone's view,
    /// exactly as `attachments` governs its rooms. `default` keeps every
    /// milestone authored before doors existed valid, as one that pins none.
    #[serde(default)]
    pub door_attachments: BTreeMap<String, String>,

    /// The same again, for **windows**. A third map for the reason the second
    /// one gives: the three entities are pushed independently and their snapshot
    /// ids do not correspond, so pinning one says nothing about the others and
    /// "the nearest" would silently pair data that never coexisted.
    ///
    /// `default` keeps every milestone authored before windows existed valid, as
    /// one that pins none -- which is what it is.
    #[serde(default)]
    pub window_attachments: BTreeMap<String, String>,

    /// And again, for **FF&E**. A fourth map, for the reason the second one
    /// gave and the third one paid: the entities are pushed independently and
    /// their snapshot ids do not correspond, so pinning one says nothing about
    /// the others and "the nearest" would silently pair data that never
    /// coexisted.
    ///
    /// That matters more here than for any other entity. A milestone exists to
    /// answer "what did the model hold on date X", and FF&E is the thing that
    /// changes between issues while the rooms stand still -- so an FF&E pin
    /// inferred from a rooms pin would be wrong precisely when someone is
    /// looking.
    ///
    /// `default` keeps every milestone authored before FF&E existed valid, as
    /// one that pins none.
    #[serde(default)]
    pub ffe_attachments: BTreeMap<String, String>,
}

impl Milestone {
    /// Startup-loud checks on one milestone's own fields (uniqueness across
    /// milestones is checked in `load_settings`, which can see the siblings).
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.name.trim().is_empty() {
            anyhow::bail!("a milestone has an empty name");
        }
        let date_ok = chrono::NaiveDate::parse_from_str(&self.date, "%Y-%m-%d").is_ok()
            || chrono::DateTime::parse_from_rfc3339(&self.date).is_ok();
        if !date_ok {
            anyhow::bail!(
                "milestone '{}' has an invalid date {:?} (expected YYYY-MM-DD or RFC3339)",
                self.name,
                self.date
            );
        }
        // Both pin maps get the same shape check — an id that is not a valid
        // RFC3339-UTC snapshot id can never name a stored snapshot, so it is a
        // startup-loud error rather than a read that silently finds nothing.
        for (label, pins) in [
            ("attachment", &self.attachments),
            ("door attachment", &self.door_attachments),
            ("window attachment", &self.window_attachments),
        ] {
            for (model_id, taken_at) in pins {
                if model_id.trim().is_empty() {
                    anyhow::bail!("milestone '{}' has an {} with an empty model id", self.name, label);
                }
                crate::contract::validate_snapshot_id(taken_at).map_err(|e| {
                    anyhow::anyhow!("milestone '{}', {} for model '{}': {}", self.name, label, model_id, e)
                })?;
            }
        }
        // Same rule as an attachments pin: a valid RFC3339-UTC snapshot id.
        // Existence is not checkable here (settings can't see storage).
        for (source, id) in &self.reference_snapshots {
            crate::contract::validate_snapshot_id(id)
                .map_err(|e| anyhow::anyhow!("milestone '{}', reference_snapshots.{}: {}", self.name, source, e))?;
        }
        Ok(())
    }
}

/// One tier of the classification hierarchy. A tier is keyed by a code and/or a
/// name property — at least one must be present (validated at startup), since a
/// tier naming neither is unkeyable.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HierarchyTier {
    /// Human label for the tier ("Building", "Department").
    pub name: String,
    /// Room property holding this tier's code. Optional per-tier.
    #[serde(default)]
    pub code_property: Option<String>,
    /// Room property holding this tier's display name. Optional per-tier.
    #[serde(default)]
    pub name_property: Option<String>,
}

impl HierarchyTier {
    /// A tier must name at least one property or it can't be keyed. Validated
    /// at startup so a misconfigured tier is a loud error, not a silent
    /// "undefined" for every room.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.code_property.is_none() && self.name_property.is_none() {
            anyhow::bail!("hierarchy tier '{}' names neither code_property nor name_property", self.name);
        }
        Ok(())
    }
}

/// A footprint exclusion for the hierarchy-areas feature (`service::areas`). The
/// match kind implies WHERE in the two-stage pipeline it applies — the handover's
/// "the match kind implies the stage":
///
/// - `group` — Case A, applied at **stage 2**: a resolved group at `tier` whose
///   value matches is computed normally but WITHHELD from its parent's dissolve,
///   so it drops out of that tier and every tier above while its own footprints
///   stay. Still reported, flagged "not counted upward" (outdoor areas: real,
///   with their own plan, but not part of the building footprint).
/// - `rooms` — Case B, applied at **stage 1**: the listed room ids never enter
///   any union, so they vanish from every tier including their own bottom group.
///
/// Matching a group reuses the resolved tier value everything else classifies
/// against — no second matching vocabulary; `value` matches the tier's resolved
/// code OR name. Internally tagged on `match`, the same self-describing shape
/// `ReferenceOrigin`/`ColourMode` use.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "match", rename_all = "lowercase")]
pub enum HierarchyExclusion {
    Group { tier: String, value: String },
    Rooms { ids: Vec<String> },
}

/// One canonical property definition: a stable name consumers (dRofus
/// `link_property`, hierarchy tier `code_property`/`name_property`) reference,
/// resolved per-source to whatever raw property name that source actually
/// uses. See `Settings::builtin_properties`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BuiltinPropertyDef {
    /// The stable name consumers reference (e.g. "Area").
    pub canonical: String,
    /// Source key (e.g. "revit") → that source's raw property name.
    pub by_source: HashMap<String, String>,
}

impl BuiltinPropertyDef {
    /// A definition with no source mappings can never resolve to anything —
    /// fail fast rather than silently never matching at request time.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.by_source.is_empty() {
            anyhow::bail!("builtin property '{}' has no by_source mappings", self.canonical);
        }
        Ok(())
    }
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

    /// `[ffe]` is defaulted and skipped when untouched, so a project file
    /// written before FF&E existed round-trips unchanged. The same guarantee
    /// `[doors]` and `[windows]` carry, asserted rather than assumed because it
    /// is what lets a fourth entity land without rewriting anyone's settings.
    #[test]
    fn test_an_untouched_ffe_section_stays_out_of_a_written_file() {
        let settings: Settings = toml::from_str("project_id = \"p1\"\n").expect("parses with no [ffe]");
        assert!(settings.ffe.is_default());
        let text = toml::to_string_pretty(&settings).expect("serializes");
        assert!(!text.contains("[ffe]"), "an all-default policy is not written: {text}");
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
}
