//! MCP tools: What the plan geometry answers: area groups, adjacency, reach, connectivity, stacks and the saved connections.
//! One `#[tool]` per HTTP read route; the router is joined with the others in `RoommateMcp::new`.

use super::*;

#[tool_router(router = tool_router_geometry, vis = "pub(super)")]
impl RoommateMcp {
    /// Hierarchy gross-area footprints for one project -- see
    /// `service::areas::assemble_areas`. Shares the exact read logic the HTTP
    /// `GET /projects/{id}/areas` uses; `None` (nothing pushed) mirrors
    /// `get_rooms`' empty-store message.
    #[tool(
        description = "Compute per-level, per-tier dissolved area footprints for one project, optionally scoped by building key and milestone name. Each group carries its resolved classification path, its measured footprint area (an aggregated ROOM FOOTPRINT — room area PLUS the enclosed wall bands between rooms, MINUS any genuine void like a courtyard or atrium; NOT net area and NOT a standards gross), whether it counts toward tiers above it (a settings exclusion can withhold a group), and its footprint polygons (each an exterior ring plus any interior rings for open voids). How much wall a footprint contains follows the boundary regime each model declared: a finish-face level fills the gaps between its rooms up to the project's declared max_wall_thickness, while a centreline level already has its walls inside the room polygons and is dissolved with no fill at all. The response echoes the gap applied per level (wall_gap_by_level) and the project's declared measurement_standard, which may be null — the figure is a house convention, not a standards gross, so read measurement_standard before quoting it. A void wider than a wall stays open and is excluded from the area."
    )]
    fn get_hierarchy_areas(&self, Parameters(p): Parameters<AreasParams>) -> Result<CallToolResult, McpError> {
        let result = areas::assemble_areas(&self.state, &p.project_id, p.building.as_deref(), p.milestone.as_deref())
            .map_err(to_mcp_error)?;
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no snapshots have been pushed to this server yet",
            )])),
            Some(result) => json_result(&result),
        }
    }

    /// Room-to-room adjacency graph for one project -- see
    /// `service::adjacency::assemble_adjacency`. Shares the exact read logic
    /// the HTTP `GET /projects/{id}/adjacency` uses, including the tolerance
    /// validation, so the two front doors cannot disagree on what a valid
    /// `wall_max` is.
    #[tool(
        description = "THE DEFAULT ANSWER IS A SUMMARY (`detail`: each room as id, name, level, centroid; 1000 rooms and 2000 shared walls a page, paged with `limit`/`offset` and `edge_limit`/`edge_offset`); `detail=full` adds classification and reference records. An answer over 1 MB is refused. Compute the room-to-room adjacency graph for one project: which rooms share a wall, and how much wall they share. Optionally scoped by building key and milestone name. Returns nodes (one per room, with its level, centroid, classification path and any joined reference records, each flattened under its source name) and undirected edges (a room pair, their level, and the accumulated shared wall length in feet). Same level only — no cross-floor adjacency. Adjacency here means SHARED WALL geometry, NOT door connectivity: two rooms can share a wall with no door in it, and a door can connect two rooms sharing almost no wall. Doors ARE collected — use get_doors, where every door names its from_room and to_room — but they are a separate edge set over the same rooms, not a refinement of this graph. The `wall_max` parameter is the gap tolerance and matters: a Revit model whose room boundaries sit on wall centrelines has neighbours touching exactly (use 0), while one using finish faces separates them by the wall thickness (use roughly that). Too large a value bridges rooms that merely face each other across a corridor."
    )]
    fn get_adjacency(&self, Parameters(p): Parameters<AdjacencyParams>) -> Result<CallToolResult, McpError> {
        let result = adjacency::assemble_adjacency(
            &self.state,
            &p.project_id,
            p.building.as_deref(),
            p.milestone.as_deref(),
            p.wall_max,
        )
        .map_err(to_mcp_error)?;
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no snapshots have been pushed to this server yet",
            )])),
            Some(result) => adjacency_result(result, &p),
        }
    }

    /// How far a walk reaches from some start points -- see `service::reach`. The same
    /// read as `POST /projects/{id}/reach`.
    #[tool(
        description = "How far a walk of a given length reaches from one or more start points, laid out as a grid over the floor plan: a heat map of walking distance, for questions such as whether everything on a floor plate lies within 30 m of these points (a hose reel, an exit, an extinguisher). \
                       The distance is the door-to-door walk `get_connectivity` computes -- around walls and columns inside rooms, through doors, open-zone walls and passages -- measured from the NEAREST source. One storey at a time: vertical links are not crossed. \
                       Pass `sources` (a room, and optionally the point in it) and `limit_mm` (30 m is 30000); `spacing_mm` sets the grid (default 1000), and `ignore_cuts` treats the closed walls inside open zones as open; `method` is `exact` (default) or `grid`, the lattice approximation that can run longer between cells (median 0.2%, worst seen 14%). \
                       THE ANSWER: `levels` holds one grid per storey reached, row by row from the lowest y; cell (col, row) has its centre at origin + ((col+0.5), (row+0.5)) * spacing_ft (feet, y up) and its value at distance_mm[row * cols + col]: -1 means no room there, -2 means a room but beyond the limit (the search stops at the limit, so a distance past it is not measured), otherwise the walk in whole mm. `rooms` gives each room's coverage -- `cells`, `covered`, `fraction`, `reached` (false means no door, zone or passage lets the walk in at all, a finding about what is connected and not an error), `nearest_mm` and `farthest_mm` -- and a room too small to hold a cell is judged on one point so a cupboard is not left out. `summary` counts rooms fully, partly and not covered, with area in square metres. \
                       LIMITS: a walk is the geometric shortest path, which hugs corners, so it is the least walk that reaches a cell; furniture is not modelled; any allowance beyond the walk is yours to add to `limit_mm`. A grid over 600000 cells is refused."
    )]
    fn get_reach(&self, Parameters(p): Parameters<ReachParams>) -> Result<CallToolResult, McpError> {
        let sources = p
            .sources
            .iter()
            .map(|s| reach::source(&s.room_id, s.model_id.as_deref(), s.x, s.y))
            .collect::<Result<Vec<_>, _>>()
            .map_err(to_mcp_error)?;
        let (limit_mm, spacing_mm) = reach::params(Some(p.limit_mm), p.spacing_mm).map_err(to_mcp_error)?;
        let method = reach::Method::parse(p.method.as_deref()).map_err(to_mcp_error)?;
        let scope = reach::ReachScope {
            method,
            building: p.building.as_deref(),
            milestone: p.milestone.as_deref(),
            ignore_cuts: p.ignore_cuts,
        };
        let result = reach::assemble_reach(&self.state, &p.project_id, &scope, &sources, limit_mm, spacing_mm)
            .map_err(to_mcp_error)?;
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no snapshots have been pushed to this server yet",
            )])),
            Some(result) => json_result(&result),
        }
    }

    /// Door connectivity and the shortest route between two rooms -- see
    /// `service::connectivity::assemble_connectivity`. The same read the HTTP
    /// `GET /projects/{id}/connectivity` uses, including the refusal to guess
    /// between models.
    #[tool(
        description = "Build the connectivity graph for one project -- which rooms are joined by a DOOR, or by an OPEN ZONE a user authored (see list_connections) -- and, when `from` and `to` are given, the shortest route between two rooms. Optionally scoped by building key and milestone name, and by `door_filter` predicates over doors (a door that fails one is not in the graph, so \"Fire Rating!=FRL120\" asks for a route avoiding those doors). \
                       THE GRAPH IS DOORS PLUS WHATEVER OPEN ZONES SOMEBODY HAS AUTHORED, and that is a limit of the method, not a fact about the building: a bay, an open-plan area, an archway modelled as a wall opening, or a shaft has no door, so unless a zone covers it it appears under `isolated` and no route reaches it. An isolated room is NOT a model fault and `isolated` is not a list of bays -- it is the list of rooms this method cannot reach, and some of them are simply rooms whose doors name no room. Each edge and each route step says `kind`: `door` (from the model) or `zone` (user-stated: a hop across a wall two members of one zone share). Read the `connections` block before trusting a route that uses a zone: it lists each zone's stale members, members sharing no wall with another member after any disconnects (`unlinked`), `closed` (walls left closed by a disconnect or the hub), `hub`, `hub_stale` (the hub left the model, so the rule was NOT applied), `stale_disconnects` and `reaches_doors`, and `pinned` is false because a milestone view applies TODAY'S connections. A route crosses a LEVEL only through a VERTICAL LINK somebody drew by hand: one link joins exactly two rooms on different storeys (a lift or stair is a run of floor-to-floor links, never a jump from level 1 to 8). Its step has `kind: vertical` and a `link_id`, its `length` is the link's stated cost (default 40 ft, an assumption and not a measurement), and `path.segments` breaks there into one polyline per level. The `connections.links` block reports every link: whether it was `applied`, why not (a room no longer in scope, both rooms on one storey, or a pair already joined), and `levels_between`, which is above 0 when a link skips storeys. A third user-stated kind, `passage` (a `passage_id`), is a straight line drawn between two rooms on one storey: its step lists the first crossing in `point`, and `connections.passages` reports each one with whether it was `applied`, why not (a room gone, rooms on different storeys, an end outside its room, a line that never leaves or enters), where it crossed and its `span_mm`; its width and height are the author's stated ones and are checked like a door's. A route between storeys with no link is reported as not found, which is a finding about what has been authored, not about the building. WIDTH: pass `width_mm` to ask whether an object of that width can make the trip; the route is then the shortest among doors and open-area walls it fits through. A door's width is an ESTIMATE (its footprint less `frame_mm`, default 150: the footprint includes the frame), an open area's is the length of the wall two of its rooms share, and a level change or a door with no footprint is NOT checked (counted in `clearance.unchecked_hops`). HEIGHT: pass `height_mm` to also require the object to fit under every door AND in every room it passes through: a door lower than that, or a ROOM whose clear height is lower, is not passable (rooms count even at the start and end). A door's height is an ESTIMATE read from the project's `[routing]` sources (its height properties, else the size in its type name such as '970 x 2040'), a room's comes from its height property (default `Ceiling Height`, overridable with `height_property`), and a door or room with no readable height is let through and counted in `clearance.unchecked_height`. `clearance.lowest` is the lowest checked door or room on the route found and `clearance.tallest_possible` the tallest object that fits by ANY route and what limits it. The width of a corridor inside a room is NOT checked, so a route reported clear has cleared its doors and openings only. `clearance` says `narrowest` (the tightest checked hop on the route found) and `widest_possible` (the widest object that could make the trip by ANY route, and the hop that limits it), so you do not have to guess a width; when a width blocks every route, `path.reason` names that limit. \
                       A route that does not exist is a FINDING, not an error: `path.found` is false and `path.reason` names the component each room is in. Do not report it as 'these rooms are not connected'; report it as 'no door route', and read `isolated` and `components` before concluding anything. An error means the request was wrong -- a room that is not in scope, or a bare room id that exists in several models (the message lists them; pass `from_model`/`to_model`). \
                       Other reported states: `exits` on a node counts doors with that room on one side and nothing on the other (the way outside, or into a model that holds no rooms) -- they are not edges; `counts` tallies doors that name no room (`unattached`), the same room twice (`same_room`) or a room outside the scope (`out_of_scope`); `doors_pushed` false means NO doors snapshot exists for the project, so every room is isolated for that reason alone. \
                       `path.distance_ft` is an ESTIMATED walking distance, not a measurement, and what it estimates depends on `method` (the answer lists every method under `methods`, each with its source). `door_to_door` (the default) walks from door to door by the exact shortest path inside each room's outline, around its corners and columns (Liu and Zlatanova 2011); it is the shortest the geometry allows but not where people walk (they keep off walls), and furniture is not modelled. `centroid` walks from each room's centre to its door to the next room's centre, which bends at every centre and can be longer, or shorter where its straight lines cut through walls. `path.start` and `path.end` say where the route began and ended (a room's centre unless from_x/from_y or to_x/to_y gave a point inside it), `path.method` says which ran and `path.note` says if a point had to be moved onto its room's outline or if any room's outline was too irregular to walk exactly (that stretch is a straight line). A route's `steps[].length` under door_to_door is the walk inside the room it leaves plus the crossing, and `path.arrival_ft` is the walk inside the last room. Each edge's `point_source` says whether its door point was the door's own insertion point, its footprint, or a midpoint between the rooms because the door had neither. Rooms are always named by `model_id` plus `room_id`. `path.segments` is the route as polylines, one per run on one level, in the project's local frame. THE DEFAULT RESPONSE OMITS `nodes` AND `edges` (a large project's graph is over a megabyte); it still carries components, isolated rooms, counts and the route. Pass detail=full only when you need the graph itself."
    )]
    fn get_connectivity(&self, Parameters(p): Parameters<ConnectivityParams>) -> Result<CallToolResult, McpError> {
        let known = self.state.settings().known_reference_sources();
        let filter =
            rooms::RoomFilter::parse(&p.door_filter, &known).map_err(|msg| to_mcp_error(ServiceError::Invalid(msg)))?;
        let metric = connectivity::Metric::parse(p.metric.as_deref()).map_err(to_mcp_error)?;
        let detail = connectivity::Detail::parse(p.detail.as_deref()).map_err(to_mcp_error)?;
        let method = roommate::service::routing::Method::parse(p.method.as_deref()).map_err(to_mcp_error)?;
        let from_at = connectivity::point(p.from_x, p.from_y, "start").map_err(to_mcp_error)?;
        let to_at = connectivity::point(p.to_x, p.to_y, "end").map_err(to_mcp_error)?;
        let route = connectivity::endpoints_at(
            p.from.as_deref(),
            p.from_model.as_deref(),
            from_at,
            p.to.as_deref(),
            p.to_model.as_deref(),
            to_at,
        )
        .map_err(to_mcp_error)?;
        let (clear_width_mm, frame_allowance_mm) =
            connectivity::width_params(p.width_mm, p.frame_mm).map_err(to_mcp_error)?;
        let scope = connectivity::ConnectivityScope {
            building: p.building.as_deref(),
            milestone: p.milestone.as_deref(),
            door_filter: Some(&filter).filter(|f| !f.is_empty()),
            clear_width_mm,
            frame_allowance_mm,
            clear_height_mm: connectivity::height_param(p.height_mm).map_err(to_mcp_error)?,
            height_property: p.height_property.as_deref().filter(|n| !n.trim().is_empty()),
            ignore_cuts: false,
        };
        let result = connectivity::assemble_connectivity(
            &self.state,
            &p.project_id,
            &scope,
            route.as_ref().map(|(a, b)| (a, b)),
            metric,
            method,
            detail,
        )
        .map_err(to_mcp_error)?;
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no snapshots have been pushed to this server yet",
            )])),
            Some(result) => json_result(&result),
        }
    }

    /// The rooms over and under one room, as candidates for a vertical link -- see
    /// `service::stacking`. The same read as `GET /projects/{id}/stack`.
    #[tool(
        description = "Suggest vertical links for one room: the rooms on the storey above and the storey below that sit over or under it, ranked, for a person to confirm. A SUGGESTION and never a write -- nothing is stored, and a link is made by hand (PUT /projects/{id}/connections). `up` and `down` each name the NEAREST storey that has a room overlapping this one (not simply the next level in the project's list: a car park's half-levels stacked between hospital floors have nothing over a hospital lift, so they are passed, and `levels_passed` counts them), or are null when nothing overlaps (the top of a shaft, or a stair that moves between floors, which stays manual). Each candidate carries `overlap` (area as a fraction of the SMALLER of the two rooms, so a stair whose landings differ still scores), `fraction_of_room` and `same_stem` (the name shares the source's name without its last word: LIFT 3 TRA003 and LIFT 3 TRA032 share LIFT 3). Candidates start at 0.3 overlap and are ordered stem-mates first, then by overlap. `confidence` is `clear` when one candidate stands alone (the only overlap of 0.7 or more, or the only such stem-mate) and `ambiguous` when the reader must choose (several lifts sharing one plant room, a stair beside a pressurised stairwell). Measured on one large project the stem-mate was the top overlap in about 92% of lift and stair hops, and a stem-mate was never missed when one existed; the name is a boost and not a rule. A room id is unique only within its model: an id two models share is refused and the models are named, and `room_model` resolves it."
    )]
    fn get_stack_candidates(&self, Parameters(p): Parameters<StackParams>) -> Result<CallToolResult, McpError> {
        let result = roommate::service::stacking::stack_candidates(
            &self.state,
            &p.project_id,
            &p.room,
            p.room_model.as_deref(),
            p.milestone.as_deref(),
        )
        .map_err(to_mcp_error)?;
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no snapshots have been pushed to this server yet",
            )])),
            Some(result) => json_result(&result),
        }
    }

    /// The authored connections for one project -- see `connections::load`.
    #[tool(
        description = "List what somebody authored for one project, in five lists (`routes`, `passages` and `reaches` are described last). `zones` are OPEN ZONES: each is a named set of rooms (model_id plus room_id) declared to be one open space, so every wall two of its members share is treated as open and the rooms connect as a door would connect them. A zone may also carry `disconnects` (pairs of its members whose shared wall stays CLOSED, for members that touch but do not connect, such as bays along a corridor) and a `hub` (one member the others connect ONLY through). `links` are VERTICAL LINKS: each joins exactly two rooms on different storeys, drawn by hand, one floor-to-floor hop per link (`cost_ft` is the walking-equivalent of that hop, default 40 ft when absent). Both are USER-STATED, not read from the model, and are applied by get_connectivity (its `connections` block reports what each did: for zones, edges added, pairs a door already joined, stale members, members sharing no wall with another member, and whether the zone reaches a door at all; for links, whether it was applied, why not, and how many storeys it skips). `taken_at` is the version of the document; an empty list with an empty taken_at means nobody has authored anything, which is not the same as a model with no open spaces. A member naming a room that no longer exists is reported as stale by get_connectivity, never an error. `routes` are SAVED ROUTES: each is a request (a name, a start and end room with an optional point in each, a routing method and a `#rrggbb` colour) and never a path, so pass its rooms, points and method to get_connectivity to get the route as it is today. `passages` are PASSAGES: each is a straight line a person drew between two rooms on ONE storey (`a`, `b`, and the line's two ends `from` and `to`, each inside its room), for a way through that no door and no shared wall shows. The points where the line leaves one room and enters the other are derived by get_connectivity and not stored; `width_mm` and `height_mm` are STATED by the author (absent means unknown, so the size checks let it through and count it as unchecked). `reaches` are SAVED REACHES: each is the request a person made to get_reach (start rooms with an optional clicked point, `limit_mm`, `spacing_mm`, `method`, `ignore_cuts`) and never a grid, so pass it to get_reach to see the coverage as the building stands today. Writing is HTTP-only (PUT /projects/{id}/connections), and a save that leaves out `passages`, `routes` or `reaches` keeps what is saved."
    )]
    fn list_connections(&self, Parameters(p): Parameters<ProjectIdParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let document = roommate::connections::load(&dir, &p.project_id).map_err(connections_to_mcp_error)?;
        json_result(&document)
    }
}
