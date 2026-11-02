//! What the dRofus read API offers, and how a tool's arguments become a request.
//!
//! **One table, four shapes.** The 89 operations differ only in their path, the path parameters they
//! take and which query options they accept, so they are rows of [`ENDPOINTS`] (generated from the
//! OpenAPI definition by `gen_endpoints.py`, never edited by hand) and a [`Kind`] says which generic
//! function in `client` runs them: a list, a get, a binary image or a metadata read. A tool is a row;
//! nothing here is written ninety times.
//!
//! **Query options are passed through, never interpreted.** `filter` is an OData expression and the
//! server owns its grammar: parsing or rewriting it here would mean a second, subtly different parser.
//! The one thing done to a value is encoding it for the URL.
//!
//! **Tool parameters drop the `$`** (`filter`, not `$filter`): MCP clients validate property names
//! against `^[a-zA-Z0-9_.-]+$`, and a name with `$` is rejected by some of them. The `$` is put back
//! here, when the request is built.
//!
//! Read-only by construction: [`Kind::method`] can only say GET or OPTIONS.

use reqwest::{Method, Url};
use serde_json::{json, Map, Value};

mod table;
pub use table::ENDPOINTS;

/// A list answer is capped here unless the caller asks for fewer or more (up to [`MAX_TOP`]).
/// A model's context is the scarce resource: the API's own default is 10,000 rows.
pub const DEFAULT_TOP: u32 = 50;
/// The most rows one call may ask for; a larger `top` is lowered to this and the answer says so.
pub const MAX_TOP: u32 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An array answer with `$select`, `$orderby`, `$filter`, `$top`, `$skip`.
    List,
    /// One record (or a small fixed answer), optionally with `$select`.
    Get,
    /// Image bytes. Returned as an image content block under a size cap, never as text.
    Binary,
    /// The `OPTIONS` read: which properties can be selected.
    Meta,
}

