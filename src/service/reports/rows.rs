//! Building a report's rows: a schedule (one row per element) or by room (one row per room, its elements grouped and summed), and the CSV
//! the result exports as. Rooms with nothing are reported, never silently dropped.

use super::*;

/// A cell: the rendered value, or empty for absent and blank alike.
///
/// The three-state presence collapses here and only here. A report is read as a
/// table, and a table has one empty cell; "absent" versus "blank" is a QA
/// distinction, which the checks reports make.
pub(super) fn cell(presence: PropertyPresence) -> String {
    match presence {
        PropertyPresence::Present(v) => v,
        PropertyPresence::Absent | PropertyPresence::Empty => String::new(),
    }
}

/// The header, in the order the cells are written.
///
/// **The grouped shape carries a `count` column, and it is named here rather
/// than by the row builder**: a row that writes a cell no header describes is
/// the kind of off-by-one that renders as every column shifted left, which
/// looks like bad data rather than a bug.
pub(super) fn columns_for(def: &ReportDefinition) -> Vec<Column> {
    let mut columns: Vec<Column> = Vec::new();
    if def.by_room {
        columns.extend(def.room_columns.iter().map(|name| Column { name: name.clone(), side: "room" }));
        if def.shape == Shape::PerRoom {
            columns.push(grouped_count_column(def.entity));
        }
    }
    columns.extend(def.columns.iter().map(|name| Column { name: name.clone(), side: "element" }));
    columns.extend(def.measures.iter().map(|name| Column { name: name.clone(), side: "join" }));
    columns
}

/// Resolve one requested column against an element, splitting `source.label`
/// exactly as a filter does.
pub(super) fn element_cell(element: &Element, name: &str, builtin: &[BuiltinPropertyDef], known: &[String]) -> String {
    match name.split_once('.') {
        Some((source, property)) if known.iter().any(|s| s == source) => {
            cell((element.read)(Some(source), property, builtin))
        }
        _ => cell((element.read)(None, name, builtin)),
    }
}

pub(super) fn room_cell(room: &RoomResponse, name: &str, builtin: &[BuiltinPropertyDef], known: &[String]) -> String {
    match name.split_once('.') {
        Some((source, property)) if known.iter().any(|s| s == source) => {
            cell(room.presence(Some(source), property, builtin))
        }
        _ => cell(room.presence(None, name, builtin)),
    }
}

/// Does this room survive the filter?
///
/// Room conditions read the room. **Associated-side conditions ask about the
/// room's whole set of matches**: a positive one is `EXISTS`, a negative one is
/// `NOT EXISTS` over its positive form. So "ceiling type is X" drops a room
/// with no ceilings — it cannot have one — while "ceiling type is not X" keeps
/// it, because nothing it has is X. That is the difference a per-row filter
/// cannot express, and it falls exactly on the rooms a report like this is
/// written for.
pub(super) fn room_survives(
    filter: &ReportFilter,
    room: Option<&RoomResponse>,
    matches: &[(&Element, &Attribution)],
    builtin: &[BuiltinPropertyDef],
) -> bool {
    evaluate(filter, &|condition| match condition.side {
        Side::Room => match room {
            Some(room) => holds_for(&condition.predicate, room, builtin),
            // No room at all: every operator fails but `is blank`, the rule a
            // missing value follows everywhere else.
            None => holds_for(&condition.predicate, &MeasureTarget(String::new()), builtin),
        },
        _ => {
            let negative = is_negative(&condition.predicate);
            let probe = Condition { side: condition.side, predicate: positive_form(&condition.predicate) };
            let found = matches
                .iter()
                .any(|(element, attribution)| condition_holds(&probe, element, Some(attribution), builtin));
            if negative {
                !found
            } else {
                found
            }
        }
    })
}

