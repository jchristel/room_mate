//! The arguments each MCP tool takes: one struct per tool (or family of tools), deserialised from the call and described to the model by its schema.

use super::*;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct ProjectIdParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct BlueprintParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// The blueprint id, as returned by `list_blueprints`.
    pub(super) blueprint_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct CaptureBlueprintParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// The room's model. Omit it and the room id is looked up among the project's rooms (refused, naming the models, if more than one has it): a room id is unique only within its model.
    pub(super) model_id: Option<String>,
    /// The room's id.
    pub(super) room_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct ResolveBlueprintParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// The blueprint id, as returned by `list_blueprints`.
    pub(super) blueprint_id: String,
    /// The target room's model. Omit it and the room id is looked up among the project's rooms.
    pub(super) model_id: Option<String>,
    /// The target room.
    pub(super) room_id: String,
    /// The revision to place; the blueprint's current one when omitted.
    pub(super) revision: Option<u32>,
    /// Pin the edge mapping: blueprint edge i goes to room edge (i + shift) % edges. Omit to use the best fit by shape.
    pub(super) shift: Option<usize>,
    /// Map the edges by hand: the room edge for each blueprint edge, in blueprint edge order. Wins over `shift`, and is how a room with a DIFFERENT number of edges is placed.
    pub(super) edge_map: Option<Vec<usize>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct ValidationParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// The most entries any list in the report keeps (default 200, ceiling 5000). The report is mostly counts,
    /// but carries lists of offenders, which can run to tens of thousands; the answer's `cut_lists` names
    /// each list that was cut and how many it held.
    #[serde(default)]
    pub(super) list_limit: Option<usize>,
}

/// A list with nothing to scope it by.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct PageParams {
    /// Entries per page (default 200, ceiling 1000).
    #[serde(default)]
    pub(super) limit: Option<usize>,
    /// Entries to skip, for the next page: pass the answer's `next_offset`.
    #[serde(default)]
    pub(super) offset: Option<usize>,
}

