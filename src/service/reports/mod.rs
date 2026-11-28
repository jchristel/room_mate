//! Tabular reports: a schedule of one entity, or one entity **by room**.
//!
//! The page is `/reports/` (`src-js/reports/`); the design, and what is still
//! unbuilt, are in `docs/STRATEGY-REPORTS.md`.
//!
//! **Why this exists at all, rather than the page joining payloads itself.**
//! The checks and the milestone comparison each read one endpoint that had
//! already done the work, so they needed nothing here. A schedule does not: it
//! reads a whole entity payload, and `/ffe` is over a hundred MB for one storey of a large project with
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
use std::sync::Arc;

use serde::Serialize;

use crate::contract::{CeilingPayload, DoorPayload, FloorPayload, PropertyPresence, WindowPayload};
use crate::settings::BuiltinPropertyDef;
use crate::state::AppState;
use crate::storage::SnapshotKind;

use super::items::{self, ItemScope};
use super::openings::{self, OpeningKind, OpeningScope};
use super::rooms::{self, holds_for, FilterTarget, Mode, Op, Predicate, RoomResponse, RoomScope};
use super::spaces::{self, SpaceScope};
use super::surfaces::{self, SurfaceScope};
use super::ServiceError;

mod catalog;
mod filter;
mod read;
mod rows;
pub use catalog::*;
pub use filter::*;
use read::*;
pub use rows::*;

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

    /// Which entities can be reported by room. Rooms cannot: they are the
    /// other side of the join.
    ///
    /// **Spaces join differently from everything else here**, and it is worth
    /// knowing before reading `read_spaces`: every other entity names its room
    /// (or overlaps it) within one model, while a space matches on a *key
    /// value*, project-wide, because the space and the room live in different
    /// Revit files by construction. The pairing comes from
    /// `spaces::match_spaces`, which `SpaceReport` also uses — one question,
    /// one answer.
    pub fn joins_rooms(self) -> bool {
        !matches!(self, Entity::Rooms)
    }
}

/// One row per match, or one row per room with the matches aggregated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    PerMatch,
    PerRoom,
}

/// Which side of the join a condition asks about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The room: read from the room the element is attributed to.
    Room,
    /// The element: a door, a ceiling, an item.
    Element,
    /// What the join measured — `overlap_area` and friends.
    Join,
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
    /// Rows to skip before `limit`, to read a long report in pages. `total_rows` still counts them all.
    pub offset: Option<usize>,
    /// The filter, or `None` for every row.
    pub filter: Option<ReportFilter>,
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
    /// Why the answer looks the way it does, when that is not visible from the
    /// rows — "no space key is configured, so nothing was matched". Absent
    /// whenever the table speaks for itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
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

    check_intrinsics(def)?;

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
    if def.entity == Entity::Spaces && def.by_room {
        result.note = spaces_note(state, scope);
    }

    result.total_rows = result.rows.len();
    if let Some(offset) = def.offset {
        result.rows.drain(..offset.min(result.rows.len()));
    }
    if let Some(limit) = def.limit {
        result.rows.truncate(limit);
    }
    Ok(Some(result))
}

