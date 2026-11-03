//! How much one MCP tool answer may hold, and how an answer says it is not everything.
//!
//! **Why this exists.** A model reads every byte it is handed, and a client may answer an oversized
//! one by dropping the connection. The HTTP API serves the viewer, which needs whole records in one
//! piece; the MCP side is read by a model with a small window, so it is bounded here and nowhere
//! else. One module so every tool bounds itself the same way and says so in the same words.
//!
//! Pure on purpose (no `rmcp`, no `axum`): the binary turns a refusal into a protocol error.

use serde::Serialize;
use serde_json::{json, Map, Value};

/// The most one answer may hold. About a quarter of a million tokens at the very least, which is
/// already more than a model should be given; refusing above it is a floor, not a target.
pub const ANSWER_MAX_BYTES: usize = 1024 * 1024;

/// A window onto a list: where to start and how many to return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub limit: usize,
    pub offset: usize,
}

impl Page {
    /// `default` and `ceiling` differ per tool because a row differs per tool: a one-line summary
    /// row can be thousands to a page, a full record with geometry only a few hundred. A `limit`
    /// above the ceiling is lowered to it, and `0` counts as the default (a client that sends zero
    /// means "unspecified", not "nothing").
    pub fn new(limit: Option<usize>, offset: Option<usize>, default: usize, ceiling: usize) -> Self {
        let limit = match limit {
            None | Some(0) => default,
            Some(n) => n.min(ceiling),
        };
        Self { limit, offset: offset.unwrap_or(0) }
    }
}

/// Page sizes for one kind of record, per form: `(default, ceiling)`.
///
/// **Named once** so a size is a decision made in one place: a summary row is a line and a full
/// record carries geometry, so the same entity has two sizes, and two tools reading the same
/// records must not drift apart.
#[derive(Debug, Clone, Copy)]
pub struct Sizes {
    pub summary: (usize, usize),
    pub full: (usize, usize),
}

impl Sizes {
    pub fn page(self, detail: Detail, limit: Option<usize>, offset: Option<usize>) -> Page {
        let (default, ceiling) = match detail {
            Detail::Summary => self.summary,
            Detail::Full => self.full,
        };
        Page::new(limit, offset, default, ceiling)
    }
}

/// Rooms, and the rooms of the adjacency graph: a full room is outline, properties and joins.
pub const ROOM_SIZES: Sizes = Sizes { summary: (1000, 5000), full: (100, 500) };
/// Doors, windows, FF&E, spaces, ceilings, floors.
pub const ELEMENT_SIZES: Sizes = Sizes { summary: (500, 2000), full: (50, 200) };
/// A list of authored records or project facts: no summary form, so both forms are one size.
pub const LIST_SIZES: Sizes = Sizes { summary: (200, 1000), full: (200, 1000) };
/// Report rows and the shared walls of the adjacency graph: a row is a few cells.
pub const ROW_SIZES: Sizes = Sizes { summary: (500, 5000), full: (500, 5000) };
pub const WALL_SIZES: Sizes = Sizes { summary: (2000, 10_000), full: (2000, 10_000) };

/// How a tool bounds its answer. **Every tool declares one**, and a test holds the declaration to the
/// tool list, so a new tool cannot be added without choosing how it behaves when its data grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A list of heavy records: `detail` summary or full, paged.
    PagedDetail,
    /// A list with no summary form: paged.
    Paged,
    /// A report built mostly of counts: its long lists are cut, and the cut is reported.
    Cut,
    /// One document or one computed result: refused above the limit, never paged.
    Document,
}

