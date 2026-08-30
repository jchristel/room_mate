//! `/rooms` fetch-side derive logic: dRofus join, classification, level dedup.
//!
//! Moved verbatim out of `handlers::get_rooms`
//! -- the join/classify logic never depended on `Query`/`Json`/`StatusCode`,
//! so the only real change here is the signature: plain `Option<&str>` filters
//! in, a plain `RoomsResult` out, no transport type touched.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::classify::{classify_room, TierValue};
use crate::contract::{
    elevation_match, lookup_property, numeric_match, property_presence, Level, PropertyPresence, Room, RoomBoundary,
    RoomPayload, SUPPORTED_SCHEMA,
};
use crate::reference::{ReferenceData, ReferenceRecord};
use crate::settings::{BuiltinPropertyDef, HierarchyTier};
use crate::state::{AppState, ModelKey, ProjectSettings, SettingsRegistry};
use crate::storage::SnapshotKind;

use super::ServiceError;

/// A stored payload scoped to one request: its key, the (possibly
/// milestone-substituted) payload, and the project settings bundle it resolves
/// against — borrowed from the request's single settings snapshot, hence the
/// lifetime. The unit the three assembly phases pass between them.
type ScopedPayload<'a> = (ModelKey, RoomPayload, &'a ProjectSettings);

/// The reference data a milestone view joins against, resolved once per
/// (project id, source name) pair. A `Some(data)` is joined instead of that
/// source's current data; a `None` *value* means "attempted, fall back to
/// current" (a missing or unparseable pin, memoised so it's neither re-parsed
/// nor re-warned). Empty on the non-milestone path.
type MilestoneReference = BTreeMap<(String, String), Option<ReferenceData>>;

/// A room as sent to the viewer: the stored room plus any attached reference-
/// source data and its resolved classification path. Separate response type
/// so the join never mutates the stored snapshot, and so each joined source
/// stays a distinct sub-object (its own lifecycle — dRofus, for one, will
/// later refresh on its own trigger, so it must not be fused into the room's
/// own properties).
#[derive(Serialize)]
pub struct RoomResponse {
    #[serde(flatten)]
    pub room: Room,

    /// Joined reference-source records, keyed by source name. `#[serde(flatten)]`
    /// spreads each entry as its own top-level wire key — `reference["drofus"]`
    /// serializes exactly as the single `drofus` field did before this
    /// generalized to N sources, so today's one-source wire shape is
    /// byte-identical; a second configured source just adds its own top-level
    /// key alongside it, no frontend change required to keep working. A source
    /// with no joined record for this room (including every source when
    /// nothing joined) is simply absent from the map — an unmatched key is a
    /// signal, not an error — and an empty map flattens to no keys at all, so
    /// no separate `skip_serializing_if` is needed.
    #[serde(flatten)]
    pub reference: BTreeMap<String, ReferenceRecord>,

    /// Full-depth classification path. Empty when no hierarchy is configured.
    pub classification: Vec<TierValue>,

    /// Resolved room-label fields, in the order configured by
    /// `Settings.room_label` (e.g. `["$name", "Area", "$id"]`). Only the
    /// fields that actually resolved — an unconfigured or unresolvable name
    /// contributes nothing, same discipline as `drofus`/`classification`.
    /// The viewer renders whatever's here without needing to know property
    /// names itself.
    pub label: Vec<String>,

    /// The owning model's `source` (e.g. "revit"). Carried so a downstream
    /// consumer — `service::comparison` — can resolve this room's canonical
    /// property names against the project's `builtin_properties` exactly the
    /// way assembly already did, rather than re-deriving it. Not part of the
    /// wire shape (the viewer never needs it), so skipped from serialization;
    /// the /rooms JSON is byte-for-byte unchanged.
    #[serde(skip)]
    pub source: String,

    /// The owning `(project, model)`, on the same skip-serialized terms as
    /// `source` above and for the same kind of reason: a downstream service
    /// consumer needs it and the viewer does not, so the `/rooms` JSON stays
    /// byte-for-byte unchanged.
    ///
    /// **`service::doors` is what needs it, and the need is a correctness one.**
    /// A door's building is its owning room's building, so resolving a building
    /// scope for doors means indexing these rooms by the room ids doors
    /// reference — and room ids are unique only *within* a model. Without the
    /// model here, that index would be keyed on a bare room id and would happily
    /// resolve a door in one model against a same-numbered room in another.
    /// `project_id` rides along because the settings bundle that decides which
    /// hierarchy tier is "Building" is per project.
    #[serde(skip)]
    pub project_id: String,
    #[serde(skip)]
    pub model_id: String,
}

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
fn resolve_label_fields(
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
fn assemble_room(
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

/// Sentinel `building` key for rooms whose "Building" tier didn't resolve —
/// distinct from any real `building_key` output since real keys never start
/// with `__`.
pub const UNCLASSIFIED_BUILDING_KEY: &str = "__unclassified__";

/// Opaque token identifying one building bucket, built from its resolved
/// `(code, name)` pair. Callers (the browser) never decode this — they just
/// echo it back to `/rooms?building=..` — so the encoding only has to be
/// stable for the lifetime of one response, not human-meaningful. Known
/// caveat: a literal `|` inside a code/name could in principle make two
/// distinct pairs collide (`("a|", "b")` and `("a", "|b")` both encode to
/// `"a||b"`); accepted, since a `|` inside a building code is not a realistic
/// input and the cost is only two buckets merging in the picker.
pub fn building_key(code: &Option<String>, name: &Option<String>) -> String {
    format!("{}|{}", code.as_deref().unwrap_or(""), name.as_deref().unwrap_or(""))
}

/// Index of the hierarchy tier named "Building", if one is configured.
/// Shared by `projects::list_buildings` and the `/rooms` building filter so
/// both resolve the exact same tier the exact same way.
pub fn building_tier_index(hierarchy: &[HierarchyTier]) -> Option<usize> {
    hierarchy.iter().position(|t| t.name == "Building")
}

/// The three ways a field name splits against the namespace vocabulary.
/// Returned by `split_namespace` so every consumer (filter parsing, comparison
/// resolution, settings validation) applies the *same* split rule and only
/// phrases the error differently — the error wording carries caller context
/// (a filter names the predicate, a settings load names the file), so the
/// message itself can't live here, but the classification must.
pub enum NamespaceSplit<'a> {
    /// `<known-source>.<property>` — a joined data source's field. `source` is
    /// owned (cloned out of the caller's `known` set) rather than borrowed, so
    /// downstream consumers can carry it without re-splitting or fighting a
    /// lifetime tied to that set.
    Joined { source: String, property: &'a str },
    /// No namespace: the room's own property vocabulary (canonical names plus
    /// the `$name`/`$id` intrinsics).
    Unqualified(&'a str),
    /// `<name>.<rest>` where `<name>` is not a known source — an error for the
    /// caller to phrase in its own context, never a silent fallback to a room
    /// property (see `Predicate::parse` for why).
    UnknownSource(&'a str),
}

/// Split one field name against the `source.property` vocabulary. The single
/// owner of the split rule: a dot after a name in `known` binds as a
/// namespace; a dot after an unrecognised *single word* is an error; a dot
/// inside a name containing spaces stays part of the property name, because a
/// raw Revit property name is far likelier to contain a dot than to be an
/// attempted namespace.
///
/// `known` is the *recognised* source-name vocabulary at the call site, not a
/// fixed list — see `SettingsRegistry::known_reference_sources` for the
/// registry-wide set a `/rooms` filter uses (unscoped, so no single project's
/// config can answer "what's before the dot"), and
/// `bootstrap::load_project_bundle` for the project-local set settings
/// validation uses instead.
pub fn split_namespace<'a>(field: &'a str, known: &std::collections::BTreeSet<String>) -> NamespaceSplit<'a> {
    match field.split_once('.') {
        Some((ns, rest)) => {
            if let Some(source) = known.get(ns) {
                NamespaceSplit::Joined { source: source.clone(), property: rest.trim() }
            } else if !ns.contains(' ') {
                NamespaceSplit::UnknownSource(ns)
            } else {
                NamespaceSplit::Unqualified(field)
            }
        }
        None => NamespaceSplit::Unqualified(field),
    }
}

/// The error text for a `NamespaceSplit::UnknownSource`, without caller
/// context. Each consumer prefixes its own ("filter …", "comparison_key …")
/// so the *vocabulary* half of the message can never drift between the two
/// surfaces while the *noun* half stays accurate to each.
pub fn unknown_source_message(ns: &str, known: &std::collections::BTreeSet<String>) -> String {
    let known: Vec<&str> = known.iter().map(String::as_str).collect();
    format!("unknown data source {ns:?} — known sources: {}", known.join(", "))
}

/// A comparison operator in a room predicate. `Contains` (`~`) is the only
/// fuzzy one — everything else is exact, numeric-tolerant where both sides
/// parse as numbers (see `Predicate::holds`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Contains,
}

/// The operator spellings paired with their `Op`. Order matters *within one
/// position*: `>=` must be tried before `>` and `!=` before `=`, or `Area>=20`
/// would split as `Gt` with the value `"=20"`. `split_operator` scans positions
/// left to right and this list at each one, so the earliest operator wins and
/// the longest spelling wins the tie.
const OPERATORS: &[(&str, Op)] = &[
    (">=", Op::Ge),
    ("<=", Op::Le),
    ("!=", Op::Ne),
    ("~", Op::Contains),
    (">", Op::Gt),
    ("<", Op::Lt),
    ("=", Op::Eq),
];

/// Find the operator in a predicate expression: the leftmost position where any
/// spelling matches, longest spelling first at that position. Returns the raw
/// (field, op, value) slices, untrimmed.
fn split_operator(expr: &str) -> Option<(&str, Op, &str)> {
    for (i, _) in expr.char_indices() {
        for (token, op) in OPERATORS {
            if expr[i..].starts_with(token) {
                return Some((&expr[..i], *op, &expr[i + token.len()..]));
            }
        }
    }
    None
}

/// One predicate: an optionally source-qualified field name, an operator, and a
/// value.
///
/// `source` is the *joined data source* namespace (a field name of
/// `settings::Sources`, e.g. `drofus`) — NOT `Model.source` ("revit"/"ifc"),
/// which says which producer created the room and stays a `lookup_property`
/// argument. Two different axes that both got called "source"; they never mix.
/// `None` means the room's own `properties`, plus the `$name`/`$id` intrinsics
/// `resolve_label_fields` already understands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Predicate {
    pub source: Option<String>,
    pub property: String,
    pub op: Op,
    pub value: String,
}

impl Predicate {
    /// Parse one `[<source>.]<property><op><value>` expression.
    ///
    /// A leading `<name>.` binds as a source namespace when `<name>` is in
    /// `known`, and is an *error* naming the known sources otherwise — never a
    /// silent fallback to a room property. Without that rule a raw property
    /// literally named `Drofus.NetArea` would bind as a room property today
    /// and silently change meaning the day a namespace of that name exists.
    ///
    /// An unknown *unqualified* property is deliberately not an error:
    /// `resolve_raw_name` falls back to using the name as a raw key, which is
    /// exactly right for a raw property no `BuiltinPropertyDef` maps. So a typo
    /// returns zero rooms rather than a complaint.
    fn parse(expr: &str, known: &std::collections::BTreeSet<String>) -> Result<Self, String> {
        let Some((field, op, value)) = split_operator(expr) else {
            return Err(format!(
                "filter {expr:?}: no operator found — expected one of = != > >= < <= ~ (e.g. \"Department=Cardiology\")"
            ));
        };

        let field = field.trim();
        if field.is_empty() {
            return Err(format!("filter {expr:?}: the field name is empty"));
        }

        // Quoting is what makes a value containing the HTTP `?filter=`
        // separator expressible: `Department="Cardiology, North"`.
        let value = value.trim();
        let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
        if value.is_empty() {
            // Always a mistake rather than a way to ask for "blank": an absent
            // or empty property never matches any operator (see `matches`), so
            // an empty value could only ever return nothing.
            return Err(format!("filter {expr:?}: the value is empty"));
        }

        // The split rule itself (including the dot-in-a-spaced-name subtlety)
        // lives in `split_namespace`; only the error phrasing is ours.
        let (source, property) = match split_namespace(field, known) {
            NamespaceSplit::Joined { source, property } => (Some(source), property),
            NamespaceSplit::UnknownSource(ns) => {
                return Err(format!("filter {expr:?}: {}", unknown_source_message(ns, known)));
            }
            NamespaceSplit::Unqualified(name) => (None, name),
        };
        if property.is_empty() {
            return Err(format!("filter {expr:?}: the field name is empty"));
        }

        Ok(Predicate { source, property: property.to_string(), op, value: value.to_string() })
    }

