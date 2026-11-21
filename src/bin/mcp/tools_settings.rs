//! MCP tools: Project settings, read only: the files, one project's settings and what it resolves to. Mutation stays behind the HTTP settings page.
//! One `#[tool]` per HTTP read route; the router is joined with the others in `RoommateMcp::new`.

use super::*;

#[tool_router(router = tool_router_settings, vis = "pub(super)")]
impl RoommateMcp {
    // Settings tools are READ-ONLY by design: this is a separate process from
    // the HTTP server, so a write from here could not hot-swap that server's
    // in-memory registry -- the file and the serving process would silently
    // disagree until a restart. Mutation stays behind the HTTP settings UI
    // (see `settings_api`'s module doc), matching the "Read-only access
    // against local state" contract `get_info` declares. `upload_reference`
    // above is not an exception: it forwards to that HTTP server rather than
    // writing anything from this process.

    /// Lists every project settings file with its headline facts -- see
    /// `settings_api::list_project_files`.
    #[tool(
        description = "List every project settings file (project id, is_default, and reference_sources -- the names of the reference sources it declares). \
                          Reads the files fresh, so a settings change saved through the HTTP UI shows here immediately; \
                          this process's own get_rooms/get_validation behavior still reflects the settings loaded at its startup."
    )]
    fn list_project_settings(&self) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let result = settings_api::list_project_files(&dir).map_err(settings_to_mcp_error)?;
        json_result(&result)
    }

    /// One project's parsed settings as JSON -- see
    /// `settings_api::get_project_file`.
    #[tool(
        description = "Get one project's settings (hierarchy, reference sources, builtin properties, room label, QA fields) as JSON. \
                          Reads the file fresh, so a settings change saved through the HTTP UI shows here immediately; \
                          this process's own get_rooms/get_validation behavior still reflects the settings loaded at its startup."
    )]
    fn get_project_settings(&self, Parameters(p): Parameters<ProjectIdParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let (file, settings) = settings_api::get_project_file(&dir, &p.project_id).map_err(settings_to_mcp_error)?;
        json_result(&serde_json::json!({ "file": file, "settings": settings }))
    }

    /// One project's settings resolved the way the *viewer* resolves them:
    /// exact `project_id` match, else the file marked `is_default` -- see
    /// `settings_api::resolve_project_file`.
    ///
    /// A separate tool rather than a flag on `get_project_settings`, mirroring
    /// the two separate HTTP routes and for the same reason: the fallback must
    /// not be reachable from the path an editor reads before it writes, where
    /// loading the default and saving it back would overwrite the wrong file.
    /// An agent has no such write path here -- both are reads -- but collapsing
    /// them would make the MCP surface disagree with the HTTP one about what
    /// "get this project's settings" means, and the two front doors answering
    /// the same question differently is the thing the service seam exists to
    /// prevent.
    #[tool(
        description = "Get the settings that APPLY to one project id, falling back to the default settings file when no file declares that id. \
                          Use this when the id came from room or door DATA -- a payload project id, as returned by list_projects -- rather than from a settings file: \
                          the two id spaces differ, so get_project_settings can legitimately report 'not found' for a project that is nonetheless being served settings. \
                          Prefer get_project_settings when you mean 'the settings file whose project_id is X': it answers about a FILE and never falls back. \
                          The response names the file the settings actually came from, so a fallback is visible rather than silent -- a 'file' unrelated to the id you \
                          passed means the default answered, and the values you are reading are not authored for that project."
    )]
    fn resolve_project_settings(&self, Parameters(p): Parameters<ProjectIdParams>) -> Result<CallToolResult, McpError> {
        let dir = self.projects_dir()?;
        let (file, settings) =
            settings_api::resolve_project_file(&dir, &p.project_id).map_err(settings_to_mcp_error)?;
        json_result(&serde_json::json!({ "file": file, "settings": settings }))
    }
}