/// A schedule filters per row, which is right here: a row IS the thing the
/// question is about, and there is no set for a condition to quantify over.
pub(super) fn schedule_rows(
    def: &ReportDefinition,
    builtin: &[BuiltinPropertyDef],
    elements: &[Element],
) -> ReportResult {
    let known: Vec<String> = Vec::new();
    let rows = elements
        .iter()
        .filter(|element| match &def.filter {
            None => true,
            Some(filter) => evaluate(filter, &|c| condition_holds(c, element, None, builtin)),
        })
        .map(|element| def.columns.iter().map(|c| element_cell(element, c, builtin, &known)).collect())
        .collect();
    ReportResult {
        revision: String::new(),
        columns: columns_for(def),
        rows,
        total_rows: 0,
        unmatched_rows: 0,
        note: None,
    }
}

/// Rows for a by-room report.
///
/// **Grouped before it is filtered**, which is the whole shape of this
/// function: a condition on the associated side asks about a room's whole set
/// of matches (see `room_survives`), so nothing can be decided element by
/// element. What is emitted afterwards still walks the elements in their own
/// order, because a reader scanning a schedule and a by-room report of the same
/// entity should see them in the same order.
pub(super) fn by_room_rows(
    state: &AppState,
    scope: &ReportScope<'_>,
    def: &ReportDefinition,
    builtin: &[BuiltinPropertyDef],
    elements: Vec<Element>,
) -> Result<ReportResult, ServiceError> {
    let room_scope = RoomScope {
        project: scope.project,
        building: scope.building,
        milestone: scope.milestone,
        filter: None,
    };
    let rooms = rooms::assemble_rooms(state, &room_scope)?.map(|r| r.rooms).unwrap_or_default();
    // Keyed by (model, room), never by room id alone: a room id is unique only
    // within its model, and a bare key would resolve an element against a
    // same-numbered room in a linked model — a wrong answer that looks right.
    let index: BTreeMap<(&str, &str), &RoomResponse> =
        rooms.iter().map(|room| ((room.model_id.as_str(), room.room.id.as_str()), room)).collect();
    let known: Vec<String> = Vec::new();

    let mut per_room: BTreeMap<(String, String), Vec<(&Element, &Attribution)>> = BTreeMap::new();
    for element in &elements {
        for attribution in &element.rooms {
            per_room
                .entry((attribution.model_id.clone(), attribution.room_id.clone()))
                .or_default()
                .push((element, attribution));
        }
    }

    // Which rooms survive, decided once per room over its whole set.
    let empty_filter = ReportFilter::Group { mode: Mode::All, items: Vec::new() };
    let filter = def.filter.as_ref().unwrap_or(&empty_filter);
    let filtering = !filter.is_empty();
    let survived: std::collections::BTreeSet<&(String, String)> = per_room
        .iter()
        .filter(|(key, members)| {
            !filtering || room_survives(filter, index.get(&(key.0.as_str(), key.1.as_str())).copied(), members, builtin)
        })
        .map(|(key, _)| key)
        .collect();

    // The conditions that also narrow which of a kept room's matches are
    // listed — the positives every ancestor group ANDs.
    let mut row_conditions: Vec<&Condition> = Vec::new();
    filter.row_filters(true, &mut row_conditions);
    let listed = |element: &Element, attribution: &Attribution| {
        row_conditions.iter().all(|c| condition_holds(c, element, Some(attribution), builtin))
    };

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut unmatched = 0usize;

    if def.shape == Shape::PerMatch {
        for element in &elements {
            if element.rooms.is_empty() {
                // An element with no room is its own group: nothing to
                // quantify over but itself, and the room columns stay empty.
                if !def.include_unattributed {
                    continue;
                }
                if filtering && !room_survives(filter, None, &[(element, &NO_ATTRIBUTION)], builtin) {
                    continue;
                }
                unmatched += 1;
                rows.push(unattributed_row(def, element, builtin, &known));
                continue;
            }
            for attribution in &element.rooms {
                let key = (attribution.model_id.clone(), attribution.room_id.clone());
                if !survived.contains(&key) || !listed(element, attribution) {
                    continue;
                }
                let room = index.get(&(attribution.model_id.as_str(), attribution.room_id.as_str())).copied();
                rows.push(match_row(def, element, attribution, room, builtin, &known));
            }
        }
    } else {
        for (key, members) in &per_room {
            if !survived.contains(key) {
                continue;
            }
            let shown: Vec<(&Element, &Attribution)> = members.iter().copied().filter(|(e, a)| listed(e, a)).collect();
            let room = index.get(&(key.0.as_str(), key.1.as_str())).copied();
            rows.push(grouped_row(def, room, &shown, builtin, &known));
        }
        if def.include_unattributed {
            for element in elements.iter().filter(|e| e.rooms.is_empty()) {
                if filtering && !room_survives(filter, None, &[(element, &NO_ATTRIBUTION)], builtin) {
                    continue;
                }
                unmatched += 1;
                rows.push(grouped_row(def, None, &[(element, &NO_ATTRIBUTION)], builtin, &known));
            }
        }
    }

    for room in rooms_with_nothing(def, filter, filtering, &rooms, &per_room, builtin) {
        unmatched += 1;
        rows.push(room_only_row(def, room, builtin, &known));
    }

    Ok(ReportResult {
        revision: String::new(),
        columns: columns_for(def),
        rows,
        total_rows: 0,
        unmatched_rows: unmatched,
        note: None,
    })
}

