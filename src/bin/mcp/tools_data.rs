//! MCP tools: Entity reads: the projects and buildings, each entity (rooms, doors, windows, FF&E, spaces, ceilings, floors) and the reports.
//! One `#[tool]` per HTTP read route; the router is joined with the others in `RoommateMcp::new`.

use super::*;

#[tool_router(router = tool_router_data, vis = "pub(super)")]
impl RoommateMcp {
    /// Lists every project with at least one stored model -- see
    /// `service::projects::list_projects`.
    #[tool(description = "List every project with at least one stored model")]
    fn list_projects(&self, Parameters(p): Parameters<PageParams>) -> Result<CallToolResult, McpError> {
        let result = projects::list_projects(&self.state).map_err(to_mcp_error)?;
        list_result(&result, "projects", p.limit, p.offset)
    }

    /// Lists the distinct "Building" classification values for one project
    /// -- see `service::projects::list_buildings`.
    #[tool(description = "List the distinct Building classification values found in one project's rooms")]
    fn list_buildings(&self, Parameters(p): Parameters<ListParams>) -> Result<CallToolResult, McpError> {
        let result = projects::list_buildings(&self.state, &p.project_id).map_err(to_mcp_error)?;
        list_result(&result, "buildings", p.limit, p.offset)
    }