/// A project's list.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct ListParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// Entries per page (default 200, ceiling 1000).
    #[serde(default)]
    pub(super) limit: Option<usize>,
    /// Entries to skip, for the next page: pass the answer's `next_offset`.
    #[serde(default)]
    pub(super) offset: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct ListTasksParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// Only the tasks for this model (the Revit document's title).
    #[serde(default)]
    pub(super) model_id: Option<String>,
    /// Only tasks in this state: queued, in_progress, completed, completed_with_warnings, failed, aborted or dismissed.
    #[serde(default)]
    pub(super) state: Option<String>,
    /// Entries per page (default 200, ceiling 1000).
    #[serde(default)]
    pub(super) limit: Option<usize>,
    /// Entries to skip, for the next page: pass the answer's `next_offset`.
    #[serde(default)]
    pub(super) offset: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct TaskParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// The task id, as returned by `list_tasks`.
    pub(super) task_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct TagRuleParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// The rule id, as returned by `list_tag_rules`.
    pub(super) rule_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct SheetParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// The sheet record id, as returned by `list_sheets`.
    pub(super) sheet_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct SuggestSheetsParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// The room's model. Omit it and the room id is looked up among the project's rooms (refused, naming the models, if more than one has it): a room id is unique only within its model.
    pub(super) model_id: Option<String>,
    /// The room to find a sheet for.
    pub(super) room_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct RoomIdsParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// The rooms, by room id (each looked up among the project's rooms; one whose id is in several models is reported, not guessed).
    pub(super) room_ids: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct PreviewRlsSetParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// The rooms of the set, by room id: they must be one group of adjacent rooms in one model.
    pub(super) room_ids: Vec<String>,
    /// The library sheet to start from (its scales and plan framing). Omit it for the best match.
    #[serde(default)]
    pub(super) sheet_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct PreviewRlsParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// The library sheet to copy, as returned by `list_sheets` or `suggest_sheets`.
    pub(super) sheet_id: String,
    /// The room to make the sheet for.
    pub(super) room_id: String,
    /// `a0` to `a3`. Omit it for the library sheet's own size.
    #[serde(default)]
    pub(super) sheet_size: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct SolveBlueprintParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// The blueprint id, as returned by `list_blueprints`.
    pub(super) blueprint_id: String,
    /// The revision to try. Omit it for the current one.
    #[serde(default)]
    pub(super) revision: Option<u32>,
    /// The rooms to try it in, by room id (at most 500).
    pub(super) room_ids: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct ModelIdParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// The model id, as returned by `list_snapshots`.
    pub(super) model_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct GetRoomsParams {
    /// Scope the merge to one project id. Omit to merge every stored model.
    #[serde(default)]
    pub(super) project: Option<String>,
    /// Opaque building key from `list_buildings`. Omit for no building filter.
    #[serde(default)]
    pub(super) building: Option<String>,
    /// Milestone name from `list_milestones`: serve the snapshots that
    /// milestone pins instead of each model's latest. Omit for latest.
    #[serde(default)]
    pub(super) milestone: Option<String>,
    /// Property predicates, ALL of which must hold (AND), one per element:
    /// ["Department=Cardiology", "Area>=20", "drofus.NetArea>20"]. Operators:
    /// = != > >= < <= and ~ (case-insensitive contains). An unqualified name
    /// is a canonical room property (as listed by get_project_settings'
    /// builtin_properties), plus $name / $id for the room's own fields; a
    /// source-prefixed name (e.g. "drofus.") reads that joined record's field label
    /// (as listed by get_reference_snapshot). A room missing the property -- or
    /// with no joined record for that source at all -- never matches, negative
    /// operators included. Quote a value containing spaces if in doubt.
    /// Omit for no filter.
    #[serde(default)]
    pub(super) filter: Vec<String>,
    /// Keep only the rooms on one level, named as `levels` names it ("LEVEL 02") or by its id;
    /// case-insensitive. Omit for every level. Unknown names are refused with the names there are.
    #[serde(default)]
    pub(super) level: Option<String>,
    /// `summary` (the default) returns each room's id, name, number and level only; `full` returns the
    /// whole record: outline, every property and the reference-source joins (about 3 KB a room).
    #[serde(default)]
    pub(super) detail: Option<String>,
    /// Rooms per page. Defaults to 1000 for a summary and 100 for `full`; lowered to 5000 and 500.
    #[serde(default)]
    pub(super) limit: Option<usize>,
    /// Rooms to skip, for the next page: pass the answer's `next_offset`.
    #[serde(default)]
    pub(super) offset: Option<usize>,
}

/// `get_ceilings` and `get_floors` parameters. Its own type rather than
/// `GetSpacesParams`: there is no `filter` (no reference source declares either
/// entity, so the grammar would advertise an empty vocabulary) and no `model` --
/// a surface is asked about per project, not per model, because the question
/// these entities exist for is "which ceilings / floors are in this room".
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct GetSurfacesParams {
    /// Scope the merge to one project id. Omit to merge every stored model.
    #[serde(default)]
    pub(super) project: Option<String>,
    /// Milestone name from `list_milestones`: serve the snapshots that
    /// milestone pins instead of each model's latest. Omit for latest. Note the
    /// ROOMS it attributes against are pinned by the same milestone, so a
    /// milestone read answers one consistent question.
    #[serde(default)]
    pub(super) milestone: Option<String>,
    /// `summary` (the default) returns each element's identity and room references only; `full` returns the
    /// whole record: footprint, every property, type properties.
    #[serde(default)]
    pub(super) detail: Option<String>,
    /// Elements per page. Defaults to 500 for a summary and 50 for `full`; lowered to 2000 and 200.
    #[serde(default)]
    pub(super) limit: Option<usize>,
    /// Elements to skip, for the next page: pass the answer's `next_offset`.
    #[serde(default)]
    pub(super) offset: Option<usize>,
}

/// `list_saved_reports` parameters.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct SavedReportsParams {
    /// Project id whose saved reports to list.
    pub(super) project: String,
    /// Entries per page (default 200, ceiling 1000).
    #[serde(default)]
    pub(super) limit: Option<usize>,
    /// Entries to skip, for the next page: pass the answer's `next_offset`.
    #[serde(default)]
    pub(super) offset: Option<usize>,
}