/// The rooms nothing matched, and whether they are listed at all.
///
/// **The filter outranks the switch when it says anything about this entity.**
/// "Has no ceiling of type X" asks for exactly these rooms, so they appear
/// whatever the switch says; "has a ceiling of type X" excludes them for the
/// same reason, since a room with none cannot have one. The switch decides only
/// when the filter is silent about the associated side — a default is what you
/// fall back to, not what you argue with.
pub(super) fn rooms_with_nothing<'a>(
    def: &ReportDefinition,
    filter: &ReportFilter,
    filtering: bool,
    rooms: &'a [RoomResponse],
    per_room: &BTreeMap<(String, String), Vec<(&Element, &Attribution)>>,
    builtin: &[BuiltinPropertyDef],
) -> Vec<&'a RoomResponse> {
    let asks_for_them = filtering && filter.has(Side::Element, true);
    let asks_against_them = filtering && filter.has(Side::Element, false);
    if !def.include_rooms_without && !asks_for_them {
        return Vec::new();
    }
    if asks_against_them && !asks_for_them {
        // A positive condition is present and these rooms cannot satisfy it.
        return Vec::new();
    }
    rooms
        .iter()
        .filter(|room| !per_room.contains_key(&(room.model_id.clone(), room.room.id.clone())))
        .filter(|room| !filtering || room_survives(filter, Some(room), &[], builtin))
        .collect()
}

/// The attribution an unattributed element is evaluated against: no room, no
/// measures. A `Join` condition therefore reads empty, which matches only
/// `is blank` — the rule every absent value follows.
pub(super) static NO_ATTRIBUTION: std::sync::LazyLock<Attribution> = std::sync::LazyLock::new(|| Attribution {
    model_id: String::new(),
    room_id: String::new(),
    measures: BTreeMap::new(),
});

pub(super) fn match_row(
    def: &ReportDefinition,
    element: &Element,
    attribution: &Attribution,
    room: Option<&RoomResponse>,
    builtin: &[BuiltinPropertyDef],
    known: &[String],
) -> Vec<String> {
    let mut row: Vec<String> = def
        .room_columns
        .iter()
        .map(|c| room.map(|r| room_cell(r, c, builtin, known)).unwrap_or_default())
        .collect();
    row.extend(def.columns.iter().map(|c| element_cell(element, c, builtin, known)));
    row.extend(
        def.measures
            .iter()
            .map(|m| attribution.measures.get(m.as_str()).cloned().unwrap_or_default()),
    );
    row
}

/// An element no room matched: element columns filled, room columns empty.
/// **Empty, not "unattributed"** — a reader sorting the sheet by room gets
/// every homeless element together, and the row count names them.
pub(super) fn unattributed_row(
    def: &ReportDefinition,
    element: &Element,
    builtin: &[BuiltinPropertyDef],
    known: &[String],
) -> Vec<String> {
    let mut row: Vec<String> = vec![String::new(); def.room_columns.len()];
    row.extend(def.columns.iter().map(|c| element_cell(element, c, builtin, known)));
    row.extend(std::iter::repeat_n(String::new(), def.measures.len()));
    row
}

