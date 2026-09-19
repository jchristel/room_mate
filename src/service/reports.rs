//! Tabular reports: a schedule of one entity, or one entity **by room**.
//!
//! The page is `/reports/` (`src-js/reports/`); the design, and what is still
//! unbuilt, are in `docs/STRATEGY-REPORTS.md`.
//!
//! **Why this exists at all, rather than the page joining payloads itself.**
//! The checks and the milestone comparison each read one endpoint that had
//! already done the work, so they needed nothing here. A schedule does not: it
//! reads a whole entity payload, and `/ffe` is 133 MB for one RHH storey with
//! every instance's property map on the wire. A report wants six columns of it.
//! So the projection happens here and the rows go out as flat strings — the
//! payload saving *is* the feature, not a side effect of it.
//!
//! **Three rules this module keeps, each of which is a way to be wrong:**
//!
//! - **It never computes an attribution.** Which room a ceiling lies over,
//!   which room owns a door, which room an item sits in: every one of those is
//!   read off the entity's own response, where the policy, the tolerance and
//!   the model scope already applied. A report that re-derived any of them
//!   would be the extractor-footprint lesson in a new place — two
//!   implementations of one answer, and the one with less context winning by
//!   accident.
//! - **A column is resolved through `FilterTarget::presence`**, which is the
//!   same vocabulary `?filter=` parses: the entity's own tiered properties,
//!   its `$intrinsics`, and `<source>.<label>` for a joined reference field. So
//!   a name that filters correctly also projects correctly, and a seventh
//!   entity needs no arm here — only its existing impl.
//! - **An unmatched row is a reported state.** A room with no ceilings and a
//!   ceiling over no room are both answers; which of them appears is the
//!   caller's two switches, and the row count says what it counted.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::contract::{CeilingPayload, DoorPayload, FloorPayload, PropertyPresence, WindowPayload};
use crate::settings::BuiltinPropertyDef;
use crate::state::AppState;
use crate::storage::SnapshotKind;

use super::items::{self, ItemScope};
use super::openings::{self, OpeningKind, OpeningScope};
use super::rooms::{self, FilterTarget, RoomResponse, RoomScope};
use super::spaces::{self, SpaceScope};
use super::surfaces::{self, SurfaceScope};
use super::ServiceError;

/// Which entity a report is about.
///
/// Rooms are in the list for schedules only: everything else can be reported
/// *by* room, and a room by room is a different report (adjacency) that already
/// has an endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Entity {
    Rooms,
    Doors,
    Windows,
    Ceilings,
    Floors,
    Spaces,
    Ffe,
}

impl Entity {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "rooms" => Entity::Rooms,
            "doors" => Entity::Doors,
            "windows" => Entity::Windows,
            "ceilings" => Entity::Ceilings,
            "floors" => Entity::Floors,
            "spaces" => Entity::Spaces,
            "ffe" => Entity::Ffe,
            _ => return None,
        })
    }

    /// The singular noun a message uses: "3 doors with no room".
    pub fn one(self) -> &'static str {
        match self {
            Entity::Rooms => "room",
            Entity::Doors => "door",
            Entity::Windows => "window",
            Entity::Ceilings => "ceiling",
            Entity::Floors => "floor",
            Entity::Spaces => "space",
            Entity::Ffe => "item",
        }
    }

    /// The snapshot kinds a read of this entity depends on — its own, plus
    /// rooms wherever the answer is attributed against them. What the ETag
    /// cursor covers, for the reason `/ceilings`' already covers rooms.
    fn kinds(self, by_room: bool) -> Vec<SnapshotKind> {
        let own = match self {
            Entity::Rooms => SnapshotKind::Rooms,
            Entity::Doors => SnapshotKind::Doors,
            Entity::Windows => SnapshotKind::Windows,
            Entity::Ceilings => SnapshotKind::Ceilings,
            Entity::Floors => SnapshotKind::Floors,
            Entity::Spaces => SnapshotKind::Spaces,
            Entity::Ffe => SnapshotKind::Ffe,
        };
        if by_room && own != SnapshotKind::Rooms {
            vec![own, SnapshotKind::Rooms]
        } else {
            vec![own]
        }
    }

    /// Which entities can be reported by room, and why the two that cannot are
    /// not a gap:
    ///
    /// - **Rooms** are the other side of the join.
    /// - **Spaces** match a room on a *key*, project-wide, and that matching
    ///   lives in `service::spaces`' report rather than on the `/spaces` rows —
    ///   there is no per-space "the room it matched" field to read, so a
    ///   by-room spaces report would have to re-implement the match. Doing that
    ///   would give the report and `SpaceReport` two answers to one question,
    ///   which is exactly the rule this module keeps. It waits for the match to
    ///   be exposed, and the checks report covers the question meanwhile.
    pub fn joins_rooms(self) -> bool {
        !matches!(self, Entity::Rooms | Entity::Spaces)
    }
}