    /// Does a resolved value satisfy this predicate?
    ///
    /// `=`/`!=` use `numeric_match` when both sides parse as numbers (so
    /// `"25.50"` equals `"25.5"` — the same stated-precision tolerance dRofus
    /// validation applies), exact string comparison otherwise. The ordering
    /// operators are numeric only: a value that doesn't parse as a number
    /// simply doesn't match, it is not an error (signal, not error). `~` is a
    /// case-insensitive substring test — the one fuzzy operator, so a caller
    /// that doesn't know a value's exact spelling still has a way in.
    fn holds(&self, actual: &str) -> bool {
        let equal = || numeric_match(actual, &self.value).unwrap_or_else(|| actual == self.value);
        match self.op {
            Op::Eq => equal(),
            Op::Ne => !equal(),
            Op::Contains => actual.to_lowercase().contains(&self.value.to_lowercase()),
            Op::Gt | Op::Ge | Op::Lt | Op::Le => {
                let (Ok(a), Ok(b)) = (actual.trim().parse::<f64>(), self.value.trim().parse::<f64>()) else {
                    return false;
                };
                match self.op {
                    Op::Gt => a > b,
                    Op::Ge => a >= b,
                    Op::Lt => a < b,
                    Op::Le => a <= b,
                    _ => unreachable!("outer match already narrowed to the ordering operators"),
                }
            }
        }
    }
}

/// Resolve one already-split field against an assembled room to its
/// three-state presence. The single place that knows the namespace vocabulary
/// at read time — a map lookup on `room.reference`, so a new joined source
/// needs no arm here at all, only a settings entry — filtering
/// (`resolve_field`) and milestone comparison (`resolve_presence`) both funnel
/// through this.
fn presence_of(
    room: &RoomResponse,
    source: Option<&str>,
    property: &str,
    builtin_defs: &[BuiltinPropertyDef],
) -> PropertyPresence {
    /// A room's own `name`/`id` fields have no absent state — the struct
    /// always carries them — so blank collapses to `Empty`, never `Absent`.
    fn intrinsic(value: &str) -> PropertyPresence {
        if value.is_empty() {
            PropertyPresence::Empty
        } else {
            PropertyPresence::Present(value.to_string())
        }
    }

    match source {
        None => match property {
            "$name" => intrinsic(&room.room.name),
            "$id" => intrinsic(&room.room.id),
            canonical => property_presence(&room.room, canonical, &room.source, builtin_defs),
        },
        // The joined record's own field labels, verbatim as the source
        // reports them — no canonical mapping, since those labels are the
        // source's vocabulary, not Revit's. A room with no joined record for
        // this source at all is `Absent` on every field of it; callers that
        // need to tell "source unjoined" from "field missing" ask
        // `source_joined` (see `service::comparison`). A `source` naming a
        // namespace no room could ever carry (rejected at settings load and
        // filter parse before reaching here) degrades to the same `Absent`
        // via the map lookup — no separate catch-all needed.
        Some(name) => match room.reference.get(name) {
            None => PropertyPresence::Absent,
            Some(record) => match record.fields.get(property) {
                None => PropertyPresence::Absent,
                Some(v) if v.is_empty() => PropertyPresence::Empty,
                Some(v) => PropertyPresence::Present(v.clone()),
            },
        },
    }
}

/// Resolve one comparable/filterable field name against an assembled room, in
/// the `source.property` vocabulary shared with the `/rooms` filter. "What can
/// I write before the dot" must have one answer across filtering, comparison,
/// and settings validation, or a name that filters correctly would silently
/// diff as nothing — the bug this function exists to close (see
/// STRATEGY-SOURCES.md).
///
/// Returns the resolved namespace alongside the presence so callers can react
/// per source (an unjoined source, a source-aware comparator) without
/// re-splitting the string.
pub fn resolve_presence(
    room: &RoomResponse,
    field: &str,
    known: &std::collections::BTreeSet<String>,
    builtin: &[BuiltinPropertyDef],
) -> (Option<String>, PropertyPresence) {
    match split_namespace(field, known) {
        NamespaceSplit::Joined { source, property } => {
            let presence = presence_of(room, Some(&source), property, builtin);
            (Some(source), presence)
        }
        NamespaceSplit::Unqualified(name) => (None, presence_of(room, None, name, builtin)),
        // Rejected at settings load and filter parse, so unreachable through
        // either configured path — degrades to "nothing to compare" rather
        // than panicking, same discipline as `presence_of`'s map-lookup arm.
        NamespaceSplit::UnknownSource(_) => (None, PropertyPresence::Absent),
    }
}

/// Does this room carry a joined record for `source` at all? Distinguishes
/// "the source never matched this room" (one per-room fact) from "the source
/// matched but lacks this field" (a per-field fact) — `service::comparison`
/// reports the former once per room rather than once per configured property.
pub fn source_joined(room: &RoomResponse, source: &str) -> bool {
    room.reference.contains_key(source)
}

/// What a `RoomFilter` predicate can be resolved against.
///
/// One filter grammar, two entities. The `?filter=` syntax, the operators, the
/// quoting rule and the numeric tolerance are all properties of the *query*, not
/// of what is being queried — so doors reuse every one of them and supply only
/// the one thing that genuinely differs: how a field name becomes a value.
///
/// Deliberately narrow. It resolves a *presence*, not a value, so the
/// Absent/Empty distinction survives for any future consumer that needs it, and
/// so the "a missing field never matches, for any operator" rule stays stated
/// once in `RoomFilter::matches` rather than per implementor.
pub trait FilterTarget {
    /// Resolve one already-split field to its three-state presence. `source` is
    /// the joined-data-source namespace (`None` for the entity's own
    /// vocabulary), never `Model.source`.
    fn presence(&self, source: Option<&str>, property: &str, builtin_defs: &[BuiltinPropertyDef]) -> PropertyPresence;
}

impl FilterTarget for RoomResponse {
    fn presence(&self, source: Option<&str>, property: &str, builtin_defs: &[BuiltinPropertyDef]) -> PropertyPresence {
        presence_of(self, source, property, builtin_defs)
    }
}

/// Resolve one predicate's field against a filter target, collapsed for
/// matching. Returns `None` for absent *and* empty, exactly as
/// `lookup_property` does — which is what makes "an entity missing the field
/// never matches" fall out of `RoomFilter::matches` for every operator rather
/// than being special-cased per operator.
fn resolve_field<T: FilterTarget>(
    target: &T,
    predicate: &Predicate,
    builtin_defs: &[BuiltinPropertyDef],
) -> Option<String> {
    match target.presence(predicate.source.as_deref(), &predicate.property, builtin_defs) {
        PropertyPresence::Present(v) => Some(v),
        PropertyPresence::Absent | PropertyPresence::Empty => None,
    }
}

/// Validate the *namespace* half of one settings-configured field name — the
/// only half checkable at load time. Serves every surface that may name a
/// joined source: `comparison_key`/`comparison_properties`, `room_label`, and a
/// colour plan's compared/date properties.
///
/// An unqualified name stays unvalidated: it is free-text that may legitimately
/// match no currently-loaded room (an empty store still boots). A bad namespace,
/// by contrast, can never resolve — and unvalidated it degrades differently on
/// each surface but always silently: an empty milestone diff indistinguishable
/// from "no changes", a label chip that never appears, a plan that greys every
/// room. One loud load error replaces all three (see CODING-CONVENTIONS.md
/// §"Loud startup over silent no-op"). Checking only the knowable half is what
/// makes leaving the property half alone safe.
///
/// Called from `bootstrap::load_project_bundle`, which the settings-save path
/// re-runs, so a save gets the same rejection.
pub fn validate_namespaced_field(field: &str, known: &std::collections::BTreeSet<String>) -> Result<(), String> {
    match split_namespace(field, known) {
        NamespaceSplit::UnknownSource(ns) => Err(unknown_source_message(ns, known)),
        NamespaceSplit::Joined { source, property: "" } => Err(format!(
            "no property named after the {source:?} namespace — expected {source}.<field label>"
        )),
        NamespaceSplit::Joined { .. } | NamespaceSplit::Unqualified(_) => Ok(()),
    }
}

/// The counterpart rule, for a setting that resolves against the room's OWN
/// properties and cannot reach a joined source at all — today, a hierarchy
/// tier's `code_property`/`name_property`.
///
/// `classify_room` takes a `&Room` and never sees the joined record, so a tier
/// naming a real source would resolve to nothing on every room and bucket the
/// whole project as `undefined`. That is the silent no-op worth a load error,
/// and it is why this is the inverse of `validate_namespaced_field` rather than
/// a stricter version of it: there, a known namespace is the *good* case.
///
/// Only a **known** source prefix is refused. An unrecognised word before a dot
/// is, on this surface, just a property name that happens to contain a dot —
/// `lookup_property` reads it literally and always has — so rejecting it would
/// break a working config to guard against a typo that costs nothing here.
pub fn validate_room_only_field(field: &str, known: &std::collections::BTreeSet<String>) -> Result<(), String> {
    match split_namespace(field, known) {
        NamespaceSplit::Joined { source, .. } => Err(format!(
            "reads data source {source:?}, which classification cannot do — a tier resolves against the room's own \
             properties only, so every room would classify as undefined"
        )),
        NamespaceSplit::UnknownSource(_) | NamespaceSplit::Unqualified(_) => Ok(()),
    }
}

/// A set of predicates, ALL of which must hold (AND). No OR and no grouping:
/// that is where a filter turns into a query engine, and a caller who needs a
/// union can make two calls.
///
/// Parsing is the only fallible step — matching never fails, it just doesn't
/// match. Applied to an *assembled* `RoomResponse` rather than a raw `Room`, so
/// predicates can reach the joined data sources (see `resolve_field`).
#[derive(Debug, Clone, Default)]
pub struct RoomFilter {
    predicates: Vec<Predicate>,
}

impl RoomFilter {
    /// Parse one predicate per element — the MCP form, where an array element
    /// per predicate means a caller never has to escape a separator.
    /// `Err` carries caller-addressable text naming the offending element.
    /// `known` is the recognised source-name vocabulary — see
    /// `split_namespace`.
    pub fn parse(exprs: &[String], known: &std::collections::BTreeSet<String>) -> Result<Self, String> {
        let predicates = exprs
            .iter()
            .filter(|e| !e.trim().is_empty())
            .map(|e| Predicate::parse(e.trim(), known))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(RoomFilter { predicates })
    }

    /// Parse the HTTP `?filter=` form: comma-separated predicates. A value
    /// containing a literal comma must be quoted (`Department="A, B"`) — the
    /// split respects those quotes.
    pub fn parse_query(s: &str, known: &std::collections::BTreeSet<String>) -> Result<Self, String> {
        let mut parts: Vec<String> = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        for c in s.chars() {
            match c {
                '"' => {
                    quoted = !quoted;
                    current.push(c);
                }
                ',' if !quoted => parts.push(std::mem::take(&mut current)),
                _ => current.push(c),
            }
        }
        parts.push(current);
        RoomFilter::parse(&parts, known)
    }

    /// True when this filter holds no predicates — the caller then passes
    /// `None` rather than an empty filter, so "no filter" has one representation
    /// downstream (it also governs level suppression, see
    /// `assemble_scoped_rooms`).
    pub fn is_empty(&self) -> bool {
        self.predicates.is_empty()
    }

    /// Does this assembled entity satisfy every predicate? A field that resolves
    /// to nothing fails *every* operator, negative ones included: "this room has
    /// no Department" is not evidence that its Department differs from
    /// Cardiology, and for a joined source an unmatched link key is a signal,
    /// not a value.
    ///
    /// Generic over `FilterTarget` so rooms and doors share one matcher — the
    /// rule above is the subtle part, and it must not exist twice.
    pub(crate) fn matches<T: FilterTarget>(&self, target: &T, builtin_defs: &[BuiltinPropertyDef]) -> bool {
        self.predicates
            .iter()
            .all(|p| resolve_field(target, p, builtin_defs).is_some_and(|actual| p.holds(&actual)))
    }
}

/// Everything that narrows a rooms read, in one named bundle. `Default` is
/// "merge every stored model, latest snapshots, no filter" — the unscoped read.
///
/// A struct rather than four positional `Option`s because the call sites were
/// already at three and heading for trailing-`None` soup; named fields also mean
/// the next scope dimension is an added field, not a re-read of every caller.
#[derive(Default)]
pub struct RoomScope<'a> {
    pub project: Option<&'a str>,
    pub building: Option<&'a str>,
    pub milestone: Option<&'a str>,
    pub filter: Option<&'a RoomFilter>,
}

