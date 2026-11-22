//! Reference data and spaces in settings: the uploaded sources and their fields (`ReferenceSourceConfig`), how a field compares, and the
//! space policy and its fields.

use super::*;

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
#[derive(Debug, Clone, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
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
#[derive(ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
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
#[derive(ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
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

/// One reference source's full configuration: where its data comes from
/// (`origin`) and its per-column type/QA declarations (`fields`). `origin` is
/// `#[serde(flatten)]`ed so its `type` tag sits at the same TOML level as
/// `fields` — `[sources.reference.drofus] type = "upload"` — rather than
/// nesting under a second table.
#[derive(Debug, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
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

/// One project's spaces policy: the space-to-room match and the property
/// comparison over it.
///
/// **Everything here defaults to off.** An unconfigured project still gets the
/// presence half of the report -- which models hold no spaces, and which spaces
/// are unenclosed -- because that needs no configuration to be true. The match
/// needs a key somebody chose, and the property comparison needs pairs somebody
/// chose, so both stay silent until they are given one.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[derive(ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct SpacePolicy {
    /// The SPACE property whose value links a space to a room, e.g. `"Number"`.
    ///
    /// Deliberately not any reference source's `link_property`: that names a
    /// join to data from outside the model, and this names a join between two
    /// models. Absent means no matching is attempted and the report says so,
    /// rather than falling back to a guess -- the same no-fallback rule
    /// `comparison_key` has kept since before joined-source comparison existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison_key: Option<String>,

    /// The ROOM property the key matches against, when the two sides spell it
    /// differently. Defaults to `comparison_key`, which is the common case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_key: Option<String>,

    /// Which models supply the ROOMS this project's spaces are matched against.
    /// Empty (the default) means every model that holds any.
    ///
    /// **This exists because "holds rooms" is not the same as "is the room
    /// authority", and the difference was measured rather than imagined.** On
    /// a real project the electrical services model holds thousands of spaces *and as many rooms of
    /// its own*, numbered `1`, `2`, `3` against the architects' `ENG137`. Pooled
    /// into the room side they are rooms that name nothing, and they bury
    /// the 44 architectural rooms that genuinely have no space -- which is the
    /// finding the report exists to surface.
    ///
    /// A value array, declared before `compared_properties` for the TOML
    /// ordering reason `comparison_key` documents.
    #[serde(default)]
    pub room_models: Vec<String>,

    /// Properties compared on a matched space/room pair. Empty (the default)
    /// means the pair is only reported as matched, never as disagreeing.
    #[serde(default)]
    pub compared_properties: Vec<SpaceFieldConfig>,
}

impl SpacePolicy {
    /// Whether this is the default (unconfigured) policy, so the settings
    /// serializer can omit the whole section.
    pub fn is_default(&self) -> bool {
        *self == SpacePolicy::default()
    }

    /// The room-side spelling of the key: `room_key` when given, else the
    /// space-side name.
    pub fn room_key_name(&self) -> Option<&str> {
        self.room_key.as_deref().or(self.comparison_key.as_deref())
    }
}

/// One property compared between a space and the room it matched.
///
/// **A pair, not a name**, and a sibling of `ReferenceFieldConfig` rather than a
/// widening of it: `label` means nothing here because there are two names, which
/// is the same test that made `Item` a sibling of `Opening`. If the reference
/// path ever wants a tolerance it is a two-line copy; do not widen the shared
/// type for a need only this entity has stated.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[derive(ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct SpaceFieldConfig {
    /// The property name on the space.
    pub space: String,

    /// The property name on the room, when it differs. Defaults to `space`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,

    /// How the values are typed for comparison. Same meaning as
    /// `ReferenceFieldConfig::field_type`.
    #[serde(default, rename = "type")]
    pub field_type: FieldType,

    /// Force exact comparison, or skip this field entirely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qa: Option<CompareMode>,

    /// Flag only past this RELATIVE difference, as a percentage of the ROOM's
    /// value: `|space - room| / room * 100`.
    ///
    /// **The room is the denominator, and the asymmetry is deliberate.** The
    /// architectural model is the authored side and the one a services model is
    /// checked against, so "varies from the room by more than 30%" is measured
    /// against the room. Any symmetric denominator would make a stated 30% mean
    /// something different in each direction, which is worse than an arbitrary
    /// choice made loudly.
    ///
    /// Absent means no tolerance: the values are compared at their stated
    /// precision, as everywhere else in the codebase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance_pct: Option<f64>,

    /// Flag only past this ABSOLUTE difference as well, in the property's own
    /// units. Both thresholds must be exceeded.
    ///
    /// **A percentage alone flags the wrong rooms, and this is the measurement
    /// that proved it.** The boundary-regime difference between an MEP space and
    /// an architectural room scales with perimeter while the value scales with
    /// area, so on a real project the median relative difference fell steeply
    /// from rooms under 2 m2 to rooms over 100. A flat 30% flagged hundreds of pairs, **most of them
    /// sub-2 m2 rooms**, which is noise that gets
    /// the whole report ignored. 30% with a 2 m2 floor flags 127.
    ///
    /// Defaults to absent, which is a pure percentage: the simple case stays
    /// simple.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance_min: Option<f64>,
}

impl SpaceFieldConfig {
    /// The room-side property name: `room` when given, else the space-side one.
    pub fn room_name(&self) -> &str {
        self.room.as_deref().unwrap_or(&self.space)
    }
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
#[derive(ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub enum ReferenceEntity {
    #[default]
    Rooms,
    Doors,
    Windows,
    Ffe,
    Spaces,
}

impl ReferenceEntity {
    /// The wire/TOML spelling, for error messages that have to name it.
    pub fn as_str(self) -> &'static str {
        match self {
            ReferenceEntity::Rooms => "rooms",
            ReferenceEntity::Doors => "doors",
            ReferenceEntity::Windows => "windows",
            ReferenceEntity::Ffe => "ffe",
            ReferenceEntity::Spaces => "spaces",
        }
    }
}

/// A reference source's data origin. `#[serde(tag = "type")]` lets the TOML
/// `type` field pick the variant — adding an `Api` variant later is a
/// loader-only change; all consumers of `AppState` stay untouched.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[derive(ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
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
