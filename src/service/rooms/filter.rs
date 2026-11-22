//! The room filter language: `field op value` predicates over a room's own fields and its joined reference sources
//! (`Predicate`, `RoomFilter`), and what a field name may be (`split_namespace`). Also read by openings, FF&E and surfaces through
//! `FilterTarget`, so one language filters every entity.
//!
//! Kept whole: one filter language. The operator table, the parser and the evaluator share `Op` and the predicate, so a
//! change to the grammar touches all three. The namespace and field-name rules are separable in principle, but they only
//! ever change with the grammar, and splitting them would lengthen the `use super::*` chain through `rooms/mod.rs`.

use super::*;

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
    /// The four the query grammar has no spelling for. They exist for the
    /// report filter builder, which sends a tree rather than a string, and are
    /// listed here rather than in a second enum so one `holds` decides what
    /// every operator means.
    NotContains,
    StartsWith,
    EndsWith,
    /// Inclusive on both ends, and its own operator rather than sugar for two
    /// predicates so a saved report loads back as the condition it was built
    /// from.
    Between,
    /// Absent or empty. **The only way to ask about a missing value**, which is
    /// why it is an operator and not a value: an empty value on any other
    /// operator is a parse error, because a blank could never match one.
    Blank,
    HasValue,
}

/// The operator spellings paired with their `Op`. Order matters *within one
/// position*: `>=` must be tried before `>` and `!=` before `=`, or `Area>=20`
/// would split as `Gt` with the value `"=20"`. `split_operator` scans positions
/// left to right and this list at each one, so the earliest operator wins and
/// the longest spelling wins the tie.
pub(super) const OPERATORS: &[(&str, Op)] = &[
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
pub(super) fn split_operator(expr: &str) -> Option<(&str, Op, &str)> {
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
    /// The upper bound of a `Between`; unused by every other operator.
    pub value2: Option<String>,
    /// Compare text exactly. **Off by default, which is the decision, not a
    /// convenience**: a modeller's capitalisation is not data, and a report
    /// that missed `Plasterboard` because someone typed `plasterboard` is
    /// wrong in the way nobody notices. The query grammar has no spelling for
    /// it, so `?filter=` is always case-insensitive — one rule rather than two
    /// equalities that differ invisibly.
    pub case_sensitive: bool,
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
    pub(super) fn parse(expr: &str, known: &std::collections::BTreeSet<String>) -> Result<Self, String> {
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

        Ok(Predicate {
            source,
            property: property.to_string(),
            op,
            value: value.to_string(),
            value2: None,
            case_sensitive: false,
        })
    }

    /// Does a resolved value satisfy this predicate?
    ///
    /// `=`/`!=` use `numeric_match` when both sides parse as numbers (so
    /// `"25.50"` equals `"25.5"` — the same stated-precision tolerance dRofus
    /// validation applies), and **text comparison folds case** unless
    /// `case_sensitive` says otherwise. The ordering operators are numeric
    /// only: a value that doesn't parse as a number simply doesn't match, it is
    /// not an error (signal, not error).
    ///
    /// `Blank`/`HasValue` never reach here — a blank resolves to `None` before
    /// this is called, and `RoomFilter::holds_for` answers those two there.
    /// Every other operator is therefore free to assume a real value, which is
    /// what keeps "a missing value matches nothing" true for negative operators
    /// without a special case in each one.
    pub(super) fn holds(&self, actual: &str) -> bool {
        let fold = |text: &str| if self.case_sensitive { text.to_string() } else { text.to_lowercase() };
        let equal = || numeric_match(actual, &self.value).unwrap_or_else(|| fold(actual) == fold(&self.value));
        let (haystack, needle) = (fold(actual), fold(self.value.trim()));
        match self.op {
            Op::Eq => equal(),
            Op::Ne => !equal(),
            Op::Contains => haystack.contains(&needle),
            Op::NotContains => !haystack.contains(&needle),
            Op::StartsWith => haystack.starts_with(&needle),
            Op::EndsWith => haystack.ends_with(&needle),
            // Answered before resolution; see the doc comment.
            Op::Blank => false,
            Op::HasValue => true,
            Op::Between => {
                let (Ok(a), Ok(low), Ok(high)) = (
                    actual.trim().parse::<f64>(),
                    self.value.trim().parse::<f64>(),
                    self.value2.as_deref().unwrap_or_default().trim().parse::<f64>(),
                ) else {
                    return false;
                };
                // Written either way round, because a form with two boxes is
                // filled either way round.
                a >= low.min(high) && a <= low.max(high)
            }
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
pub(super) fn presence_of(
    room: &RoomResponse,
    source: Option<&str>,
    property: &str,
    builtin_defs: &[BuiltinPropertyDef],
) -> PropertyPresence {
    /// A room's own `name`/`id` fields have no absent state — the struct
    /// always carries them — so blank collapses to `Empty`, never `Absent`.
    pub(super) fn intrinsic(value: &str) -> PropertyPresence {
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
            "$level_id" => intrinsic(&room.room.level_id),
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
pub(super) fn resolve_field<T: FilterTarget>(
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

/// How a group combines its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    All,
    Any,
}

/// A filter is a tree: predicates inside groups that match **all** or **any**
/// of their children, nested.
///
/// **It used to be a flat AND**, and the comma form still parses to exactly
/// that — one `All` group of predicates — so `?filter=` and the MCP array form
/// mean what they always did. What the tree adds is OR, which the reports
/// filter builder needs and a query string has no readable spelling for.
///
/// An empty group is **true**, not false: a half-built filter in a form should
/// show everything rather than nothing, and "no filter" is the same statement
/// as "a filter with nothing in it".
#[derive(Debug, Clone)]
pub enum FilterNode {
    Group { mode: Mode, items: Vec<FilterNode> },
    Predicate(Predicate),
}

/// A filter over an assembled entity.
///
/// Parsing is the only fallible step — matching never fails, it just doesn't
/// match. Applied to an *assembled* `RoomResponse` rather than a raw `Room`, so
/// predicates can reach the joined data sources (see `resolve_field`).
#[derive(Debug, Clone)]
pub struct RoomFilter {
    root: FilterNode,
}

impl Default for RoomFilter {
    fn default() -> Self {
        RoomFilter { root: FilterNode::Group { mode: Mode::All, items: Vec::new() } }
    }
}

impl RoomFilter {
    /// Parse one predicate per element — the MCP form, where an array element
    /// per predicate means a caller never has to escape a separator.
    /// `Err` carries caller-addressable text naming the offending element.
    /// `known` is the recognised source-name vocabulary — see
    /// `split_namespace`.
    pub fn parse(exprs: &[String], known: &std::collections::BTreeSet<String>) -> Result<Self, String> {
        let items = exprs
            .iter()
            .filter(|e| !e.trim().is_empty())
            .map(|e| Predicate::parse(e.trim(), known).map(FilterNode::Predicate))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(RoomFilter { root: FilterNode::Group { mode: Mode::All, items } })
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
        match &self.root {
            FilterNode::Group { items, .. } => items.is_empty(),
            FilterNode::Predicate(_) => false,
        }
    }

    /// Build one from a tree the caller assembled — the reports filter
    /// builder's path, where the structure arrives as JSON rather than as a
    /// string to parse.
    pub fn from_node(root: FilterNode) -> Self {
        RoomFilter { root }
    }

    /// The root, for a caller that walks the tree itself. `service::reports`
    /// does: a condition on the associated side asks about a room's whole set
    /// of matches, which is a question this matcher cannot ask on its own.
    pub fn root(&self) -> &FilterNode {
        &self.root
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
        matches_node(&self.root, &|p| holds_for(p, target, builtin_defs))
    }
}

/// Walk a tree, deciding each leaf with `decide`.
///
/// Public to the crate because `service::reports` walks the same tree with a
/// *different* leaf rule — its associated-side conditions ask about a set — and
/// the two must agree about what a group means even when they disagree about
/// what a leaf means.
pub(crate) fn matches_node(node: &FilterNode, decide: &impl Fn(&Predicate) -> bool) -> bool {
    match node {
        FilterNode::Predicate(p) => decide(p),
        FilterNode::Group { mode, items } => match mode {
            Mode::All => items.iter().all(|item| matches_node(item, decide)),
            Mode::Any => items.is_empty() || items.iter().any(|item| matches_node(item, decide)),
        },
    }
}

/// One predicate against one entity, including the two operators that ask
/// about the absence of a value.
///
/// **This is where "a missing value matches nothing" lives.** A field that
/// resolves to nothing fails every operator, negative ones included: "this room
/// has no Department" is not evidence that its Department differs from
/// Cardiology, and for a joined source an unmatched link key is a signal, not a
/// value. `Blank` and `HasValue` are the two ways to ask about that state
/// directly, which is why they are answered here rather than in `holds`.
pub(crate) fn holds_for<T: FilterTarget>(
    predicate: &Predicate,
    target: &T,
    builtin_defs: &[BuiltinPropertyDef],
) -> bool {
    match resolve_field(target, predicate, builtin_defs) {
        Some(actual) => match predicate.op {
            Op::Blank => false,
            Op::HasValue => true,
            _ => predicate.holds(&actual),
        },
        None => matches!(predicate.op, Op::Blank),
    }
}
