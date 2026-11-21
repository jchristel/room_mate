//! A report's filter: the tree of conditions the page sends (`FilterWire`), its checked form (`ReportFilter`), and how one decides over an
//! element, a measure or a room. Conditions reuse the room filter language (`rooms::filter`).

use super::*;

/// One condition in a report filter: a predicate, plus the side it reads.
#[derive(Debug)]
pub struct Condition {
    pub side: Side,
    pub predicate: Predicate,
}

/// A report filter: the same tree shape `RoomFilter` carries, with a side on
/// every leaf.
#[derive(Debug)]
pub enum ReportFilter {
    Group { mode: Mode, items: Vec<ReportFilter> },
    Condition(Condition),
}

impl ReportFilter {
    pub(super) fn is_empty(&self) -> bool {
        match self {
            ReportFilter::Group { items, .. } => items.iter().all(ReportFilter::is_empty),
            ReportFilter::Condition(_) => false,
        }
    }

    /// Does any leaf ask about the associated side? Positive and negative are
    /// separated because they pull the unmatched rows in opposite directions —
    /// see `room_survives`.
    pub(super) fn has(&self, side: Side, negative: bool) -> bool {
        match self {
            ReportFilter::Group { items, .. } => items.iter().any(|i| i.has(side, negative)),
            ReportFilter::Condition(c) => {
                (c.side == side || (side == Side::Element && c.side == Side::Join))
                    && is_negative(&c.predicate) == negative
            }
        }
    }

    /// The associated-side conditions every ancestor group ANDs, which are the
    /// ones that also narrow the ROWS under a kept room.
    ///
    /// A condition inside an `any` group decides whether the room appears and
    /// is not applied per row: "this room has a plasterboard ceiling OR is a
    /// wet area" says nothing about which of its ceilings to list.
    pub(super) fn row_filters<'a>(&'a self, conjunctive: bool, out: &mut Vec<&'a Condition>) {
        match self {
            ReportFilter::Group { mode, items } => {
                let all = *mode == Mode::All;
                for item in items {
                    item.row_filters(conjunctive && all, out);
                }
            }
            ReportFilter::Condition(c) => {
                if conjunctive && c.side != Side::Room && !is_negative(&c.predicate) {
                    out.push(c);
                }
            }
        }
    }
}

/// `is not` and `does not contain`, and the positive form each negates.
///
/// **On the associated side a negation is about the SET**, not about one row:
/// "ceiling type is not X" asks for a room with NO ceiling of type X, which is
/// not "a ceiling whose type is not X". Per-row negation answers a different
/// question and differs exactly on the rooms the report is for — a room keeps
/// appearing because of its *other* ceilings.
pub(super) fn is_negative(predicate: &Predicate) -> bool {
    matches!(predicate.op, Op::Ne | Op::NotContains)
}

pub(super) fn positive_form(predicate: &Predicate) -> Predicate {
    let mut positive = predicate.clone();
    positive.op = match predicate.op {
        Op::Ne => Op::Eq,
        Op::NotContains => Op::Contains,
        other => other,
    };
    positive
}

/// The filter as it arrives, and the single place it becomes predicates.
///
/// **In the service rather than in each adapter**, because the HTTP handler and
/// the MCP tool must not each write their own parse: two mappings of one shape
/// is how an operator ends up meaning something slightly different depending on
/// who asked.
///
/// A group is `{"mode": "all"|"any", "items": [...]}`; a condition is
/// `{"side", "field", "op", "value"}`. The side is a field rather than a prefix
/// on the name (`room.Level`) because `<source>.<label>` already owns the dot,
/// and a reference source named `room` would make the two unreadable.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
pub enum FilterWire {
    Group {
        mode: String,
        #[serde(default)]
        items: Vec<FilterWire>,
    },
    Condition {
        /// `room`, `element` or `join`. Defaults to `element`, which is the
        /// only side a schedule has.
        #[serde(default)]
        side: Option<String>,
        field: String,
        op: String,
        #[serde(default)]
        value: Option<String>,
        /// The upper bound of a `between`.
        #[serde(default)]
        value2: Option<String>,
        #[serde(default)]
        case_sensitive: bool,
    },
}

impl FilterWire {
    /// Turn the wire shape into a filter, naming the offending piece on
    /// failure. `known` is the recognised reference-source vocabulary, so
    /// `drofus.NetArea` binds as a joined field exactly as it does in
    /// `?filter=`.
    pub fn parse(&self, known: &std::collections::BTreeSet<String>) -> Result<ReportFilter, String> {
        match self {
            FilterWire::Group { mode, items } => {
                let mode = match mode.as_str() {
                    "all" => Mode::All,
                    "any" => Mode::Any,
                    other => return Err(format!("unknown filter mode {other:?} — expected all or any")),
                };
                let items = items.iter().map(|i| i.parse(known)).collect::<Result<Vec<_>, _>>()?;
                Ok(ReportFilter::Group { mode, items })
            }
            FilterWire::Condition { side, field, op, value, value2, case_sensitive } => {
                let side = match side.as_deref() {
                    None | Some("element") => Side::Element,
                    Some("room") => Side::Room,
                    Some("join") => Side::Join,
                    Some(other) => {
                        return Err(format!("unknown filter side {other:?} — expected room, element or join"))
                    }
                };
                let op = parse_op(op)?;
                // A value is required by every operator except the two that ask
                // about absence — where a value could only ever contradict the
                // question.
                let needs_value = !matches!(op, Op::Blank | Op::HasValue);
                let value = value.clone().unwrap_or_default();
                if needs_value && value.trim().is_empty() {
                    return Err(format!("filter on {field:?}: {op:?} needs a value"));
                }
                if op == Op::Between && value2.as_deref().unwrap_or("").trim().is_empty() {
                    return Err(format!("filter on {field:?}: between needs both ends"));
                }
                // The join's measures are its own vocabulary, not the entity's,
                // so they never split on a source namespace.
                let predicate = if side == Side::Join {
                    Predicate {
                        source: None,
                        property: field.clone(),
                        op,
                        value,
                        value2: value2.clone(),
                        case_sensitive: *case_sensitive,
                    }
                } else {
                    let (source, property) = match rooms::split_namespace(field, known) {
                        rooms::NamespaceSplit::Joined { source, property } => (Some(source), property.to_string()),
                        rooms::NamespaceSplit::Unqualified(name) => (None, name.to_string()),
                        rooms::NamespaceSplit::UnknownSource(name) => {
                            return Err(format!(
                                "filter field {field:?}: {name:?} is not a known data source — known: {}",
                                known.iter().cloned().collect::<Vec<_>>().join(", ")
                            ))
                        }
                    };
                    Predicate {
                        source,
                        property,
                        op,
                        value,
                        value2: value2.clone(),
                        case_sensitive: *case_sensitive,
                    }
                };
                Ok(ReportFilter::Condition(Condition { side, predicate }))
            }
        }
    }
}