impl Kind {
    pub fn method(self) -> Method {
        match self {
            Kind::Meta => Method::OPTIONS,
            _ => Method::GET,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtraTy {
    Int,
    Bool,
}

/// A query option beyond the standard five (`includeSubs`, `depth`), passed by its own name.
#[derive(Clone, Copy, Debug)]
pub struct Extra {
    pub name: &'static str,
    pub description: &'static str,
    pub ty: ExtraTy,
}

#[derive(Clone, Copy, Debug)]
pub struct Endpoint {
    pub name: &'static str,
    pub kind: Kind,
    /// Under `/api/{db}/{pr}`, with `{param}` placeholders.
    pub path: &'static str,
    /// `(name, description)` of each placeholder; all are integers.
    pub path_params: &'static [(&'static str, &'static str)],
    pub select: bool,
    pub extras: &'static [Extra],
    pub summary: &'static str,
    /// The definition's own longer description (extra filter fields and so on), when it has one.
    pub notes: &'static str,
}

/// A refused or failed call, in words for the caller. Never holds a credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolError(pub String);

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<crate::auth::AuthError> for ToolError {
    fn from(e: crate::auth::AuthError) -> Self {
        ToolError(e.0)
    }
}

const ORDERBY_DOC: &str = "How to sort the result: comma-separated fields, each optionally followed by asc or desc, \
    e.g. `id desc,name asc`.";
const FILTER_DOC: &str = "OData filter, passed to dRofus unchanged. Literals are 'strings' in single quotes (a quote \
    inside is doubled: 'O''Brian'), numbers with . as decimal separator, true/false, null. Operators: eq ne lt gt le ge in; \
    combine with and/or and parentheses. Functions: contains, notcontains, startswith, endswith. `in` takes a parenthesised \
    list. Examples: name eq 'Storage'; Address in ('Redmond','London'); contains(CompanyName,'freds'); startswith(Name,'dR').";
const SELECT_DOC: &str =
    "Comma-separated list of fields to return (a JSON array of names is also accepted). Omit for all \
    fields; naming only what you need keeps the answer small.";

impl Endpoint {
    pub fn is_list(&self) -> bool {
        self.kind == Kind::List
    }

    /// What a model chooses the tool by: what it returns, what it needs, what it leaves out.
    pub fn description(&self) -> String {
        let http = if self.kind == Kind::Meta { "OPTIONS" } else { "GET" };
        let mut d = format!("{} (dRofus {http} /api/{{db}}/{{pr}}{}).", self.summary.trim_end_matches('.'), self.path);
        if !self.notes.is_empty() {
            d.push_str("\n\n");
            d.push_str(self.notes);
        }
        d.push_str("\n\nRead-only. ");
        match self.kind {
            Kind::List => d.push_str(&format!(
                "Returns {{items, returned, top, skip, more_may_exist, next_skip, note}}: at most `top` rows (default \
                 {DEFAULT_TOP}, ceiling {MAX_TOP}); when more_may_exist is true, call again with skip=next_skip. Use \
                 `select` and `filter` to narrow before paging."
            )),
            Kind::Get => d.push_str("Returns the record as dRofus gives it."),
            Kind::Binary => d.push_str(&format!(
                "Returns the image as an image content block, refused above {} bytes with a message that says the \
                 size; use the matching `_meta` tool for its description instead.",
                crate::client::IMAGE_MAX_BYTES
            )),
            Kind::Meta => d.push_str(
                "Returns which properties can be selected for the signed-in user; `depth` includes related ones.",
            ),
        }
        d.push_str(
            " Needs an API key (DROFUS_API_KEY): without one every call stops with an error that says so, before any \
             request is sent. `db` and `pr` override the configured database and project.",
        );
        d
    }

    /// The JSON Schema of the tool's arguments.
    pub fn input_schema(&self) -> Map<String, Value> {
        let mut props = Map::new();
        props.insert(
            "db".into(),
            json!({"type": "string", "description": "Database; overrides the configured one."}),
        );
        props.insert(
            "pr".into(),
            json!({"type": "string", "description": "Project; overrides the configured one."}),
        );
        let mut required = Vec::new();
        for (name, desc) in self.path_params {
            props.insert((*name).into(), json!({"type": "integer", "minimum": 1, "description": desc}));
            required.push(Value::from(*name));
        }
        if self.select {
            props.insert("select".into(), json!({"type": "string", "description": SELECT_DOC}));
        }
        if self.is_list() {
            props.insert("orderby".into(), json!({"type": "string", "description": ORDERBY_DOC}));
            props.insert("filter".into(), json!({"type": "string", "description": FILTER_DOC}));
            props.insert(
                "top".into(),
                json!({"type": "integer", "minimum": 1, "maximum": MAX_TOP,
                       "description": format!("Rows to return. Default {DEFAULT_TOP}, ceiling {MAX_TOP}.")}),
            );
            props.insert(
                "skip".into(),
                json!({"type": "integer", "minimum": 0, "description": "Rows to skip, for paging."}),
            );
        }
        for e in self.extras {
            let ty = if e.ty == ExtraTy::Int { "integer" } else { "boolean" };
            let desc = if e.description.is_empty() {
                "Passed to dRofus unchanged."
            } else {
                e.description
            };
            props.insert(e.name.into(), json!({"type": ty, "description": desc}));
        }
        let mut schema = Map::new();
        schema.insert("type".into(), "object".into());
        schema.insert("properties".into(), Value::Object(props));
        if !required.is_empty() {
            schema.insert("required".into(), Value::Array(required));
        }
        schema.insert("additionalProperties".into(), Value::Bool(false));
        schema
    }
}

/// How a list was bounded, so the answer can say it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paging {
    pub top: u32,
    pub skip: u32,
    /// The caller asked for more than [`MAX_TOP`] and was given [`MAX_TOP`].
    pub capped: bool,
    /// The caller gave no `top` and got [`DEFAULT_TOP`].
    pub defaulted: bool,
}

/// A request checked and ready to be built: everything but the address.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub db: String,
    pub pr: String,
    /// The path with its placeholders filled.
    pub path: String,
    pub query: Vec<(String, String)>,
    pub paging: Option<Paging>,
}

fn text_arg(args: &Map<String, Value>, key: &str) -> Result<Option<String>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(ToolError(format!("`{key}` must be a string"))),
    }
}

fn int_arg(args: &Map<String, Value>, key: &str, min: i64) -> Result<Option<i64>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => match n.as_i64() {
            Some(i) if i >= min => Ok(Some(i)),
            _ => Err(ToolError(format!("`{key}` must be a whole number, at least {min}"))),
        },
        Some(_) => Err(ToolError(format!("`{key}` must be a whole number"))),
    }
}

