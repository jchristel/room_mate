//! MCP tools: Authored data: blueprints and their assignments, tasks, the sheet library, tag rules and the RLS previews.
//! One `#[tool]` per HTTP read route; the router is joined with the others in `RoommateMcp::new`.

use super::*;

#[tool_router(router = tool_router_authored, vis = "pub(super)")]
impl RoommateMcp {
    /// The project's authored blueprints -- see `blueprints::list`.
    #[tool(
        description = "List the blueprints authored for one project, by name. A BLUEPRINT is a room and the FF&E in it written as RECIPES against the room's edges (which edge, how far along it, how far into the room, which way it faces), so the same layout can be placed in another room; it is authored data, not read from the model. Each row has `id`, `name`, `taken_at` (the blueprint's version), `source` (the room it was captured from: a record, not a reference that must still resolve), `current_revision` (the revision new assignments use), `in_use` (a blueprint is never deleted, only marked not in use: it then takes no new assignment and its sheets are not suggested), `revisions` (how many), `pending` (revisions pushed from elsewhere and waiting for a person to accept or reject them) and `items` (in the current revision, excluded ones not counted) and `sheets` (how many library sheets are bound to it: a blueprint holds no sheets itself, list_sheets shows them with their `blueprint`). Revisions are append-only: an edit is a new revision. Read one with get_blueprint; to see what a room would capture as, use capture_blueprint. A blueprint is made only by the Revit push, which sends its room layout sheets to the library bound to it; writing is HTTP-only (POST /projects/{id}/blueprints/from-revit)."
    )]
    fn list_blueprints(&self, Parameters(p): Parameters<ListParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let rows =
            roommate::blueprints_api::list_with_sheet_counts(&dir, &p.project_id).map_err(blueprints_to_mcp_error)?;
        list_result(&rows, "blueprints", p.limit, p.offset)
    }

    /// One blueprint with all its revisions -- see `blueprints::load`.
    #[tool(
        description = "Read one blueprint with every revision. A revision holds an `outline` (`coarse`: jogs under about 350 mm removed, and the one every placement's `edge` index refers to, counter-clockwise with edge i running from vertex i to i+1; `fine`: corners only, for checking whether something is inside the room; both in millimetres with the room's centroid as the origin) and `items`. An item is the FF&E record as exported (`family_name` when the extractor carried it, `type_name`, `category`, `properties` and `type_properties` verbatim) with a `placement`: `edge`, `along_mm` (the item's centre from the edge's start, as captured) and `along_mode` (`fixed_from_start`, `fixed_from_end` or `percent`: how that distance is read in a room of another size), `off_mm` and `off_mode` (`fixed` or `percent` of the room's `depth_mm` in front of the edge), `edge_len_mm`, `facing_deg` (from the edge's inward normal) and `size_mm` (along the facing, then across). `class` is `wall_device` for something under 100 mm thick (a switch or socket: located by wall and height, no footprint check) and `free` otherwise; `excluded` marks an item removed from the blueprint but kept; `follows` says the item abuts the previous one in a run, or sits `inside` another, and so keeps its gap to it instead of getting its own percentage. `corner_anchored` is true for an item with no edge beside it. `pending` lists revisions that ARRIVED (an import, later a Revit push) and have not been reviewed: they are NOT part of the history, are not current, and no assignment can pin one, until a person accepts it (it becomes the next revision and the current one) or rejects it (it moves to `rejected`, kept). Do not describe a pending revision as the blueprint's layout. Nothing here is derived from today's model."
    )]
    fn get_blueprint(&self, Parameters(p): Parameters<BlueprintParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let blueprint =
            roommate::blueprints::load(&dir, &p.project_id, &p.blueprint_id).map_err(blueprints_to_mcp_error)?;
        json_result(&blueprint)
    }

    /// What capturing a room would produce -- see `service::blueprint_capture`.
    #[tool(
        description = "Work out the blueprint revision that capturing one room would produce, WITHOUT storing anything. Reads the room's outline and the FF&E attributed to it (under the project's own attribution policy) and returns the same `outline` and `items` get_blueprint shows, plus `room_name` and `skipped`: how many items capture left out because they are components of another instance (`nested`; they move with their parent) or have no footprint to write a recipe against (`no_footprint`). A silence would read as 'nothing was there', which is why they are counted. To keep one, POST the result to /projects/{id}/blueprints with a name."
    )]
    fn capture_blueprint(&self, Parameters(p): Parameters<CaptureBlueprintParams>) -> Result<CallToolResult, McpError> {
        let room = roommate::service::blueprint_capture::qualify_room(
            &self.state,
            &p.project_id,
            &p.room_id,
            p.model_id.as_deref(),
        )
        .map_err(to_mcp_error)?
        .ok_or_else(|| McpError::invalid_params("no such room in this project".to_string(), None))?;
        match roommate::service::blueprint_capture::capture_room(&self.state, &p.project_id, &room)
            .map_err(to_mcp_error)?
        {
            None => Err(McpError::invalid_params("no such room in this project".to_string(), None)),
            Some((captured, room_name)) => json_result(&serde_json::json!({
                "room_name": room_name,
                "model_id": room.model_id,
                "outline": captured.content.outline,
                "items": captured.content.items,
                "skipped": captured.skipped,
            })),
        }
    }

    /// Where a blueprint's items go in one room -- see `service::blueprint_resolve`.
    #[tool(
        description = "Work out where a blueprint revision's items go in ONE target room, without storing or applying anything. Each entry of `placements` has the item's `centre` and unit `facing` in the PROJECT's plan frame in millimetres (so it draws straight onto the plan), its `size_mm` (along the facing, then across), the target `edge` it sits against and `rule`, the rule that put it there in words. Scaling is the ROOM's and never the item's: a fixed distance keeps its millimetres, a percentage keeps its fraction of the edge or of the room's depth, measured on the edge it is anchored to. An item that follows another (a run of casework, a shelf in a wardrobe) moves rigidly with it. `fit` says how the blueprint's outline matched the room's: `kind` is `plain` (the same shape up to rotation), `mirror_image` (the room is the blueprint REFLECTED: the blueprint never mirrors, so the answer is a blueprint authored for the other hand; it, and placements are still returned for reference), `no_fit` or `edge_count_differs` (nothing is placed). `shift` is the edge mapping used and `cost` its shape cost (about 1.0 or under is a fit). A rectangle fits in two mappings a half turn apart, so pass `shift` to choose. A lot of resolving is manual: pass `edge_map` (the room edge for each blueprint edge) to place a blueprint in a room with a DIFFERENT number of edges, or to override the shape fit; the fit `kind` is then `manual` and nothing is judged about the mapping. `findings` lists what to look at, never an error: `outside` (a corner is outside the room's true outline), `overlap` (two items that did not overlap in the blueprint now do), `follow_broken`, `corner_anchored`, `mirror_image`, `shape_mismatch`. Wall devices are left out of outside and overlap findings. `excluded` counts items the blueprint holds but excludes."
    )]
    fn resolve_blueprint(&self, Parameters(p): Parameters<ResolveBlueprintParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let blueprint =
            roommate::blueprints::load(&dir, &p.project_id, &p.blueprint_id).map_err(blueprints_to_mcp_error)?;
        let number = p.revision.unwrap_or(blueprint.current_revision);
        let revision = blueprint
            .revision(number)
            .ok_or_else(|| McpError::invalid_params(format!("the blueprint has no revision {number}"), None))?;
        let room = roommate::service::blueprint_capture::qualify_room(
            &self.state,
            &p.project_id,
            &p.room_id,
            p.model_id.as_deref(),
        )
        .map_err(to_mcp_error)?
        .ok_or_else(|| McpError::invalid_params("no such room in this project".to_string(), None))?;
        use roommate::service::blueprint_resolve::EdgeMap;
        match roommate::service::blueprint_resolve::resolve_in_room(
            &self.state,
            &p.project_id,
            &revision.content,
            &room,
            &p.edge_map
                .map_or_else(|| p.shift.map_or(EdgeMap::Auto, EdgeMap::Shift), EdgeMap::Explicit),
        )
        .map_err(to_mcp_error)?
        {
            None => Err(McpError::invalid_params("no such room in this project".to_string(), None)),
            Some((room_name, resolution)) => json_result(&serde_json::json!({
                "blueprint_id": blueprint.id,
                "revision": number,
                "room_name": room_name,
                "fit": resolution.fit,
                "outline": resolution.outline,
                "placements": resolution.placements,
                "findings": resolution.findings,
                "excluded": resolution.excluded,
                "candidates": resolution.candidates,
                "doors": resolution.doors,
                "mapped_doors": resolution.mapped_doors,
            })),
        }
    }

    /// Which blueprint is placed in which room -- see `assignments::list`.
    #[tool(
        description = "List the project's blueprint ASSIGNMENTS: which blueprint a person placed in which room. Authored data, one per room. Each row has `id`, `blueprint_id` and `blueprint_name`, the pinned `revision` (an edit to the blueprint later changes nothing here until a person re-assigns), `current_revision`, `newer_revision` (the blueprint has moved on since), `blueprint_missing` (it was deleted: a reported state), `room` ({model_id, room_id}), `room_name` as it was when assigned, `mapping` (a `shift`, or an `edge_map` the person chose by hand, or neither for the best fit) and an optional `note`. Placements are NOT stored: to see where the items land, call resolve_blueprint with the row's blueprint, revision, room and mapping. Writing is HTTP-only (POST /projects/{id}/assignments)."
    )]
    fn list_assignments(&self, Parameters(p): Parameters<ListParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let rows = roommate::assignments::list(&dir, &p.project_id).map_err(blueprints_to_mcp_error)?;
        list_result(&rows, "assignments", p.limit, p.offset)
    }

    /// What a room holds against what its blueprint says -- see `service::blueprint_audit`.
    #[tool(
        description = "AUDIT one blueprint assignment: set the layout the blueprint gives the room today (the pinned revision, resolved with the room's mapping and overrides) against the FF&E the room actually holds, and say per element and per item whether it is the blueprint's, by CODE (the project's `tag_type_code` parameter, never family or type names) and by PLACE. Hosts only: a component nested in another family is part of its host. `state`: `empty` (nothing in the room), `untracked` (furnished by hand or another process; nothing records what the blueprint placed, so every verdict is an inference from code and place) or `cannot_judge` (the blueprint does not fit the room, so presence by code only). `verdicts` each have a `kind`: `in_place`, `moved` (right code, within `near_tolerance_mm` but not in `place_tolerance_mm`, or facing off by more than `turn_tolerance_deg`; with `distance_mm` and `turn_deg`), `missing` (a layout item nothing is), `extra` (right code, in the room, too far from any item of that code), `surplus` (more elements of a code than the layout has items), `foreign` (an element whose code the layout does not have at all: not the blueprint's, never touched), `substituted` (another code stands where an item should), `kit_differs` (the right host in the right place, but different nested parts inside: `kit_diff` lists the parts the blueprint has and the element lacks, and the reverse; reported only, since a nested instance changes through its host alone and a family reload is refused), `present_by_code` (a room the blueprint does not fit), and the two flags nothing is done about: `no_code` (the type has no code, a modelling issue for the author to fix in Revit first) and `unsupported_host` (not level-based or work-plane-on-a-level: wall, face or line hosted, or the model did not say; `unsupported` says why, `side` says which side). `counts` is the verdicts by kind. The tolerances are the project's `[blueprint_audit]` settings and ride the answer. Each matched pair also carries `kit` (`same`, `differs` or `unknown`: an older snapshot or blueprint cannot say, and that is never read as agreement; `kit_unknown` counts them). NOT judged yet: anything that needs the ledger of what Revit placed (drift, retired, removed by hand), a nested instance's own properties, and height. It writes nothing. Needs the project's `tag_type_code`; refused without it."
    )]
    fn audit_assignment(&self, Parameters(p): Parameters<AssignmentParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        roommate::service::blueprint_audit::audit_assignment(&self.state, &dir, &p.project_id, &p.assignment_id)
            .map_err(to_mcp_error)?
            .map_or_else(
                || {
                    Err(McpError::invalid_params(
                        "no such assignment, or its room is not in the model any more".to_string(),
                        None,
                    ))
                },
                |audit| json_result(&audit),
            )
    }

    /// Room tasks, as a Revit session sees them -- see `tasks::list`.
    #[tool(
        description = "List the project's ROOM TASKS: work the server holds for a Revit session. A task is a RECORD, unlike an assignment: it holds what Revit is told as it was when the task was made. Each row has `id`, `kind` (`populate_room`: place a blueprint's items in the room and, if `create_rls`, its sheets; `create_rls`: make room layout sheets from a library sheet and place nothing; `update_room`: bring a room populated earlier back in line with its blueprint, a dry run unless turned off; `create_rls_set`: the sheets of a SET of adjacent rooms as a person laid them out, its `room` the lead room and `set_rooms` every room, lead first), `state` (`queued`, `in_progress` once a Revit session has been handed it, then `completed`, `completed_with_warnings`, `failed` or `aborted` as that session reports, or `dismissed` by a person), `model_id` (the Revit document the task is for), `room` ({room_id, number, name}), `phase`, `overwrite_layout`, `create_rls`, `overwrite_rls`, `blueprint` ({id, name, revision}) for a populate task, counts of `items`, `sheets` (for a set, the sheets it makes), `tag_rules` and `findings`, `attempts` and `last_messages` (what the last session said, for why a task failed or aborted). Optional `model_id` and `state` narrow the list. Read one in full with get_task. Making, dismissing and resending tasks, and the Revit session's asking and reporting, are HTTP-only (POST /projects/{id}/tasks, /tasks/{task}/dismiss, /tasks/{task}/resend, /roomtasks, /roomtasks/report)."
    )]
    fn list_tasks(&self, Parameters(p): Parameters<ListTasksParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let state = match p.state.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            None => None,
            Some(s) => Some(
                serde_json::from_value::<roommate::tasks::TaskState>(serde_json::Value::String(s.to_string()))
                    .map_err(|_| McpError::invalid_params(format!("{s:?} is not a task state"), None))?,
            ),
        };
        let rows =
            roommate::tasks::list(&dir, &p.project_id, p.model_id.as_deref(), state).map_err(tasks_to_mcp_error)?;
        list_result(&rows, "tasks", p.limit, p.offset)
    }

    /// One task in full -- see `tasks::load`.
    #[tool(
        description = "Read one room task in full. `items` are the family instances to place, in the target MODEL's own frame, millimetres: `x_mm`, `y_mm` (centre), `facing_x`/`facing_y` (unit vector), `size_mm` (along the facing, then across), `family_name`, `type_name`, `category`, `placement_type` (`free` or `wall_device`), `host` and the instance and type `properties` as exported. `sheets` are the room layout sheet layouts to make (as exported by duHast: viewports with sheet positions and types; no tags: the tag rules for them are in `tag_rules`), with the number, name and id of the SOURCE sheet, which are not for the new sheets. `findings` is what the resolver said the room changes about the layout and any warning from making the task. `attempts` lists each go by a Revit session with its status and messages. A `create_rls` task carries `rls`, where everything goes in the room (the same answer as preview_rls). A `create_rls_set` task carries `rls_set` instead: `rooms` (lead first), `lead_room`, `sheet_size` and `paper_mm`, the `plan` (crop and scale), each room's `markers` with the elevations made, the rooms with a `schedules` row, whether the `three_d` view is made, and `sheets`: per sheet its viewports (`kind`, `room_id`, `marker_index`, `centre_mm` from the paper's bottom left with y up, `size_mm`); a schedule may appear on several sheets, whole. Its `sheets` (top level) holds the one library sheet it was laid out from."
    )]
    fn get_task(&self, Parameters(p): Parameters<TaskParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let task = roommate::tasks::load(&dir, &p.project_id, &p.task_id).map_err(tasks_to_mcp_error)?;
        json_result(&task)
    }

    /// The project's room layout sheet library -- see `sheets::list`.
    #[tool(
        description = "List the room layout sheets in one project's sheet LIBRARY: sheets a person chose to export from Revit and keep as exported, so the closest one can be offered for a room. It is authored data, kept apart from blueprints, not read from the model. Each row has `id`, `taken_at` (the record's version), `revit_sheet_id` and `document_title` (together a sheet's identity: a sheet id is unique only within its document), `sheet_number`, `sheet_name`, `blueprint` (the room blueprint the sheet is bound to, absent for a GENERIC sheet), `room_key` (a generic sheet's identifier: its room's bounding rectangle in mm and its item count, like `4000x3000-12`), `size_mm` (the paper size measured from the title block), `rooms` (labels of the rooms it covers), `room_summary` (their combined figures), `room` (the room it was made for, set by a blueprint push: a reference that may no longer resolve), `viewports` (how many) and `pending` (pushes held back for a person to confirm, because the sheet number changed). Read one with get_sheet; to find the sheets closest to a room use suggest_sheets. Writing is HTTP-only (POST /projects/{id}/sheets)."
    )]
    fn list_sheets(&self, Parameters(p): Parameters<ListParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let rows = roommate::sheets::list(&dir, &p.project_id).map_err(sheets_to_mcp_error)?;
        list_result(&rows, "sheets", p.limit, p.offset)
    }

    /// Tag rules -- see `tag_rules::list`.
    #[tool(
        description = "List the project's TAG RULES: where a tag goes relative to the element it tags, learned from tags in Revit views and kept as ONE rule per key. The key is the tagged item's TYPE CODE (`type_code`: the value of the type parameter the project names in `tag_type_code`, read under `code_parameter`; never Revit ids or family and type names, because a blueprint places new elements with new ids), the tag's category, family and type (`tag_*`) and the `viewport_type` it was read in. A rule is generic (`blueprint` empty) or bound to a room blueprint (`blueprint`, and for an item's tag the blueprint item it is for in `instance`); a room tag's `placement.boundary` is its place against the room's boundary. `placement` is the answer: `head`, `elbow` and `end` are offsets in PAPER millimetres in the element's own frame (x along its facing, y to its left), `rotation` is radians relative to the facing, plus `orientation`, `has_leader` and `leader_end_condition`; `sample_count` is how many tags it was made from (the median of them). `tag_kind` is `independent`, `room`, `space` or `area`. Read one, with its samples, with get_tag_rule. Writing is HTTP-only (POST /projects/{id}/tagrules/observations)."
    )]
    fn list_tag_rules(&self, Parameters(p): Parameters<ListParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let rows = roommate::tag_rules::list(&dir, &p.project_id).map_err(tag_rules_to_mcp_error)?;
        list_result(&rows, "tag_rules", p.limit, p.offset)
    }

    /// One tag rule with its samples -- see `tag_rules::load`.
    #[tool(
        description = "Read one tag rule with the `samples` it is the median of. Each sample says where it was read (`source`: the sheet, the tag's id and the tagged element's id in the source model, which are provenance and never how a rule is found) and the placement it measured, so a rule that looks wrong can be traced to the tag that made it."
    )]
    fn get_tag_rule(&self, Parameters(p): Parameters<TagRuleParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let rule = roommate::tag_rules::load(&dir, &p.project_id, &p.rule_id).map_err(tag_rules_to_mcp_error)?;
        json_result(&rule)
    }

    /// One library sheet -- see `sheets::load`.
    #[tool(
        description = "Read one room layout sheet from the library, as exported. `sheet` is duHast's export of the Revit sheet unchanged: its instance and type properties, its `view_ports` (each with a type of floor plan, elevation, 3D or schedule, a `bounding_box` and `centre_point` giving its position on the sheet, and per view the crop box, plus the marker index and view direction for an elevation, the eye, forward and up for a 3D view, or the row count and segment heights for a schedule) and its sheet size. NOT in the export: view template and view names. Tags are taken out before a sheet is stored (see list_tag_rules). The record also carries `blueprint`, `size_mm`, `rooms`, `room_summary` and `room_key` as list_sheets describes them. `pending` lists pushes of this sheet that arrived with a CHANGED sheet number and are being held for a person to accept or reject: they are not the sheet, so do not describe one as the sheet's current number or layout. `rejected` keeps the ones that were declined."
    )]
    fn get_sheet(&self, Parameters(p): Parameters<SheetParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let record = roommate::sheets::load(&dir, &p.project_id, &p.sheet_id).map_err(sheets_to_mcp_error)?;
        json_result(&record)
    }

    /// Which wall each elevation on a library sheet looks at -- see `service::elevation_facing`.
    #[tool(
        description = "For one library sheet, which elevation viewport is which index of which elevation marker, and which wall(s) of the sheet's room each looks at. `markers` lists each marker (`marker_id`, `location`, and its `views` as [index, view_id] in index order). `elevations` has one row per elevation viewport: `view_id`, `marker_id`, `marker_index`, the elevation line (`line_start`, `line_end`, model mm), `looks` (the plan direction it looks along), `depth_mm` (how far, absent when not depth limited) and `faces`: the room edges it covers, largest first, each with `edge` (index in the room's 30 mm counter-clockwise outline), its end points `a`/`b`, `edge_mm`, `visible_mm` (the part inside the view), `square_on` (cosine of the angle between the wall and the view: 1 straight on) and `nearest_mm`. Worked out when asked from the room as it is now; occlusion by other walls is not judged. A sheet exported before markers were read, or with no room, says so in `findings` instead of failing."
    )]
    fn get_sheet_elevations(&self, Parameters(p): Parameters<SheetParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let record = roommate::sheets::load(&dir, &p.project_id, &p.sheet_id).map_err(sheets_to_mcp_error)?;
        let out = roommate::service::elevation_facing::for_sheet(&self.state, &p.project_id, &record)
            .map_err(to_mcp_error)?;
        json_result(&out)
    }

    /// The library sheets closest to one room -- see `service::sheet_suggest`.
    #[tool(
        description = "Rank the project's library sheets by how close the room(s) each covers are to ONE target room, closest first, on the figures stored with the sheet. Closeness weighs three figures EQUALLY: the room's area, its proportions (the long side over the short side of the smallest rectangle enclosing the outline) and the number of FF&E items in it, each compared as a ratio, so twice the area is as far as half the area. `distance` is the mean of `parts`, and `signature` is each room's own three figures, so the reason a sheet was offered is in the answer. Worked out from the rooms and FF&E as they are today, and never stored. This OFFERS and never applies: it is for sheet creation mode, a person chooses, and a low distance is a similarity of room size and contents and says nothing about whether the sheet's layout suits the room. `unassigned` counts sheets with no room figures, which cannot be ranked; sheets bound to a blueprint that is not in use are left out. `unresolved` lists sheets whose room is not in the model today, with the reason."
    )]
    fn suggest_sheets(&self, Parameters(p): Parameters<SuggestSheetsParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        // the library a person is offered, as the HTTP route reads it: not the sheets of a blueprint not in use
        let records = roommate::sheets::load_in_use(&dir, &p.project_id).map_err(sheets_to_mcp_error)?;
        let room = roommate::service::blueprint_capture::qualify_room(
            &self.state,
            &p.project_id,
            &p.room_id,
            p.model_id.as_deref(),
        )
        .map_err(to_mcp_error)?
        .ok_or_else(|| McpError::invalid_params("no such room in this project".to_string(), None))?;
        match roommate::service::sheet_suggest::suggest(&self.state, &p.project_id, &room, &records)
            .map_err(to_mcp_error)?
        {
            None => Err(McpError::invalid_params("no such room in this project".to_string(), None)),
            Some(result) => json_result(&result),
        }
    }

    /// The closest library sheet for each of many rooms -- see `service::sheet_suggest::solve`.
    #[tool(
        description = "For each of MANY rooms at once, its three closest library sheets, as suggest_sheets ranks them for one room (area, proportions and FF&E count weighed equally, from the rooms and FF&E as they are today). Each row is `room_id`, `room_name` and either `choices` (`sheet_id`, `sheet_number`, `sheet_name`, `room_name`, `fit`: 100 * exp(-distance), a scale to read and not a probability) or a `reason` (no such room, the id is in several models, no sheet can be ranked). The rooms and FF&E are read once for the whole list. Offers and never applies: this is the viewer's RLS auto mode, where a person reviews each match. Sheets bound to a blueprint that is not in use are left out."
    )]
    fn solve_sheets(&self, Parameters(p): Parameters<RoomIdsParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let records = roommate::sheets::load_in_use(&dir, &p.project_id).map_err(sheets_to_mcp_error)?;
        match roommate::service::sheet_suggest::context(&self.state, &p.project_id).map_err(to_mcp_error)? {
            None => Err(McpError::invalid_params("this project has no rooms".to_string(), None)),
            Some(ctx) => json_result(&roommate::service::sheet_suggest::solve(&ctx, &p.room_ids, &records)),
        }
    }

    /// What a room layout sheet task for one room would say -- see `task_build::rls_from_sheet`.
    #[tool(
        description = "Preview a create-RLS task for ONE room from ONE library sheet, storing nothing: exactly what the task would carry. `room` is the room as the task names it. `rls` is where everything goes IN THIS ROOM, in the room's MODEL frame, millimetres: `sheet_size`; `markers`, each elevation marker's `location` and its `views` (`line_start`/`line_end`, `looks`, `depth_mm`, `depth_to_wall_mm`, the template `view_id`), carried over by pairing the template room's walls with this room's; `plan_crop` (the template crop's offsets from its room, and a `turn` when the room is at an angle to the template's); `three_ds` (each 3D view's camera, the room edges whose walls are hidden and the items seen through); `room_tag` (the tag's head); and `tags`, which items each view tags and where, and the ones no rule tags. `findings` says what could not be carried over (a different wall count, a room much unlike the template's). Reads the project's FF&E for the tags, so it takes seconds on a large project. Making the task is HTTP-only (POST /projects/{id}/tasks)."
    )]
    fn preview_rls(&self, Parameters(p): Parameters<PreviewRlsParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let room = roommate::service::blueprint_capture::qualify_room(&self.state, &p.project_id, &p.room_id, None)
            .map_err(to_mcp_error)?
            .ok_or_else(|| McpError::invalid_params("no such room in this project".to_string(), None))?;
        let request = roommate::service::rls_plan::RlsRequest {
            sheet_size: p.sheet_size,
            depths: Vec::new(),
            overwrite_rls: false,
            tags: Some(Default::default()),
        };
        let new = roommate::service::task_build::rls_from_sheet(
            &self.state,
            &dir,
            &p.project_id,
            &p.sheet_id,
            &room,
            "preview",
            &request,
        )
        .map_err(to_mcp_error)?;
        json_result(&serde_json::json!({ "room": new.room, "rls": new.rls, "findings": new.findings }))
    }

    /// What a room layout sheet SET of adjacent rooms would be -- see `service::rls_set::preview`.
    #[tool(
        description = "Preview a room layout sheet SET (multi-room RLS): several ADJACENT rooms on shared sheets, storing nothing. `valid` is false when the rooms do not make a set (unknown, in several models, or not one group of rooms that share walls by the server's adjacency), and `findings` say why. `rooms` gives each room's `name`, `number`, `area_m2` and clear `height_mm` with `height_from` (`ceiling`, `room_property` or `default` 2700). `lead_room` is the largest: its parameters name the set's sheets, plan and 3D view. `suggestions` are the three library sheets closest to the set measured as ONE room (the rectangle round all its rooms, their summed area and items), with a `fit` out of 100; `sheet_id` is the one used. `views`, on that sheet's scales, in the rooms' MODEL frame, millimetres: one `plan` round every room (its `crop`, turned to the rooms' walls together, `scale` and `paper_mm`), four `elevations` per room square to its own walls (`marker`, `line_start`/`line_end`, `looks`, `depth_mm`, `height_mm`, `paper_mm`), a `schedules` row per room and one `three_d`, both sized as on the template and marked `estimate`. Nothing is placed on a sheet: a person lays the views out in the viewer's RLS set mode, which queues one create_rls_set task (HTTP-only)."
    )]
    fn preview_rls_set(&self, Parameters(p): Parameters<PreviewRlsSetParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let records = roommate::sheets::load_in_use(&dir, &p.project_id).map_err(sheets_to_mcp_error)?;
        let Some(ctx) = roommate::service::sheet_suggest::context(&self.state, &p.project_id).map_err(to_mcp_error)?
        else {
            return Err(McpError::invalid_params("this project has no rooms".to_string(), None));
        };
        let request = roommate::service::rls_set::SetRequest { room_ids: p.room_ids, sheet_id: p.sheet_id };
        let preview = roommate::service::rls_set::preview(&self.state, &p.project_id, &ctx, &records, &request)
            .map_err(to_mcp_error)?;
        json_result(&preview)
    }

    /// How one blueprint does in each of several rooms -- see `service::blueprint_solve`.
    #[tool(
        description = "Try ONE blueprint revision in each of SEVERAL rooms at once, best fit by shape and doors, storing nothing. Each row is one room: `room_id`, `model_id`, `room_name`, a `verdict` (`clear`: fits by shape and the room changes nothing about the layout; `check`: items were placed and there is something to look at; `cannot`: nothing placed, for a shape that does not fit, a different wall count or a room not found) and its `reason` in a sentence, then the fit used (`fit`, `shift`, `cost`, `door_gap_mm`) and counts: `placed` of `items`, `outside` and `overlap` (what the room changes), and `doors_disagree`. resolve_blueprint gives one room's placements in full. Advice, never a gate: the viewer's batch assign shows it before a person assigns. A mapping chosen by hand is a decision about one room and is not tried here (use resolve_blueprint). At most 500 rooms; the rooms and doors are read once for the list."
    )]
    fn solve_blueprint(&self, Parameters(p): Parameters<SolveBlueprintParams>) -> Result<CallToolResult, McpError> {
        const MAX_ROOMS: usize = 500;
        if p.room_ids.len() > MAX_ROOMS {
            return Err(McpError::invalid_params(format!("at most {MAX_ROOMS} rooms at a time"), None));
        }
        let dir = self.projects_dir()?;
        let blueprint =
            roommate::blueprints::load(&dir, &p.project_id, &p.blueprint_id).map_err(blueprints_to_mcp_error)?;
        let number = p.revision.unwrap_or(blueprint.current_revision);
        let revision = blueprint
            .revision(number)
            .ok_or_else(|| McpError::invalid_params(format!("the blueprint has no revision {number}"), None))?;
        let rows =
            roommate::service::blueprint_solve::solve_rooms(&self.state, &p.project_id, &revision.content, &p.room_ids)
                .map_err(to_mcp_error)?;
        json_result(&rows)
    }

    /// A blueprint's bundle -- see `blueprint_bundle::export`.
    #[tool(
        description = "Read one blueprint's BUNDLE: what travels with it when it is exported to another project or file, beside the blueprint itself (read that with get_blueprint). `sheets` are the library sheets bound to it, as exported from Revit; `rules` are the tag rules bound to it, with their samples; `type_code_parameter` names the type parameter the rules' type codes were read from. list_sheets and list_tag_rules show everything in the project."
    )]
    fn get_blueprint_bundle(&self, Parameters(p): Parameters<BlueprintParams>) -> Result<CallToolResult, McpError> {
        match roommate::blueprint_bundle::export(&self.state, &p.project_id, &p.blueprint_id) {
            Ok(bundle) => json_result(&bundle),
            Err((status, msg)) if status.is_client_error() => Err(McpError::invalid_params(msg, None)),
            Err((_, msg)) => Err(McpError::internal_error(msg, None)),
        }
    }
}
