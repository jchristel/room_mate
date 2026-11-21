//! Bounding an answer: every tool answers within the size limit (\`mcp_bounds\`), paged with its total and a next offset, and says how to ask for less. Summaries, listing and the rooms answer.

use super::*;

/// Advice for a tool with no paging of its own: scope the request.
pub(super) const GENERIC_ADVICE: &str =
    "Narrow the request (a project, a milestone, a single id, a smaller area) or use \
    build_report for chosen columns. For bulk data use the HTTP API, which is not bounded.";

/// An entity read whose answer is one list of elements, so one function can page and summarise it.
pub(super) trait Listing: serde::Serialize {
    type Element: serde::Serialize;
    /// The answer's list key, and what a summary row keeps of an element.
    const KEY: &'static str;
    const SUMMARY: &'static [&'static str];
    fn elements(&mut self) -> &mut Vec<Self::Element>;
    /// The shared type-property table and each element's row in it, for the entities that have one.
    fn table(&mut self) -> Option<&mut Vec<std::sync::Arc<roommate::contract::PropertyMap>>> {
        None
    }
    fn row(_: &mut Self::Element) -> Option<&mut Option<u32>> {
        None
    }
}

macro_rules! listing {
    ($result:ty, $element:ty, $key:literal, $summary:expr, $list:ident, tabled) => {
        impl Listing for $result {
            type Element = $element;
            const KEY: &'static str = $key;
            const SUMMARY: &'static [&'static str] = $summary;
            fn elements(&mut self) -> &mut Vec<$element> {
                &mut self.$list
            }
            fn table(&mut self) -> Option<&mut Vec<std::sync::Arc<roommate::contract::PropertyMap>>> {
                Some(&mut self.type_property_sets)
            }
            fn row(e: &mut $element) -> Option<&mut Option<u32>> {
                Some(&mut e.type_properties_ref)
            }
        }
    };
    ($result:ty, $element:ty, $key:literal, $summary:expr, $list:ident) => {
        impl Listing for $result {
            type Element = $element;
            const KEY: &'static str = $key;
            const SUMMARY: &'static [&'static str] = $summary;
            fn elements(&mut self) -> &mut Vec<$element> {
                &mut self.$list
            }
        }
    };
}

pub(super) const OPENING_SUMMARY: &[&str] = &[
    "id",
    "type_name",
    "level_id",
    "model_id",
    "from_room",
    "to_room",
    "owner_rooms",
];

pub(super) const SURFACE_SUMMARY: &[&str] = &["id", "type_name", "level_id", "model_id", "height_offset", "rooms"];

listing!(openings::DoorsResult, openings::OpeningResponse, "doors", OPENING_SUMMARY, doors, tabled);
listing!(
    openings::WindowsResult,
    openings::OpeningResponse,
    "windows",
    OPENING_SUMMARY,
    windows,
    tabled
);
listing!(
    items::FfeResult,
    items::ItemResponse,
    "ffe",
    &[
        "id",
        "category",
        "type_name",
        "family_name",
        "level_id",
        "model_id",
        "room",
        "owner_rooms"
    ],
    ffe,
    tabled
);
listing!(
    spaces::SpacesResult,
    spaces::SpaceResponse,
    "spaces",
    &["id", "name", "level_id", "model_id"],
    spaces
);
listing!(
    surfaces::CeilingsResult,
    surfaces::SurfaceResponse,
    "ceilings",
    SURFACE_SUMMARY,
    ceilings,
    tabled
);
listing!(
    surfaces::FloorsResult,
    surfaces::SurfaceResponse,
    "floors",
    SURFACE_SUMMARY,
    floors,
    tabled
);

/// What to try when an element answer is too big to give.
pub(super) const LISTING_ADVICE: &str =
    "Ask for less: `filter`, `building`, a smaller `limit` with `offset` to page, or \
    `detail=summary`. To list or count elements with chosen columns use build_report. For bulk geometry use the \
    HTTP API or the SVG export, which are not bounded.";