/// How each tool bounds its answer. Held to the registered tool list by a test in `bin/mcp.rs`.
pub const TOOL_KINDS: &[(&str, Kind)] = &[
    ("get_rooms", Kind::PagedDetail),
    ("get_doors", Kind::PagedDetail),
    ("get_windows", Kind::PagedDetail),
    ("get_ffe", Kind::PagedDetail),
    ("get_spaces", Kind::PagedDetail),
    ("get_ceilings", Kind::PagedDetail),
    ("get_floors", Kind::PagedDetail),
    ("get_adjacency", Kind::PagedDetail),
    ("build_report", Kind::Paged),
    ("list_projects", Kind::Paged),
    ("list_buildings", Kind::Paged),
    ("list_milestones", Kind::Paged),
    ("list_snapshots", Kind::Paged),
    ("list_blueprints", Kind::Paged),
    ("list_assignments", Kind::Paged),
    ("list_tasks", Kind::Paged),
    ("list_sheets", Kind::Paged),
    ("list_tag_rules", Kind::Paged),
    ("list_saved_reports", Kind::Paged),
    ("get_validation", Kind::Cut),
    ("capture_blueprint", Kind::Document),
    ("compare_milestones", Kind::Document),
    ("get_blueprint", Kind::Document),
    ("get_connectivity", Kind::Document),
    ("get_hierarchy_areas", Kind::Document),
    ("get_latest_snapshot", Kind::Document),
    ("get_pending_snapshot", Kind::Document),
    ("get_project_settings", Kind::Document),
    ("get_reach", Kind::Document),
    ("get_reference_snapshot", Kind::Document),
    ("get_saved_report", Kind::Document),
    ("get_sheet", Kind::Document),
    ("get_sheet_elevations", Kind::Document),
    ("get_stack_candidates", Kind::Document),
    ("get_tag_rule", Kind::Document),
    ("get_task", Kind::Document),
    ("list_connections", Kind::Document),
    ("list_project_settings", Kind::Document),
    ("list_reference_snapshots", Kind::Document),
    ("list_report_columns", Kind::Document),
    ("resolve_blueprint", Kind::Document),
    ("resolve_project_settings", Kind::Document),
    ("suggest_sheets", Kind::Document),
    ("upload_reference", Kind::Document),
];

fn names_of(kind: Kind) -> String {
    TOOL_KINDS
        .iter()
        .filter(|(_, k)| *k == kind)
        .map(|(n, _)| *n)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The part of the server instructions that tells a client how answers are bounded, generated from
/// [`TOOL_KINDS`] so it cannot drift from what the tools do.
pub fn bounding_note() -> String {
    format!(
        "Answers are bounded (1 MB at most). To LIST or COUNT things use build_report. \
         Paged with `limit`/`offset`, each answer stating `total`, `returned`, `truncated` and `next_offset`: \
         {detail}, {lists}. Of those, {detail} take `detail`: `summary` (the default) is identity, level and room \
         references and says what it `omitted`; `detail=full` is the whole record. {cut} cuts its long lists \
         to `list_limit` and names each cut in `cut_lists`. Every other tool returns one document and refuses \
         an answer over the limit, saying how to ask for less.",
        detail = names_of(Kind::PagedDetail),
        lists = names_of(Kind::Paged),
        cut = names_of(Kind::Cut),
    )
}

/// What a list answer says about itself, flattened into the top level of the answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PageInfo {
    /// How many matched before any paging.
    pub total: usize,
    pub returned: usize,
    pub offset: usize,
    /// More matched than were returned from `offset`.
    pub truncated: bool,
    /// Pass this as `offset` for the next page; absent on the last.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
}

impl PageInfo {
    /// For a list that was paged elsewhere (a report cuts its rows inside the service): `total` matched,
    /// `returned` of them came back starting at `offset`.
    pub fn of(total: usize, offset: usize, returned: usize) -> Self {
        let start = offset.min(total);
        let end = start + returned;
        Self {
            total,
            returned,
            offset: start,
            truncated: end < total,
            next_offset: (end < total).then_some(end),
        }
    }
}

/// Cut `items` to the page and describe what was cut.
pub fn paginate<T>(items: Vec<T>, page: Page) -> (Vec<T>, PageInfo) {
    let total = items.len();
    let start = page.offset.min(total);
    let kept: Vec<T> = items.into_iter().skip(start).take(page.limit).collect();
    let end = start + kept.len();
    let info = PageInfo {
        total,
        returned: kept.len(),
        offset: start,
        truncated: end < total,
        next_offset: (end < total).then_some(end),
    };
    (kept, info)
}

/// Which form of a record an answer holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    /// Identity and the few fields a list is read by.
    Summary,
    /// The whole record, geometry and properties included.
    Full,
}