/// `get_saved_report` parameters.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct SavedReportParams {
    pub(super) project: String,
    /// Report id, from `list_saved_reports`.
    pub(super) report: String,
}

/// `list_report_columns` parameters.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct ReportColumnsParams {
    /// Project id to ask about.
    pub(super) project: String,
    /// Which entity: rooms, doors, windows, ceilings, floors, spaces or ffe.
    pub(super) entity: String,
}

/// `build_report` parameters -- the report definition, in the shape
/// `service::reports::ReportDefinition` takes. One tool rather than one per
/// report type: the type IS the parameters, and a tool per type would be
/// fourteen tools that differ by two fields.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct BuildReportParams {
    /// Project id to report on.
    pub(super) project: String,
    /// Which entity: rooms, doors, windows, ceilings, floors, spaces or ffe.
    pub(super) entity: String,
    /// Report the entity BY ROOM rather than as a flat schedule. Not available
    /// for rooms (they are the other side of the join) or spaces (their match
    /// is a key match that lives in the QA report, not on the rows).
    #[serde(default)]
    pub(super) by_room: bool,
    /// Columns over the entity, in the same vocabulary `filter` parses: a
    /// property name, a `$intrinsic`, or `source.label` for a joined field.
    #[serde(default)]
    pub(super) columns: Vec<String>,
    /// Columns over the room, for a by-room report.
    #[serde(default)]
    pub(super) room_columns: Vec<String>,
    /// What the join measured. Ceilings and floors carry `overlap_area`,
    /// `fraction_of_element`, `fraction_of_room` and `mean_width`; FF&E carries
    /// `room_origin`.
    #[serde(default)]
    pub(super) measures: Vec<String>,
    /// `per_match` (default) or `per_room`, which groups and aggregates.
    #[serde(default)]
    pub(super) shape: Option<String>,
    /// List rooms nothing matched. Off by default.
    #[serde(default)]
    pub(super) include_rooms_without: bool,
    /// List elements no room matched. ON by default -- an unattributed element
    /// is part of the answer, and dropping it silently makes the totals lie.
    #[serde(default = "mcp_default_true")]
    pub(super) include_unattributed: bool,
    /// Scope to one building, through the room each element is attributed to.
    #[serde(default)]
    pub(super) building: Option<String>,
    /// Milestone name from `list_milestones`; omit for each model's latest.
    #[serde(default)]
    pub(super) milestone: Option<String>,
    /// Cap the rows returned. `total_rows` still counts them all.
    #[serde(default)]
    pub(super) limit: Option<usize>,
    /// Rows to skip before `limit`, to read a long report in pages: pass the answer's `next_offset`.
    #[serde(default)]
    pub(super) offset: Option<usize>,
    /// The filter tree, as JSON: a group is `{"mode": "all"|"any", "items":
    /// [...]}` and a condition is `{"side": "room"|"element"|"join", "field",
    /// "op", "value"}`. Operators: eq, ne, gt, ge, lt, le, contains,
    /// not_contains, starts_with, ends_with, between (with `value2`), blank,
    /// has_value. Text folds case unless `case_sensitive` is set. ON THE
    /// ASSOCIATED SIDE A CONDITION ASKS ABOUT THE ROOM'S WHOLE SET: `ne` and
    /// `not_contains` mean the room has NONE that match, so a room with no
    /// elements at all satisfies them and fails every positive condition.
    /// Untyped here because the shape is a tree; the server names anything it
    /// cannot read.
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Value>")]
    pub(super) filter: Option<reports::FilterWire>,
}