/// Refuse a `$` name nothing answers.
///
/// **An unknown intrinsic used to read as an empty cell**, which looks exactly like a field the data
/// leaves blank: `$level` once returned a column of nothing and no error. A `$` name is never a
/// property of the model (those are named plainly), so a stranger is a typo and is said so, with
/// the names that exist. The room side of a by-room report has fewer than the entity side: a room
/// is read as a stored room there, which has no level NAME, only `$level_id`.
fn check_intrinsics(def: &ReportDefinition) -> Result<(), ServiceError> {
    let entity: Vec<String> = intrinsics(def.entity).into_iter().map(|c| c.name).collect();
    let room: Vec<String> = ["$id", "$name", "$level_id"].iter().map(|s| (*s).to_string()).collect();
    let mut asked: Vec<(&str, &[String], &'static str)> = Vec::new();
    asked.extend(def.columns.iter().map(|c| (c.as_str(), entity.as_slice(), def.entity.one())));
    asked.extend(def.room_columns.iter().map(|c| (c.as_str(), room.as_slice(), "room")));
    if let Some(filter) = &def.filter {
        filter_fields(filter, &mut |side, field| {
            match side {
                Side::Element => asked.push((field, entity.as_slice(), def.entity.one())),
                Side::Room => asked.push((field, room.as_slice(), "room")),
                Side::Join => {}
            };
        });
    }
    for (name, valid, what) in asked {
        if name.starts_with('$') && !valid.iter().any(|v| v == name) {
            return Err(ServiceError::Invalid(format!(
                "unknown {what} column \"{name}\"; the $ names for {what} are: {}",
                valid.join(", ")
            )));
        }
    }
    Ok(())
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

    /// Rooms are the one entity with no by-room shape -- they are the other
    /// side of the join -- and the refusal is a message rather than an empty
    /// report, since an empty table would read as "nothing matched".
    ///
    /// **Spaces joined this list on 2026-09-19**, once `spaces::match_spaces`
    /// existed for both this and the QA report to read. Before that a by-room
    /// spaces report would have had to match again, which is one question with
    /// two answers.
    #[test]
    fn test_only_the_entities_that_carry_an_attribution_join_rooms() {
        assert!(Entity::Ceilings.joins_rooms());
        assert!(Entity::Floors.joins_rooms());
        assert!(Entity::Doors.joins_rooms());
        assert!(Entity::Ffe.joins_rooms());
        assert!(Entity::Spaces.joins_rooms());
        assert!(!Entity::Rooms.joins_rooms());
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
            tag_type_code: None,
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            windows: Default::default(),
            ffe: Default::default(),
            hierarchy_exclusions: vec![],
            routing: Default::default(),
            blueprint_audit: Default::default(),
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
            sub_components: Vec::new(),
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
            offset: None,
            filter: None,
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

    /// The catalog lists a name, an arm answers it. A listed name no arm answers is a column of
    /// blanks that reads as missing data (`$level_id` on rooms was exactly that).
    #[test]
    fn test_every_listed_room_intrinsic_answers_for_a_room() {
        let state = state_with(vec![room("r1", "ENTRY", &[])], vec![]);
        let mut def = definition(Entity::Rooms, false);
        def.columns = vec!["$id".into(), "$name".into(), "$level_id".into()];
        let result = run(&state, &def);
        assert!(result.rows[0].iter().all(|cell| !cell.is_empty()), "{:?}", result.rows[0]);
    }

    #[test]
    fn test_offset_skips_rows_and_the_total_still_counts_them() {
        let state = state_with(
            vec![room("r1", "ENTRY", &[("Number", "G01")])],
            vec![
                door("d1", Some("r1"), &[]),
                door("d2", Some("r1"), &[]),
                door("d3", Some("r1"), &[]),
            ],
        );
        let mut def = definition(Entity::Doors, false);
        def.offset = Some(1);
        def.limit = Some(1);
        let result = run(&state, &def);
        assert_eq!((result.rows.len(), result.total_rows), (1, 3));
        def.offset = Some(99);
        assert!(run(&state, &def).rows.is_empty(), "an offset past the end is an empty page");
    }

    #[test]
    fn test_an_unknown_dollar_name_is_refused_with_the_valid_ones() {
        let state = state_with(vec![room("r1", "ENTRY", &[])], vec![door("d1", Some("r1"), &[])]);
        let mut def = definition(Entity::Doors, false);
        def.columns = vec!["$levle".to_string()];
        let err = format!("{:?}", build_report(&state, &scope(), &def).expect_err("a typo is refused"));
        assert!(err.contains("$levle") && err.contains("$level_id") && err.contains("$level,"), "{err}");
        def.columns = vec!["$level".to_string(), "Mark".to_string()];
        assert!(build_report(&state, &scope(), &def).is_ok(), "$level and a plain property are fine");
    }

    #[test]
    fn test_level_reads_as_the_name_the_models_level_declares() {
        let names: LevelNames = Arc::new(BTreeMap::from([("10".to_string(), "LEVEL 2".to_string())]));
        let read = |id: &'static str| -> FieldReader {
            Box::new(move |_, property, _| match property {
                "$level_id" => PropertyPresence::Present(id.to_string()),
                _ => PropertyPresence::Absent,
            })
        };
        let named = with_level_name(read("10"), names.clone());
        assert_eq!(named(None, "$level", &[]), PropertyPresence::Present("LEVEL 2".to_string()));
        let unknown = with_level_name(read("99"), names);
        assert_eq!(
            unknown(None, "$level", &[]),
            PropertyPresence::Absent,
            "an id is never passed off as a name"
        );
        assert_eq!(unknown(None, "$level_id", &[]), PropertyPresence::Present("99".to_string()));
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

    // ---------- filters ----------

    fn wire(json: &str) -> ReportFilter {
        let parsed: FilterWire = serde_json::from_str(json).expect("wire shape parses");
        parsed.parse(&std::collections::BTreeSet::new()).expect("filter parses")
    }

    fn filtered(state: &AppState, mut def: ReportDefinition, json: &str) -> Vec<Vec<String>> {
        def.filter = Some(wire(json));
        run(state, &def).rows
    }

    /// Two doors in one room, one in another: enough for every set rule below.
    fn two_rooms() -> AppState {
        state_with(
            vec![
                room("r1", "ENTRY", &[("Number", "G01"), ("Department", "Circulation")]),
                room("r2", "KITCHEN", &[("Number", "G02"), ("Department", "Living")]),
                room("r3", "POOL EX", &[("Number", "G05"), ("Department", "External")]),
            ],
            vec![
                door("d1", Some("r1"), &[("Mark", "D-01"), ("Type", "SGL")]),
                door("d2", Some("r1"), &[("Mark", "D-02"), ("Type", "EXT")]),
                door("d3", Some("r2"), &[("Mark", "D-03"), ("Type", "SGL")]),
            ],
        )
    }

    /// A schedule filters per row, which is what a row means there.
    #[test]
    fn test_a_schedule_filters_row_by_row() {
        let state = two_rooms();
        let mut def = definition(Entity::Doors, false);
        def.columns = vec!["Mark".to_string()];

        let rows = filtered(&state, def, r#"{"mode":"all","items":[{"field":"Type","op":"eq","value":"SGL"}]}"#);

        assert_eq!(rows, vec![vec!["D-01".to_string()], vec!["D-03".to_string()]]);
    }

    /// Text folds case unless asked otherwise. A modeller's capitalisation is
    /// not data, and the miss it causes is the kind nobody notices.
    #[test]
    fn test_text_ignores_case_by_default_and_respects_it_when_asked() {
        let state = two_rooms();
        let mut def = definition(Entity::Doors, false);
        def.columns = vec!["Mark".to_string()];

        let folded = filtered(
            &state,
            def_clone(&def),
            r#"{"mode":"all","items":[{"field":"Type","op":"eq","value":"sgl"}]}"#,
        );
        assert_eq!(folded.len(), 2, "sgl finds SGL");

        let exact = filtered(
            &state,
            def,
            r#"{"mode":"all","items":[{"field":"Type","op":"eq","value":"sgl","case_sensitive":true}]}"#,
        );
        assert!(exact.is_empty(), "with Match case on, sgl is not SGL");
    }

    /// A positive condition on the associated side asks whether the room HAS
    /// one, so it keeps every row of a room that does -- including the doors
    /// that do not match it themselves? No: the conjunctive positives also
    /// narrow the rows, so only the matching doors are listed.
    #[test]
    fn test_a_positive_condition_keeps_the_room_and_narrows_its_rows() {
        let state = two_rooms();
        let mut def = definition(Entity::Doors, true);
        def.columns = vec!["Mark".to_string()];
        def.room_columns = vec!["Number".to_string()];

        let rows = filtered(&state, def, r#"{"mode":"all","items":[{"field":"Type","op":"eq","value":"EXT"}]}"#);

        assert_eq!(rows, vec![vec!["G01".to_string(), "D-02".to_string()]]);
    }

    /// **The rule the whole builder turns on.** "Type is not EXT" asks for
    /// rooms with NO door of that type -- so ENTRY is dropped whole, because
    /// one of its doors IS EXT, and KITCHEN keeps both. Per-row negation would
    /// have kept ENTRY for its other door, answering a different question.
    #[test]
    fn test_a_negative_condition_asks_about_the_whole_set() {
        let state = two_rooms();
        let mut def = definition(Entity::Doors, true);
        def.columns = vec!["Mark".to_string()];
        def.room_columns = vec!["Number".to_string()];

        let rows = filtered(&state, def, r#"{"mode":"all","items":[{"field":"Type","op":"ne","value":"EXT"}]}"#);

        let numbers: Vec<&String> = rows.iter().map(|r| &r[0]).collect();
        assert!(!numbers.contains(&&"G01".to_string()), "ENTRY is dropped whole for having one: {rows:?}");
        assert!(
            rows.contains(&vec!["G02".to_string(), "D-03".to_string()]),
            "KITCHEN keeps its door: {rows:?}"
        );
    }

    /// ...and a room with none at all satisfies a negative condition, whatever
    /// the switch says: nothing it has is EXT. The switch decides only when the
    /// filter says nothing about this entity.
    #[test]
    fn test_a_negative_condition_pulls_in_rooms_with_no_elements() {
        let state = two_rooms();
        let mut def = definition(Entity::Doors, true);
        def.columns = vec!["Mark".to_string()];
        def.room_columns = vec!["Number".to_string()];
        def.include_rooms_without = false;

        let rows = filtered(&state, def, r#"{"mode":"all","items":[{"field":"Type","op":"ne","value":"EXT"}]}"#);

        assert!(
            rows.contains(&vec!["G05".to_string(), String::new()]),
            "POOL EX has no door of any type: {rows:?}"
        );
    }

    /// The other direction: a positive condition excludes a room with none even
    /// when the switch asks for them. An explicit condition beats a default.
    #[test]
    fn test_a_positive_condition_excludes_rooms_with_none_despite_the_switch() {
        let state = two_rooms();
        let mut def = definition(Entity::Doors, true);
        def.columns = vec!["Mark".to_string()];
        def.room_columns = vec!["Number".to_string()];
        def.include_rooms_without = true;

        let rows = filtered(&state, def, r#"{"mode":"all","items":[{"field":"Type","op":"eq","value":"SGL"}]}"#);

        assert!(
            !rows.iter().any(|r| r[0] == "G05"),
            "a room with no doors cannot have an SGL one: {rows:?}"
        );
    }

    /// A room-side condition reads the room, and an `any` group is an OR.
    #[test]
    fn test_room_conditions_and_or_groups() {
        let state = two_rooms();
        let mut def = definition(Entity::Doors, true);
        def.columns = vec!["Mark".to_string()];
        def.room_columns = vec!["Number".to_string()];

        let rows = filtered(
            &state,
            def,
            r#"{"mode":"any","items":[
                 {"side":"room","field":"Department","op":"eq","value":"Living"},
                 {"side":"room","field":"Number","op":"eq","value":"G01"}]}"#,
        );

        let numbers: Vec<&String> = rows.iter().map(|r| &r[0]).collect();
        assert!(numbers.contains(&&"G01".to_string()) && numbers.contains(&&"G02".to_string()));
        assert!(!numbers.contains(&&"G05".to_string()));
    }

    /// `is blank` is the only way to ask about a missing value, and the only
    /// operator a missing value satisfies.
    #[test]
    fn test_blank_is_the_only_operator_an_absent_value_matches() {
        let state = state_with(
            vec![room("r1", "ENTRY", &[("Number", "G01")])],
            vec![door("d1", Some("r1"), &[("Mark", "D-01")]), door("d2", Some("r1"), &[])],
        );
        let mut def = definition(Entity::Doors, false);
        def.columns = vec!["$id".to_string()];

        let blank = filtered(&state, def_clone(&def), r#"{"mode":"all","items":[{"field":"Mark","op":"blank"}]}"#);
        assert_eq!(blank, vec![vec!["d2".to_string()]]);

        let present =
            filtered(&state, def_clone(&def), r#"{"mode":"all","items":[{"field":"Mark","op":"has_value"}]}"#);
        assert_eq!(present, vec![vec!["d1".to_string()]]);

        // The door with no Mark does not match "is not D-01" either: no value
        // is not evidence of a different value.
        let negative = filtered(&state, def, r#"{"mode":"all","items":[{"field":"Mark","op":"ne","value":"D-01"}]}"#);
        assert!(negative.is_empty(), "absent matches nothing but blank: {negative:?}");
    }

    /// A malformed filter is named, not ignored: a condition nobody can see is
    /// worse than an error nobody wanted.
    #[test]
    fn test_a_bad_filter_says_what_is_wrong() {
        let parsed: FilterWire =
            serde_json::from_str(r#"{"mode":"all","items":[{"field":"Mark","op":"sounds_like","value":"D"}]}"#)
                .unwrap();
        let err = parsed.parse(&std::collections::BTreeSet::new()).expect_err("unknown operator");
        assert!(err.contains("sounds_like"), "{err}");

        let missing: FilterWire =
            serde_json::from_str(r#"{"mode":"all","items":[{"field":"Mark","op":"eq"}]}"#).unwrap();
        let err = missing.parse(&std::collections::BTreeSet::new()).expect_err("a value is required");
        assert!(err.contains("needs a value"), "{err}");
    }

    fn def_clone(def: &ReportDefinition) -> ReportDefinition {
        ReportDefinition {
            entity: def.entity,
            by_room: def.by_room,
            columns: def.columns.clone(),
            room_columns: def.room_columns.clone(),
            measures: def.measures.clone(),
            shape: def.shape,
            include_rooms_without: def.include_rooms_without,
            include_unattributed: def.include_unattributed,
            limit: def.limit,
            offset: def.offset,
            filter: None,
        }
    }

    /// An empty room column on a spaces report means one of three things and
    /// the table cannot say which, so the report says it. This is the state a
    /// project hits first: spaces pushed, no key configured, nothing attempted.
    #[test]
    fn test_a_spaces_report_says_why_nothing_matched() {
        // `bundle()` configures no space key, which is the default a project
        // starts from.
        let state = state_with(vec![room("r1", "ENTRY", &[("Number", "G01")])], vec![]);
        let scope = scope();

        let note = spaces_note(&state, &scope).expect("a note, because nothing was attempted");

        assert!(note.contains("No space key is configured"), "{note}");
        assert!(note.contains("comparison_key"), "and it names the setting to change: {note}");
    }

    // ---------- the column catalog ----------

    /// The names a picker offers come from the stored dictionary, so a property
    /// nobody has pushed is not offered and one that was is -- without
    /// assembling the entity to find out.
    #[test]
    fn test_the_catalog_offers_what_the_snapshots_actually_carry() {
        let state = state_with(
            vec![room("r1", "ENTRY", &[("Number", "G01"), ("Department", "Circulation")])],
            vec![door("d1", Some("r1"), &[("Mark", "D-01"), ("Fire Rating", "60")])],
        );

        let catalog = column_catalog(&state, Some("p1"), Entity::Doors);
        let names: Vec<&str> = catalog.entity.iter().map(|c| c.name.as_str()).collect();

        assert!(names.contains(&"Mark") && names.contains(&"Fire Rating"), "door properties: {names:?}");
        assert!(!names.contains(&"Department"), "a ROOM property is not a door column: {names:?}");
        // Intrinsics are code facts and ride along, so `$to_room` is offered
        // even though no dictionary mentions it.
        assert!(names.contains(&"$to_room"), "intrinsics too: {names:?}");
    }

    /// The room side is offered only for an entity that can be reported by
    /// room, and it is the ROOMS vocabulary rather than the entity's.
    #[test]
    fn test_the_room_side_is_offered_only_where_it_means_something() {
        let state = state_with(
            vec![room("r1", "ENTRY", &[("Number", "G01")])],
            vec![door("d1", Some("r1"), &[("Mark", "D-01")])],
        );

        let doors = column_catalog(&state, Some("p1"), Entity::Doors);
        assert!(doors.rooms.iter().any(|c| c.name == "Number"));

        let rooms = column_catalog(&state, Some("p1"), Entity::Rooms);
        assert!(rooms.rooms.is_empty(), "a room schedule has no second side");
    }

    /// The measures are the join's, so they are served rather than left for the
    /// page to remember -- and an entity whose join measures nothing offers
    /// none rather than an empty-looking picker.
    #[test]
    fn test_measures_are_per_entity() {
        let state = state_with(vec![room("r1", "ENTRY", &[])], vec![door("d1", Some("r1"), &[])]);

        let ceilings = column_catalog(&state, Some("p1"), Entity::Ceilings);
        assert!(ceilings.measures.iter().any(|m| m.name == "overlap_area"));

        let doors = column_catalog(&state, Some("p1"), Entity::Doors);
        assert!(doors.measures.is_empty(), "an opening's attribution measures nothing");
    }

    /// The value type decides which operators a filter offers, and it comes
    /// from what the export stated rather than from the name.
    #[test]
    fn test_value_type_reads_revits_storage_type() {
        assert_eq!(value_type(Some("Double")), "number");
        assert_eq!(value_type(Some("Integer")), "number");
        assert_eq!(value_type(Some("String")), "text");
        // An ElementId is compared, never ordered.
        assert_eq!(value_type(Some("ElementId")), "text");
        // No stated type is text: the safe half, since every operator text
        // offers works on a number written as one.
        assert_eq!(value_type(None), "text");
    }

    /// A reference label's type comes from the field config that declares it,
    /// which is the only place anything states it -- a CSV column is text on
    /// the wire whatever it holds.
    #[test]
    fn test_a_reference_label_takes_its_type_from_its_field_config() {
        use crate::settings::{FieldType, ReferenceFieldConfig};
        let field = |label: &str, field_type: FieldType| ReferenceFieldConfig {
            label: label.to_string(),
            field_type,
            format: None,
            revit_format: None,
            qa: None,
        };
        let fields = vec![
            field("NetArea", FieldType::Numeric),
            field("Department", FieldType::String),
            field("Issued", FieldType::Date),
        ];

        assert_eq!(reference_value_type(&fields, "NetArea"), "number");
        assert_eq!(reference_value_type(&fields, "Department"), "text");
        // A date is text: ordering them as strings is right only for ISO-8601,
        // and the config carries a `format` precisely because they are not.
        assert_eq!(reference_value_type(&fields, "Issued"), "text");
        // A label nobody configured is text -- the safe half.
        assert_eq!(reference_value_type(&fields, "Unconfigured"), "text");
    }

    /// CSV is written here so a download and an MCP host cannot differ. RFC
    /// 4180: CRLF records, doubled quotes, and a field quoted only when it
    /// carries a separator.
    #[test]
    fn test_csv_quotes_only_what_needs_it() {
        let result = ReportResult {
            note: None,
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
