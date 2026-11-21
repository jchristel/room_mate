//! MCP tools: Snapshots and milestones: what was pushed when, the comparison between milestones, the validation report and the reference data (the one tool that writes forwards to the HTTP server).
//! One `#[tool]` per HTTP read route; the router is joined with the others in `RoommateMcp::new`.

use super::*;

#[tool_router(router = tool_router_snapshots, vis = "pub(super)")]
impl RoommateMcp {
    /// Lists one project's milestones (named dated snapshot pins) -- see
    /// `service::milestones::list_milestones`.
    #[tool(
        description = "List one project's milestones: named dates with data snapshots pinned to them, newest first — each carries its model-pin count and `reference_snapshots`, a map of reference source name to the snapshot id that milestone pins for it (absent sources are joined at their current data). Pass a milestone's name to get_rooms to view the project as captured at that milestone, rooms AND every reference source."
    )]
    fn list_milestones(&self, Parameters(p): Parameters<ListParams>) -> Result<CallToolResult, McpError> {
        let result = milestones::list_milestones(&self.state, &p.project_id).map_err(to_mcp_error)?;
        list_result(&result, "milestones", p.limit, p.offset)
    }

    /// Compares N milestones against a baseline for one project -- see
    /// `service::comparison::compare_milestones`. A project with no
    /// `comparison_key` configured returns `comparison_key_configured: false`.
    #[tool(
        description = "Compare milestones for one project: one baseline milestone versus each of the others (a star diff, not all-pairs). \
                          Reports rooms added and removed relative to the baseline, and per-property differences on rooms present in both, \
                          over the project's configured comparison property set. Rooms are matched by the project's user-defined comparison_key \
                          property (its own setting, NOT any reference source's link property); if none is configured the result is comparison_key_configured: false. \
                          A separate 'doors' section reports the same diff over the project's DOORS -- doors_added, doors_removed and changed_doors, each changed door carrying \
                          its model_id because a door id is unique only within one model. Doors are configured independently, by [doors] comparison_key and [doors] \
                          comparison_properties, so a project can compare rooms and not doors or the reverse: check 'doors.comparison_key_configured' separately from the \
                          top-level one. Comparing '$to_room'/'$from_room' is how a door that MOVED between rooms shows up; losing a room reference entirely is reported as a \
                          missing property rather than a difference against an empty value. Doors come from the snapshots a milestone pins in door_attachments, which are \
                          separate pins from the rooms ones."
    )]
    fn compare_milestones(
        &self,
        Parameters(p): Parameters<CompareMilestonesParams>,
    ) -> Result<CallToolResult, McpError> {
        let result =
            comparison::compare_milestones(&self.state, &p.project_id, &p.baseline, &p.others).map_err(to_mcp_error)?;
        json_result(&result)
    }

    /// Lists every stored snapshot id for one project, grouped per model --
    /// see `service::snapshots::list_project_snapshots`.
    #[tool(
        description = "List every stored snapshot id (RFC3339 UTC taken_at) for one project, grouped per model, each group carrying its latest"
    )]
    fn list_snapshots(&self, Parameters(p): Parameters<ListParams>) -> Result<CallToolResult, McpError> {
        let result = snapshots::list_project_snapshots(&self.state, &p.project_id).map_err(to_mcp_error)?;
        list_result(&result, "models", p.limit, p.offset)
    }

    /// The latest snapshot id for one model -- see
    /// `service::snapshots::latest_snapshot`. The service's `None` (the HTTP
    /// 404 case) becomes a short plain-text answer, same convention as
    /// `get_rooms`' empty-store case.
    #[tool(description = "Get the latest snapshot id (taken_at) for one model of one project")]
    fn get_latest_snapshot(&self, Parameters(p): Parameters<ModelIdParams>) -> Result<CallToolResult, McpError> {
        let result = snapshots::latest_snapshot(&self.state, &p.project_id, &p.model_id).map_err(to_mcp_error)?;
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no snapshots stored for that project/model",
            )])),
            Some(latest) => json_result(&latest),
        }
    }

    /// The quarantined push waiting on one model -- see
    /// `service::snapshots::pending_snapshot`. Read-only: activating one is a
    /// mutation and stays HTTP-only, same line ingest draws.
    #[tool(
        description = "Get the push waiting to be activated for one model, if any. A model's Revit phase is fixed by its first phased push, so a push declaring a different phase is stored but NOT made live -- nothing reads it until someone activates it over HTTP. Returns its snapshot id, the phase it would move the model to ('phase'), the phase the model is on now ('current_phase'), and its room count."
    )]
    fn get_pending_snapshot(&self, Parameters(p): Parameters<ModelIdParams>) -> Result<CallToolResult, McpError> {
        let result = snapshots::pending_snapshot(&self.state, &p.project_id, &p.model_id).map_err(to_mcp_error)?;
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no pending push for that project/model",
            )])),
            Some(pending) => json_result(&pending),
        }
    }

    /// Runs the reference reconciliation QA report for one project -- see
    /// `service::validation::compute_project_validation`.
    #[tool(
        description = "LISTS IN THE REPORT ARE CUT to `list_limit` entries (default 200); `cut_lists` names each list cut and its true total, so counts stay exact. Run the reference reconciliation validation report for one project. Returns one report per configured reference source under 'sources' (keyed by source name, each with its own link_property, discrepancy lists, field coverage and 'error_rooms' room_id -> number/name/link value for the flagged rooms), plus a cross-source 'discrepancies' summary for a one-shot count. \
                       Unmatched is reported in BOTH directions and they mean different things: 'rooms_unmatched' lists room ids whose link value finds no record, while 'reference_unmatched' lists the source's own link values that no room resolves to (bare values, not room ids -- there is no room, which is the finding). A value shared by several rooms counts as matched and is reported once under 'duplicate_link_values' instead. \
                       Check 'link_property_absent_everywhere' BEFORE reading either list: when true, the source's link property names no property on any room in the project, so the join never ran -- every room appears under 'rooms_missing_link_value' and every row under 'reference_unmatched' for that one reason, and neither list says anything about the data. It is a settings fault (a mistyped link property, or one needing a [[builtin_properties]] alias to the Revit property that holds it), not a data-quality finding, and reporting the lists as findings would be wrong. False when the property exists but is blank everywhere -- that is an ordinary data gap. \
                       An empty 'sources' map means the project reconciles against nothing -- normal, not an error. \
                       Separately from any reference source, 'phases' reports which Revit phase each of the project's models was filtered to ('by_model') and whether they \
                       disagree ('disagree'). A true 'disagree' means /rooms is merging rooms from two different phases into one plan that will nonetheless look complete; \
                       an unphased model (null) counts as a distinct value there, since its rooms were never filtered at all. This is reported, never rejected. \
                       Also separate from any reference source, 'openings' reports whether the project's openings link to rooms that exist. It is a MAP KEYED BY ENTITY -- 'doors' and 'windows' -- and every report in it has the identical shape, so read one and you can read them all. An entity with nothing pushed still appears, reporting zero: absent would be ambiguous with 'not supported'. \n                       Per report: 'without_room_reference' lists openings naming no room on either side, and 'unresolved_room' lists room references naming a room the opening's own model does not have (one entry per dangling side, each carrying model_id, opening_id, side and room_id). References resolve WITHIN one model, because room ids are unique only within a model. \n                       An opening with a room on exactly one side is EXTERNAL -- normal, counted under 'external', and deliberately not a discrepancy. For doors that is the exception; for windows in a facade model it is the rule, and a facade file that links its interiors reports EVERY opening under 'without_room_reference' because Revit cannot see a room across a link. Read 'room_resolution' before treating that as a data fault: 'off' means the geometric fallback never ran, which is not the same as clean. \n                       Each report has its own 'discrepancies', NOT included in the top-level 'discrepancies', which counts reference sources only. \n                       'room_attribution' echoes the policy deciding which room owns an opening (default: the room it opens into, else the one it opens from, else homeless); \n                       'unattributed' lists openings that name a room the policy declines to use, which is a policy consequence rather than a data gap and is empty under the default. \n                       'room_reference_mismatches' lists openings whose AUTHORED room reference disagrees with the attributed room -- reported, never corrected, since the geometry and \n                       the modeller are making different claims. It is empty when that entity's room_reference_property is unset, which means the check is OFF, not clean. \n\
                       Beside 'openings' is 'items', keyed 'ffe', which reports the same question for FF&E and is a DIFFERENT SHAPE because an item is a different record. \
                       There is no 'external' and no 'unattributed': an item sits in ONE room, so there are no sides to be external on and no attribution policy to decline anything. \
                       'without_room' lists items naming no room, and unlike the doors report it is deliberately NOT counted in 'discrepancies' -- an item outside every room is very \
                       often correct (a bollard, external furniture, plant on a roof), and on a measured model it was 75 of 647. Treat a large 'without_room' as information, not a fault. \
                       'excluded_components' counts items this project's [ffe] nested_components policy removed from BOTH this report and /ffe, and 'nested_components' echoes that \
                       policy. A large value is normal: 179 of 647 on the measured model, nearly all hardware nested inside other families. \
                       Its 'discrepancies' counts only unresolved_room, room_reference_mismatches, phase_drift and room_geometry_mismatches, and like the openings reports it is NOT \
                       included in the top-level 'discrepancies'. \
                       Finally 'spaces' reconciles the project's SPACES against its ROOMS, and it is a third shape again -- not a map, because there is one such report per project. \
                       Read it in two halves. The PRESENCE half needs no configuration and is always populated: 'models_not_audited' names models the project knows that have no \
                       spaces snapshot at all, 'models_without_spaces' names models that pushed one holding ZERO spaces. Those two are NOT the same finding -- the second means \
                       the model was checked and genuinely has none, which is usually what somebody wants to know. A model that has never pushed anything appears in NEITHER, \
                       which is a stated limitation rather than an oversight. 'enclosure' counts enclosed / unenclosed / unmeasured across the project; unenclosed is a model \
                       defect and unmeasured a pipeline one. \
                       The MATCH half is empty unless [spaces] comparison_key is set -- check 'comparison_key' before reading any empty list as clean, because empty means \
                       nothing was attempted. 'by_model' has one row per SERVICES model, each with matched / unmatched / without_key / ambiguous_keys. Ambiguity is counted \
                       WITHIN one model only: one services file per service means the same room number legitimately names a space in each of them, so the same key in four \
                       models is four disciplines and NOT a duplicate. 'rooms_without_space' is the other direction in full, and 'room_models' says which models supplied the \
                       rooms -- read it before concluding a room set is small. \
                       'property_mismatches' carries 'delta_pct' where a numeric tolerance is configured; a large one on a very small room is usually the boundary-regime \
                       difference between an MEP space and an architectural room rather than a data fault, which is what [spaces] tolerance_min exists to filter. \
                       'incomparable' is a pair whose ROOM value is zero: neither a pass nor a mismatch, since a relative difference against zero is undefined."
    )]
    fn get_validation(&self, Parameters(p): Parameters<ValidationParams>) -> Result<CallToolResult, McpError> {
        let result = validation::compute_project_validation(&self.state, &p.project_id).map_err(to_mcp_error)?;
        let mut answer = serde_json::to_value(&result)
            .map_err(|e| McpError::internal_error(format!("failed to serialize response: {e}"), None))?;
        let cap = Page::new(p.list_limit, None, 200, 5000).limit;
        let cut = mcp_bounds::cap_lists(&mut answer, cap);
        if let (false, serde_json::Value::Object(map)) = (cut.is_empty(), &mut answer) {
            map.insert("cut_lists".into(), serde_json::json!(cut));
            map.insert(
                "cut_note".into(),
                serde_json::json!(
                    "lists above list_limit were cut to their first entries (see cut_lists for each total); raise \
                     list_limit, or list the offenders with build_report or the element read"
                ),
            );
        }
        json_result(&answer)
    }

    /// Lists every uploaded snapshot id for one project's reference source --
    /// see `service::reference::list_reference_snapshots`.
    #[tool(
        description = "List every uploaded CSV snapshot id (RFC3339 UTC taken_at) for one project's reference source, ascending, with the latest. \
                          `source` names the reference source (e.g. \"drofus\") and may be omitted when the project configures exactly one; \
                          when it configures several, the error names them. \
                          Reads the shared store fresh, so an upload forwarded moments ago shows here immediately."
    )]
    fn list_reference_snapshots(
        &self,
        Parameters(p): Parameters<ReferenceSourceParams>,
    ) -> Result<CallToolResult, McpError> {
        let source = resolve_source(&self.state, &p.project_id, p.source.as_deref())?;
        let result = reference::list_reference_snapshots(&self.state, &p.project_id, &source).map_err(to_mcp_error)?;
        json_result(&result)
    }

    /// A parsed summary of one uploaded reference CSV -- see
    /// `service::reference::get_reference_snapshot`. The service's `None` (the
    /// HTTP 404 case) becomes a short plain-text answer, same convention as
    /// `get_latest_snapshot`.
    #[tool(
        description = "Get a parsed summary (record count, link property, field labels) of one uploaded reference CSV -- the given taken_at, or the latest when omitted. \
                          `source` names the reference source (e.g. \"drofus\") and may be omitted when the project configures exactly one. \
                          Reads the shared store fresh."
    )]
    fn get_reference_snapshot(
        &self,
        Parameters(p): Parameters<GetReferenceSnapshotParams>,
    ) -> Result<CallToolResult, McpError> {
        let source = resolve_source(&self.state, &p.project_id, p.source.as_deref())?;
        let result = reference::get_reference_snapshot(&self.state, &p.project_id, &source, p.taken_at.as_deref())
            .map_err(to_mcp_error)?;
        match result {
            None => Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "no such '{source}' upload stored for that project"
            ))])),
            Some(info) => json_result(&info),
        }
    }

    /// Uploads a reference CSV by FORWARDING it to the running HTTP server --
    /// this process never writes the store itself (see the module doc): the
    /// server validates, stores, and hot-swaps its own registry, staying the
    /// single writer.
    #[tool(description = "Upload a reference-source CSV export (given as an absolute file path) for one project. \
                          `source` names the reference source (e.g. \"drofus\") and may be omitted when the project configures exactly one. \
                          Forwards the file over HTTP to the running roommate server, which validates it against that source's \
                          declared fields before storing it as a dated snapshot and applying it live -- so the HTTP server must be running. \
                          The project's settings must declare [sources.reference.<source>] type = \"upload\". \
                          Note the staleness asymmetry: after an upload, this process's own get_rooms/get_validation still join the \
                          data loaded at ITS startup; list_reference_snapshots/get_reference_snapshot read the store fresh and see the new upload immediately.")]
    async fn upload_reference(
        &self,
        Parameters(p): Parameters<UploadReferenceParams>,
    ) -> Result<CallToolResult, McpError> {
        if !self.allow_upload {
            return Err(McpError::invalid_params(
                "upload_reference is turned off on this server (start it with --allow-upload to enable it)",
                None,
            ));
        }
        let source = resolve_source(&self.state, &p.project_id, p.source.as_deref())?;
        let bytes = std::fs::read(&p.path)
            .map_err(|e| McpError::invalid_params(format!("could not read CSV file {:?}: {e}", p.path), None))?;

        let mut url = format!(
            "{}/projects/{}/reference/{}",
            self.server_url.trim_end_matches('/'),
            urlencode(&p.project_id),
            urlencode(&source)
        );
        if let Some(taken_at) = &p.taken_at {
            url.push_str(&format!("?taken_at={}", urlencode(taken_at)));
        }

        let response = reqwest::Client::new()
            .post(&url)
            .header("Content-Type", "text/csv")
            .body(bytes)
            .send()
            .await
            .map_err(|e| {
                McpError::internal_error(
                    format!(
                        "the roommate HTTP server is not reachable at {} ({e}) -- \
                         start it (it is the single writer for uploads) and retry",
                        self.server_url
                    ),
                    None,
                )
            })?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(CallToolResult::success(vec![ContentBlock::text(body)]))
        } else {
            // The server's rejection text is the real validator -- pass it on
            // verbatim, marked caller-addressable.
            Err(McpError::invalid_params(format!("server answered {status}: {body}"), None))
        }
    }
}