/// One row per match, or one row per room with the matches aggregated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    PerMatch,
    PerRoom,
}

/// What to report, as the caller asked for it.
pub struct ReportDefinition {
    pub entity: Entity,
    /// Report the entity *by room*, rather than as a flat schedule.
    pub by_room: bool,
    /// Columns over the entity itself, in the `?filter=` vocabulary.
    pub columns: Vec<String>,
    /// Columns over the room, for a by-room report.
    pub room_columns: Vec<String>,
    /// What the join produced — `overlap_area`, `room_origin` and friends.
    /// Named per entity; see `measure_value`.
    pub measures: Vec<String>,
    pub shape: Shape,
    /// List a room no element matched. Ignored by a schedule.
    pub include_rooms_without: bool,
    /// List an element no room matched. Ignored by a schedule.
    pub include_unattributed: bool,
    /// Cap the rows returned; `total_rows` still counts them all, so a caller
    /// can say "first 500 of 39,412" honestly.
    pub limit: Option<usize>,
}

/// What a report read is scoped to. The same three dimensions every entity read
/// takes, so a report and the plan it was read beside agree on scope.
pub struct ReportScope<'a> {
    pub project: Option<&'a str>,
    pub building: Option<&'a str>,
    pub milestone: Option<&'a str>,
}

/// One column, and which side of the join it came from — the page groups its
/// headers by this, and a CSV writer needs nothing else.
#[derive(Debug, Clone, Serialize)]
pub struct Column {
    pub name: String,
    pub side: &'static str,
}

/// A report, ready to render.
///
/// Rows are flat strings rather than typed cells: every consumer (a table, a
/// CSV, an MCP host reading it as text) wants the rendered value, and a number
/// that arrived as a string in the export has no better type to be given back.
#[derive(Debug, Clone, Serialize)]
pub struct ReportResult {
    pub revision: String,
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<String>>,
    /// Rows before `limit` — what makes "first N of M" true rather than
    /// reassuring.
    pub total_rows: usize,
    /// How many rows are findings of the "nothing matched" kind: an element
    /// with no room, or a room with no element. Reported rather than filtered,
    /// so a reader knows the total includes them.
    pub unmatched_rows: usize,
}

/// Nothing of this entity has ever been pushed — the service's `None`, which
/// every entity read already distinguishes from "pushed and empty". A report
/// that rendered the two the same way would tell a reader their filter was
/// wrong when in fact nobody has run the exporter.
pub type MaybeReport = Option<ReportResult>;

/// Build one report.
pub fn build_report(
    state: &AppState,
    scope: &ReportScope<'_>,
    def: &ReportDefinition,
) -> Result<MaybeReport, ServiceError> {
    if def.by_room && !def.entity.joins_rooms() {
        return Err(ServiceError::Invalid(format!(
            "{} cannot be reported by room — see service::reports::Entity::joins_rooms",
            def.entity.one()
        )));
    }

    let registry = state.settings();
    let builtin: Vec<BuiltinPropertyDef> = scope
        .project
        .and_then(|p| registry.settings_for(p))
        .map(|b| b.builtin_properties.clone())
        .unwrap_or_default();

    let revision = super::scope_cursor(state, scope.project, scope.milestone, &def.entity.kinds(def.by_room))?;

    let elements = read_elements(state, scope, def.entity)?;
    let Some(elements) = elements else { return Ok(None) };

    let mut result = if def.by_room {
        by_room_rows(state, scope, def, &builtin, elements)?
    } else {
        schedule_rows(def, &builtin, &elements)
    };
    result.revision = revision;

    result.total_rows = result.rows.len();
    if let Some(limit) = def.limit {
        result.rows.truncate(limit);
    }
    Ok(Some(result))
}