/// Result of merging every stored model's levels and rooms into one flat
/// payload. Derives `Serialize` so both adapters (HTTP handler, MCP server)
/// can return it directly -- every field here is wire shape, nothing needs
/// stripping. "Nothing has ever been pushed" is not a field on this type; it
/// is `assemble_rooms` returning `None` (see there).
#[derive(serde::Serialize)]
pub struct RoomsResult {
    pub schema_version: u32,
    /// A stable content revision summarising *which snapshot* each contributing
    /// model provides (see `scoped_revision`). Two idle responses return a
    /// byte-identical value; a real push bumps it. The viewer compares this one
    /// field instead of re-stringifying the whole payload every poll, so a quiet
    /// system triggers no re-render (see STRATEGY-BROWSER.md, "Endpoints follow
    /// fetch lifecycle").
    pub revision: String,
    pub levels: Vec<Level>,
    pub rooms: Vec<RoomResponse>,
    /// Each contributing project's reference column vocabulary, keyed by
    /// project id and then by source name — **per project, not flat**, because
    /// the unscoped read merges every stored project and a source resolves per
    /// project; a flat list would silently mean "some project's labels" in that
    /// case. The second level is per source for the same reason one level down:
    /// two sources may both declare a label called `NetArea`, so the source has
    /// to be part of the address.
    ///
    /// Sourced from the same resolved datasets the rows were joined against (a
    /// milestone's pinned snapshots included), so the column set always
    /// describes the data actually on the response, never current headers over
    /// pinned rows. A project with no reference source simply has no entry.
    /// Exists so a tabular consumer can render the *complete* column set — a
    /// column that matched no room in scope is undiscoverable from the rooms
    /// alone, and that is precisely the column the coverage report shows as
    /// "not checked" rather than omitting (see STRATEGY-SOURCES.md).
    pub reference_labels: BTreeMap<String, BTreeMap<String, ReferenceLabels>>,

    /// The boundary regime in force on each **canonical** level in this
    /// response, keyed by the same level ids `levels` carries.
    ///
    /// Per level rather than per model or per project, because that is the
    /// granularity every consumer actually needs and the only one that stays
    /// honest at both ends. The regime is a *model* fact
    /// (`contract::RoomBoundary`) — a project can mix both, since each linked
    /// model carries its own document setting — but level dedup deliberately
    /// merges "the same" architectural level across those models, so one level
    /// can be fed by several models at once. When they disagree, **finish face
    /// wins**: it is the regime that still needs a gap bridged, and sizing for
    /// the narrower one would leave those rooms as disjoint islands. A model
    /// declaring nothing resolves through its project's `[areas]
    /// boundary_location` and then to finish face, so an undeclared model also
    /// keeps the wider reading — the conservative direction in both cases.
    ///
    /// `service::areas` sizes its per-level wall zone from this, and
    /// `service::adjacency` derives its default `wall_max`. On the wire because
    /// an area figure whose regime is unstated is exactly the ambiguity this
    /// whole change exists to remove.
    pub boundary_by_level: BTreeMap<String, RoomBoundary>,

    /// The Revit phase each contributing model's rooms were filtered to, keyed
    /// by project id and then model id.
    ///
    /// **On the wire because this response can legitimately span two phases.**
    /// A model's phase is fixed per `(project, model)` lineage, and nothing
    /// forces the models of one project to agree — enforcing that would deadlock
    /// (moving a project from phase A to B would need model 1 pushed first, and
    /// it would be refused for disagreeing with model 2). So the merge proceeds,
    /// and without this field a consumer would render a plan mixing "New
    /// Construction" and "Existing" rooms with no way to tell. The validation
    /// report names the disagreement as a *finding*; this is the raw fact, for
    /// labelling what is on screen.
    ///
    /// Taken from each **snapshot**, never from the lineage's current phase in
    /// the manifest: a snapshot pushed before phases existed reports `null`,
    /// because its rooms genuinely were not filtered to a phase, and that stays
    /// true after a later push phases the lineage (PLAN-phasing.md "D8"). A
    /// `null` is therefore a real signal — unfiltered, mixed-phase content —
    /// not merely missing metadata.
    ///
    /// Nested per project rather than flat by model id for the same reason
    /// `reference_labels` is: an unscoped read merges every stored project, and
    /// model ids are only unique within one.
    pub phase_by_model: BTreeMap<String, BTreeMap<String, Option<String>>>,
}

