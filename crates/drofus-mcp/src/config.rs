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
    /// The dRofus API's base address, for example `https://api-no.drofus.com`. Which regional host a
    /// project lives on is the operator's to say, so there is no default.
    pub base_url: Option<String>,
    /// The database every path starts with (`/api/{db}/{pr}/...`); a tool's `db` argument overrides it.
    pub database: Option<String>,
    /// The project, likewise overridden by a tool's `pr` argument.
    pub project: Option<String>,
    /// The dRofus API key, sent as `Authorization: Reference {key}` (`auth::ReferenceKey`). A [`Secret`], so
    /// it can never reach a log line.
    pub api_key: Option<Secret>,
}

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
        Self {
            base_url: clean(base_url)
                .or_else(|| clean(env("DROFUS_BASE_URL")))
                .map(|u| u.trim_end_matches('/').to_string()),
            database: clean(database).or_else(|| clean(env("DROFUS_DATABASE"))),
            project: clean(project).or_else(|| clean(env("DROFUS_PROJECT"))),
            api_key: clean(api_key).or_else(|| clean(env("DROFUS_API_KEY"))).map(Secret::new),
        }
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
            out.push("api_key (--api-key or DROFUS_API_KEY)");
        }
        out
    }
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
}