impl Detail {
    pub fn parse(raw: Option<&str>) -> Result<Self, String> {
        match raw.map(str::trim).filter(|s| !s.is_empty()) {
            None | Some("summary") => Ok(Self::Summary),
            Some("full") => Ok(Self::Full),
            Some(other) => Err(format!("unknown detail \"{other}\"; use \"summary\" (the default) or \"full\"")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Full => "full",
        }
    }
}

/// Put the paging and detail description at the top level of an answer object.
///
/// `omitted` names what a summary leaves out, so a reader does not take its absence for "none";
/// `more` says how to get it. Both appear only on a summary.
pub fn describe(answer: &mut Map<String, Value>, detail: Detail, info: &PageInfo, omitted: &[&str], more: &str) {
    answer.insert("detail".into(), json!(detail.as_str()));
    describe_page(answer, info);
    if detail == Detail::Summary {
        answer.insert("omitted".into(), json!(omitted));
        answer.insert("more".into(), json!(more));
    }
}

/// Just the paging half of [`describe`], for a list that has no summary form: `total`, `returned`,
/// `offset`, `truncated`, `next_offset`, and a sentence saying where the next page starts.
pub fn describe_page(answer: &mut Map<String, Value>, info: &PageInfo) {
    if let Value::Object(page) = serde_json::to_value(info).unwrap_or(Value::Null) {
        answer.extend(page);
    }
    if info.truncated {
        answer.insert(
            "paging".into(),
            json!(format!(
                "{} of {} returned; pass offset={} for the next page",
                info.returned,
                info.total,
                info.next_offset.unwrap_or(info.total)
            )),
        );
    }
}

/// Page the list in `value` and put the paging description beside it.
///
/// A bare array becomes `{key: [...], total, ...}`; an object keeps its other fields and has its `key`
/// list paged in place. **Always an object, even when nothing is cut**, so a client never has to
/// handle two shapes depending on how much data there happens to be. An object with no such list is
/// passed on untouched.
pub fn page_list(value: Value, key: &str, page: Page) -> Value {
    let (mut answer, items) = match value {
        Value::Array(items) => (Map::new(), items),
        Value::Object(mut map) => match map.remove(key) {
            Some(Value::Array(items)) => (map, items),
            Some(other) => {
                map.insert(key.to_string(), other);
                return Value::Object(map);
            }
            None => return Value::Object(map),
        },
        other => return other,
    };
    let (kept, info) = paginate(items, page);
    answer.insert(key.to_string(), Value::Array(kept));
    describe_page(&mut answer, &info);
    Value::Object(answer)
}

/// One list that [`cap_lists`] cut short.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CutList {
    /// Where it sits, as keys and indexes joined by `.` (for example `items.ffe.without_room`).
    pub path: String,
    pub total: usize,
    pub returned: usize,
}

/// Cut every list in a nested answer to `cap` entries and say which were cut and from how many.
///
/// For a report that is mostly counts but carries, deep inside it, a list of every offender: a
/// project with thirty thousand unattributed items would otherwise answer a question about
/// totals with thirty thousand lines. The first `cap` entries are kept in their order, so the
/// answer stays a prefix of the whole; the totals are in `cut`, never silently lost.
pub fn cap_lists(value: &mut Value, cap: usize) -> Vec<CutList> {
    let mut cut = Vec::new();
    walk(value, cap, &mut String::new(), &mut cut);
    cut
}

fn walk(value: &mut Value, cap: usize, path: &mut String, cut: &mut Vec<CutList>) {
    let mark = path.len();
    match value {
        Value::Array(items) => {
            if items.len() > cap {
                cut.push(CutList { path: path.clone(), total: items.len(), returned: cap });
                items.truncate(cap);
            }
            for (i, item) in items.iter_mut().enumerate() {
                path.push_str(&format!("[{i}]"));
                walk(item, cap, path, cut);
                path.truncate(mark);
            }
        }
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(key);
                walk(item, cap, path, cut);
                path.truncate(mark);
            }
        }
        _ => {}
    }
}