/// A room nothing matched: room columns filled, the rest empty.
pub(super) fn room_only_row(
    def: &ReportDefinition,
    room: &RoomResponse,
    builtin: &[BuiltinPropertyDef],
    known: &[String],
) -> Vec<String> {
    let mut row: Vec<String> = def.room_columns.iter().map(|c| room_cell(room, c, builtin, known)).collect();
    row.extend(std::iter::repeat_n(String::new(), def.columns.len() + def.measures.len()));
    row
}

/// One room with its matches aggregated.
///
/// **A numeric measure sums and everything else lists its distinct values**,
/// which is the rule the ceilings case forced: a ceiling over three rooms
/// carries its whole area against each of them, so summing *element* columns
/// would count it three times. Only the join's own measures are summed, and
/// `overlap_area` is the one that means anything added up.
pub(super) fn grouped_row(
    def: &ReportDefinition,
    room: Option<&RoomResponse>,
    members: &[(&Element, &Attribution)],
    builtin: &[BuiltinPropertyDef],
    known: &[String],
) -> Vec<String> {
    let mut row: Vec<String> = def
        .room_columns
        .iter()
        .map(|c| room.map(|r| room_cell(r, c, builtin, known)).unwrap_or_default())
        .collect();
    row.push(members.len().to_string());
    for column in &def.columns {
        row.push(distinct(members.iter().map(|(e, _)| element_cell(e, column, builtin, known))));
    }
    for measure in &def.measures {
        let values: Vec<String> = members
            .iter()
            .map(|(_, a)| a.measures.get(measure.as_str()).cloned().unwrap_or_default())
            .collect();
        row.push(match summed(&values) {
            Some(total) => number(total),
            None => distinct(values.into_iter()),
        });
    }
    row
}

/// Sum, when every value is a number. A measure that is a word (`room_origin`)
/// lists instead — and a mixed column is treated as words, because a partial
/// sum is a wrong number rather than a missing one.
pub(super) fn summed(values: &[String]) -> Option<f64> {
    let mut total = 0.0;
    for value in values {
        if value.is_empty() {
            continue;
        }
        total += value.parse::<f64>().ok()?;
    }
    Some(total)
}

pub(super) fn distinct(values: impl Iterator<Item = String>) -> String {
    let mut seen: Vec<String> = Vec::new();
    for value in values {
        if !value.is_empty() && !seen.contains(&value) {
            seen.push(value);
        }
    }
    seen.join(", ")
}

/// The extra header a grouped report carries: how many of this entity the room
/// matched. Named for the entity ("ceilings"), because "count" alone leaves a
/// reader to guess what was counted once the sheet is out of the page.
pub(super) fn grouped_count_column(entity: Entity) -> Column {
    Column { name: format!("{} count", entity.one()), side: "join" }
}

/// The same rows as CSV.
///
/// **Rendered here, not in the page**, which is the whole reason the rows are
/// built server-side at all: a browser that wrote its own CSV would be a second
/// implementation of quoting, column order and how an empty cell is spelled,
/// and the two would disagree the first time either was fixed. An MCP host
/// asking for `format=csv` gets these bytes, not a lookalike.
///
/// RFC 4180: CRLF between records, quotes doubled inside a quoted field, and a
/// field quoted only when it carries a comma, a quote or a newline.
pub fn to_csv(result: &ReportResult) -> String {
    fn escape(value: &str) -> String {
        if value.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", value.replace('"', "\"\""))
        } else {
            value.to_string()
        }
    }

    let mut out = String::new();
    let header: Vec<String> = result.columns.iter().map(|c| escape(&c.name)).collect();
    out.push_str(&header.join(","));
    for row in &result.rows {
        out.push_str("\r\n");
        let cells: Vec<String> = row.iter().map(|c| escape(c)).collect();
        out.push_str(&cells.join(","));
    }
    out
}