impl Endpoint {
    /// Check `args` against this endpoint and turn them into a request. `db` and `pr` are the
    /// configured defaults; an argument overrides them.
    pub fn prepare(
        &self,
        args: &Map<String, Value>,
        default_db: Option<&str>,
        default_pr: Option<&str>,
    ) -> Result<Prepared, ToolError> {
        let mut known: Vec<&str> = vec!["db", "pr"];
        known.extend(self.path_params.iter().map(|(n, _)| *n));
        if self.select {
            known.push("select");
        }
        if self.is_list() {
            known.extend(["orderby", "filter", "top", "skip"]);
        }
        known.extend(self.extras.iter().map(|e| e.name));
        if let Some(bad) = args.keys().find(|k| !known.contains(&k.as_str())) {
            return Err(ToolError(format!(
                "unknown argument `{bad}` for {}; it takes: {} (query options are named without the `$`)",
                self.name,
                known.join(", ")
            )));
        }

        let db = text_arg(args, "db")?.or_else(|| default_db.map(str::to_string)).ok_or_else(|| {
            ToolError("no database: pass `db` or start the server with --database / DROFUS_DATABASE".into())
        })?;
        let pr = text_arg(args, "pr")?.or_else(|| default_pr.map(str::to_string)).ok_or_else(|| {
            ToolError("no project: pass `pr` or start the server with --project / DROFUS_PROJECT".into())
        })?;

        let mut path = self.path.to_string();
        for (name, _) in self.path_params {
            let id = int_arg(args, name, 1)?.ok_or_else(|| ToolError(format!("`{name}` is required")))?;
            path = path.replace(&format!("{{{name}}}"), &id.to_string());
        }

        let mut query = Vec::new();
        if self.select {
            match args.get("select") {
                Some(Value::Array(items)) => {
                    let names: Option<Vec<&str>> = items.iter().map(Value::as_str).collect();
                    let names =
                        names.ok_or_else(|| ToolError("`select` must be a string or an array of strings".into()))?;
                    if !names.is_empty() {
                        query.push(("$select".to_string(), names.join(",")));
                    }
                }
                _ => {
                    if let Some(s) = text_arg(args, "select")? {
                        query.push(("$select".to_string(), s));
                    }
                }
            }
        }
        let mut paging = None;
        if self.is_list() {
            for key in ["orderby", "filter"] {
                if let Some(v) = text_arg(args, key)? {
                    query.push((format!("${key}"), v));
                }
            }
            let asked = int_arg(args, "top", 1)?;
            let skip = int_arg(args, "skip", 0)?.unwrap_or(0).min(i64::from(u32::MAX)) as u32;
            let top = asked.map_or(DEFAULT_TOP, |t| t.min(i64::from(MAX_TOP)) as u32);
            query.push(("$top".to_string(), top.to_string()));
            if skip > 0 {
                query.push(("$skip".to_string(), skip.to_string()));
            }
            paging = Some(Paging {
                top,
                skip,
                capped: asked.is_some_and(|t| t > i64::from(MAX_TOP)),
                defaulted: asked.is_none(),
            });
        }
        for e in self.extras {
            let value = match e.ty {
                ExtraTy::Int => int_arg(args, e.name, 0)?.map(|i| i.to_string()),
                ExtraTy::Bool => match args.get(e.name) {
                    None | Some(Value::Null) => None,
                    Some(Value::Bool(b)) => Some(b.to_string()),
                    Some(_) => return Err(ToolError(format!("`{}` must be true or false", e.name))),
                },
            };
            if let Some(v) = value {
                query.push((e.name.to_string(), v));
            }
        }
        Ok(Prepared { db, pr, path, query, paging })
    }
}