/// Serialise `value`, refusing above [`ANSWER_MAX_BYTES`] with `advice` on how to ask for less.
/// An empty `tool` leaves the name out, for the caller that does not know it.
pub fn bounded<T: Serialize>(value: &T, tool: &str, advice: &str) -> Result<String, String> {
    let text = serde_json::to_string(value).map_err(|e| format!("failed to serialize response: {e}"))?;
    if text.len() > ANSWER_MAX_BYTES {
        let who = if tool.is_empty() { String::new() } else { format!("{tool}: ") };
        return Err(format!(
            "{who}the answer is {:.1} MB, over the {} MB limit. {advice}",
            text.len() as f64 / (1024.0 * 1024.0),
            ANSWER_MAX_BYTES / (1024 * 1024)
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_page_is_cut_and_says_where_the_next_starts() {
        let (kept, info) = paginate((0..10).collect::<Vec<_>>(), Page { limit: 4, offset: 0 });
        assert_eq!(kept, vec![0, 1, 2, 3]);
        assert_eq!((info.total, info.returned, info.truncated, info.next_offset), (10, 4, true, Some(4)));
        let (kept, info) = paginate((0..10).collect::<Vec<_>>(), Page { limit: 4, offset: 8 });
        assert_eq!(kept, vec![8, 9]);
        assert_eq!((info.truncated, info.next_offset), (false, None), "the last page names no next");
    }

    #[test]
    fn test_an_offset_past_the_end_is_an_empty_page_not_an_error() {
        let (kept, info) = paginate(vec![1, 2, 3], Page { limit: 5, offset: 99 });
        assert!(kept.is_empty());
        assert_eq!((info.total, info.returned, info.offset, info.truncated), (3, 0, 3, false));
    }

    #[test]
    fn test_limit_defaults_when_absent_or_zero_and_is_lowered_to_the_ceiling() {
        assert_eq!(Page::new(None, None, 100, 500).limit, 100);
        assert_eq!(Page::new(Some(0), None, 100, 500).limit, 100);
        assert_eq!(Page::new(Some(9999), Some(7), 100, 500), Page { limit: 500, offset: 7 });
    }

    #[test]
    fn test_detail_defaults_to_summary_and_rejects_a_stranger() {
        assert_eq!(Detail::parse(None), Ok(Detail::Summary));
        assert_eq!(Detail::parse(Some(" full ")), Ok(Detail::Full));
        assert!(Detail::parse(Some("everything")).unwrap_err().contains("summary"));
    }

    #[test]
    fn test_a_summary_says_what_it_left_out_and_a_full_answer_does_not() {
        let info = PageInfo { total: 5, returned: 2, offset: 0, truncated: true, next_offset: Some(2) };
        let mut a = Map::new();
        describe(&mut a, Detail::Summary, &info, &["loops"], "pass detail=full");
        assert_eq!(a["detail"], "summary");
        assert_eq!(a["omitted"], json!(["loops"]));
        assert_eq!(a["next_offset"], 2);
        assert!(a["paging"].as_str().unwrap().contains("offset=2"));
        let mut b = Map::new();
        describe(&mut b, Detail::Full, &info, &["loops"], "x");
        assert!(!b.contains_key("omitted") && !b.contains_key("more"));
    }

    #[test]
    fn test_an_oversized_answer_is_refused_with_the_advice() {
        let big = vec!["x".repeat(1024); 2048];
        let err = bounded(&big, "get_rooms", "Narrow it.").unwrap_err();
        assert!(err.starts_with("get_rooms:") && err.ends_with("Narrow it."), "{err}");
        assert!(bounded(&vec![1, 2, 3], "t", "a").is_ok());
    }

    #[test]
    fn test_cap_lists_keeps_a_prefix_and_names_what_it_cut() {
        let mut v = json!({"total": 5, "items": {"ffe": {"without_room": [1, 2, 3, 4, 5], "short": [1]}}});
        let cut = cap_lists(&mut v, 2);
        assert_eq!(v["items"]["ffe"]["without_room"], json!([1, 2]));
        assert_eq!(v["items"]["ffe"]["short"], json!([1]));
        assert_eq!(cut, vec![CutList { path: "items.ffe.without_room".into(), total: 5, returned: 2 }]);
        assert!(cap_lists(&mut v, 2).is_empty(), "a second pass has nothing left to cut");
    }

    #[test]
    fn test_page_list_wraps_an_array_and_pages_an_object_in_place() {
        let a = page_list(json!([1, 2, 3]), "things", Page { limit: 2, offset: 0 });
        assert_eq!(a["things"], json!([1, 2]));
        assert_eq!((a["total"].as_u64(), a["next_offset"].as_u64()), (Some(3), Some(2)));
        let b = page_list(
            json!({"tier_configured": true, "buildings": [1, 2, 3]}),
            "buildings",
            Page { limit: 5, offset: 1 },
        );
        assert_eq!(b["buildings"], json!([2, 3]));
        assert_eq!(b["tier_configured"], true);
        assert_eq!(b["truncated"], false);
        let c = page_list(json!({"other": 1}), "buildings", Page { limit: 5, offset: 0 });
        assert_eq!(c, json!({"other": 1}), "an object without the list is left alone");
        assert!(page_list(json!([]), "things", Page { limit: 5, offset: 0 })["things"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_page_info_of_matches_what_paginate_says() {
        let (_, via_paginate) = paginate((0..10).collect::<Vec<_>>(), Page { limit: 3, offset: 4 });
        assert_eq!(PageInfo::of(10, 4, 3), via_paginate);
        assert_eq!(PageInfo::of(10, 99, 0).offset, 10, "an offset past the end is clamped");
    }

    #[test]
    fn test_sizes_pick_the_form_and_lower_to_the_ceiling() {
        assert_eq!(ELEMENT_SIZES.page(Detail::Summary, None, None).limit, 500);
        assert_eq!(ELEMENT_SIZES.page(Detail::Full, None, None).limit, 50);
        assert_eq!(ELEMENT_SIZES.page(Detail::Full, Some(10_000), Some(3)), Page { limit: 200, offset: 3 });
    }
}