/// One element, reduced to what a report needs: a way to read its fields, its
/// owning model, and the rooms it is attributed to with their measures.
///
/// **Assembled per entity and then never branched on again.** Every difference
/// between a door, a ceiling and an item is resolved here, which is what keeps
/// the row building below one implementation rather than seven.
struct Element {
    /// `(source, field)` resolution, boxed so the row builder holds elements of
    /// several concrete types without knowing which.
    read: FieldReader,
    /// The rooms this element is attributed to, in the entity's own order, with
    /// whatever the join measured. Empty means unattributed — a reported state.
    rooms: Vec<Attribution>,
}

/// One element's field lookup, in the `?filter=` vocabulary: `None` source for
/// the entity's own properties and intrinsics, `Some(name)` for a joined
/// reference record.
type FieldReader = Box<dyn Fn(Option<&str>, &str, &[BuiltinPropertyDef]) -> PropertyPresence>;

/// One element-to-room attribution, as the entity read reported it.
struct Attribution {
    model_id: String,
    room_id: String,
    measures: BTreeMap<&'static str, String>,
}

fn presence_reader<T>(element: T) -> FieldReader
where
    T: FilterTarget + 'static,
{
    Box::new(move |source, property, defs| element.presence(source, property, defs))
}

fn number(value: f64) -> String {
    format!("{value:.2}")
}

/// Read one entity, in the report's scope, and flatten it to `Element`s.
///
/// One arm per entity and nothing else: everything that differs between a door
/// and a ceiling is resolved in these readers, so the row building below never
/// asks what it is holding.
fn read_elements(
    state: &AppState,
    scope: &ReportScope<'_>,
    entity: Entity,
) -> Result<Option<Vec<Element>>, ServiceError> {
    match entity {
        Entity::Rooms => read_rooms(state, scope),
        Entity::Doors => read_openings::<DoorPayload>(state, scope, OpeningKind::Doors),
        Entity::Windows => read_openings::<WindowPayload>(state, scope, OpeningKind::Windows),
        Entity::Ceilings => read_surfaces::<CeilingPayload>(state, scope),
        Entity::Floors => read_surfaces::<FloorPayload>(state, scope),
        Entity::Ffe => read_items(state, scope),
        Entity::Spaces => read_spaces(state, scope),
    }
}

fn read_rooms(state: &AppState, scope: &ReportScope<'_>) -> Result<Option<Vec<Element>>, ServiceError> {
    let room_scope = RoomScope {
        project: scope.project,
        building: scope.building,
        milestone: scope.milestone,
        filter: None,
    };
    Ok(rooms::assemble_rooms(state, &room_scope)?.map(|result| {
        result
            .rooms
            .into_iter()
            .map(|room| {
                let model_id = room.model_id.clone();
                let room_id = room.room.id.clone();
                Element {
                    read: presence_reader(room),
                    // A room is its own attribution, so a room schedule can
                    // carry room columns without a special case.
                    rooms: vec![Attribution { model_id, room_id, measures: BTreeMap::new() }],
                }
            })
            .collect()
    }))
}

/// Doors and windows, whose attribution is `owner_rooms_qualified` — the
/// policy's answer, model-qualified, which is the only form safe to index
/// rooms with across linked models.
fn read_openings<P: crate::contract::OpeningEnvelope + serde::de::DeserializeOwned>(
    state: &AppState,
    scope: &ReportScope<'_>,
    kind: OpeningKind,
) -> Result<Option<Vec<Element>>, ServiceError> {
    let opening_scope = OpeningScope {
        project: scope.project,
        building: scope.building,
        milestone: scope.milestone,
        filter: None,
        storeys: None,
    };
    Ok(openings::assemble_openings::<P>(state, kind, &opening_scope)?.map(|assembled| {
        assembled
            .openings
            .into_iter()
            .map(|opening| {
                let rooms = opening
                    .owner_rooms_qualified
                    .iter()
                    .map(|owner| Attribution {
                        model_id: owner.model_id.clone(),
                        room_id: owner.room_id.clone(),
                        measures: BTreeMap::new(),
                    })
                    .collect();
                Element { read: presence_reader(opening), rooms }
            })
            .collect()
    }))
}

