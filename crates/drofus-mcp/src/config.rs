//! How this process is told where dRofus is.
//!
//! **Flags win over the environment, the environment wins over nothing**, and a value is never
//! guessed. Nothing is read from a file in the repository: a credential that lived there would be
//! one `git add` from being published, which is why [`Secret`] exists and why the only way to read one
//! is to ask for it by name.
//!
//! Every credential is a [`Secret`] (see `auth` for how the API key is used).

use std::fmt;

/// A value that must never be printed: its `Debug` and `Display` say only that it is set.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value, for the one place that sends it. Named so a reader sees it is deliberate.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<set>)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<set>")
    }
}

/// What the server was configured with. `None` is "not given", which a tool reports as a
/// finding of its own rather than failing the whole process: a client should be able to connect and
/// ask what is missing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// The dRofus API's base address, for example `https://api-no.drofus.com` or an organisation's own
    /// `https://api.example.org`. Which host a project lives on is the operator's to say, so there is no
    /// default and no assumed suffix.
    pub base_url: Option<String>,
    /// The database every path starts with (`/api/{db}/{pr}/...`); a tool's `db` argument overrides it.
    pub database: Option<String>,
    /// The project, likewise overridden by a tool's `pr` argument.
    pub project: Option<String>,
    /// The dRofus API key, sent as `Authorization: Reference {key}` (`auth::ReferenceKey`). A [`Secret`], so
    /// it can never reach a log line.
    pub api_key: Option<Secret>,
    /// How long one request may take, in seconds; `None` is [`DEFAULT_TIMEOUT_SECS`]. Some endpoints are slow on
    /// the service's side (a one-row read of a table's change log was measured at over thirty seconds), so
    /// this is the operator's to raise.
    pub timeout_secs: Option<u64>,
}

/// The request timeout when none is configured.
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;

// A one-row read of a table's change log took about 34 s on the live service; a default under that makes it fail always.
const _: () = assert!(DEFAULT_TIMEOUT_SECS >= 60);

impl Config {
    /// Build from flag values (each `None` when the flag was absent), falling back to the
    /// environment variables `DROFUS_BASE_URL`, `DROFUS_DATABASE`, `DROFUS_PROJECT` and `DROFUS_API_KEY`
    /// through `env`.
    pub fn resolve(
        base_url: Option<String>,
        api_key: Option<String>,
        database: Option<String>,
        project: Option<String>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Self {
        let clean = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let address = clean(base_url).or_else(|| clean(env("DROFUS_BASE_URL"))).map(|u| split_address(&u));
        let (base_url, from_db, from_pr) = match address {
            Some((base, db, pr)) => (Some(base), db, pr),
            None => (None, None, None),
        };
        Self {
            base_url,
            database: clean(database).or_else(|| clean(env("DROFUS_DATABASE"))).or(from_db),
            project: clean(project).or_else(|| clean(env("DROFUS_PROJECT"))).or(from_pr),
            api_key: clean(api_key).or_else(|| clean(env("DROFUS_API_KEY"))).map(Secret::new),
            timeout_secs: None,
        }
    }

    /// Fill the API key from a file when no flag or environment variable gave one.
    ///
    /// The file exists so the key need not sit in the MCP host's own configuration, which is plain text:
    /// the host then holds only a path. A key given directly always wins, so an operator who sets both is
    /// never surprised by the file. A file that cannot be read leaves the key unset, which `missing` reports,
    /// and the error names the path and never any content.
    pub fn with_key_file(mut self, path: Option<&str>) -> Result<Self, String> {
        let Some(path) = path.map(str::trim).filter(|p| !p.is_empty()) else {
            return Ok(self);
        };
        if self.api_key.is_some() {
            return Ok(self);
        }
        let path = std::path::Path::new(path);
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("the API key file {} could not be read: {}", path.display(), e.kind()))?;
        let key = text.trim();
        if key.is_empty() {
            return Err(format!("the API key file {} is empty", path.display()));
        }
        self.api_key = Some(Secret::new(key));
        Ok(self)
    }

    /// Fill the API key from the OS credential store when nothing else gave one. The lookup is passed in, so the order
    /// (a given key always wins, and the store is not even asked) is testable without touching a real store.
    pub fn with_stored_key(mut self, lookup: impl FnOnce() -> Result<Option<Secret>, String>) -> Result<Self, String> {
        if self.api_key.is_none() {
            self.api_key = lookup()?;
        }
        Ok(self)
    }

    /// The names of what is not set yet, for a tool that says what is missing.
    pub fn missing(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.base_url.is_none() {
            out.push("base_url (--base-url or DROFUS_BASE_URL)");
        }
        if self.database.is_none() {
            out.push("database (--database or DROFUS_DATABASE; or pass `db` to a tool)");
        }
        if self.project.is_none() {
            out.push("project (--project or DROFUS_PROJECT; or pass `pr` to a tool)");
        }
        if self.api_key.is_none() {
            out.push("api_key (DROFUS_API_KEY, DROFUS_API_KEY_FILE naming a file that holds it, or the OS credential store: `drofus-mcp --store-key`)");
        }
        out
    }
}