/// One page of an element read, as a summary row each or the whole record, in the shared envelope.
pub(super) fn listing_result<R: Listing>(
    mut result: R,
    detail: Option<&str>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<CallToolResult, McpError> {
    let detail = Detail::parse(detail).map_err(|m| McpError::invalid_params(m, None))?;
    let (mut kept, info) =
        mcp_bounds::paginate(std::mem::take(result.elements()), mcp_bounds::ELEMENT_SIZES.page(detail, limit, offset));
    let internal = |e: serde_json::Error| McpError::internal_error(format!("failed to serialize response: {e}"), None);
    let mut answer = match detail {
        Detail::Full => {
            if let Some(table) = result.table() {
                let all = std::mem::take(table);
                *table = roommate::service::type_table::restrict(&mut kept, &all, |e| {
                    R::row(e).expect("an entity with a table has a row")
                });
            }
            *result.elements() = kept;
            serde_json::to_value(&result).map_err(internal)?
        }
        Detail::Summary => {
            result.table().map(std::mem::take);
            let mut answer = serde_json::to_value(&result).map_err(internal)?;
            let rows = kept
                .iter()
                .map(|e| {
                    let full = serde_json::to_value(e).map_err(internal)?;
                    let row: serde_json::Map<String, serde_json::Value> = R::SUMMARY
                        .iter()
                        .filter_map(|k| full.get(*k).map(|v| ((*k).to_string(), v.clone())))
                        .collect();
                    Ok(serde_json::Value::Object(row))
                })
                .collect::<Result<Vec<_>, McpError>>()?;
            if let serde_json::Value::Object(map) = &mut answer {
                map.remove("type_property_sets");
                map.insert(R::KEY.to_string(), serde_json::Value::Array(rows));
            }
            answer
        }
    };
    if let serde_json::Value::Object(map) = &mut answer {
        mcp_bounds::describe(
            map,
            detail,
            &info,
            &[
                "footprint (loops/polygons)",
                "properties",
                "type properties",
                "everything not listed in a row",
            ],
            "pass detail=full for footprints and properties (combine with filter, building and limit/offset), or \
             use build_report for chosen columns",
        );
    }
    let text = mcp_bounds::bounded(&answer, R::KEY, LISTING_ADVICE).map_err(|m| McpError::invalid_params(m, None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

/// A project's adjacency graph, paged: rooms as summary or full, shared walls as their own page.
pub(super) fn adjacency_result(
    mut result: adjacency::AdjacencyResult,
    p: &AdjacencyParams,
) -> Result<CallToolResult, McpError> {
    let detail = Detail::parse(p.detail.as_deref()).map_err(|m| McpError::invalid_params(m, None))?;
    let (nodes, node_page) =
        mcp_bounds::paginate(std::mem::take(&mut result.nodes), mcp_bounds::ROOM_SIZES.page(detail, p.limit, p.offset));
    let (edges, edge_page) = mcp_bounds::paginate(
        std::mem::take(&mut result.edges),
        mcp_bounds::WALL_SIZES.page(Detail::Summary, p.edge_limit, p.edge_offset),
    );
    result.edges = edges;
    let internal = |e: serde_json::Error| McpError::internal_error(format!("failed to serialize response: {e}"), None);
    let mut answer = serde_json::to_value(&result).map_err(internal)?;
    let rows = match detail {
        Detail::Full => serde_json::to_value(&nodes).map_err(internal)?,
        Detail::Summary => serde_json::Value::Array(
            nodes
                .iter()
                .map(|n| {
                    serde_json::json!({
                        "room_id": n.room_id,
                        "name": n.name,
                        "level_id": n.level_id,
                        "centroid": n.centroid,
                    })
                })
                .collect(),
        ),
    };
    if let serde_json::Value::Object(map) = &mut answer {
        map.insert("nodes".into(), rows);
        mcp_bounds::describe(
            map,
            detail,
            &node_page,
            &["classification", "joined reference records"],
            "pass detail=full for each room's classification and reference records, or use get_rooms",
        );
        map.insert("edges_page".into(), serde_json::json!(edge_page));
    }
    let text =
        mcp_bounds::bounded(&answer, "get_adjacency", GENERIC_ADVICE).map_err(|m| McpError::invalid_params(m, None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

/// A list tool's answer: one page, in the shared envelope, under the key the list sits at.
pub(super) fn list_result<T: serde::Serialize>(
    value: &T,
    key: &str,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<CallToolResult, McpError> {
    let value = serde_json::to_value(value)
        .map_err(|e| McpError::internal_error(format!("failed to serialize response: {e}"), None))?;
    json_result(&mcp_bounds::page_list(
        value,
        key,
        mcp_bounds::LIST_SIZES.page(Detail::Summary, limit, offset),
    ))
}

/// What to try when a report is too big to give.
pub(super) const REPORT_ADVICE: &str = "Ask for fewer columns, a smaller `limit`, or a `filter`; page with `offset`.";

/// What to try when a rooms answer is too big to give.
pub(super) const ROOMS_ADVICE: &str =
    "Ask for less: `level` (one storey), `building`, `filter`, a smaller `limit` with `offset` \
    to page, or `detail=summary`. To list or count rooms use build_report. For bulk geometry use the HTTP API or \
    the SVG export, which are not bounded.";

/// Shape a rooms answer for a model: a summary row per room, or the whole record, one page of either.
pub(super) fn rooms_answer(
    mut result: rooms::RoomsResult,
    detail: Detail,
    page: Page,
) -> Result<serde_json::Value, String> {
    let level_name: HashMap<String, String> = result.levels.iter().map(|l| (l.id.clone(), l.name.clone())).collect();
    let (kept, info) = mcp_bounds::paginate(std::mem::take(&mut result.rooms), page);
    let mut answer = match detail {
        Detail::Full => {
            result.rooms = kept;
            serde_json::to_value(&result).map_err(|e| format!("failed to serialize response: {e}"))?
        }
        Detail::Summary => {
            let rows: Vec<serde_json::Value> = kept
                .iter()
                .map(|r| {
                    let number = r.room.properties.get("Number").map(|v| v.value.as_str());
                    serde_json::json!({
                        "id": r.room.id,
                        "name": r.room.name,
                        "number": number,
                        "level_id": r.room.level_id,
                        "level": level_name.get(&r.room.level_id),
                    })
                })
                .collect();
            serde_json::json!({
                "schema_version": result.schema_version,
                "revision": result.revision,
                "levels": result.levels,
                "phase_by_model": result.phase_by_model,
                "rooms": rows,
            })
        }
    };
    if let serde_json::Value::Object(map) = &mut answer {
        mcp_bounds::describe(
            map,
            detail,
            &info,
            &[
                "outline (loops)",
                "properties",
                "reference-source joins",
                "reference_labels",
                "boundary_by_level",
            ],
            "pass detail=full for outlines, properties and reference joins (about 3 KB a room: combine with \
             level, building, filter and limit/offset), or use build_report for chosen columns",
        );
    }
    Ok(answer)
}

/// Narrow a rooms answer to one level, by name or id. Refuses an unknown one and lists what exists.
pub(super) fn keep_level(result: &mut rooms::RoomsResult, wanted: &str) -> Result<(), String> {
    let wanted = wanted.trim();
    let ids: Vec<String> = result
        .levels
        .iter()
        .filter(|l| l.id == wanted || l.name.eq_ignore_ascii_case(wanted))
        .map(|l| l.id.clone())
        .collect();
    if ids.is_empty() {
        let names: Vec<&str> = result.levels.iter().map(|l| l.name.as_str()).collect();
        return Err(format!("no level \"{wanted}\"; the levels are: {}", names.join(", ")));
    }
    result.levels.retain(|l| ids.contains(&l.id));
    result.rooms.retain(|r| ids.contains(&r.room.level_id));
    result.boundary_by_level.retain(|k, _| ids.contains(k));
    Ok(())
}

/// Minimal percent-encoding for URL path/query components. The values that
/// pass through here are constrained by construction — project ids are
/// path-safe (`is_path_safe_component`) and `taken_at` is RFC3339, whose only
/// URL-reserved character is the `+` of a numeric offset — so a tiny
/// encode-everything-non-unreserved loop beats a dependency.
pub(super) fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Which reference source a reference tool call means.
///
/// The tools used to hardcode `"drofus"`, which quietly made every other
/// configured source unreachable over MCP. Making `source` *required* instead
/// would have been the honest fix and a bad trade: the overwhelmingly common
/// project configures exactly one source, and an agent should not have to make
/// a `get_project_settings` round trip to learn a name the server can infer.
///
/// So: an explicit `source` always wins. Omitted, it resolves to the project's
/// sole configured source. Ambiguity is refused rather than guessed — picking
/// "the first one" would silently answer about the wrong dataset, and the
/// error names the actual choices so the retry is obvious.
pub(super) fn resolve_source(state: &Shared, project_id: &str, requested: Option<&str>) -> Result<String, McpError> {
    if let Some(name) = requested {
        return Ok(name.to_string());
    }
    let registry = state.settings();
    let names: Vec<&String> = registry
        .settings_for(project_id)
        .map(|bundle| bundle.reference.keys().collect())
        .unwrap_or_default();
    match names.as_slice() {
        [only] => Ok((*only).clone()),
        [] => Err(McpError::invalid_params(
            format!("project '{project_id}' configures no reference source"),
            None,
        )),
        many => Err(McpError::invalid_params(
            format!(
                "project '{project_id}' configures {} reference sources ({}); pass `source` to say which",
                many.len(),
                many.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            ),
            None,
        )),
    }
}
