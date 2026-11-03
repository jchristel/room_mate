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
    if detail == Detail::Summary {
        answer.insert("omitted".into(), json!(omitted));
        answer.insert("more".into(), json!(more));
    }
}

/// Serialise `value`, refusing above [`ANSWER_MAX_BYTES`] with `advice` on how to ask for less.
pub fn bounded<T: Serialize>(value: &T, tool: &str, advice: &str) -> Result<String, String> {
    let text = serde_json::to_string(value).map_err(|e| format!("failed to serialize response: {e}"))?;
    if text.len() > ANSWER_MAX_BYTES {
        return Err(format!(
            "{tool}: the answer is {:.1} MB, over the {} MB limit. {advice}",
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
}