/// Reduce whatever was given as the address to the part that names the host, plus the database and
/// project when it was a whole request address.
///
/// Which host serves a project is the operator's: dRofus runs on its regional hosts
/// (`api-no.drofus.com`) and also on an organisation's own domain, so nothing here assumes a suffix. A
/// person copying the address out of a working request (`https://host/api/{db}/{pr}/rooms?$select=...`)
/// gets the host, and the database and project from the path; a bare host gets `https://`. An address
/// that is not a URL is passed on as typed, for the request to report.
fn split_address(raw: &str) -> (String, Option<String>, Option<String>) {
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let Ok(mut url) = reqwest::Url::parse(&with_scheme) else {
        return (raw.trim_end_matches('/').to_string(), None, None);
    };
    let segs: Vec<String> = url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).map(String::from).collect())
        .unwrap_or_default();
    let (prefix, db, pr) = match segs.iter().position(|s| s.eq_ignore_ascii_case("api")) {
        Some(i) => (segs[..i].join("/"), segs.get(i + 1).cloned(), segs.get(i + 2).cloned()),
        None => (segs.join("/"), None, None),
    };
    url.set_path(&prefix);
    url.set_query(None);
    url.set_fragment(None);
    (url.as_str().trim_end_matches('/').to_string(), db, pr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_string())
    }

    #[test]
    fn a_flag_beats_the_environment_and_the_environment_fills_what_a_flag_left() {
        let env = env_of(&[("DROFUS_BASE_URL", "https://env"), ("DROFUS_API_KEY", "envkey")]);
        let c = Config::resolve(Some("https://flag/".into()), None, None, None, env);
        assert_eq!(c.base_url.as_deref(), Some("https://flag"), "the trailing slash is dropped");
        assert_eq!(c.api_key.as_ref().map(Secret::expose), Some("envkey"));
    }

    #[test]
    fn database_and_project_come_from_flags_then_environment() {
        let env = env_of(&[("DROFUS_DATABASE", "envdb"), ("DROFUS_PROJECT", "envpr")]);
        let c = Config::resolve(None, None, Some("flagdb".into()), None, env);
        assert_eq!((c.database.as_deref(), c.project.as_deref()), (Some("flagdb"), Some("envpr")));
    }

    #[test]
    fn blank_values_count_as_not_given() {
        let c = Config::resolve(Some("  ".into()), Some(String::new()), None, None, env_of(&[]));
        assert_eq!(c, Config::default());
        assert_eq!(c.missing().len(), 4);
    }

    #[test]
    fn a_secret_never_prints() {
        let c = Config::resolve(None, Some("hunter2".into()), None, None, env_of(&[]));
        let shown = format!("{c:?} {}", c.api_key.clone().unwrap());
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("<set>"));
    }

    #[test]
    fn any_host_is_accepted_and_a_bare_one_gets_https() {
        let c = Config::resolve(Some("api.example.org/".into()), None, None, None, env_of(&[]));
        assert_eq!(c.base_url.as_deref(), Some("https://api.example.org"));
        let c = Config::resolve(Some("http://localhost:8080".into()), None, None, None, env_of(&[]));
        assert_eq!(c.base_url.as_deref(), Some("http://localhost:8080"));
    }

    #[test]
    fn a_pasted_request_address_gives_host_database_and_project() {
        let pasted = "https://api.example.org/api/db-1/01/rooms?$select=id,name&$filter=a+eq+%271%27";
        let c = Config::resolve(Some(pasted.into()), None, None, None, env_of(&[]));
        assert_eq!(c.base_url.as_deref(), Some("https://api.example.org"));
        assert_eq!((c.database.as_deref(), c.project.as_deref()), (Some("db-1"), Some("01")));
        let c = Config::resolve(Some(pasted.into()), None, Some("other".into()), None, env_of(&[]));
        assert_eq!(c.database.as_deref(), Some("other"), "a flag still wins");
    }

    #[test]
    fn a_path_in_front_of_api_is_kept() {
        let c = Config::resolve(Some("https://gw.example.org/drofus/api/d/p".into()), None, None, None, env_of(&[]));
        assert_eq!(c.base_url.as_deref(), Some("https://gw.example.org/drofus"));
    }

    #[test]
    fn a_key_file_fills_a_missing_key_and_a_given_key_wins() {
        let dir = std::env::temp_dir().join(format!("drofus-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("key");
        std::fs::write(&file, "filekey\n").unwrap();
        let path = file.to_str().unwrap();

        let c = Config::resolve(None, None, None, None, env_of(&[])).with_key_file(Some(path)).unwrap();
        assert_eq!(c.api_key.as_ref().map(Secret::expose), Some("filekey"), "trailing newline trimmed");

        let c = Config::resolve(None, Some("flagkey".into()), None, None, env_of(&[]))
            .with_key_file(Some(path))
            .unwrap();
        assert_eq!(c.api_key.as_ref().map(Secret::expose), Some("flagkey"));

        std::fs::write(&file, "  \n").unwrap();
        let err = Config::default().with_key_file(Some(path)).unwrap_err();
        assert!(err.contains("empty") && !err.contains("filekey"), "{err}");
        let err = Config::default().with_key_file(Some(dir.join("nope").to_str().unwrap())).unwrap_err();
        assert!(err.contains("could not be read"), "{err}");
        assert!(Config::default().with_key_file(None).unwrap().api_key.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_credential_store_is_asked_last_and_only_when_nothing_gave_a_key() {
        let c = Config::resolve(None, Some("given".into()), None, None, env_of(&[]))
            .with_stored_key(|| panic!("a given key must not consult the store"))
            .unwrap();
        assert_eq!(c.api_key.as_ref().map(Secret::expose), Some("given"));

        let c = Config::default().with_stored_key(|| Ok(Some(Secret::new("stored")))).unwrap();
        assert_eq!(c.api_key.as_ref().map(Secret::expose), Some("stored"));

        assert!(Config::default().with_stored_key(|| Ok(None)).unwrap().api_key.is_none());
        assert!(Config::default().with_stored_key(|| Err("locked".into())).is_err());
    }
}