/// `get_spaces` parameters. Its own type rather than `GetDoorsParams`: there is
/// no `building` -- a space cannot be scoped by one -- and there is a `model`,
/// because this entity's first question is asked per services model.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct GetSpacesParams {
    /// Scope the merge to one project id. Omit to merge every stored model.
    #[serde(default)]
    pub(super) project: Option<String>,
    /// Scope to ONE model id -- the services model whose spaces you want. This
    /// entity's first question ("does this model have spaces at all") is asked
    /// per model, which is why this exists here and not on get_rooms.
    #[serde(default)]
    pub(super) model: Option<String>,
    /// Milestone name from `list_milestones`: serve the spaces snapshots that
    /// milestone pins instead of each model's latest. Omit for latest.
    #[serde(default)]
    pub(super) milestone: Option<String>,
    /// Property predicates, ALL of which must hold (AND), one per element:
    /// ["$enclosure!=enclosed"]. Same operators as get_rooms' filter. An
    /// unqualified name reads the space's own properties; the intrinsics are
    /// $id, $name, $level_id, $model_id and $enclosure. Omit for no filter.
    #[serde(default)]
    pub(super) filter: Vec<String>,
    /// `summary` (the default) returns each element's identity and room references only; `full` returns the
    /// whole record: footprint, every property, type properties.
    #[serde(default)]
    pub(super) detail: Option<String>,
    /// Elements per page. Defaults to 500 for a summary and 50 for `full`; lowered to 2000 and 200.
    #[serde(default)]
    pub(super) limit: Option<usize>,
    /// Elements to skip, for the next page: pass the answer's `next_offset`.
    #[serde(default)]
    pub(super) offset: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