/// Ceilings and floors, whose attribution is geometric and carries the three
/// fractions plus the width the tolerance is expressed in. All four ride every
/// row, because which of them matters is the consumer's policy.
fn read_surfaces<P: surfaces::SurfacePayloadKind>(
    state: &AppState,
    scope: &ReportScope<'_>,
) -> Result<Option<Vec<Element>>, ServiceError> {
    let surface_scope = SurfaceScope { project: scope.project, milestone: scope.milestone, storeys: None };
    Ok(surfaces::assemble_surfaces::<P>(state, &surface_scope)?.map(|assembled| {
        assembled
            .surfaces
            .into_iter()
            .map(|surface| {
                let rooms = surface
                    .rooms
                    .iter()
                    .map(|room| Attribution {
                        model_id: room.model_id.clone(),
                        room_id: room.room_id.clone(),
                        measures: BTreeMap::from([
                            ("overlap_area", number(room.overlap_area)),
                            ("fraction_of_element", number(room.fraction_of_element)),
                            ("fraction_of_room", number(room.fraction_of_room)),
                            ("mean_width", number(room.mean_width)),
                        ]),
                    })
                    .collect();
                Element { read: presence_reader(surface), rooms }
            })
            .collect()
    }))
}

fn read_items(state: &AppState, scope: &ReportScope<'_>) -> Result<Option<Vec<Element>>, ServiceError> {
    let item_scope = ItemScope {
        project: scope.project,
        building: scope.building,
        milestone: scope.milestone,
        filter: None,
        storeys: None,
    };
    Ok(items::assemble_items(state, &item_scope)?.map(|result| {
        result
            .ffe
            .into_iter()
            .map(|item| {
                // Where the room came from — stated by the modeller, or filled
                // by geometry. A column, because the two are different claims.
                let origin = format!("{:?}", item.room_origin).to_lowercase();
                let rooms = item
                    .owner_rooms_qualified
                    .iter()
                    .map(|owner| Attribution {
                        model_id: owner.model_id.clone(),
                        room_id: owner.room_id.clone(),
                        measures: BTreeMap::from([("room_origin", origin.clone())]),
                    })
                    .collect();
                Element { read: presence_reader(item), rooms }
            })
            .collect()
    }))
}

/// Spaces carry no per-row room match (see `Entity::joins_rooms`), so they
/// report as a schedule and nothing else.
fn read_spaces(state: &AppState, scope: &ReportScope<'_>) -> Result<Option<Vec<Element>>, ServiceError> {
    let space_scope = SpaceScope {
        project: scope.project,
        milestone: scope.milestone,
        model: None,
        filter: None,
        storeys: None,
    };
    Ok(spaces::assemble_spaces(state, &space_scope)?.map(|result| {
        result
            .spaces
            .into_iter()
            .map(|space| Element { read: presence_reader(space), rooms: Vec::new() })
            .collect()
    }))
}