/// One reference source's column vocabulary for one project, as joined into
/// this response.
#[derive(Serialize)]
pub struct ReferenceLabels {
    /// Every row-1 CSV label, mapped or not (`ReferenceData::all_labels`).
    pub all_labels: Vec<String>,
    /// Source label → the Revit property row 2 maps it to — the mapped
    /// subset, so a consumer can mark which columns have a Revit counterpart.
    pub reconciliation: BTreeMap<String, String>,
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
fn scoped_revision(scoped: &[ScopedPayload]) -> String {
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

/// Merge every stored model's levels and rooms into one flat payload, scoped
/// by `RoomScope`: an optional project id, an optional opaque building key
/// (from `projects::list_buildings`), an optional milestone name (from
/// `milestones::list_milestones`), and an optional property `RoomFilter`.
/// When a building filter is given, a project
/// whose hierarchy has no tier named "Building" matches *nothing* -- not
/// everything. The caller asked for a building; a project with no notion of
/// one can't answer that question, and `list_buildings` already tells a
/// well-behaved client `tier_configured: false` so it never sends this
/// combination. An empty result is honest; a silently ignored filter is not
/// (it used to leak a tier-less project's entire room set into a filtered
/// multi-project merge). A model contributes its `levels` only when it contributed at
/// least one matching room: levels are their own array from a separate Revit
/// export, so a floor can legitimately have zero rooms of a given building
/// right now yet still belong to it — dropping it would make the slider
/// flicker as classification changes. This rule only applies when a building
/// filter is actually active; with no filter, every scoped model's levels are
/// included exactly as before.
///
/// The milestone filter follows the same discipline: a project whose settings
/// define no milestone of that name contributes nothing, a model the
/// milestone doesn't pin contributes nothing, and a pinned model's payload is
/// the *pinned snapshot* loaded from the store instead of the latest — after
/// which every downstream step (level dedup, building filter, dRofus join,
/// classification) runs on the substituted payloads unchanged, so milestone
/// and building filters compose. A pin whose snapshot no longer exists is
/// skipped with a warning — a dangling pin is a signal, not an error, same as
/// an unmatched dRofus key.
///
/// dRofus join and classification are resolved here at response assembly — the
/// stored snapshots stay raw; derived data is never written back to state.
/// A milestone that pins a `drofus_snapshot` joins *that* stored CSV instead of
/// the project's current dRofus, resolved once per project (see below); a pin
/// whose snapshot is missing or unparseable falls back to the current dRofus
/// with a warning, the same signal-not-error stance as a dangling model pin.
///
/// The property filter (`scope.filter`) is applied *after* assembly, on the
/// finished `RoomResponse` rather than the raw `Room` — that is what lets a
/// predicate reach a joined data source (`drofus.NetArea>20`) and the resolved
/// classification path, neither of which exists yet on the near side of the
/// join. It composes with the other scopes for free: milestone substitution
/// already happened in phase 1, so a filter under `?milestone=` matches the
/// *pinned* rooms and the *pinned* dRofus.
///
/// Returns `Ok(None)` when nothing has ever been pushed to this server at all
/// -- the HTTP adapter's "204 No Content" case. A filter that merely matches
/// nothing is still `Ok(Some)` with empty vecs: the store has data, the
/// question just has an empty answer.
pub fn assemble_rooms(state: &AppState, scope: &RoomScope<'_>) -> Result<Option<RoomsResult>, ServiceError> {
    // Asked of the index, not of the scoped read below — see
    // `AppState::has_any_snapshot`. `?project=nonexistent` against a populated
    // store is a 200 with empty vecs, and scoping the read is exactly what
    // would otherwise have turned it into a 204.
    if !state.has_any_snapshot(SnapshotKind::Rooms).map_err(ServiceError::Internal)? {
        return Ok(None);
    }
    let stored = state.all_snapshots(scope.project).map_err(ServiceError::Internal)?;

    // One settings snapshot for the whole request — a save landing mid-merge
    // can't mix old and new bundles in one response. Held here for the length
    // of the request so `scoped`'s `&ProjectSettings` borrows stay valid.
    let registry = state.settings();

    // Three phases, each its own helper: scope the stored payloads to the
    // request (and resolve any milestone substitutions), dedup levels across
    // linked models, then derive the response rooms/levels.
    let (scoped, milestone_reference) = scope_payloads(state, &registry, stored, scope.project, scope.milestone)?;
    let revision = scoped_revision(&scoped);
    let phase_by_model = phases_of(&scoped);
    let level_remap = dedup_levels(&scoped);
    let AssembledRooms { levels, rooms, reference_labels, boundary_by_level } =
        assemble_scoped_rooms(&scoped, &level_remap, &milestone_reference, scope);

    Ok(Some(RoomsResult {
        schema_version: SUPPORTED_SCHEMA,
        revision,
        levels,
        rooms,
        reference_labels,
        boundary_by_level,
        phase_by_model,
    }))
}

/// Each contributing model's phase, read off the payload actually in this
/// response — so under `?milestone=` it reports the *pinned* snapshot's phase,
/// not the model's current one. Reading it from the payload rather than the
/// manifest is what keeps that true (PLAN-phasing.md "D8").
fn phases_of(scoped: &[ScopedPayload<'_>]) -> BTreeMap<String, BTreeMap<String, Option<String>>> {
    let mut out: BTreeMap<String, BTreeMap<String, Option<String>>> = BTreeMap::new();
    for (key, payload, _) in scoped {
        out.entry(key.project_id.clone())
            .or_default()
            .insert(key.model_id.clone(), payload.phase.clone());
    }
    out
}

/// Phase 1 — scope the stored payloads to the request. Drops any payload whose
/// project has no registered settings bundle (an unscoped merge is
/// per-project, so a model with nothing to classify/join against has no
/// home), and, under a
/// milestone filter, *replaces* each surviving model's latest payload with the
/// snapshot the milestone pins for it (owned payloads, hence no `&` on the
/// tuple's payload slot). A project without the named milestone, or a model it
/// doesn't pin, contributes nothing — the building-filter discipline.
///
/// `pub(crate)` since the doors read needs the same scoped room payloads to
/// resolve a door against room *geometry* — the cheap first move this codebase
/// prescribes when a second consumer appears, rather than a second copy of the
/// milestone-substitution rules that could pin a different snapshot than
/// `/rooms` does.
///
/// The second return value is each milestone-pinned reference source,
/// resolved once per (project id, source name): `Some(data)` = joined instead
/// of that source's current data; a `None` *value* means "attempted, fall
/// back to current" (a missing or unparseable pin), memoised so it's neither
/// re-parsed nor re-warned across a project's models. Empty on the
/// non-milestone path. Kept together with the scoping loop that fills it,
/// since that's where the pin is known.
pub(crate) fn scope_payloads<'r>(
    state: &AppState,
    registry: &'r SettingsRegistry,
    stored: Vec<(ModelKey, RoomPayload)>,
    project: Option<&str>,
    milestone: Option<&str>,
) -> Result<(Vec<ScopedPayload<'r>>, MilestoneReference), ServiceError> {
    let mut milestone_reference: MilestoneReference = BTreeMap::new();
    let mut scoped: Vec<ScopedPayload> = Vec::new();

    for (key, payload) in stored {
        if project.is_some_and(|p| payload.project.id != p) {
            continue;
        }
        let Some(bundle) = registry.settings_for(&payload.project.id) else {
            continue;
        };
        match milestone {
            None => scoped.push((key, payload, bundle)),
            Some(wanted) => {
                let Some(ms) = bundle.milestones.iter().find(|m| m.name == wanted) else {
                    continue;
                };
                let Some(pinned_id) = ms.attachments.get(&key.model_id) else {
                    continue;
                };
                match state.get_snapshot(&key, pinned_id).map_err(ServiceError::Internal)? {
                    Some(pinned) => {
                        // One resolution per (project, source) pin — a
                        // project may pin several sources for one milestone,
                        // each memoised independently the first time any of
                        // its models is seen.
                        for (source, pin) in &ms.reference_snapshots {
                            let map_key = (key.project_id.clone(), source.clone());
                            if let std::collections::btree_map::Entry::Vacant(e) = milestone_reference.entry(map_key) {
                                let resolved = resolve_pinned_reference(state, wanted, &key.project_id, source, pin)?;
                                e.insert(resolved);
                            }
                        }
                        scoped.push((key, pinned, bundle));
                    }
                    None => {
                        tracing::warn!(
                        "milestone '{}' pins snapshot {:?} for {}/{}, but no such snapshot exists — skipping the model",
                        wanted, pinned_id, key.project_id, key.model_id
                    )
                    }
                }
            }
        }
    }

    Ok((scoped, milestone_reference))
}

/// Load and parse a milestone's pinned CSV for one project's reference
/// source. A missing or unparseable pin resolves to `None` with a warning
/// (fall back to that source's current data — signal, not error, same stance
/// as a dangling model pin).
fn resolve_pinned_reference(
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
fn dedup_levels(scoped: &[ScopedPayload<'_>]) -> BTreeMap<(String, String, String), String> {
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
fn assemble_scoped_rooms(
    scoped: &[ScopedPayload<'_>],
    level_remap: &BTreeMap<(String, String, String), String>,
    milestone_reference: &MilestoneReference,
    scope: &RoomScope<'_>,
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
fn widest_boundary(a: RoomBoundary, b: RoomBoundary) -> RoomBoundary {
    match (a, b) {
        (RoomBoundary::Centreline, RoomBoundary::Centreline) => RoomBoundary::Centreline,
        _ => RoomBoundary::FinishFace,
    }
}

/// Phase 3's four outputs. A named struct rather than a 4-tuple: the two maps
/// are both `BTreeMap<String, _>` keyed on different things (project id vs
/// level id), and a positional tuple is one refactor away from swapping them
/// silently.
struct AssembledRooms {
    levels: Vec<Level>,
    rooms: Vec<RoomResponse>,
    reference_labels: BTreeMap<String, BTreeMap<String, ReferenceLabels>>,
    boundary_by_level: BTreeMap<String, RoomBoundary>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{CustomValue, Model, Project, RoomPayload, Snapshot};
    use crate::state::AppState;
    use crate::storage::MemStore;

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

    fn make_drofus(link_property: &str) -> ReferenceData {
        ReferenceData {
            link_property: link_property.to_string(),
            by_id: BTreeMap::new(),
            reconciliation: BTreeMap::new(),
            all_labels: vec![],
            duplicate_ids: vec![],
            blank_id_rows: 0,
        }
    }

    /// Set (or, with `None`, clear) a bundle's "drofus"-named reference
    /// source — the test-side equivalent of what `bootstrap::load_project_bundle`
    /// wires up for the one source name the read path currently recognises.
    fn with_drofus(mut bundle: ProjectSettings, data: Option<ReferenceData>) -> ProjectSettings {
        match data {
            Some(d) => {
                bundle.reference.insert(
                    "drofus".to_string(),
                    crate::state::ProjectReferenceSource {
                        entity: crate::settings::ReferenceEntity::Rooms,
                        data: Some(d),
                        fields: vec![],
                    },
                );
            }
            None => {
                bundle.reference.remove("drofus");
            }
        }
        bundle
    }

    /// A dRofus dataset with one record: link id `id` carries `label` = `value`.
    /// Used by the milestone-pinning tests to make the *current* dRofus differ
    /// from the *pinned* one for the same room, so the join source is what the
    /// assertion actually distinguishes.
    fn make_drofus_with_record(link_property: &str, id: &str, label: &str, value: &str) -> ReferenceData {
        ReferenceData {
            link_property: link_property.to_string(),
            by_id: BTreeMap::from([(
                id.to_string(),
                ReferenceRecord { fields: BTreeMap::from([(label.to_string(), value.to_string())]) },
            )]),
            reconciliation: BTreeMap::new(),
            all_labels: vec![label.to_string()],
            duplicate_ids: vec![],
            blank_id_rows: 0,
        }
    }

    /// A bundle whose *current* dRofus yields `current_value` (`NetArea`) for
    /// link id "1", with one "Design Freeze" milestone pinning model "m1" to
    /// `pinned_ts` and optionally a `drofus_snapshot`.
    fn bundle_for_drofus_pin(current_value: &str, pinned_ts: &str, drofus_ts: Option<&str>) -> ProjectSettings {
        with_drofus(
            ProjectSettings {
                milestones: vec![crate::settings::Milestone {
                    name: "Design Freeze".to_string(),
                    date: "2026-06-30".to_string(),
                    reference_snapshots: drofus_ts
                        .map(|s| std::collections::BTreeMap::from([("drofus".to_string(), s.to_string())]))
                        .unwrap_or_default(),
                    attachments: std::collections::BTreeMap::from([("m1".to_string(), pinned_ts.to_string())]),
                    door_attachments: std::collections::BTreeMap::new(),
                }],
                ..make_bundle("Number")
            },
            Some(make_drofus_with_record("Number", "1", "NetArea", current_value)),
        )
    }

    /// A two-header-row dRofus CSV pinning link id "1" to one `NetArea` value —
    /// the on-store form a `drofus_snapshot` pin loads and parses.
    fn drofus_csv(net_area: &str) -> Vec<u8> {
        format!("DrofusRoomId,NetArea\nNumber,NetArea\n1,{net_area}\n").into_bytes()
    }

    /// A minimal `ProjectSettings` bundle for tests that only care about the
    /// dRofus link property and the default room label.
    fn make_bundle(link_property: &str) -> ProjectSettings {
        with_drofus(
            ProjectSettings {
                reference: BTreeMap::new(),
                hierarchy: vec![],
                builtin_properties: vec![],
                room_label: vec!["$name".to_string(), "$id".to_string()],
                milestones: vec![],
                comparison_key: None,
                comparison_properties: vec![],
                areas: Default::default(),
                doors: Default::default(),
                hierarchy_exclusions: vec![],
            },
            Some(make_drofus(link_property)),
        )
    }

    /// The two scope dimensions most tests vary; building and filter tests
    /// spell out a `RoomScope` literal instead.
    fn scope<'a>(project: Option<&'a str>, milestone: Option<&'a str>) -> RoomScope<'a> {
        RoomScope { project, milestone, ..Default::default() }
    }

    /// Each contributing model's phase reaches the response, nested per project
    /// and taken from the snapshot rather than the lineage. Two models on
    /// different phases still merge — enforcing agreement would deadlock — so
    /// the field is the only way a consumer can tell it is looking at a plan
    /// spanning two phases, and an unphased model must read as `None` rather
    /// than be silently omitted.
    #[test]
    fn test_assemble_rooms_reports_each_models_phase() {
        let phased = |model: &str, phase: Option<&str>| RoomPayload {
            phase: phase.map(str::to_string),
            ..make_payload("p1", model, vec![], vec![])
        };

        let state = AppState::new(Box::new(MemStore::new()), single_project("p1", make_bundle("Number")), None);
        state.set_snapshot(phased("arch", Some("New Construction"))).unwrap();
        state.set_snapshot(phased("struct", Some("Existing"))).unwrap();
        state.set_snapshot(phased("legacy", None)).unwrap();

        let result = assemble_rooms(&state, &scope(Some("p1"), None)).unwrap().expect("store has data");
        let by_model = result.phase_by_model.get("p1").expect("keyed by project id");

        assert_eq!(by_model["arch"].as_deref(), Some("New Construction"));
        assert_eq!(
            by_model["struct"].as_deref(),
            Some("Existing"),
            "a second phase merges, it is not dropped"
        );
        assert_eq!(by_model["legacy"], None, "a pre-phasing snapshot reports null, not absent");
        assert_eq!(by_model.len(), 3);
    }

    /// The recognised source-name vocabulary every test filter/predicate
    /// parses against — "drofus" is the one source these tests configure.
    fn known() -> BTreeSet<String> {
        BTreeSet::from(["drofus".to_string()])
    }

    /// Parse one predicate expression, panicking on a parse error -- the
    /// matcher tests are about matching, not about parsing.
    fn filter(exprs: &[&str]) -> RoomFilter {
        let owned: Vec<String> = exprs.iter().map(|s| (*s).to_string()).collect();
        RoomFilter::parse(&owned, &known()).expect("test filter must parse")
    }

    /// Registers one project's bundle under its id -- the shape
    /// `AppState::new` now takes in place of the old five flat fields.
    fn single_project(project_id: &str, bundle: ProjectSettings) -> std::collections::HashMap<String, ProjectSettings> {
        std::collections::HashMap::from([(project_id.to_string(), bundle)])
    }

    /// A bundle with a one-tier "Building" hierarchy keyed on `bldg_code`,
    /// for tests exercising the building filter across projects.
    fn make_bundle_with_building_tier() -> ProjectSettings {
        ProjectSettings {
            hierarchy: vec![HierarchyTier {
                name: "Building".to_string(),
                code_property: Some("bldg_code".to_string()),
                name_property: None,
            }],
            ..make_bundle("Number")
        }
    }

    fn make_payload(project_id: &str, model_id: &str, levels: Vec<Level>, rooms: Vec<Room>) -> RoomPayload {
        RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: project_id.to_string(), name: "P".to_string() },
            model: Model { id: model_id.to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
            phase: None,
            model_to_shared: None,
            room_boundary: None,
            levels,
            rooms,
        }
    }

    /// `$name`/`$id` resolve to the room's own fields, not `room.properties`.
    #[test]
    fn test_resolve_label_fields_intrinsic_tokens() {
        let room = make_room("324772", "Room 101", &[]);
        let fields = vec!["$name".to_string(), "$id".to_string()];
        let label = resolve_label_fields(&room, &fields, "revit", &[], &BTreeMap::new(), &BTreeSet::new());
        assert_eq!(label, vec!["Room 101".to_string(), "324772".to_string()]);
    }

    /// Any other configured name falls through to the same canonical/source
    /// resolution dRofus and classification already use.
    #[test]
    fn test_resolve_label_fields_canonical_fallback() {
        let room = make_room("1", "Room", &[("Area", "25.5")]);
        let defs = vec![BuiltinPropertyDef {
            canonical: "Area".to_string(),
            by_source: std::collections::HashMap::from([("revit".to_string(), "Area".to_string())]),
        }];
        let fields = vec!["Area".to_string()];
        let label = resolve_label_fields(&room, &fields, "revit", &defs, &BTreeMap::new(), &BTreeSet::new());
        assert_eq!(label, vec!["25.5".to_string()]);
    }

    /// A configured name that doesn't resolve is silently skipped, not turned
    /// into an empty-string entry.
    #[test]
    fn test_resolve_label_fields_skips_unresolved() {
        let room = make_room("1", "Room", &[]);
        let fields = vec!["$name".to_string(), "Nonexistent".to_string(), "$id".to_string()];
        let label = resolve_label_fields(&room, &fields, "revit", &[], &BTreeMap::new(), &BTreeSet::new());
        assert_eq!(label, vec!["Room".to_string(), "1".to_string()]);
    }

    /// A `<source>.<label>` field reads the joined reference record, so a room
    /// tag can show a CSV column the model itself never carried.
    #[test]
    fn test_resolve_label_fields_reads_joined_reference() {
        let room = make_room("1", "Room", &[]);
        let reference = BTreeMap::from([(
            "sample".to_string(),
            ReferenceRecord { fields: BTreeMap::from([("NetArea".to_string(), "30.5".to_string())]) },
        )]);
        let known = BTreeSet::from(["sample".to_string()]);
        let fields = vec!["sample.NetArea".to_string()];
        let label = resolve_label_fields(&room, &fields, "revit", &[], &reference, &known);
        assert_eq!(label, vec!["30.5".to_string()]);
    }

    /// The rule that makes the vocabulary the *configured* sources rather than
    /// the joined ones: a room that matched no record still reads
    /// `sample.NetArea` as a namespace and resolves it to nothing. It must NOT
    /// fall back to a room property that happens to be spelled that way, or
    /// unmatched rooms would silently follow a different rule from matched ones.
    #[test]
    fn test_resolve_label_fields_unmatched_source_does_not_fall_back_to_property() {
        let room = make_room("1", "Room", &[("sample.NetArea", "decoy")]);
        let defs = vec![BuiltinPropertyDef {
            canonical: "sample.NetArea".to_string(),
            by_source: std::collections::HashMap::from([("revit".to_string(), "sample.NetArea".to_string())]),
        }];
        let known = BTreeSet::from(["sample".to_string()]);
        let fields = vec!["$id".to_string(), "sample.NetArea".to_string()];
        let label = resolve_label_fields(&room, &fields, "revit", &defs, &BTreeMap::new(), &known);
        assert_eq!(label, vec!["1".to_string()], "the decoy property must not stand in for the joined field");
    }

    /// A blank CSV cell contributes nothing, matching `lookup_property`'s
    /// collapse of missing and blank — no empty chip on the room tag.
    #[test]
    fn test_resolve_label_fields_skips_blank_reference_cell() {
        let room = make_room("1", "Room", &[]);
        let reference = BTreeMap::from([(
            "sample".to_string(),
            ReferenceRecord { fields: BTreeMap::from([("NetArea".to_string(), "   ".to_string())]) },
        )]);
        let known = BTreeSet::from(["sample".to_string()]);
        let fields = vec!["$id".to_string(), "sample.NetArea".to_string()];
        let label = resolve_label_fields(&room, &fields, "revit", &[], &reference, &known);
        assert_eq!(label, vec!["1".to_string()]);
    }

    /// Two models under the same project each define "the same" level (same
    /// name, near-identical elevation, different model-local `Level.id`) --
    /// `assemble_rooms` must collapse them into one `Level` in the response
    /// and remap both models' rooms to point at that one canonical id.
    #[test]
    fn test_assemble_rooms_dedups_levels_by_name_and_elevation() {
        let mut room_a = make_room("r1", "Room A", &[]);
        room_a.level_id = "lvlA".to_string();
        let mut room_b = make_room("r2", "Room B", &[]);
        room_b.level_id = "lvlB".to_string();

        let payload_a = RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: "modelA".to_string(), name: "A".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
            phase: None,
            model_to_shared: None,
            room_boundary: None,
            levels: vec![Level { id: "lvlA".to_string(), name: "Level 1".to_string(), elevation: 0.0 }],
            rooms: vec![room_a],
        };
        let payload_b = RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "p1".to_string(), name: "P".to_string() },
            model: Model { id: "modelB".to_string(), name: "B".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:01Z".to_string() },
            phase: None,
            model_to_shared: None,
            room_boundary: None,
            // Same name, elevation drifted by float noise well within tolerance.
            levels: vec![Level { id: "lvlB".to_string(), name: "Level 1".to_string(), elevation: 0.000000001 }],
            rooms: vec![room_b],
        };

        let state = AppState::new(Box::new(MemStore::new()), single_project("p1", make_bundle("Number")), None);
        state.set_snapshot(payload_a).unwrap();
        state.set_snapshot(payload_b).unwrap();

        let result = assemble_rooms(&state, &scope(Some("p1"), None)).unwrap().expect("store has data");

        assert_eq!(result.levels.len(), 1, "same name+elevation levels must collapse to one");

        let canonical_id = result.levels[0].id.clone();
        assert_eq!(result.rooms.len(), 2);
        for room in &result.rooms {
            assert_eq!(room.room.level_id, canonical_id);
        }
    }

    /// The boundary regime is resolved **per canonical level**, and a level fed
    /// by two linked models that disagree keeps **finish face**.
    ///
    /// This is the case the whole per-level shape exists for: level dedup
    /// merges "the same" floor across linked models, so one level really can be
    /// fed by a centreline model and a finish-face one at once. Sizing that
    /// level for centreline would leave the finish-face rooms as disjoint
    /// islands, so the wider regime has to win — and it must win regardless of
    /// which model the store happens to iterate first, which is what the second
    /// level here (the same pair, declared the other way round) pins down.
    #[test]
    fn test_boundary_by_level_resolves_per_level_and_widens_on_disagreement() {
        let level =
            |id: &str, name: &str, elev: f64| Level { id: id.to_string(), name: name.to_string(), elevation: elev };
        let room_on = |id: &str, level_id: &str| {
            let mut r = make_room(id, id, &[]);
            r.level_id = level_id.to_string();
            r
        };
        let payload =
            |model: &str, ts: &str, boundary: Option<RoomBoundary>, levels: Vec<Level>, rooms: Vec<Room>| RoomPayload {
                schema_version: SUPPORTED_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: model.to_string(), name: model.to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: ts.to_string() },
                phase: None,
                model_to_shared: None,
                room_boundary: boundary,
                levels,
                rooms,
            };

        // Level 1 is shared by both models (same name+elevation, model-local
        // ids), one centreline and one finish face. Level 2 belongs to the
        // centreline model alone.
        let state = AppState::new(Box::new(MemStore::new()), single_project("p1", make_bundle("Number")), None);
        state
            .set_snapshot(payload(
                "centreline-model",
                "2026-01-01T00:00:00Z",
                Some(RoomBoundary::Centreline),
                vec![level("a1", "Level 1", 0.0), level("a2", "Level 2", 10.0)],
                vec![room_on("r1", "a1"), room_on("r2", "a2")],
            ))
            .unwrap();
        state
            .set_snapshot(payload(
                "finish-face-model",
                "2026-01-01T00:00:01Z",
                Some(RoomBoundary::FinishFace),
                vec![level("b1", "Level 1", 0.0)],
                vec![room_on("r3", "b1")],
            ))
            .unwrap();

        let result = assemble_rooms(&state, &scope(Some("p1"), None)).unwrap().expect("store has data");
        let by_name = |name: &str| {
            let id = &result.levels.iter().find(|l| l.name == name).expect("level present").id;
            result.boundary_by_level[id]
        };
        assert_eq!(by_name("Level 1"), RoomBoundary::FinishFace, "the mixed level widens");
        assert_eq!(
            by_name("Level 2"),
            RoomBoundary::Centreline,
            "the single-model level keeps its own regime"
        );
        assert_eq!(result.boundary_by_level.len(), result.levels.len(), "every level in scope is covered");
    }

    /// A model that declares nothing resolves through its project's `[areas]`
    /// fallback — and, with no fallback either, to finish face, which is the
    /// behaviour that predates the field entirely.
    #[test]
    fn test_boundary_by_level_falls_back_to_project_policy() {
        let undeclared = |project: &str| RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: project.to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
            phase: None,
            model_to_shared: None,
            room_boundary: None,
            levels: vec![Level { id: "L1".to_string(), name: "Level 1".to_string(), elevation: 0.0 }],
            rooms: vec![make_room("r1", "Room", &[])],
        };

        let with_policy = |boundary: Option<RoomBoundary>| ProjectSettings {
            areas: crate::settings::AreaPolicy { boundary_location: boundary, ..Default::default() },
            ..make_bundle("Number")
        };

        for (fallback, expected) in [
            (Some(RoomBoundary::Centreline), RoomBoundary::Centreline),
            (None, RoomBoundary::FinishFace),
        ] {
            let state = AppState::new(Box::new(MemStore::new()), single_project("p1", with_policy(fallback)), None);
            state.set_snapshot(undeclared("p1")).unwrap();
            let result = assemble_rooms(&state, &scope(Some("p1"), None)).unwrap().expect("store has data");
            assert_eq!(result.boundary_by_level["L1"], expected, "fallback {fallback:?}");
        }
    }

    /// An empty store is reported as `None`, distinct from a filter that
    /// simply matches nothing (which is `Some` with empty vecs).
    #[test]
    fn test_assemble_rooms_reports_store_empty() {
        let state = AppState::new(Box::new(MemStore::new()), single_project("p1", make_bundle("Number")), None);

        let result = assemble_rooms(&state, &RoomScope::default()).unwrap();
        assert!(result.is_none(), "nothing has ever been pushed");
    }

    /// The response revision is stable while idle, moves when a push replaces a
    /// model's snapshot, and moves again when the set of contributing models
    /// changes -- this is the one value the viewer polls on instead of
    /// re-stringifying the whole payload (see `scoped_revision`).
    #[test]
    fn test_assemble_rooms_revision_tracks_pushes() {
        let level = vec![Level { id: "l1".to_string(), name: "Level 1".to_string(), elevation: 0.0 }];
        let state = AppState::new(Box::new(MemStore::new()), single_project("p1", make_bundle("Number")), None);
        state
            .set_snapshot(make_payload("p1", "m1", level.clone(), vec![make_room("r1", "Room A", &[])]))
            .unwrap();

        let rev1 = assemble_rooms(&state, &RoomScope::default()).unwrap().expect("store has data").revision;
        let rev1_again = assemble_rooms(&state, &RoomScope::default()).unwrap().expect("store has data").revision;
        assert_eq!(rev1, rev1_again, "an idle store must return a byte-identical revision every poll");

        // Re-push the same model slot with a newer snapshot id: revision moves.
        let mut newer = make_payload("p1", "m1", level.clone(), vec![make_room("r1", "Room A", &[])]);
        newer.snapshot.taken_at = "2026-02-02T00:00:00Z".to_string();
        state.set_snapshot(newer).unwrap();
        let rev2 = assemble_rooms(&state, &RoomScope::default()).unwrap().expect("store has data").revision;
        assert_ne!(rev1, rev2, "a new snapshot for a model must change the revision");

        // A second contributing model changes the set, hence the revision again.
        state
            .set_snapshot(make_payload("p1", "m2", level, vec![make_room("r2", "Room B", &[])]))
            .unwrap();
        let rev3 = assemble_rooms(&state, &RoomScope::default()).unwrap().expect("store has data").revision;
        assert_ne!(rev2, rev3, "adding a contributing model must change the revision");
    }

    /// A payload whose project has no registered settings (and no default
    /// bundle configured) is skipped from an unscoped merge entirely -- it's
    /// not enough for the store to be non-empty; the project must actually be
    /// registered for its rooms to appear.
    #[test]
    fn test_assemble_rooms_skips_unregistered_project() {
        let payload = RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: "unregistered".to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
            phase: None,
            model_to_shared: None,
            room_boundary: None,
            levels: vec![Level { id: "l1".to_string(), name: "Level 1".to_string(), elevation: 0.0 }],
            rooms: vec![make_room("r1", "Room A", &[])],
        };

        // Registry only knows "p1" -- "unregistered" has no bundle and no
        // default is configured.
        let state = AppState::new(Box::new(MemStore::new()), single_project("p1", make_bundle("Number")), None);
        state.set_snapshot(payload).unwrap();

        let result = assemble_rooms(&state, &RoomScope::default())
            .unwrap()
            .expect("the store did receive a push");
        assert!(result.rooms.is_empty(), "but the unregistered project's rooms must not appear");
        assert!(result.levels.is_empty());
    }

    /// Two *different* projects each define "Level 1" @ 0.0 -- an unscoped
    /// merge must keep both levels (level identity is only meaningful within
    /// a project), and each project's room must keep a level id minted from
    /// its own project's model, never remapped onto the other project's.
    #[test]
    fn test_assemble_rooms_level_dedup_does_not_cross_projects() {
        let mut room_a = make_room("r1", "Room A", &[]);
        room_a.level_id = "lvlA".to_string();
        let mut room_b = make_room("r2", "Room B", &[]);
        room_b.level_id = "lvlB".to_string();

        let payload_a = make_payload(
            "p1",
            "modelA",
            vec![Level { id: "lvlA".to_string(), name: "Level 1".to_string(), elevation: 0.0 }],
            vec![room_a],
        );
        let payload_b = make_payload(
            "p2",
            "modelB",
            vec![Level { id: "lvlB".to_string(), name: "Level 1".to_string(), elevation: 0.0 }],
            vec![room_b],
        );

        let registry = std::collections::HashMap::from([
            ("p1".to_string(), make_bundle("Number")),
            ("p2".to_string(), make_bundle("Number")),
        ]);
        let state = AppState::new(Box::new(MemStore::new()), registry, None);
        state.set_snapshot(payload_a).unwrap();
        state.set_snapshot(payload_b).unwrap();

        let result = assemble_rooms(&state, &RoomScope::default()).unwrap().expect("store has data");

        assert_eq!(result.levels.len(), 2, "same (name, elevation) in different projects must NOT collapse");
        assert_eq!(result.rooms.len(), 2);
        for room in &result.rooms {
            let expected = if room.room.id == "r1" { "lvlA" } else { "lvlB" };
            assert_eq!(room.room.level_id, expected, "each room keeps its own project's level id");
        }
    }

    /// Unscoped merge with a building filter: project A (Building tier, room
    /// in building B01) contributes its matching room and its levels; project
    /// B (no hierarchy at all) can't answer a building question, so it
    /// contributes nothing -- neither rooms nor levels.
    #[test]
    fn test_assemble_rooms_building_filter_excludes_tierless_project() {
        let mut room_a = make_room("r1", "Room A", &[("bldg_code", "B01")]);
        room_a.level_id = "lvlA".to_string();
        let mut room_b = make_room("r2", "Room B", &[]);
        room_b.level_id = "lvlB".to_string();

        let payload_a = make_payload(
            "p1",
            "modelA",
            vec![Level { id: "lvlA".to_string(), name: "Level 1".to_string(), elevation: 0.0 }],
            vec![room_a],
        );
        let payload_b = make_payload(
            "p2",
            "modelB",
            vec![Level { id: "lvlB".to_string(), name: "Level 9".to_string(), elevation: 30.0 }],
            vec![room_b],
        );

        let registry = std::collections::HashMap::from([
            ("p1".to_string(), make_bundle_with_building_tier()),
            ("p2".to_string(), make_bundle("Number")), // no hierarchy
        ]);
        let state = AppState::new(Box::new(MemStore::new()), registry, None);
        state.set_snapshot(payload_a).unwrap();
        state.set_snapshot(payload_b).unwrap();

        let key = building_key(&Some("B01".to_string()), &None);
        let result = assemble_rooms(&state, &RoomScope { building: Some(&key), ..Default::default() })
            .unwrap()
            .expect("store has data");

        assert_eq!(result.rooms.len(), 1, "only project A's matching room");
        assert_eq!(result.rooms[0].room.id, "r1");
        assert_eq!(result.levels.len(), 1, "only project A's levels");
        assert_eq!(result.levels[0].name, "Level 1");
    }

    /// A bundle defining one milestone that pins model "m1" to `pinned_ts`.
    fn make_bundle_with_milestone(pinned_ts: &str) -> ProjectSettings {
        make_bundle_with_milestone_drofus(pinned_ts, None)
    }

    /// Like `make_bundle_with_milestone`, but the milestone also pins a
    /// `drofus_snapshot` when `drofus_ts` is `Some`.
    fn make_bundle_with_milestone_drofus(pinned_ts: &str, drofus_ts: Option<&str>) -> ProjectSettings {
        ProjectSettings {
            milestones: vec![crate::settings::Milestone {
                name: "Design Freeze".to_string(),
                date: "2026-06-30".to_string(),
                reference_snapshots: drofus_ts
                    .map(|s| std::collections::BTreeMap::from([("drofus".to_string(), s.to_string())]))
                    .unwrap_or_default(),
                attachments: std::collections::BTreeMap::from([("m1".to_string(), pinned_ts.to_string())]),
                door_attachments: std::collections::BTreeMap::new(),
            }],
            ..make_bundle("Number")
        }
    }

    /// A milestone view serves the *pinned* (older) snapshot's rooms while
    /// the default view keeps serving the latest — the core milestone
    /// behavior. Uses FsStore because pinning to history needs a store that
    /// actually keeps it.
    #[test]
    fn test_assemble_rooms_milestone_serves_pinned_snapshot() {
        let dir = std::env::temp_dir().join(format!("roommate-ms-pin-{}", std::process::id()));
        let store = crate::storage::FsStore::new(dir.clone()).unwrap();

        let old_ts = "2026-06-01T00:00:00Z";
        let mut old = make_payload("p1", "m1", vec![], vec![make_room("r1", "Old Room", &[])]);
        old.snapshot.taken_at = old_ts.to_string();
        let mut new = make_payload("p1", "m1", vec![], vec![make_room("r2", "New Room", &[])]);
        new.snapshot.taken_at = "2026-07-01T00:00:00Z".to_string();

        let state = AppState::new(Box::new(store), single_project("p1", make_bundle_with_milestone(old_ts)), None);
        state.set_snapshot(old).unwrap();
        state.set_snapshot(new).unwrap();

        let latest = assemble_rooms(&state, &scope(Some("p1"), None)).unwrap().expect("store has data");
        assert_eq!(latest.rooms.len(), 1);
        assert_eq!(latest.rooms[0].room.name, "New Room");

        let pinned = assemble_rooms(&state, &scope(Some("p1"), Some("Design Freeze")))
            .unwrap()
            .expect("store has data");
        assert_eq!(pinned.rooms.len(), 1);
        assert_eq!(pinned.rooms[0].room.name, "Old Room", "milestone view serves the pinned snapshot");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Under a milestone filter, a model the milestone doesn't pin
    /// contributes nothing, and a project defining no milestone of that name
    /// contributes nothing at all — same discipline as the building filter.
    #[test]
    fn test_assemble_rooms_milestone_excludes_unpinned_and_unknown() {
        let dir = std::env::temp_dir().join(format!("roommate-ms-excl-{}", std::process::id()));
        let store = crate::storage::FsStore::new(dir.clone()).unwrap();

        let ts = "2026-06-01T00:00:00Z";
        let mut pinned_model = make_payload("p1", "m1", vec![], vec![make_room("r1", "Pinned", &[])]);
        pinned_model.snapshot.taken_at = ts.to_string();
        let mut unpinned_model = make_payload("p1", "m2", vec![], vec![make_room("r2", "Unpinned", &[])]);
        unpinned_model.snapshot.taken_at = ts.to_string();

        let state = AppState::new(Box::new(store), single_project("p1", make_bundle_with_milestone(ts)), None);
        state.set_snapshot(pinned_model).unwrap();
        state.set_snapshot(unpinned_model).unwrap();

        let result = assemble_rooms(&state, &scope(Some("p1"), Some("Design Freeze")))
            .unwrap()
            .expect("store has data");
        assert_eq!(result.rooms.len(), 1, "only the pinned model contributes");
        assert_eq!(result.rooms[0].room.name, "Pinned");

        // A milestone name this project never defined matches nothing.
        let unknown = assemble_rooms(&state, &scope(Some("p1"), Some("Nonexistent")))
            .unwrap()
            .expect("store has data");
        assert!(unknown.rooms.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The whole dRofus-pinning feature in one test: a milestone that pins a
    /// `drofus_snapshot` joins that stored CSV, while the default (latest) view
    /// joins the project's current dRofus — same room, different join source.
    #[test]
    fn test_assemble_rooms_milestone_joins_pinned_drofus() {
        let dir = std::env::temp_dir().join(format!("roommate-ms-drofus-{}", std::process::id()));
        let store = crate::storage::FsStore::new(dir.clone()).unwrap();

        let old_model_ts = "2026-06-01T00:00:00Z";
        let old_drofus_ts = "2026-06-01T09:00:00Z";
        // Same room (link id "1") in both snapshots, so only the dRofus differs.
        let mut old = make_payload("p1", "m1", vec![], vec![make_room("r1", "Room", &[("Number", "1")])]);
        old.snapshot.taken_at = old_model_ts.to_string();
        let mut new = make_payload("p1", "m1", vec![], vec![make_room("r1", "Room", &[("Number", "1")])]);
        new.snapshot.taken_at = "2026-07-01T00:00:00Z".to_string();

        // Current dRofus yields "new-value"; the pinned CSV yields "old-value".
        let bundle = bundle_for_drofus_pin("new-value", old_model_ts, Some(old_drofus_ts));
        let state = AppState::new(Box::new(store), single_project("p1", bundle), None);
        state.set_snapshot(old).unwrap();
        state.set_snapshot(new).unwrap();
        state.put_reference("p1", "drofus", old_drofus_ts, &drofus_csv("old-value")).unwrap();

        let latest = assemble_rooms(&state, &scope(Some("p1"), None)).unwrap().expect("store has data");
        assert_eq!(
            latest.rooms[0].reference.get("drofus").unwrap().fields.get("NetArea"),
            Some(&"new-value".to_string()),
            "default view joins the current dRofus"
        );

        let pinned = assemble_rooms(&state, &scope(Some("p1"), Some("Design Freeze")))
            .unwrap()
            .expect("store has data");
        assert_eq!(
            pinned.rooms[0].reference.get("drofus").unwrap().fields.get("NetArea"),
            Some(&"old-value".to_string()),
            "milestone view joins the pinned dRofus snapshot"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `drofus_snapshot` pointing at an id that was never uploaded falls back
    /// to the current dRofus with a warning — the room is still returned, not
    /// dropped (dRofus is a join, not the room itself).
    #[test]
    fn test_assemble_rooms_milestone_missing_drofus_pin_falls_back() {
        let dir = std::env::temp_dir().join(format!("roommate-ms-drofus-miss-{}", std::process::id()));
        let store = crate::storage::FsStore::new(dir.clone()).unwrap();

        let model_ts = "2026-06-01T00:00:00Z";
        let mut pinned_model = make_payload("p1", "m1", vec![], vec![make_room("r1", "Room", &[("Number", "1")])]);
        pinned_model.snapshot.taken_at = model_ts.to_string();

        // Pins a dRofus id that is never put into the store.
        let bundle = bundle_for_drofus_pin("current-value", model_ts, Some("2026-01-01T00:00:00Z"));
        let state = AppState::new(Box::new(store), single_project("p1", bundle), None);
        state.set_snapshot(pinned_model).unwrap();

        let result = assemble_rooms(&state, &scope(Some("p1"), Some("Design Freeze")))
            .unwrap()
            .expect("store has data");
        assert_eq!(result.rooms.len(), 1, "the room is still returned (fallback, not dropped)");
        assert_eq!(
            result.rooms[0].reference.get("drofus").unwrap().fields.get("NetArea"),
            Some(&"current-value".to_string()),
            "falls back to the current dRofus"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A milestone with model pins but no `drofus_snapshot` joins the current
    /// dRofus — guards the default (pre-pinning) path.
    #[test]
    fn test_assemble_rooms_milestone_without_drofus_pin_uses_current() {
        let dir = std::env::temp_dir().join(format!("roommate-ms-drofus-none-{}", std::process::id()));
        let store = crate::storage::FsStore::new(dir.clone()).unwrap();

        let model_ts = "2026-06-01T00:00:00Z";
        let mut pinned_model = make_payload("p1", "m1", vec![], vec![make_room("r1", "Room", &[("Number", "1")])]);
        pinned_model.snapshot.taken_at = model_ts.to_string();

        let bundle = bundle_for_drofus_pin("current-value", model_ts, None);
        let state = AppState::new(Box::new(store), single_project("p1", bundle), None);
        state.set_snapshot(pinned_model).unwrap();

        let result = assemble_rooms(&state, &scope(Some("p1"), Some("Design Freeze")))
            .unwrap()
            .expect("store has data");
        assert_eq!(
            result.rooms[0].reference.get("drofus").unwrap().fields.get("NetArea"),
            Some(&"current-value".to_string())
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Multi-project isolation: in an unscoped `?milestone=` merge, project A's
    /// pinned dRofus must not leak onto project B's rooms — B keeps its own
    /// current dRofus.
    #[test]
    fn test_assemble_rooms_milestone_drofus_pin_does_not_cross_projects() {
        let dir = std::env::temp_dir().join(format!("roommate-ms-drofus-iso-{}", std::process::id()));
        let store = crate::storage::FsStore::new(dir.clone()).unwrap();

        let model_ts = "2026-06-01T00:00:00Z";
        let a_drofus_ts = "2026-06-01T09:00:00Z";

        let mut a = make_payload("pA", "m1", vec![], vec![make_room("rA", "Room A", &[("Number", "1")])]);
        a.snapshot.taken_at = model_ts.to_string();
        let mut b = make_payload("pB", "m1", vec![], vec![make_room("rB", "Room B", &[("Number", "1")])]);
        b.snapshot.taken_at = model_ts.to_string();

        // A pins a dRofus snapshot; B has no pin, so its current dRofus stands.
        let registry = std::collections::HashMap::from([
            ("pA".to_string(), bundle_for_drofus_pin("A-current", model_ts, Some(a_drofus_ts))),
            ("pB".to_string(), bundle_for_drofus_pin("B-current", model_ts, None)),
        ]);
        let state = AppState::new(Box::new(store), registry, None);
        state.set_snapshot(a).unwrap();
        state.set_snapshot(b).unwrap();
        state.put_reference("pA", "drofus", a_drofus_ts, &drofus_csv("A-pinned")).unwrap();

        let result = assemble_rooms(&state, &scope(None, Some("Design Freeze")))
            .unwrap()
            .expect("store has data");
        let room_a = result.rooms.iter().find(|r| r.room.id == "rA").expect("A present");
        let room_b = result.rooms.iter().find(|r| r.room.id == "rB").expect("B present");
        assert_eq!(
            room_a.reference.get("drofus").unwrap().fields.get("NetArea"),
            Some(&"A-pinned".to_string()),
            "A joins its own pinned dRofus"
        );
        assert_eq!(
            room_b.reference.get("drofus").unwrap().fields.get("NetArea"),
            Some(&"B-current".to_string()),
            "B keeps its current dRofus — A's pin did not leak across"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Scoped to a project with no Building tier while a building filter is
    /// active: the project can't answer the question, so the result is empty
    /// (not the project's whole room set) -- but the store is not empty.
    #[test]
    fn test_assemble_rooms_building_filter_on_tierless_project_is_empty() {
        let mut room_b = make_room("r2", "Room B", &[]);
        room_b.level_id = "lvlB".to_string();
        let payload_b = make_payload(
            "p2",
            "modelB",
            vec![Level { id: "lvlB".to_string(), name: "Level 9".to_string(), elevation: 30.0 }],
            vec![room_b],
        );

        let state = AppState::new(Box::new(MemStore::new()), single_project("p2", make_bundle("Number")), None);
        state.set_snapshot(payload_b).unwrap();

        let key = building_key(&Some("B01".to_string()), &None);
        let result =
            assemble_rooms(&state, &RoomScope { project: Some("p2"), building: Some(&key), ..Default::default() })
                .unwrap()
                .expect("store is not empty, so this is Some with empty vecs");

        assert!(result.rooms.is_empty(), "a filter the project can't answer matches nothing");
        assert!(result.levels.is_empty());
    }

    /// A `RoomResponse` as `assemble_room` would produce it -- for the matcher
    /// tests, which are about matching rather than about assembly.
    fn response(room: Room, drofus: Option<ReferenceRecord>) -> RoomResponse {
        let reference = drofus.map(|d| BTreeMap::from([("drofus".to_string(), d)])).unwrap_or_default();
        RoomResponse {
            room,
            reference,
            classification: vec![],
            label: vec![],
            source: "revit".to_string(),
            project_id: "p1".to_string(),
            model_id: "m1".to_string(),
        }
    }

    /// Every operator, including the two spellings a naive left-to-right scan
    /// would mis-split: `>=` must not read as `>` with the value "=20", and
    /// `!=` must not read as `=` with the field "Area!".
    #[test]
    fn test_predicate_parse_operators() {
        let cases = [
            ("Area=20", Op::Eq, "Area", "20"),
            ("Area!=20", Op::Ne, "Area", "20"),
            ("Area>20", Op::Gt, "Area", "20"),
            ("Area>=20", Op::Ge, "Area", "20"),
            ("Area<20", Op::Lt, "Area", "20"),
            ("Area<=20", Op::Le, "Area", "20"),
            ("Name~ward", Op::Contains, "Name", "ward"),
        ];
        for (expr, op, property, value) in cases {
            let p = Predicate::parse(expr, &known()).unwrap_or_else(|e| panic!("{expr:?} must parse: {e}"));
            assert_eq!((p.op, p.property.as_str(), p.value.as_str()), (op, property, value), "{expr:?}");
        }
    }

    /// Surrounding whitespace is trimmed and a quoted value keeps its inner
    /// separator -- the escape hatch that makes the HTTP comma-separated form
    /// able to express a value containing a comma.
    #[test]
    fn test_predicate_parse_trims_and_unquotes() {
        let p = Predicate::parse("  Department = \"Cardiology, North\"  ", &known()).expect("must parse");
        assert_eq!(p.property, "Department");
        assert_eq!(p.value, "Cardiology, North");
    }

    /// Each malformed shape is rejected with a message rather than silently
    /// becoming a predicate that could never match.
    #[test]
    fn test_predicate_parse_rejects_malformed() {
        for expr in ["Department", "=Cardiology", "Department="] {
            assert!(Predicate::parse(expr, &known()).is_err(), "{expr:?} must not parse");
        }
    }

    /// A known namespace binds as a joined source; an unknown one is an error
    /// naming the known sources, never a silent fallback to a room property --
    /// which is what stops a future source from changing what an existing
    /// filter means.
    #[test]
    fn test_predicate_parse_binds_known_namespace_only() {
        let p = Predicate::parse("drofus.NetArea>20", &known()).expect("must parse");
        assert_eq!(p.source.as_deref(), Some("drofus"));
        assert_eq!(p.property, "NetArea");

        let err =
            Predicate::parse("cobie.Space=1", &known()).expect_err("an unknown source must not become a property");
        assert!(err.contains("drofus"), "the error must name the known sources, got {err:?}");
    }

    /// The settings-side namespace check applies the same split rule as the
    /// filter parser (both go through `split_namespace`): known namespaces and
    /// unqualified names pass, an unknown namespace and an empty property
    /// after the dot are rejected with a message naming the known sources.
    #[test]
    fn test_validate_namespaced_field() {
        assert!(validate_namespaced_field("Area", &known()).is_ok());
        assert!(validate_namespaced_field("drofus.NetArea", &known()).is_ok());
        // A dot inside a spaced name stays part of the property name — the
        // same subtlety `Predicate::parse` applies, via the same helper.
        assert!(validate_namespaced_field("Room Ref. Number", &known()).is_ok());
        // The room-label intrinsics need no special case: no dot, so they
        // split as `Unqualified` and pass straight through.
        assert!(validate_namespaced_field("$name", &known()).is_ok());

        let err =
            validate_namespaced_field("drofuss.NetArea", &known()).expect_err("unknown namespace must be rejected");
        assert!(
            err.contains("unknown data source") && err.contains("drofus"),
            "names the known sources: {err:?}"
        );

        let err =
            validate_namespaced_field("drofus.", &known()).expect_err("empty property after the dot must be rejected");
        assert!(err.contains("drofus"), "names the namespace: {err:?}");
    }

    /// The inverse rule for a room-only surface: a *known* source is the
    /// rejection (classification can't read one), while an unrecognised word
    /// before a dot stays a legal property name — the opposite verdict on both
    /// inputs from `validate_namespaced_field`, which is the whole point of
    /// having two.
    #[test]
    fn test_validate_room_only_field() {
        assert!(validate_room_only_field("Department", &known()).is_ok());
        assert!(
            validate_room_only_field("Dept.Code", &known()).is_ok(),
            "unknown prefix is just a dotted name here"
        );

        let err =
            validate_room_only_field("drofus.Department", &known()).expect_err("a tier must not name a joined source");
        assert!(err.contains("drofus") && err.contains("undefined"), "explains the consequence: {err:?}");
    }

    /// `resolve_presence` spans both vocabularies: unqualified names resolve
    /// against the room (canonical mapping included), `drofus.`-qualified ones
    /// against the joined record, and an unjoined room is `Absent` on every
    /// field of that source — with `source_joined` telling that state apart
    /// from a joined-but-fieldless record.
    #[test]
    fn test_resolve_presence_spans_room_and_joined_vocabularies() {
        let joined = response(
            make_room("r1", "Room", &[("Area", "25")]),
            Some(ReferenceRecord {
                fields: std::collections::BTreeMap::from([("NetArea".to_string(), "20".to_string())]),
            }),
        );
        assert_eq!(
            resolve_presence(&joined, "Area", &known(), &[]),
            (None, PropertyPresence::Present("25".to_string()))
        );
        assert_eq!(
            resolve_presence(&joined, "drofus.NetArea", &known(), &[]),
            (Some("drofus".to_string()), PropertyPresence::Present("20".to_string()))
        );
        assert_eq!(
            resolve_presence(&joined, "drofus.Dept", &known(), &[]),
            (Some("drofus".to_string()), PropertyPresence::Absent)
        );
        assert!(source_joined(&joined, "drofus"));

        let unjoined = response(make_room("r2", "Room", &[]), None);
        assert_eq!(
            resolve_presence(&unjoined, "drofus.NetArea", &known(), &[]),
            (Some("drofus".to_string()), PropertyPresence::Absent)
        );
        assert!(!source_joined(&unjoined, "drofus"));
    }

    /// The HTTP form splits on commas that aren't inside quotes.
    #[test]
    fn test_filter_parse_query_splits_on_unquoted_commas_only() {
        let f = RoomFilter::parse_query("Department=\"Cardiology, North\",Area>20", &known()).expect("must parse");
        assert_eq!(f.predicates.len(), 2);
        assert_eq!(f.predicates[0].value, "Cardiology, North");
        assert_eq!(f.predicates[1].op, Op::Gt);
    }

    /// Canonical names resolve through the project's `by_source` mapping, the
    /// same resolution the dRofus join and the room label already use -- so a
    /// filter means the same thing everywhere.
    #[test]
    fn test_filter_matches_resolves_canonical_name_per_source() {
        let defs = vec![BuiltinPropertyDef {
            canonical: "Department".to_string(),
            by_source: std::collections::HashMap::from([("revit".to_string(), "Dept".to_string())]),
        }];
        let room = response(make_room("r1", "Room", &[("Dept", "Cardiology")]), None);
        assert!(filter(&["Department=Cardiology"]).matches(&room, &defs));
        assert!(!filter(&["Department=Radiology"]).matches(&room, &defs));
    }

    /// `$name`/`$id` reach the room's own fields, which `lookup_property`
    /// cannot see.
    #[test]
    fn test_filter_matches_intrinsic_tokens() {
        let room = response(make_room("324772", "Ward 3", &[]), None);
        assert!(filter(&["$id=324772"]).matches(&room, &[]));
        assert!(filter(&["$name~ward"]).matches(&room, &[]), "~ is case-insensitive");
        assert!(!filter(&["$name=ward 3"]).matches(&room, &[]), "= is not");
    }

    /// `=` inherits the stated-precision tolerance dRofus comparison uses, so
    /// a value authored as "25.50" answers a query for 25.5; an ordering
    /// operator against a non-numeric value is a no-match, not an error.
    #[test]
    fn test_filter_matches_numeric_tolerance_and_ordering() {
        let room = response(make_room("r1", "Room", &[("Area", "25.50"), ("Dept", "Cardiology")]), None);
        assert!(filter(&["Area=25.5"]).matches(&room, &[]));
        assert!(filter(&["Area>25"]).matches(&room, &[]));
        assert!(!filter(&["Area>26"]).matches(&room, &[]));
        assert!(
            !filter(&["Dept>5"]).matches(&room, &[]),
            "non-numeric under an ordering operator: no match, no error"
        );
    }

    /// The rule that makes an empty result readable: a room missing the field
    /// fails EVERY operator, `!=` included -- "no Department" is not evidence
    /// that the Department differs from Cardiology.
    #[test]
    fn test_filter_matches_absent_and_empty_never_match() {
        let absent = response(make_room("r1", "Room", &[]), None);
        let empty = response(make_room("r2", "Room", &[("Department", "")]), None);
        for room in [&absent, &empty] {
            assert!(!filter(&["Department=Cardiology"]).matches(room, &[]));
            assert!(!filter(&["Department!=Cardiology"]).matches(room, &[]));
            assert!(!filter(&["Department~card"]).matches(room, &[]));
        }
    }

    /// A `drofus.`-qualified predicate reads the joined record's own field
    /// labels; a room whose link value matched no record fails both the
    /// positive and the negative form (an unmatched key is a signal, not a
    /// value).
    #[test]
    fn test_filter_matches_joined_drofus_fields() {
        let record = ReferenceRecord { fields: BTreeMap::from([("NetArea".to_string(), "30".to_string())]) };
        let joined = response(make_room("r1", "Room", &[]), Some(record));
        assert!(filter(&["drofus.NetArea>20"]).matches(&joined, &[]));
        assert!(!filter(&["drofus.NetArea>40"]).matches(&joined, &[]));

        let unmatched = response(make_room("r2", "Room", &[]), None);
        assert!(!filter(&["drofus.NetArea=30"]).matches(&unmatched, &[]));
        assert!(!filter(&["drofus.NetArea!=30"]).matches(&unmatched, &[]));
    }

    /// Predicates AND, and each project resolves the filtered name through its
    /// OWN bundle: two projects mapping the same canonical name to different
    /// raw properties both answer correctly inside one unscoped merge.
    #[test]
    fn test_assemble_rooms_filter_ands_and_resolves_per_project() {
        let a = make_payload(
            "pA",
            "m1",
            vec![],
            vec![
                make_room("rA1", "Room", &[("Dept", "Cardiology"), ("Area", "30")]),
                make_room("rA2", "Room", &[("Dept", "Cardiology"), ("Area", "10")]),
            ],
        );
        let b = make_payload(
            "pB",
            "m1",
            vec![],
            vec![make_room(
                "rB1",
                "Room",
                &[("Department", "Cardiology"), ("Area", "40")],
            )],
        );

        let mapped = ProjectSettings {
            builtin_properties: vec![BuiltinPropertyDef {
                canonical: "Department".to_string(),
                by_source: std::collections::HashMap::from([("revit".to_string(), "Dept".to_string())]),
            }],
            ..make_bundle("Number")
        };
        let registry =
            std::collections::HashMap::from([("pA".to_string(), mapped), ("pB".to_string(), make_bundle("Number"))]);
        let state = AppState::new(Box::new(MemStore::new()), registry, None);
        state.set_snapshot(a).unwrap();
        state.set_snapshot(b).unwrap();

        let f = filter(&["Department=Cardiology", "Area>20"]);
        let result = assemble_rooms(&state, &RoomScope { filter: Some(&f), ..Default::default() })
            .unwrap()
            .expect("store has data");

        let mut ids: Vec<&str> = result.rooms.iter().map(|r| r.room.id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            vec!["rA1", "rB1"],
            "rA2 fails the area predicate; both projects resolve Department their own way"
        );
    }

    /// A model whose rooms all fail the filter contributes no levels either --
    /// the building filter's rule, now counting POST-filter rooms (the phase-3
    /// reordering this filter required).
    #[test]
    fn test_assemble_rooms_filter_suppresses_levels_of_non_contributing_model() {
        let mut room_a = make_room("rA", "Room A", &[("Department", "Cardiology")]);
        room_a.level_id = "lvlA".to_string();
        let mut room_b = make_room("rB", "Room B", &[("Department", "Radiology")]);
        room_b.level_id = "lvlB".to_string();

        let state = AppState::new(Box::new(MemStore::new()), single_project("p1", make_bundle("Number")), None);
        state
            .set_snapshot(make_payload(
                "p1",
                "mA",
                vec![Level { id: "lvlA".to_string(), name: "Level 1".to_string(), elevation: 0.0 }],
                vec![room_a],
            ))
            .unwrap();
        state
            .set_snapshot(make_payload(
                "p1",
                "mB",
                vec![Level { id: "lvlB".to_string(), name: "Level 9".to_string(), elevation: 30.0 }],
                vec![room_b],
            ))
            .unwrap();

        let f = filter(&["Department=Cardiology"]);
        let result = assemble_rooms(&state, &RoomScope { project: Some("p1"), filter: Some(&f), ..Default::default() })
            .unwrap()
            .expect("store has data");

        assert_eq!(result.rooms.len(), 1);
        assert_eq!(
            result.levels.len(),
            1,
            "model mB contributed no matching room, so none of its levels either"
        );
        assert_eq!(result.levels[0].name, "Level 1");
    }

    /// Building and property scopes both apply, not either.
    #[test]
    fn test_assemble_rooms_filter_composes_with_building() {
        let rooms = vec![
            make_room("r1", "A", &[("bldg_code", "B01"), ("Department", "Cardiology")]),
            make_room("r2", "B", &[("bldg_code", "B01"), ("Department", "Radiology")]),
            make_room("r3", "C", &[("bldg_code", "B02"), ("Department", "Cardiology")]),
        ];
        let state =
            AppState::new(Box::new(MemStore::new()), single_project("p1", make_bundle_with_building_tier()), None);
        state.set_snapshot(make_payload("p1", "m1", vec![], rooms)).unwrap();

        let key = building_key(&Some("B01".to_string()), &None);
        let f = filter(&["Department=Cardiology"]);
        let result = assemble_rooms(
            &state,
            &RoomScope {
                project: Some("p1"),
                building: Some(&key),
                filter: Some(&f),
                ..Default::default()
            },
        )
        .unwrap()
        .expect("store has data");

        assert_eq!(result.rooms.len(), 1);
        assert_eq!(result.rooms[0].room.id, "r1");
    }

    /// The response carries each project's dRofus column vocabulary — every
    /// row-1 label including one matching no room in scope (the column a
    /// consumer could never discover by unioning per-room `fields`), plus the
    /// row-2 reconciliation subset.
    #[test]
    fn test_rooms_result_carries_project_drofus_labels() {
        let mut drofus = make_drofus_with_record("Number", "1", "NetArea", "20");
        drofus.all_labels = vec!["NetArea".to_string(), "UnmatchedCol".to_string()];
        drofus.reconciliation = BTreeMap::from([("NetArea".to_string(), "Area".to_string())]);
        let bundle = with_drofus(make_bundle("Number"), Some(drofus));
        let state = AppState::new(Box::new(MemStore::new()), single_project("p1", bundle), None);
        // The room carries no link value, so NOTHING joins: the labels must
        // come from the dataset, not from any room's joined fields.
        state
            .set_snapshot(make_payload("p1", "m1", vec![], vec![make_room("r1", "Room", &[])]))
            .unwrap();

        let result = assemble_rooms(&state, &scope(Some("p1"), None)).unwrap().expect("store has data");

        let labels = &result.reference_labels.get("p1").expect("one entry for the project")["drofus"];
        assert_eq!(labels.all_labels, vec!["NetArea".to_string(), "UnmatchedCol".to_string()]);
        assert_eq!(labels.reconciliation.get("NetArea").map(String::as_str), Some("Area"));
    }

    /// An unscoped merge spans projects, so the label sets are keyed per
    /// project — a flat list would silently mean "some project's labels". A
    /// project with no dRofus has no entry at all (absent, not empty).
    #[test]
    fn test_drofus_labels_keyed_per_project() {
        let registry = std::collections::HashMap::from([
            (
                "p1".to_string(),
                with_drofus(make_bundle("Number"), Some(make_drofus_with_record("Number", "1", "ColA", "x"))),
            ),
            (
                "p2".to_string(),
                with_drofus(make_bundle("Number"), Some(make_drofus_with_record("Number", "1", "ColB", "y"))),
            ),
            ("p3".to_string(), with_drofus(make_bundle("Number"), None)),
        ]);
        let state = AppState::new(Box::new(MemStore::new()), registry, None);
        for p in ["p1", "p2", "p3"] {
            state
                .set_snapshot(make_payload(p, "m1", vec![], vec![make_room("r1", "Room", &[])]))
                .unwrap();
        }

        let result = assemble_rooms(&state, &scope(None, None)).unwrap().expect("store has data");

        assert_eq!(result.reference_labels.len(), 2, "one entry per dRofus-bearing project");
        assert_eq!(result.reference_labels["p1"]["drofus"].all_labels, vec!["ColA".to_string()]);
        assert_eq!(result.reference_labels["p2"]["drofus"].all_labels, vec!["ColB".to_string()]);
        assert!(!result.reference_labels.contains_key("p3"), "no dRofus, no entry");
    }

    /// Two configured reference sources, on different projects, prove the
    /// registry-wide known-source vocabulary end to end: a `doors`-qualified
    /// filter joins and matches only the project that actually configures
    /// "doors", an unrecognised namespace still errors naming both known
    /// sources, and — the fallback `presence_of`'s map lookup already
    /// provides for free — a project that doesn't configure "doors" treats it
    /// as *recognised but absent* rather than an error, so an unscoped query
    /// never errors just because one project out of several configures a
    /// source another doesn't.
    #[test]
    fn test_multiple_reference_sources_across_projects() {
        let mut p2_bundle = make_bundle("Number");
        p2_bundle.reference.insert(
            "doors".to_string(),
            crate::state::ProjectReferenceSource {
                entity: crate::settings::ReferenceEntity::Rooms,
                data: Some(make_drofus_with_record("DoorKey", "D1", "Mark", "101A")),
                fields: vec![],
            },
        );
        let registry = std::collections::HashMap::from([
            ("p1".to_string(), make_bundle("Number")), // only "drofus" configured
            ("p2".to_string(), p2_bundle),             // "drofus" AND "doors" configured
        ]);
        let state = AppState::new(Box::new(MemStore::new()), registry, None);
        state
            .set_snapshot(make_payload("p1", "m1", vec![], vec![make_room("r1", "Room", &[("Number", "1")])]))
            .unwrap();
        state
            .set_snapshot(make_payload(
                "p2",
                "m1",
                vec![],
                vec![make_room("r2", "Room", &[("Number", "1"), ("DoorKey", "D1")])],
            ))
            .unwrap();

        let known: BTreeSet<String> = BTreeSet::from(["drofus".to_string(), "doors".to_string()]);

        // Unscoped, "doors.Mark=101A": matches only p2's room. p1's room has
        // no "doors" join at all — `Absent`, not an error, even though p1
        // never configures that source.
        let f = RoomFilter::parse_query("doors.Mark=101A", &known).expect("recognised namespace parses");
        let result = assemble_rooms(&state, &RoomScope { filter: Some(&f), ..Default::default() })
            .unwrap()
            .expect("store has data");
        assert_eq!(result.rooms.len(), 1, "only the room actually joined to \"doors\" matches");
        assert_eq!(result.rooms[0].room.id, "r2");

        // Both sources are independently filterable in the same query.
        let both = RoomFilter::parse_query("drofus.NetArea=25.5,doors.Mark=101A", &known).expect("must parse");
        assert!(!both.is_empty());

        // An unrecognised namespace still errors, naming every known source.
        let err = RoomFilter::parse_query("cobie.Space=1", &known).expect_err("unknown source must error");
        assert!(err.contains("drofus") && err.contains("doors"), "names both known sources: {err}");
    }

    /// Under a milestone with a pinned dRofus snapshot the label set comes
    /// from the PINNED CSV, not the current dataset — otherwise a milestone
    /// view would show current column headers over pinned rows.
    #[test]
    fn test_drofus_labels_follow_milestone_pin() {
        let dir = std::env::temp_dir().join(format!("roommate-labels-pin-{}", std::process::id()));
        let store = crate::storage::FsStore::new(dir.clone()).unwrap();

        let model_ts = "2026-06-01T00:00:00Z";
        let drofus_ts = "2026-06-01T09:00:00Z";
        let mut pinned = make_payload("p1", "m1", vec![], vec![make_room("r1", "Room", &[("Number", "1")])]);
        pinned.snapshot.taken_at = model_ts.to_string();

        // Current dRofus's one column is CurrentCol; the pinned CSV's is NetArea.
        let bundle = with_drofus(
            ProjectSettings {
                milestones: vec![crate::settings::Milestone {
                    name: "Design Freeze".to_string(),
                    date: "2026-06-30".to_string(),
                    reference_snapshots: std::collections::BTreeMap::from([(
                        "drofus".to_string(),
                        drofus_ts.to_string(),
                    )]),
                    attachments: std::collections::BTreeMap::from([("m1".to_string(), model_ts.to_string())]),
                    door_attachments: std::collections::BTreeMap::new(),
                }],
                ..make_bundle("Number")
            },
            Some(make_drofus_with_record("Number", "1", "CurrentCol", "99")),
        );
        let state = AppState::new(Box::new(store), single_project("p1", bundle), None);
        state.set_snapshot(pinned).unwrap();
        state.put_reference("p1", "drofus", drofus_ts, &drofus_csv("20")).unwrap();

        let at_milestone = assemble_rooms(&state, &scope(Some("p1"), Some("Design Freeze")))
            .unwrap()
            .expect("store has data");
        assert_eq!(
            at_milestone.reference_labels["p1"]["drofus"].all_labels,
            vec!["NetArea".to_string()],
            "pinned CSV's columns"
        );

        let latest = assemble_rooms(&state, &scope(Some("p1"), None)).unwrap().expect("store has data");
        assert_eq!(
            latest.reference_labels["p1"]["drofus"].all_labels,
            vec!["CurrentCol".to_string()],
            "current dataset's columns"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `drofus.`-qualified predicate under a milestone matches the PINNED
    /// dRofus values, not the project's current ones -- the proof that the
    /// filter sits downstream of pin substitution rather than beside it.
    #[test]
    fn test_assemble_rooms_filter_sees_milestone_pinned_drofus() {
        let dir = std::env::temp_dir().join(format!("roommate-filter-pin-{}", std::process::id()));
        let store = crate::storage::FsStore::new(dir.clone()).unwrap();

        let model_ts = "2026-06-01T00:00:00Z";
        let drofus_ts = "2026-06-01T09:00:00Z";
        let mut pinned = make_payload("p1", "m1", vec![], vec![make_room("r1", "Room", &[("Number", "1")])]);
        pinned.snapshot.taken_at = model_ts.to_string();

        // Current dRofus says "new-value" for this room; the pinned CSV says
        // "old-value", so the predicate itself distinguishes the join source.
        let bundle = bundle_for_drofus_pin("new-value", model_ts, Some(drofus_ts));
        let state = AppState::new(Box::new(store), single_project("p1", bundle), None);
        state.set_snapshot(pinned).unwrap();
        state.put_reference("p1", "drofus", drofus_ts, &drofus_csv("old-value")).unwrap();

        let f = filter(&["drofus.NetArea=old-value"]);
        let at_milestone = assemble_rooms(
            &state,
            &RoomScope {
                project: Some("p1"),
                milestone: Some("Design Freeze"),
                filter: Some(&f),
                ..Default::default()
            },
        )
        .unwrap()
        .expect("store has data");
        assert_eq!(at_milestone.rooms.len(), 1, "the predicate sees the pinned dRofus");

        let latest = assemble_rooms(&state, &RoomScope { project: Some("p1"), filter: Some(&f), ..Default::default() })
            .unwrap()
            .expect("store has data");
        assert!(
            latest.rooms.is_empty(),
            "the current dRofus says new-value, so the same predicate matches nothing"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