/// Parameters for both  and .
///
/// One struct, because the two reads take identical arguments -- the entity is
/// the tool you call, not a field you pass. A second identical type would only
/// give the two descriptions somewhere to drift apart.
pub(super) struct GetDoorsParams {
    /// Scope the merge to one project id. Omit to merge every stored model.
    #[serde(default)]
    pub(super) project: Option<String>,
    /// Opaque building key from `list_buildings`. A door's building is the
    /// building of the room it is attributed to, so a door attributed to no
    /// room ("homeless") matches no building. Omit for no building filter.
    #[serde(default)]
    pub(super) building: Option<String>,
    /// Milestone name from `list_milestones`: serve the doors snapshots that
    /// milestone pins instead of each model's latest. Omit for latest.
    #[serde(default)]
    pub(super) milestone: Option<String>,
    /// Property predicates, ALL of which must hold (AND), one per element:
    /// ["$to_room=2621156", "Mark=29"]. Same operators as get_rooms' filter.
    /// An unqualified name reads the door's own properties, instance tier then
    /// family type tier; the intrinsics are $id, $type_id, $type_name,
    /// $level_id, $from_room and $to_room. A door missing the property never
    /// matches, negative operators included -- so an external door (null on one
    /// side) does not match "$to_room!=x" either. Omit for no filter.
    #[serde(default)]
    pub(super) filter: Vec<String>,
    /// `summary` (the default) returns each element's identity and room references only; `full` returns the
    /// whole record: footprint, every property, type properties.
    #[serde(default)]
    pub(super) detail: Option<String>,
    /// Elements per page. Defaults to 500 for a summary and 50 for `full`; lowered to 2000 and 200.
    #[serde(default)]
    pub(super) limit: Option<usize>,
    /// Elements to skip, for the next page: pass the answer's `next_offset`.
    #[serde(default)]
    pub(super) offset: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct AdjacencyParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// Opaque building key from `list_buildings`. Omit for no building filter.
    #[serde(default)]
    pub(super) building: Option<String>,
    /// Milestone name from `list_milestones`: build the graph from the
    /// snapshots that milestone pins instead of each model's latest. Omit for
    /// latest.
    #[serde(default)]
    pub(super) milestone: Option<String>,
    /// Largest gap between two room boundaries, in decimal feet, still counted
    /// as a shared wall. **Omit it unless you have a reason not to:** the
    /// server then derives the tolerance from the project's declared
    /// `[areas] max_wall_thickness` and the boundary regime the models
    /// declared — zero when every level in scope is drawn to wall centrelines
    /// (neighbours touch exactly), the declared thickness otherwise. The
    /// response echoes the value actually applied. Pass one only to probe a
    /// different tolerance than the project declares; must be between 0 and 5.
    #[serde(default)]
    pub(super) wall_max: Option<f64>,
    /// `summary` (the default) returns each room as id, name, level and centroid; `full` adds its classification
    /// and every joined reference record (about 750 bytes a room).
    #[serde(default)]
    pub(super) detail: Option<String>,
    /// Rooms per page. Defaults to 1000 for a summary and 100 for `full`; lowered to 5000 and 500.
    #[serde(default)]
    pub(super) limit: Option<usize>,
    /// Rooms to skip, for the next page: pass `next_offset`.
    #[serde(default)]
    pub(super) offset: Option<usize>,
    /// Shared walls per page (default 2000, ceiling 10000), skipped by `edge_offset`.
    #[serde(default)]
    pub(super) edge_limit: Option<usize>,
    #[serde(default)]
    pub(super) edge_offset: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct StackParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// The room to find a stack for.
    pub(super) room: String,
    /// The room's model, only needed when its id exists in several models.
    #[serde(default)]
    pub(super) room_model: Option<String>,
    /// Milestone name from `list_milestones`. Omit for the latest snapshots.
    #[serde(default)]
    pub(super) milestone: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct ReachSourceParams {
    /// The room the start point is in. A room id is unique only within its model: if two models
    /// hold it the call is refused and names them, and `model_id` resolves it.
    pub(super) room_id: String,
    /// The model of the room, only needed when `room_id` exists in several.
    #[serde(default)]
    pub(super) model_id: Option<String>,
    /// Where in the room the start point is, in the plan's coordinates (feet, y up, the frame
    /// `get_connectivity`'s route points are in): both `x` and `y`, or neither for a point
    /// inside the room. A point outside the room is moved onto it.
    #[serde(default)]
    pub(super) x: Option<f64>,
    #[serde(default)]
    pub(super) y: Option<f64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct ReachParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// The start points: every cell takes the walk to its NEAREST source.
    pub(super) sources: Vec<ReachSourceParams>,
    /// The length of walk to measure, in mm (30 m is 30000). Required.
    pub(super) limit_mm: f64,
    /// Grid spacing in mm; 1000 when omitted. Finer is slower and larger: a request that
    /// would hold more than 600000 cells is refused with the count.
    #[serde(default)]
    pub(super) spacing_mm: Option<f64>,
    /// Treat every open zone's closed walls (its `disconnects` and its `hub`) as open. A
    /// cut says where a person routes; a coverage question is not bound by it.
    #[serde(default)]
    pub(super) ignore_cuts: bool,
    /// `exact` (default): the exact walk to each cell. `grid`: Dijkstra over the cells of each
    /// room, the lattice approximation, a little longer between cells (median 0.2%, worst seen 14%) and
    /// dependent on `spacing_mm`.
    #[serde(default)]
    pub(super) method: Option<String>,
    /// Opaque building key from `list_buildings`. Omit for no building filter.
    #[serde(default)]
    pub(super) building: Option<String>,
    /// Milestone name from `list_milestones`. Omit for the latest snapshots.
    #[serde(default)]
    pub(super) milestone: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct ConnectivityParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// Opaque building key from `list_buildings`. Scopes the ROOMS; a door is in
    /// the graph when its rooms are. Omit for no building filter.
    #[serde(default)]
    pub(super) building: Option<String>,
    /// Milestone name from `list_milestones`: build the graph from the rooms and
    /// doors that milestone pins instead of each model's latest. Omit for latest.
    #[serde(default)]
    pub(super) milestone: Option<String>,
    /// Property predicates over DOORS, ALL of which must hold (AND). A door that
    /// fails one is not in the graph, so a route that needed it stops existing:
    /// ["Fire Rating!=FRL120"] asks for a route that avoids those doors. Same
    /// grammar as get_doors' filter. Omit for every door.
    #[serde(default)]
    pub(super) door_filter: Vec<String>,
    /// Start room id. Give BOTH `from` and `to` to get a route, or neither for
    /// the graph alone. A room id is unique only within its model: if two models
    /// hold it the call is refused and names them, and `from_model` resolves it.
    #[serde(default)]
    pub(super) from: Option<String>,
    /// The model of the start room, only needed when `from` exists in several.
    #[serde(default)]
    pub(super) from_model: Option<String>,
    /// End room id. See `from`.
    #[serde(default)]
    pub(super) to: Option<String>,
    /// The model of the end room, only needed when `to` exists in several.
    #[serde(default)]
    pub(super) to_model: Option<String>,
    /// Where in the start room the route begins, in the plan's coordinates (a
    /// route's `start` and `end` are in the same frame). Both `from_x` and `from_y`,
    /// or neither for the room's centre. A point outside the room is moved onto
    /// its outline and `path.note` says so.
    #[serde(default)]
    pub(super) from_x: Option<f64>,
    #[serde(default)]
    pub(super) from_y: Option<f64>,
    /// Where in the end room the route finishes; same rules as `from_x` and `from_y`.
    #[serde(default)]
    pub(super) to_x: Option<f64>,
    #[serde(default)]
    pub(super) to_y: Option<f64>,
    /// The width, in mm, of an object that has to make the trip (a bed, a trolley, a
    /// piece of plant). A door or open wall narrower than this is not passable, so the
    /// route is the shortest among those it fits through. 0 or omitted checks nothing.
    #[serde(default)]
    pub(super) width_mm: Option<f64>,
    /// What a door's frame takes off its footprint width to give its clear opening, in
    /// mm. Omit for the project's setting: a door's width here is an ESTIMATE.
    #[serde(default)]
    pub(super) frame_mm: Option<f64>,
    /// The height, in mm, of an object that has to make the trip. A door or a ROOM lower
    /// than this is not passable, so the route is the shortest among those it fits.
    /// 0 or omitted checks nothing.
    #[serde(default)]
    pub(super) height_mm: Option<f64>,
    /// The room property holding a room's clear height. Omit for the project's setting.
    #[serde(default)]
    pub(super) height_property: Option<String>,
    /// `distance` (default, approximate walking distance in feet) or `hops`
    /// (fewest doors).
    #[serde(default)]
    pub(super) metric: Option<String>,
    /// How a route runs inside rooms. `door_to_door` (default) takes the exact
    /// shortest walk inside each room, from door to door (Liu and Zlatanova 2011);
    /// `centroid` walks from each room's centre to its door to the next centre, the
    /// original and longer-looking baseline. The answer lists both under `methods`.
    #[serde(default)]
    pub(super) method: Option<String>,
    /// `summary` (default) returns components, isolated rooms, counts and the
    /// route. `full` adds every node and edge, which on a large project is over
    /// a megabyte: ask for it only when you need the graph itself.
    #[serde(default)]
    pub(super) detail: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct AreasParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// Opaque building key from `list_buildings`. Omit for no building filter.
    #[serde(default)]
    pub(super) building: Option<String>,
    /// Milestone name from `list_milestones`: measure the snapshots that
    /// milestone pins instead of each model's latest. Omit for latest.
    #[serde(default)]
    pub(super) milestone: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct CompareMilestonesParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// The baseline milestone name (from `list_milestones`) every other is
    /// compared against.
    pub(super) baseline: String,
    /// The milestone names to compare against the baseline. Any equal to the
    /// baseline is skipped.
    #[serde(default)]
    pub(super) others: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct ReferenceSourceParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// Which reference source, e.g. "drofus". Omit when the project
    /// configures exactly one — see `resolve_source`.
    #[serde(default)]
    pub(super) source: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct GetReferenceSnapshotParams {
    /// The project id, as returned by `list_projects`.
    pub(super) project_id: String,
    /// Which reference source, e.g. "drofus". Omit when the project
    /// configures exactly one — see `resolve_source`.
    #[serde(default)]
    pub(super) source: Option<String>,
    /// A snapshot id from `list_reference_snapshots`. Omit for the latest.
    #[serde(default)]
    pub(super) taken_at: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(super) struct UploadReferenceParams {
    /// The project id, as returned by `list_projects`. Its settings must
    /// declare `[sources.reference.<source>] type = "upload"`.
    pub(super) project_id: String,
    /// Which reference source to upload for, e.g. "drofus". Omit when the
    /// project configures exactly one — see `resolve_source`.
    #[serde(default)]
    pub(super) source: Option<String>,
    /// Absolute path to the CSV export to upload.
    pub(super) path: String,
    /// Snapshot id (RFC3339 UTC date-time) to store the upload under. Omit
    /// to let the server mint one; the result reports the resolved id.
    #[serde(default)]
    pub(super) taken_at: Option<String>,
}

// `tool_router` is read by the `#[tool_handler]`-generated dispatch code,
// but rustc's dead-code analysis doesn't see through that -- same false
// positive the rmcp SDK's own examples suppress this way.

pub(super) fn mcp_default_true() -> bool {
    true
}