/// A cell: the rendered value, or empty for absent and blank alike.
///
/// The three-state presence collapses here and only here. A report is read as a
/// table, and a table has one empty cell; "absent" versus "blank" is a QA
/// distinction, which the checks reports make.
fn cell(presence: PropertyPresence) -> String {
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
fn columns_for(def: &ReportDefinition) -> Vec<Column> {
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
fn element_cell(element: &Element, name: &str, builtin: &[BuiltinPropertyDef], known: &[String]) -> String {
    match name.split_once('.') {
        Some((source, property)) if known.iter().any(|s| s == source) => {
            cell((element.read)(Some(source), property, builtin))
        }
        _ => cell((element.read)(None, name, builtin)),
    }
}

fn room_cell(room: &RoomResponse, name: &str, builtin: &[BuiltinPropertyDef], known: &[String]) -> String {
    match name.split_once('.') {
        Some((source, property)) if known.iter().any(|s| s == source) => {
            cell(room.presence(Some(source), property, builtin))
        }
        _ => cell(room.presence(None, name, builtin)),
    }
}

fn schedule_rows(def: &ReportDefinition, builtin: &[BuiltinPropertyDef], elements: &[Element]) -> ReportResult {
    let known: Vec<String> = Vec::new();
    let rows = elements
        .iter()
        .map(|element| def.columns.iter().map(|c| element_cell(element, c, builtin, &known)).collect())
        .collect();
    ReportResult {
        revision: String::new(),
        columns: columns_for(def),
        rows,
        total_rows: 0,
        unmatched_rows: 0,
    }
}

fn by_room_rows(
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

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut unmatched = 0usize;
    // Which rooms an element reached, so "rooms with none" can be answered
    // without asking the entity again.
    let mut matched_rooms: std::collections::BTreeSet<(String, String)> = std::collections::BTreeSet::new();
    // Per-room groups, for the aggregated shape.
    let mut per_room: BTreeMap<(String, String), Vec<(&Element, &Attribution)>> = BTreeMap::new();

    for element in &elements {
        if element.rooms.is_empty() {
            if def.include_unattributed {
                unmatched += 1;
                if def.shape == Shape::PerMatch {
                    rows.push(unattributed_row(def, element, builtin, &known));
                }
            }
            continue;
        }
        for attribution in &element.rooms {
            let key = (attribution.model_id.clone(), attribution.room_id.clone());
            matched_rooms.insert(key.clone());
            match def.shape {
                Shape::PerMatch => {
                    let room = index.get(&(attribution.model_id.as_str(), attribution.room_id.as_str()));
                    rows.push(match_row(def, element, attribution, room.copied(), builtin, &known));
                }
                Shape::PerRoom => per_room.entry(key).or_default().push((element, attribution)),
            }
        }
    }

    if def.shape == Shape::PerRoom {
        let mut grouped: Vec<Vec<String>> = Vec::new();
        for (key, members) in &per_room {
            let room = index.get(&(key.0.as_str(), key.1.as_str())).copied();
            grouped.push(grouped_row(def, room, members, builtin, &known));
        }
        rows = grouped;
    }

    if def.include_rooms_without {
        for room in &rooms {
            if matched_rooms.contains(&(room.model_id.clone(), room.room.id.clone())) {
                continue;
            }
            unmatched += 1;
            rows.push(room_only_row(def, room, builtin, &known));
        }
    }

    Ok(ReportResult {
        revision: String::new(),
        columns: columns_for(def),
        rows,
        total_rows: 0,
        unmatched_rows: unmatched,
    })
}

fn match_row(
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
fn unattributed_row(
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
fn room_only_row(
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
fn grouped_row(
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
fn summed(values: &[String]) -> Option<f64> {
    let mut total = 0.0;
    for value in values {
        if value.is_empty() {
            continue;
        }
        total += value.parse::<f64>().ok()?;
    }
    Some(total)
}

fn distinct(values: impl Iterator<Item = String>) -> String {
    let mut seen: Vec<String> = Vec::new();
    for value in values {
        if !value.is_empty() && !seen.contains(&value) {
            seen.push(value);
        }
    }
    seen.join(", ")
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

/// The extra header a grouped report carries: how many of this entity the room
/// matched. Named for the entity ("ceilings"), because "count" alone leaves a
/// reader to guess what was counted once the sheet is out of the page.
fn grouped_count_column(entity: Entity) -> Column {
    Column { name: format!("{} count", entity.one()), side: "join" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_distinct_keeps_first_seen_order_and_drops_blanks() {
        let values = ["B".to_string(), String::new(), "A".to_string(), "B".to_string()];
        assert_eq!(distinct(values.into_iter()), "B, A");
    }

    /// A numeric measure sums; a word measure cannot, and saying so is what
    /// stops `room_origin` being rendered as `0.00`.
    #[test]
    fn test_summed_only_answers_for_numbers() {
        assert_eq!(summed(&["1.50".to_string(), "2.25".to_string()]), Some(3.75));
        assert_eq!(summed(&["stated".to_string(), "geometry".to_string()]), None);
        // A blank is not a zero and not a failure: an element that carried no
        // value for this measure simply contributes nothing.
        assert_eq!(summed(&["1.00".to_string(), String::new()]), Some(1.0));
    }

    /// Spaces and rooms are excluded from the by-room shape deliberately, and
    /// the refusal is a message rather than an empty report -- an empty table
    /// would read as "nothing matched".
    #[test]
    fn test_only_the_entities_that_carry_an_attribution_join_rooms() {
        assert!(Entity::Ceilings.joins_rooms());
        assert!(Entity::Floors.joins_rooms());
        assert!(Entity::Doors.joins_rooms());
        assert!(Entity::Ffe.joins_rooms());
        assert!(!Entity::Rooms.joins_rooms());
        assert!(!Entity::Spaces.joins_rooms());
    }

    /// The cursor has to move when the ROOMS move, not only when the entity
    /// does: attribution is derived from the rooms in scope, so a rooms push
    /// alone changes every by-room answer.
    #[test]
    fn test_a_by_room_read_depends_on_rooms_as_well() {
        assert_eq!(Entity::Ceilings.kinds(false), vec![SnapshotKind::Ceilings]);
        assert_eq!(Entity::Ceilings.kinds(true), vec![SnapshotKind::Ceilings, SnapshotKind::Rooms]);
        // A rooms schedule does not ask for rooms twice.
        assert_eq!(Entity::Rooms.kinds(true), vec![SnapshotKind::Rooms]);
    }

    // ---------- row building, against a real store ----------
    //
    // Doors carry the simplest attribution there is -- the policy resolves
    // `to_room` then `from_room` -- so they exercise every row rule here
    // without a geometry fixture standing between the test and what it checks.
    // The measure arithmetic is covered by `summed`/`distinct` above, which is
    // where it lives.

    use crate::contract::{CustomValue, DoorPayload, Model, Opening, Project, Room, RoomPayload, Snapshot};
    use crate::contract::{SUPPORTED_DOOR_SCHEMA, SUPPORTED_SCHEMA};
    use crate::state::ProjectSettings;
    use crate::storage::MemStore;
    use std::collections::HashMap;

    fn bundle() -> ProjectSettings {
        ProjectSettings {
            spaces: Default::default(),
            reference: BTreeMap::new(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec![],
            milestones: vec![],
            anchor_model: None,
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            windows: Default::default(),
            ffe: Default::default(),
            hierarchy_exclusions: vec![],
        }
    }

    fn room(id: &str, name: &str, props: &[(&str, &str)]) -> Room {
        let mut properties = BTreeMap::new();
        for (k, v) in props {
            properties.insert((*k).into(), CustomValue { value: v.to_string(), storage_type: None });
        }
        Room {
            enclosure: None,
            id: id.to_string(),
            name: name.to_string(),
            level_id: "1".to_string(),
            loops: vec![],
            properties,
        }
    }

    fn door(id: &str, to_room: Option<&str>, props: &[(&str, &str)]) -> Opening {
        let mut properties = BTreeMap::new();
        for (k, v) in props {
            properties.insert((*k).into(), CustomValue { value: v.to_string(), storage_type: None });
        }
        Opening {
            id: id.to_string(),
            level_id: "1".to_string(),
            loops: vec![],
            from_room: None,
            to_room: to_room.map(str::to_string),
            insertion_point: None,
            through_wall_normal: None,
            type_id: "t1".to_string(),
            type_name: "Single".to_string(),
            properties,
            type_properties: Default::default(),
        }
    }

    fn state_with(rooms: Vec<Room>, doors: Vec<Opening>) -> AppState {
        let state = AppState::new(Box::new(MemStore::new()), HashMap::from([("p1".to_string(), bundle())]), None);
        state
            .set_snapshot(RoomPayload {
                schema_version: SUPPORTED_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
                phase: Some("New Construction".to_string()),
                model_to_shared: None,
                room_boundary: None,
                levels: vec![],
                rooms,
            })
            .unwrap();
        state
            .set_door_snapshot(DoorPayload {
                schema_version: SUPPORTED_DOOR_SCHEMA,
                project: Project { id: "p1".to_string(), name: "P".to_string() },
                model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
                snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
                phase: Some("New Construction".to_string()),
                model_to_shared: None,
                levels: vec![],
                doors,
            })
            .unwrap();
        state
    }

    fn definition(entity: Entity, by_room: bool) -> ReportDefinition {
        ReportDefinition {
            entity,
            by_room,
            columns: vec!["$id".to_string()],
            room_columns: vec!["Number".to_string()],
            measures: vec![],
            shape: Shape::PerMatch,
            include_rooms_without: false,
            include_unattributed: true,
            limit: None,
        }
    }

    fn scope() -> ReportScope<'static> {
        ReportScope { project: Some("p1"), building: None, milestone: None }
    }

    fn run(state: &AppState, def: &ReportDefinition) -> ReportResult {
        build_report(state, &scope(), def).unwrap().expect("the store has data")
    }

    /// Only the columns asked for are read. That IS the reason this module
    /// exists rather than the page reading /doors, so it is the first thing
    /// pinned.
    #[test]
    fn test_a_schedule_projects_only_the_requested_columns() {
        let state = state_with(
            vec![room("r1", "ENTRY", &[("Number", "G01")])],
            vec![door("d1", Some("r1"), &[("Mark", "D-01")])],
        );
        let mut def = definition(Entity::Doors, false);
        def.columns = vec!["Mark".to_string(), "$type_name".to_string()];

        let result = run(&state, &def);

        assert_eq!(result.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Mark", "$type_name"]);
        assert_eq!(result.rows, vec![vec!["D-01".to_string(), "Single".to_string()]]);
    }

    /// A field the element does not carry is an empty cell, never a missing
    /// one: a short row would shift every later column.
    #[test]
    fn test_a_column_the_element_lacks_is_an_empty_cell() {
        let state = state_with(vec![], vec![door("d1", None, &[])]);
        let mut def = definition(Entity::Doors, false);
        def.columns = vec!["Mark".to_string(), "$id".to_string()];

        let result = run(&state, &def);

        assert_eq!(result.rows, vec![vec![String::new(), "d1".to_string()]]);
    }

    /// One row per attribution, with the room's own columns filled from the
    /// room the entity named -- not from a room id matched loosely.
    #[test]
    fn test_by_room_emits_one_row_per_attribution() {
        let state = state_with(
            vec![
                room("r1", "ENTRY", &[("Number", "G01")]),
                room("r2", "KITCHEN", &[("Number", "G02")]),
            ],
            vec![door("d1", Some("r1"), &[]), door("d2", Some("r2"), &[])],
        );
        let result = run(&state, &definition(Entity::Doors, true));

        assert_eq!(
            result.rows,
            vec![
                vec!["G01".to_string(), "d1".to_string()],
                vec!["G02".to_string(), "d2".to_string()],
            ]
        );
    }

    /// An element no room matched is part of the answer, and its room columns
    /// are empty rather than filled with a stand-in.
    #[test]
    fn test_an_unattributed_element_rides_with_empty_room_columns() {
        let state = state_with(vec![room("r1", "ENTRY", &[("Number", "G01")])], vec![door("d1", None, &[])]);
        let result = run(&state, &definition(Entity::Doors, true));

        assert_eq!(result.rows, vec![vec![String::new(), "d1".to_string()]]);
        assert_eq!(result.unmatched_rows, 1, "counted, so a reader knows the total includes it");
    }

    /// ...and it can be left out, which is the switch saying so rather than
    /// the report deciding.
    #[test]
    fn test_unattributed_elements_can_be_excluded() {
        let state = state_with(vec![room("r1", "ENTRY", &[("Number", "G01")])], vec![door("d1", None, &[])]);
        let mut def = definition(Entity::Doors, true);
        def.include_unattributed = false;

        let result = run(&state, &def);

        assert!(result.rows.is_empty());
        assert_eq!(result.unmatched_rows, 0);
    }

    /// A room nothing matched is the other unmatched direction, and off by
    /// default: on House A most of them are external, which is why it is asked
    /// for rather than assumed.
    #[test]
    fn test_rooms_with_no_element_appear_only_when_asked() {
        let state = state_with(
            vec![
                room("r1", "ENTRY", &[("Number", "G01")]),
                room("r2", "POOL EX", &[("Number", "G05")]),
            ],
            vec![door("d1", Some("r1"), &[])],
        );

        let quiet = run(&state, &definition(Entity::Doors, true));
        assert_eq!(quiet.rows.len(), 1);

        let mut def = definition(Entity::Doors, true);
        def.include_rooms_without = true;
        let loud = run(&state, &def);
        assert_eq!(
            loud.rows,
            vec![
                vec!["G01".to_string(), "d1".to_string()],
                vec!["G05".to_string(), String::new()],
            ]
        );
    }

    /// Grouped: one row per room, the count first, element columns listed
    /// rather than summed -- the ceilings rule, applied to every entity so
    /// there is one reading.
    #[test]
    fn test_per_room_groups_and_counts() {
        let state = state_with(
            vec![room("r1", "ENTRY", &[("Number", "G01")])],
            vec![
                door("d1", Some("r1"), &[("Mark", "D-01")]),
                door("d2", Some("r1"), &[("Mark", "D-02")]),
            ],
        );
        let mut def = definition(Entity::Doors, true);
        def.shape = Shape::PerRoom;
        def.columns = vec!["Mark".to_string()];

        let result = run(&state, &def);

        assert_eq!(result.rows, vec![vec!["G01".to_string(), "2".to_string(), "D-01, D-02".to_string()]]);
    }

    /// Every cell has a header. The grouped shape writes a count the caller
    /// never asked for, so it has to be named -- a row one cell wider than its
    /// header renders as every column shifted left, which reads as bad data
    /// rather than as a bug.
    #[test]
    fn test_the_grouped_count_cell_has_a_column_of_its_own() {
        let state = state_with(
            vec![room("r1", "ENTRY", &[("Number", "G01")])],
            vec![door("d1", Some("r1"), &[("Mark", "D-01")])],
        );
        let mut def = definition(Entity::Doors, true);
        def.shape = Shape::PerRoom;
        def.columns = vec!["Mark".to_string()];

        let result = run(&state, &def);

        assert_eq!(
            result.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["Number", "door count", "Mark"]
        );
        assert_eq!(result.rows[0].len(), result.columns.len());
    }

    /// The cap truncates the rows and leaves the count honest, which is what
    /// makes "first N of M" true rather than reassuring.
    #[test]
    fn test_limit_truncates_rows_but_not_the_total() {
        let state = state_with(
            vec![room("r1", "ENTRY", &[("Number", "G01")])],
            vec![
                door("d1", Some("r1"), &[]),
                door("d2", Some("r1"), &[]),
                door("d3", Some("r1"), &[]),
            ],
        );
        let mut def = definition(Entity::Doors, true);
        def.limit = Some(2);

        let result = run(&state, &def);

        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.total_rows, 3);
    }

    /// Nothing pushed is `None` -- the 204 -- and a different answer from a
    /// report with no rows. A table cannot tell those apart on its own.
    #[test]
    fn test_an_entity_never_pushed_is_none() {
        let state = state_with(vec![room("r1", "ENTRY", &[])], vec![]);
        let def = definition(Entity::Windows, false);
        assert!(build_report(&state, &scope(), &def).unwrap().is_none());
    }

    /// The refusal is a message, not an empty table: an empty report would
    /// read as "nothing matched".
    #[test]
    fn test_by_room_is_refused_for_an_entity_that_carries_no_attribution() {
        let state = state_with(vec![room("r1", "ENTRY", &[])], vec![door("d1", Some("r1"), &[])]);
        let def = definition(Entity::Rooms, true);
        let err = build_report(&state, &scope(), &def).expect_err("rooms cannot be reported by room");
        assert!(format!("{err:?}").contains("cannot be reported by room"));
    }

    /// CSV is written here so a download and an MCP host cannot differ. RFC
    /// 4180: CRLF records, doubled quotes, and a field quoted only when it
    /// carries a separator.
    #[test]
    fn test_csv_quotes_only_what_needs_it() {
        let result = ReportResult {
            revision: String::new(),
            columns: vec![
                Column { name: "Room".to_string(), side: "room" },
                Column { name: "Finding".to_string(), side: "element" },
            ],
            rows: vec![
                vec!["G01".to_string(), "plain".to_string()],
                vec!["G02".to_string(), "Cardiology, North".to_string()],
                vec!["G03".to_string(), "says \"hello\"".to_string()],
            ],
            total_rows: 3,
            unmatched_rows: 0,
        };

        let csv = to_csv(&result);
        let lines: Vec<&str> = csv.split("\r\n").collect();
        assert_eq!(lines[0], "Room,Finding");
        assert_eq!(lines[1], "G01,plain");
        assert_eq!(lines[2], "G02,\"Cardiology, North\"");
        assert_eq!(lines[3], "G03,\"says \"\"hello\"\"\"");
    }

    #[test]
    fn test_entity_names_round_trip_the_wire_spelling() {
        for (name, entity) in [
            ("rooms", Entity::Rooms),
            ("doors", Entity::Doors),
            ("windows", Entity::Windows),
            ("ceilings", Entity::Ceilings),
            ("floors", Entity::Floors),
            ("spaces", Entity::Spaces),
            ("ffe", Entity::Ffe),
        ] {
            assert_eq!(Entity::parse(name), Some(entity));
        }
        assert_eq!(Entity::parse("walls"), None);
    }
}