    /// Merges every stored model's levels and rooms, optionally scoped by
    /// project and building -- see `service::rooms::assemble_rooms`. The
    /// service's `None` ("nothing has ever been pushed" -- the HTTP 204 case)
    /// has no MCP status-code equivalent, so it becomes a short plain-text
    /// answer instead of a JSON body; an LLM client reads either just fine.
    #[tool(
        description = "Fetch merged rooms and levels across stored models, optionally scoped by project id, building key, milestone name, level and property filter. \
                          THE DEFAULT ANSWER IS A SUMMARY (`detail`: id, name, number, level per room; one page of up to 1000), which says what it leaves out and how to get it. \
                          `detail=full` returns the whole record -- outline, every property, reference joins, about 3-5 KB a room -- 100 per page by default, paged with `limit`/`offset` (see `next_offset`); \
                          an answer over 1 MB is refused. To LIST or COUNT rooms with chosen columns, prefer build_report. \
                          A project whose hierarchy has no 'Building' tier matches nothing under a building filter (check list_buildings' tier_configured before filtering); \
                          under a milestone filter, models are served from the snapshots that milestone pins instead of their latest. \
                          Prefer the 'filter' parameter over fetching every room and matching client-side -- it answers property questions ('which rooms are Department = Cardiology?') \
                          server-side, against the same canonical property names the rest of the settings use. Note that a room missing the filtered property never matches, \
                          negative operators included, so an empty result can mean 'no room has that property' rather than 'no room has that value'. \
                          IMPORTANT -- results are scoped to ONE Revit phase per model, not the whole model: each model's rooms were filtered at export to the phase named in \
                          'phase_by_model' (project id -> model id -> phase). This is a partial view of the building by design, so do not read it as a complete model. \
                          Different models of one project CAN be on different phases, in which case the merged result spans two phases -- compare the values in 'phase_by_model' \
                          before aggregating across models, and see get_validation's 'phases.disagree' for that as a reported finding. A null phase means a model pushed before \
                          phasing existed, whose rooms were never filtered to any phase at all."
    )]
    fn get_rooms(&self, Parameters(p): Parameters<GetRoomsParams>) -> Result<CallToolResult, McpError> {
        // Parsed here, in the adapter holding the raw strings, then passed
        // down as a domain type -- `service` never sees the filter syntax.
        // The known-source vocabulary comes from the live registry, not a
        // fixed list -- see `handlers::get_rooms`'s matching call.
        let known = self.state.settings().known_reference_sources();
        let filter =
            rooms::RoomFilter::parse(&p.filter, &known).map_err(|msg| to_mcp_error(ServiceError::Invalid(msg)))?;
        let scope = rooms::RoomScope {
            project: p.project.as_deref(),
            building: p.building.as_deref(),
            milestone: p.milestone.as_deref(),
            filter: Some(&filter).filter(|f| !f.is_empty()),
        };
        let result = rooms::assemble_rooms(&self.state, &scope).map_err(to_mcp_error)?;
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no snapshots have been pushed to this server yet",
            )])),
            Some(mut result) => {
                if let Some(level) = p.level.as_deref() {
                    keep_level(&mut result, level).map_err(|msg| to_mcp_error(ServiceError::Invalid(msg)))?;
                }
                let detail = Detail::parse(p.detail.as_deref()).map_err(|m| to_mcp_error(ServiceError::Invalid(m)))?;
                let rooms_text = rooms_answer(result, detail, mcp_bounds::ROOM_SIZES.page(detail, p.limit, p.offset))
                    .map_err(|m| McpError::internal_error(m, None))?;
                let text = mcp_bounds::bounded(&rooms_text, "get_rooms", ROOMS_ADVICE)
                    .map_err(|m| McpError::invalid_params(m, None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
        }
    }

    /// Merges every stored model's doors, optionally scoped -- see
    /// `service::openings::assemble_openings`. Same `None` -> plain-text handling as
    /// `get_rooms`, for the same reason.
    #[tool(
        description = "THE DEFAULT ANSWER IS A SUMMARY (`detail`: identity, level, room references; one page of up to 500) that says what it leaves out; `detail=full` returns whole records with footprints and properties, 50 a page, paged with `limit`/`offset` (see `next_offset`); an answer over 1 MB is refused. To LIST or COUNT with chosen columns prefer build_report. Fetch merged doors across stored models, optionally scoped by project id, milestone name, and property filter. Each door carries its own instance \
                          properties AND its family type's properties, its footprint, its level, and BOTH room references: 'from_room' and 'to_room'. \
                          The family-type properties are sent ONCE per distinct bag, in the top-level 'type_property_sets' array: each door names its row in \
                          'type_properties_ref', an index into THIS response only. \
                          A null on one side is an EXTERNAL door -- a normal state, not missing data. Use '$from_room=<room id>' or '$to_room=<room id>' in the filter to ask \
                          which doors touch a given room; the other intrinsics are $id, $type_id, $type_name and $level_id, and any other unqualified name reads the door's \
                          properties, INSTANCE tier first then the family TYPE tier (a blank instance value does not hide the type's). \
                          Room references are model-scoped: a room id is unique only within one model, so resolve 'from_room'/'to_room' against rooms of the SAME 'model_id' \
                          this door carries. get_validation's 'doors' section reports the ones that resolve to nothing. \
                          Doors carry no joined reference sources yet, so a source-prefixed filter (e.g. 'drofus.') matches no door rather than erroring. \
                          Each door also carries 'owner_rooms': the room(s) it is attributed to under the project's [doors] room_attribution policy, whose default is 'the room it opens                           into, else the room it opens from'. An EMPTY owner_rooms means the door is homeless -- a reported state, not missing data -- and a homeless door matches                           no 'building' filter. It is a list because the 'both' policy attributes a door between two rooms twice. \
                          IMPORTANT -- like get_rooms, results are scoped to ONE Revit phase per model, named in 'phase_by_model'. Do not read them as a complete door schedule."
    )]
    fn get_doors(&self, Parameters(p): Parameters<GetDoorsParams>) -> Result<CallToolResult, McpError> {
        let known = self.state.settings().known_reference_sources();
        let filter =
            rooms::RoomFilter::parse(&p.filter, &known).map_err(|msg| to_mcp_error(ServiceError::Invalid(msg)))?;
        let scope = openings::OpeningScope {
            project: p.project.as_deref(),
            building: p.building.as_deref(),
            milestone: p.milestone.as_deref(),
            filter: Some(&filter).filter(|f| !f.is_empty()),
            // Every storey: a storey scope is how the viewer narrows its polls to
            // what it is drawing, and a tool caller is drawing nothing.
            storeys: None,
        };
        let result = openings::assemble_openings::<roommate::contract::DoorPayload>(
            &self.state,
            openings::OpeningKind::Doors,
            &scope,
        )
        .map_err(to_mcp_error)?
        .map(openings::DoorsResult::from_assembled);
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no doors have been pushed to this server yet",
            )])),
            Some(result) => listing_result(result, p.detail.as_deref(), p.limit, p.offset),
        }
    }

    #[tool(
        description = "THE DEFAULT ANSWER IS A SUMMARY (`detail`: identity, level, room references; one page of up to 500) that says what it leaves out; `detail=full` returns whole records with footprints and properties, 50 a page, paged with `limit`/`offset` (see `next_offset`); an answer over 1 MB is refused. To LIST or COUNT with chosen columns prefer build_report. Fetch merged windows across stored models, optionally scoped by project id, building key, milestone name, and property filter. \
                          The SAME record shape as get_doors -- instance and family-type properties, footprint, level, both room references, owner_rooms -- because a window \
                          and a door are the same record in this contract. Every filter intrinsic get_doors accepts works here unchanged. \
                          WHAT DIFFERS IS THE DATA, and reading it like a door schedule will mislead you. A null on one side is an EXTERNAL window, which for doors is the \
                          exception and for windows is most of a facade. A window with null on BOTH sides is normal in a facade or envelope model: such a file links its \
                          interiors rather than containing them, and Revit cannot resolve a room across a link -- measured on a real facade file, 0 of 158 windows carried a \
                          reference on either side, and so did 0 of its 191 doors. Do NOT report that as missing data. \
                          Consequently owner_rooms is empty far more often than for doors, and an empty owner_rooms means homeless -- a reported state, and a homeless window \
                          matches no 'building' filter. If a project cares about window-to-room attribution in such a model it must set [windows] room_resolution, which \
                          derives the rooms geometrically; get_validation's openings.windows.room_resolution echoes whether it ran, and 'off' is NOT the same as clean. \
                          Room references are model-scoped: a room id is unique only within one model, so resolve them against rooms of the SAME 'model_id'. \
                          The windows contract is schema_version 1 and moves independently of doors' 2; the payload's element list is keyed 'windows'. \
                          IMPORTANT -- like get_rooms and get_doors, results are scoped to ONE Revit phase per model, named in 'phase_by_model'. Not a complete window schedule."
    )]
    fn get_windows(&self, Parameters(p): Parameters<GetDoorsParams>) -> Result<CallToolResult, McpError> {
        let known = self.state.settings().known_reference_sources();
        let filter =
            rooms::RoomFilter::parse(&p.filter, &known).map_err(|msg| to_mcp_error(ServiceError::Invalid(msg)))?;
        let scope = openings::OpeningScope {
            project: p.project.as_deref(),
            building: p.building.as_deref(),
            milestone: p.milestone.as_deref(),
            filter: Some(&filter).filter(|f| !f.is_empty()),
            // Every storey: a storey scope is how the viewer narrows its polls to
            // what it is drawing, and a tool caller is drawing nothing.
            storeys: None,
        };
        let result = openings::assemble_openings::<roommate::contract::WindowPayload>(
            &self.state,
            openings::OpeningKind::Windows,
            &scope,
        )
        .map_err(to_mcp_error)?
        .map(openings::WindowsResult::from_assembled);
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no windows have been pushed to this server yet",
            )])),
            Some(result) => listing_result(result, p.detail.as_deref(), p.limit, p.offset),
        }
    }

    #[tool(
        description = "THE DEFAULT ANSWER IS A SUMMARY (`detail`: identity, level, room references; one page of up to 500) that says what it leaves out; `detail=full` returns whole records with footprints and properties, 50 a page, paged with `limit`/`offset` (see `next_offset`); an answer over 1 MB is refused. To LIST or COUNT with chosen columns prefer build_report. Fetch merged FF&E (furniture, fixtures and equipment) across stored models, optionally scoped by project id, building key, milestone name, \
                          and property filter. A DIFFERENT record from get_doors/get_windows, and reading it as one will mislead you. \
                          Its family-type properties are sent as get_doors sends them: ONCE per distinct bag in the top-level 'type_property_sets' array, each item \
                          naming its row in 'type_properties_ref'. \
                          An item sits IN one room, so it carries a single 'room' and 'owner_rooms' is a list of AT MOST ONE -- there is no from_room/to_room, no swing, \
                          no through-wall direction, and no attribution policy, because there are no sides to choose between. Empty owner_rooms means homeless: an item in \
                          a corridor void, a shaft or outside the envelope, which is an ordinary reported state, and a homeless item matches no 'building' filter. \
                          'owner_spaces' is a SEPARATE, extractor-authored list, populated only when the push that sent this item identified it against Revit Spaces \
                          rather than Rooms -- the entry a services model with no Room elements uses. It is unrelated to 'owner_rooms'/'building': nothing server-side \
                          joins it to anything, there is no geometric fallback, and a 'building' filter cannot see it. In practice a given item carries one or the other, \
                          never both. \
                          Each item carries 'category' (the Revit category it was collected under, e.g. OST_Furniture) -- filter on it with $category, which is usually the \
                          first question worth asking. There is no equivalent on any other entity. \
                          'loops' is EMPTY on every item today: the upstream exporter carries no footprint for FF&E yet, so an item is placed by 'insertion_point' and \
                          oriented by 'facing'. Absent geometry is expected and is NOT missing data. \
                          COMPONENTS: a family instance nested inside another (a joinery handle inside a casework run) carries 'super_component_id'. By default these are \
                          EXCLUDED from this response and counted in 'excluded_components' -- on one real model 179 of 647 instances were components, nearly all hardware. \
                          If excluded_components is large that is normal, not a fault. A project may set [ffe] nested_components = \"include\", and then they are returned \
                          with is_component true. Do NOT infer that a component is not a real item from its room or its Mark: components DO name rooms (97.8% of them on \
                          the measured model), and only the absence of a Mark weakly distinguishes them. \
                          Room references are model-scoped: a room id is unique only within one model, so resolve them against rooms of the SAME 'model_id'. \
                          The FF&E contract is schema_version 1 and moves independently of rooms', doors' and windows'; the payload's element list is keyed 'ffe'. \
                          IMPORTANT -- like get_rooms, get_doors and get_windows, results are scoped to ONE Revit phase per model, named in 'phase_by_model'. \
                          Not a complete FF&E schedule."
    )]
    fn get_ffe(&self, Parameters(p): Parameters<GetDoorsParams>) -> Result<CallToolResult, McpError> {
        let known = self.state.settings().known_reference_sources();
        let filter =
            rooms::RoomFilter::parse(&p.filter, &known).map_err(|msg| to_mcp_error(ServiceError::Invalid(msg)))?;
        let scope = items::ItemScope {
            project: p.project.as_deref(),
            building: p.building.as_deref(),
            milestone: p.milestone.as_deref(),
            filter: Some(&filter).filter(|f| !f.is_empty()),
            // Every storey: a storey scope is how the viewer narrows its polls to
            // what it is drawing, and a tool caller is drawing nothing.
            storeys: None,
            with_components: false,
        };
        match items::assemble_items(&self.state, &scope).map_err(to_mcp_error)? {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no FF&E has been pushed to this server yet",
            )])),
            Some(result) => listing_result(result, p.detail.as_deref(), p.limit, p.offset),
        }
    }

    #[tool(
        description = "THE DEFAULT ANSWER IS A SUMMARY (`detail`: identity, level, room references; one page of up to 500) that says what it leaves out; `detail=full` returns whole records with footprints and properties, 50 a page, paged with `limit`/`offset` (see `next_offset`); an answer over 1 MB is refused. To LIST or COUNT with chosen columns prefer build_report. Fetch merged SPACES across stored models, optionally scoped by project id, model id, milestone name, and property filter. \
                          A Revit SPACE is an MEP concept and is NOT a room: spaces live in the services models (mechanical, hydraulic, electrical, fire) while rooms live in \
                          the architectural ones, and one services file per service means the SAME room number legitimately names a space in each of them. Two spaces sharing \
                          a number are therefore two disciplines, NOT a duplicate -- always read 'model_id' before concluding anything about duplication. \
                          A space carries NO room reference of any kind: it is matched to a room by a user-chosen property value, across models, and that match is not \
                          performed by this tool. There is no from_room/to_room, no owner_rooms and no room id to resolve. \
                          Every space carries 'enclosure', one of enclosed / unenclosed / unmeasured, and it is the most useful thing here. 'unenclosed' means the model says \
                          the space bounds nothing -- a MODEL defect somebody wants to know about. 'unmeasured' means Revit reported an area and the pipeline produced no \
                          polygon anyway -- a PIPELINE defect. Both carry empty 'loops', which is why they are distinguished on the wire rather than inferred from geometry. \
                          Filter with $enclosure, e.g. $enclosure!=enclosed to list every defective space; also $id, $name, $level_id and $model_id. \
                          An EMPTY result for a model is meaningful and is not an error: it means that services model was audited and holds no spaces. That is a different \
                          fact from the model never having been pushed, which returns no data at all. \
                          The spaces contract is schema_version 1 and moves independently of every other entity's; the payload's element list is keyed 'spaces'. \
                          IMPORTANT -- results are scoped to ONE Revit phase per model, named in 'phase_by_model', and on a real project the disciplines DO NOT agree on the \
                          phase name. A model reporting a different phase from its siblings is ordinary; check phase_by_model before reading a low count as missing data."
    )]
    fn get_spaces(&self, Parameters(p): Parameters<GetSpacesParams>) -> Result<CallToolResult, McpError> {
        let known = self.state.settings().known_reference_sources();
        let filter =
            rooms::RoomFilter::parse(&p.filter, &known).map_err(|msg| to_mcp_error(ServiceError::Invalid(msg)))?;
        let scope = spaces::SpaceScope {
            project: p.project.as_deref(),
            model: p.model.as_deref(),
            milestone: p.milestone.as_deref(),
            filter: Some(&filter).filter(|f| !f.is_empty()),
            // Every storey: a storey scope is how the viewer narrows its polls to
            // what it is drawing, and a tool caller is drawing nothing.
            storeys: None,
        };
        match spaces::assemble_spaces(&self.state, &scope).map_err(to_mcp_error)? {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no spaces have been pushed to this server yet",
            )])),
            Some(result) => listing_result(result, p.detail.as_deref(), p.limit, p.offset),
        }
    }

    /// Serves one project's ceilings with the rooms each lies over -- see
    /// `service::surfaces::assemble_surfaces`.
    #[tool(
        description = "THE DEFAULT ANSWER IS A SUMMARY (`detail`: identity, level, room references; one page of up to 500) that says what it leaves out; `detail=full` returns whole records with footprints and properties, 50 a page, paged with `limit`/`offset` (see `next_offset`); an answer over 1 MB is refused. To LIST or COUNT with chosen columns prefer build_report. List one project's ceilings, each with the rooms it lies over. Optionally scoped by milestone name. THE ROOM ASSOCIATION IS GEOMETRIC, not authored: a Revit ceiling has no room parameter and a room has no ceiling parameter, so every entry in a ceiling's `rooms` list was derived from polygon overlap on this read and is stored nowhere. Each entry carries `overlap_area` in square feet plus `mean_width` and two fractions that answer different questions — `fraction_of_element` (how much of this ceiling is in that room) and `fraction_of_room` (how much of that room this ceiling covers, which is the coverage question a finishes take-off asks). `rooms` is a LIST because a ceiling can lie over several, ordered largest overlap first, so `rooms[0]` is a usable single owner. AN EMPTY `rooms` IS A REPORTED STATE, NOT AN ERROR: a ceiling over a stairwell, an external soffit, or one on a level carrying no rooms legitimately belongs to nothing — 8 of House A's 30 do. A ceiling with empty `loops` is one duHast could not measure; it is exported rather than dropped so that 'no such ceiling' and 'a ceiling nobody could measure' stay distinguishable, and it attributes to nothing. Attribution ignores only overlaps narrower than 10 mm mean width, which is imprecision (edges that merely touch, a finish a few mm past a wall line), not a judgement about size: EVERY real overlap is listed, including a sliver where a large ceiling grazes a neighbouring room and a degenerate exported footprint. To ignore slivers, filter on `fraction_of_element`, `fraction_of_room` or `mean_width` -- the read deliberately does not. Type properties are sent ONCE per distinct bag in the top-level `type_property_sets` array; each ceiling names its row in `type_properties_ref`."
    )]
    fn get_ceilings(&self, Parameters(p): Parameters<GetSurfacesParams>) -> Result<CallToolResult, McpError> {
        let scope =
            surfaces::SurfaceScope { project: p.project.as_deref(), milestone: p.milestone.as_deref(), storeys: None };
        match surfaces::assemble_surfaces::<CeilingPayload>(&self.state, &scope).map_err(to_mcp_error)? {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no ceilings have been pushed to this server yet",
            )])),
            Some(assembled) => {
                listing_result(surfaces::CeilingsResult::from(assembled), p.detail.as_deref(), p.limit, p.offset)
            }
        }
    }

    /// Serves one project's floors with the rooms each lies over -- the
    /// ceilings read under its own key.
    #[tool(
        description = "THE DEFAULT ANSWER IS A SUMMARY (`detail`: identity, level, room references; one page of up to 500) that says what it leaves out; `detail=full` returns whole records with footprints and properties, 50 a page, paged with `limit`/`offset` (see `next_offset`); an answer over 1 MB is refused. To LIST or COUNT with chosen columns prefer build_report. List one project's floors (Revit category Floors), each with the rooms it lies over. Optionally scoped by milestone name. THE ROOM ASSOCIATION IS GEOMETRIC, not authored: a Revit floor has no room parameter, so every entry in a floor's `rooms` list was derived from polygon overlap on this read and is stored nowhere. Each entry carries `overlap_area` in square feet, `fraction_of_element` (how much of this floor is in that room), `fraction_of_room` (how much of that room this floor covers -- the finishes take-off question) and `mean_width` (2 x overlap area / overlap perimeter, in feet). `rooms` is a LIST ordered largest overlap first, so `rooms[0]` is a usable single owner. The category holds structural slabs AND finish floors AND balconies; the `Structural` instance property tells them apart, and a room can legitimately lie on both a slab and a finish floor -- `height_offset` (feet above the level, to the TOP of the floor) separates them. AN EMPTY `rooms` IS A REPORTED STATE, NOT AN ERROR. A floor with empty `polygons` is one duHast could not measure and attributes to nothing. Attribution is the ceilings rule exactly: only overlaps narrower than 10 mm mean width are ignored, as imprecision. EVERY real overlap is listed, so a strip of floor reaching under a wall into a neighbour appears behind the room the floor serves, with a small `fraction_of_room` and a `mean_width` under a wall's thickness -- filter on those to ignore it. The room association is model-scoped: floors in one Revit document are never attributed to rooms in another. Type properties are sent ONCE per distinct bag in the top-level `type_property_sets` array; each floor names its row in `type_properties_ref`."
    )]
    fn get_floors(&self, Parameters(p): Parameters<GetSurfacesParams>) -> Result<CallToolResult, McpError> {
        let scope =
            surfaces::SurfaceScope { project: p.project.as_deref(), milestone: p.milestone.as_deref(), storeys: None };
        match surfaces::assemble_surfaces::<FloorPayload>(&self.state, &scope).map_err(to_mcp_error)? {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no floors have been pushed to this server yet",
            )])),
            Some(assembled) => {
                listing_result(surfaces::FloorsResult::from(assembled), p.detail.as_deref(), p.limit, p.offset)
            }
        }
    }

    /// Builds one tabular report -- see `service::reports::build_report`.
    #[tool(
        description = "THE CHEAP WAY TO LIST OR COUNT: use this, not the entity reads, for \"list the rooms on level 2\", \"how many doors\", \"names and numbers\". 500 rows a page by default (ceiling 5000), `total_rows` before paging, `next_offset` for the next page. `$level` is the LEVEL NAME as the element's own model declares it (filter on it: `$level` = \"LEVEL 2\"); `$level_id` is that model's own id. An unknown `$` name is refused with the valid ones. Build a tabular report over one entity: a flat schedule, or that entity BY ROOM. Returns `columns` (each with the side it came from -- room, element or join) and `rows` of flat string cells, plus `total_rows` before any `limit` so 'first N of M' is honest. ONLY THE COLUMNS ASKED FOR ARE READ, which is the point of this tool over the entity endpoints: /ffe can be over a hundred MB per storey on a large project and a report of it is a few. A BY-ROOM REPORT NEVER RECOMPUTES THE ATTRIBUTION -- which room a ceiling lies over, which room owns a door, which room an item sits in are read off the entity's own answer, where the policy, the tolerance and the model scope already applied. An element attributed to several rooms appears once per room (a ceiling over three rooms is three rows), so element columns must not be summed across rows; only the join's own numeric measures add up, and `overlap_area` is the one that means anything added up. AN UNMATCHED ROW IS A REPORTED STATE: `include_unattributed` (on by default) lists elements no room matched -- every external door, every facade-package window -- with the room columns empty, and `include_rooms_without` lists rooms nothing matched. A 204 means nothing of that entity has ever been pushed, which is a different answer from a report with no rows."
    )]
    fn build_report(&self, Parameters(p): Parameters<BuildReportParams>) -> Result<CallToolResult, McpError> {
        let Some(entity) = reports::Entity::parse(&p.entity) else {
            return Err(McpError::invalid_params(format!("unknown entity {:?} for a report", p.entity), None));
        };
        let shape = match p.shape.as_deref() {
            None | Some("per_match") => reports::Shape::PerMatch,
            Some("per_room") => reports::Shape::PerRoom,
            Some(other) => {
                return Err(McpError::invalid_params(
                    format!("unknown shape {other:?} -- expected per_match or per_room"),
                    None,
                ))
            }
        };
        let filter = match &p.filter {
            None => None,
            Some(wire) => {
                let known = self.state.settings().known_reference_sources();
                Some(wire.parse(&known).map_err(|msg| McpError::invalid_params(msg, None))?)
            }
        };
        let definition = reports::ReportDefinition {
            entity,
            by_room: p.by_room,
            columns: p.columns,
            room_columns: p.room_columns,
            measures: p.measures,
            shape,
            include_rooms_without: p.include_rooms_without,
            include_unattributed: p.include_unattributed,
            limit: Some(mcp_bounds::ROW_SIZES.page(Detail::Summary, p.limit, None).limit),
            offset: p.offset,
            filter,
        };
        let scope = reports::ReportScope {
            project: Some(&p.project),
            building: p.building.as_deref(),
            milestone: p.milestone.as_deref(),
        };
        match reports::build_report(&self.state, &scope, &definition).map_err(to_mcp_error)? {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "no {} have been pushed to this server yet",
                p.entity
            ))])),
            Some(result) => {
                let info = mcp_bounds::PageInfo::of(result.total_rows, p.offset.unwrap_or(0), result.rows.len());
                let mut answer = serde_json::to_value(&result)
                    .map_err(|e| McpError::internal_error(format!("failed to serialize response: {e}"), None))?;
                if let serde_json::Value::Object(map) = &mut answer {
                    // `total_rows` stays for what already reads it; `total` is the name every other tool uses.
                    mcp_bounds::describe_page(map, &info);
                }
                let text = mcp_bounds::bounded(&answer, "build_report", REPORT_ADVICE)
                    .map_err(|m| McpError::invalid_params(m, None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
        }
    }

    /// One project's saved reports -- see `reports_api::list_reports`.
    #[tool(
        description = "List the reports somebody saved for one project: id, name, entity, and whether it is a by-room report. A saved report is a request kept rather than a setting -- deleting one changes no other answer in this system -- so these live as JSON documents beside the project settings. Pair with get_saved_report and build_report to run one."
    )]
    fn list_saved_reports(&self, Parameters(p): Parameters<SavedReportsParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let listed = reports_api::list_reports(&dir, &p.project).map_err(reports_to_mcp_error)?;
        list_result(&listed, "reports", p.limit, p.offset)
    }

    /// One saved report, whole -- see `reports_api::get_report`.
    #[tool(
        description = "Read one saved report by id: the whole definition, which is exactly the request build_report takes (entity, by_room, columns, room_columns, measures, shape, the unmatched switches, the filter). Running a saved report is reading it here and passing its fields to build_report; there is no second shape in between."
    )]
    fn get_saved_report(&self, Parameters(p): Parameters<SavedReportParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let report = reports_api::get_report(&dir, &p.project, &p.report).map_err(reports_to_mcp_error)?;
        json_result(&report)
    }

    /// What a report over one entity may name -- see
    /// `service::reports::column_catalog`.
    #[tool(
        description = "List what a report over one entity can put in a column or a filter, for one project: the entity's own property names with the value type Revit stated for each, its `$intrinsics`, any joined reference labels as `source.label`, the room side for a by-room report, and the measures the join produces. READ FROM EACH SNAPSHOT'S PROPERTY DICTIONARY, not by assembling the entity, so it costs a tail read rather than a parse. A name it offers is one SOME element carries -- it does not promise every element has it, which is why a cell can still come back empty. A snapshot stored before the property codec has no dictionary and contributes no names; its properties are still filterable by name."
    )]
    fn list_report_columns(&self, Parameters(p): Parameters<ReportColumnsParams>) -> Result<CallToolResult, McpError> {
        let Some(entity) = reports::Entity::parse(&p.entity) else {
            return Err(McpError::invalid_params(format!("unknown entity {:?} for a report", p.entity), None));
        };
        json_result(&reports::column_catalog(&self.state, Some(&p.project), entity))
    }
}