impl Prepared {
    /// The full address. Segments are encoded as segments, so a database or project name holding a `/`
    /// or a space cannot change which endpoint is called.
    pub fn url(&self, base: &str) -> Result<Url, ToolError> {
        let mut url =
            Url::parse(base).map_err(|e| ToolError(format!("the configured base address is not a URL: {e}")))?;
        {
            let mut segs = url
                .path_segments_mut()
                .map_err(|()| ToolError("the configured base address cannot carry a path".into()))?;
            segs.pop_if_empty().extend(["api", self.db.as_str(), self.pr.as_str()]);
            segs.extend(self.path.trim_start_matches('/').split('/'));
        }
        if !self.query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(self.query.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        }
        Ok(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(name: &str) -> &'static Endpoint {
        ENDPOINTS.iter().find(|e| e.name == name).unwrap_or_else(|| panic!("no tool {name}"))
    }

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn query_of(p: &Prepared) -> Vec<(String, String)> {
        p.url("https://x.example")
            .unwrap()
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    }

    #[test]
    fn the_table_is_read_only_and_names_are_unique() {
        let mut names: Vec<_> = ENDPOINTS.iter().map(|e| e.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ENDPOINTS.len());
        assert!(ENDPOINTS.iter().all(|e| matches!(e.kind.method(), Method::GET | Method::OPTIONS)));
        assert!(ENDPOINTS.iter().all(|e| e.name.starts_with("drofus_") && e.name.len() <= 64));
    }

    #[test]
    fn a_list_defaults_to_a_bounded_page_and_passes_the_filter_through_unchanged() {
        let p = ep("drofus_rooms_list")
            .prepare(&args(json!({"filter": "name eq 'O''Brian' and (a gt 1)", "orderby": "id desc", "select": ["id", "name"]})), Some("db1"), Some("p1"))
            .unwrap();
        assert_eq!(p.paging, Some(Paging { top: DEFAULT_TOP, skip: 0, capped: false, defaulted: true }));
        let q = query_of(&p);
        assert!(q.contains(&("$filter".into(), "name eq 'O''Brian' and (a gt 1)".into())), "{q:?}");
        assert!(
            q.contains(&("$select".into(), "id,name".into())) && q.contains(&("$orderby".into(), "id desc".into()))
        );
        assert!(q.contains(&("$top".into(), "50".into())));
        assert!(!q.iter().any(|(k, _)| k == "$skip"), "a zero skip is not sent");
    }

    #[test]
    fn top_is_lowered_to_the_ceiling_and_says_so() {
        let p = ep("drofus_rooms_list")
            .prepare(&args(json!({"top": 10_000, "skip": 400})), Some("d"), Some("p"))
            .unwrap();
        assert_eq!(p.paging, Some(Paging { top: MAX_TOP, skip: 400, capped: true, defaulted: false }));
        assert!(query_of(&p).contains(&("$top".into(), "200".into())));
    }

    #[test]
    fn the_url_fills_placeholders_and_encodes_the_database_as_a_segment() {
        let p = ep("drofus_rooms_get")
            .prepare(&args(json!({"id": 7, "db": "a/b c", "pr": "p"})), None, None)
            .unwrap();
        assert_eq!(
            p.url("https://api.example/").unwrap().as_str(),
            "https://api.example/api/a%2Fb%20c/p/rooms/7"
        );
        let m = ep("drofus_rooms_properties")
            .prepare(&args(json!({"depth": 1})), Some("d"), Some("p"))
            .unwrap();
        assert_eq!(
            m.url("https://api.example").unwrap().as_str(),
            "https://api.example/api/d/p/rooms?depth=1"
        );
        let n = ep("drofus_occurrences_status")
            .prepare(&args(json!({"id": 3, "statustypeid": 9})), Some("d"), Some("p"))
            .unwrap();
        assert!(n.url("https://h").unwrap().path().ends_with("/occurrences/3/statuses/9"));
    }

    #[test]
    fn a_missing_id_database_or_a_misspelt_option_is_refused_in_words() {
        let e = ep("drofus_rooms_get");
        assert!(e
            .prepare(&args(json!({})), Some("d"), Some("p"))
            .unwrap_err()
            .0
            .contains("`id` is required"));
        assert!(e.prepare(&args(json!({"id": 1})), None, Some("p")).unwrap_err().0.contains("--database"));
        assert!(e.prepare(&args(json!({"id": 1})), Some("d"), None).unwrap_err().0.contains("--project"));
        assert!(e.prepare(&args(json!({"id": 1.5})), Some("d"), Some("p")).is_err());
        let typo = ep("drofus_rooms_list")
            .prepare(&args(json!({"$filter": "x"})), Some("d"), Some("p"))
            .unwrap_err();
        assert!(
            typo.0.contains("unknown argument `$filter`") && typo.0.contains("without the `$`"),
            "{typo}"
        );
        assert!(
            ep("drofus_rooms_get")
                .prepare(&args(json!({"id": 1, "top": 5})), Some("d"), Some("p"))
                .is_err(),
            "get takes no top"
        );
    }

    #[test]
    fn schemas_use_names_a_client_accepts() {
        for e in ENDPOINTS {
            let schema = e.input_schema();
            for key in schema["properties"].as_object().unwrap().keys() {
                assert!(
                    key.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)),
                    "{} has `{key}`",
                    e.name
                );
            }
        }
        let s = ep("drofus_rooms_images").input_schema();
        assert_eq!(s["required"], json!(["id"]));
        assert!(ep("drofus_occurrences_member_of_systems").input_schema()["properties"]
            .get("includeSubs")
            .is_some());
    }
}