/// The operator names the builder sends. Spelled out rather than derived from
/// the enum so the wire vocabulary is a decision rather than a rename away from
/// breaking every saved report.
pub(super) fn parse_op(name: &str) -> Result<Op, String> {
    Ok(match name {
        "eq" => Op::Eq,
        "ne" => Op::Ne,
        "gt" => Op::Gt,
        "ge" => Op::Ge,
        "lt" => Op::Lt,
        "le" => Op::Le,
        "contains" => Op::Contains,
        "not_contains" => Op::NotContains,
        "starts_with" => Op::StartsWith,
        "ends_with" => Op::EndsWith,
        "between" => Op::Between,
        "blank" => Op::Blank,
        "has_value" => Op::HasValue,
        other => {
            return Err(format!(
                "unknown filter operator {other:?} — expected one of eq, ne, gt, ge, lt, le, contains, \
                 not_contains, starts_with, ends_with, between, blank, has_value"
            ))
        }
    })
}

pub(super) fn filter_fields<'a>(filter: &'a ReportFilter, visit: &mut impl FnMut(Side, &'a str)) {
    match filter {
        ReportFilter::Group { items, .. } => items.iter().for_each(|i| filter_fields(i, visit)),
        ReportFilter::Condition(c) => visit(c.side, c.predicate.property.as_str()),
    }
}

/// Evaluate a report filter tree, deciding each leaf with `decide`.
///
/// The group rule is `rooms::matches_node`'s, restated for this tree rather
/// than shared through a trait: an empty group is true, `all` is every child,
/// `any` is one. What differs between the two callers is only the leaf.
pub(super) fn evaluate(filter: &ReportFilter, decide: &impl Fn(&Condition) -> bool) -> bool {
    match filter {
        ReportFilter::Condition(c) => decide(c),
        ReportFilter::Group { mode, items } => match mode {
            Mode::All => items.iter().all(|item| evaluate(item, decide)),
            Mode::Any => items.is_empty() || items.iter().any(|item| evaluate(item, decide)),
        },
    }
}

/// One condition against one element and one attribution.
///
/// A `Join` condition reads the measures the attribution carries, which are
/// strings already rendered — so `fraction_of_room > 0.9` compares the same
/// text the row shows, and a reader who filters on what they can see gets what
/// they expect.
pub(super) fn condition_holds(
    condition: &Condition,
    element: &Element,
    attribution: Option<&Attribution>,
    builtin: &[BuiltinPropertyDef],
) -> bool {
    match condition.side {
        Side::Element => holds_for(&condition.predicate, &ElementTarget(element), builtin),
        Side::Join => {
            let value = attribution.and_then(|a| a.measures.get(condition.predicate.property.as_str()));
            match value {
                Some(v) => holds_for(&condition.predicate, &MeasureTarget(v.clone()), builtin),
                // A measure this entity does not carry is absent, and absent
                // matches only `is blank` -- the rule every other field follows.
                None => holds_for(&condition.predicate, &MeasureTarget(String::new()), builtin),
            }
        }
        // Decided by the caller, which is holding the room.
        Side::Room => true,
    }
}

/// An `Element` as a `FilterTarget`, so a condition on the element side reads
/// through the same vocabulary a column does.
pub(super) struct ElementTarget<'a>(pub(super) &'a Element);

impl FilterTarget for ElementTarget<'_> {
    fn presence(&self, source: Option<&str>, property: &str, builtin_defs: &[BuiltinPropertyDef]) -> PropertyPresence {
        (self.0.read)(source, property, builtin_defs)
    }
}

/// One measure value as a `FilterTarget`. The measure is named by the
/// predicate, so the target is the value itself; an empty string is the absent
/// state, which is what makes "a measure this entity does not carry matches
/// only `is blank`" fall out rather than being special-cased.
pub(super) struct MeasureTarget(pub(super) String);

impl FilterTarget for MeasureTarget {
    fn presence(&self, _source: Option<&str>, _property: &str, _defs: &[BuiltinPropertyDef]) -> PropertyPresence {
        if self.0.is_empty() {
            PropertyPresence::Empty
        } else {
            PropertyPresence::Present(self.0.clone())
        }
    }
}
