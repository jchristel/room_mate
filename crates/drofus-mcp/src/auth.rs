//! How a request is authenticated, in one place.
//!
//! **The scheme is `Authorization: Reference {key}`**, the one the duHast PushIt client uses against this
//! API and the third scheme the API's own `401` advertises (see `AUTH.md`). The key is an API key the
//! user generates in dRofus's Power Query credentials dialog; it is per user and project-scoped, so what
//! it may read is what that user may read. It is supplied as `DROFUS_API_KEY` (or `--api-key`) and kept as
//! a [`Secret`], which never prints.
//!
//! **Not verified against the live service from this crate**: no credentialed call has been made here.
//! The behaviour is taken from PushIt, which does make them.
//!
//! **To add another scheme** (an OAuth bearer token, say): implement [`Authenticator`] and return it from
//! [`for_config`]. Nothing else knows how a request is authenticated: the client calls the trait in
//! exactly one place, just before sending.
//!
//! **Never put a credential in the error you return.** The client forwards [`AuthError`] to the model
//! verbatim.

use std::{fmt, future::Future, pin::Pin, sync::Arc};

use reqwest::header::{self, HeaderValue};

use crate::config::{Config, Secret};

/// Why a request could not be authenticated. The text is shown to the caller, so it names what is
/// missing and never a credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthError(pub String);

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub type AuthFuture<'a> = Pin<Box<dyn Future<Output = Result<reqwest::RequestBuilder, AuthError>> + Send + 'a>>;

/// Adds what authenticates a request. Asynchronous because the likely schemes (a token that has to be
/// fetched or refreshed) need the network.
pub trait Authenticator: Send + Sync {
    /// Given the request being built, return it with whatever authenticates it, or an error that stops
    /// the call **before any network traffic**.
    fn authenticate(&self, request: reqwest::RequestBuilder) -> AuthFuture<'_>;
}

/// Used when no API key is configured: every call fails before any network traffic, so a tool works end
/// to end up to this error and says what to set.
#[derive(Clone, Copy, Debug, Default)]
pub struct Unauthenticated;

impl Authenticator for Unauthenticated {
    fn authenticate(&self, _request: reqwest::RequestBuilder) -> AuthFuture<'_> {
        Box::pin(async {
            Err(AuthError(
                "no API key is configured, so no dRofus request was sent. Set DROFUS_API_KEY (or --api-key) to \
                 an API key from dRofus's Power Query credentials dialog; AUTH.md has the details."
                    .to_string(),
            ))
        })
    }
}

/// `Authorization: Reference {key}`. The header is marked sensitive, so neither `reqwest` nor `hyper`
/// prints its value if a request is ever logged or debug-formatted.
#[derive(Clone, Debug)]
pub struct ReferenceKey {
    key: Secret,
}

impl ReferenceKey {
    pub fn new(key: Secret) -> Self {
        Self { key }
    }
}

impl Authenticator for ReferenceKey {
    fn authenticate(&self, request: reqwest::RequestBuilder) -> AuthFuture<'_> {
        Box::pin(async move {
            let mut value = HeaderValue::from_str(&format!("Reference {}", self.key.expose())).map_err(|_| {
                // The value is not echoed: it is the secret.
                AuthError(
                    "the API key holds characters that cannot be sent in a header (a stray line break from \
                     copy and paste?); set it again"
                        .to_string(),
                )
            })?;
            value.set_sensitive(true);
            Ok(request.header(header::AUTHORIZATION, value))
        })
    }
}

/// The authenticator a configuration calls for: the API key when there is one, else the stand-in that
/// refuses and says what to set. The one place the choice is made.
pub fn for_config(config: &Config) -> Arc<dyn Authenticator> {
    match &config.api_key {
        Some(key) => Arc::new(ReferenceKey::new(key.clone())),
        None => Arc::new(Unauthenticated),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> reqwest::RequestBuilder {
        reqwest::Client::new().get("http://127.0.0.1:1/")
    }

    #[tokio::test]
    async fn without_a_key_every_call_is_refused_and_says_what_to_set() {
        let err = Unauthenticated.authenticate(req()).await.unwrap_err();
        assert!(err.0.contains("DROFUS_API_KEY") && err.0.contains("no dRofus request was sent"), "{err}");
    }

    #[tokio::test]
    async fn the_key_goes_out_as_a_sensitive_reference_header() {
        let auth = ReferenceKey::new(Secret::new("k-123"));
        let built = auth.authenticate(req()).await.unwrap().build().unwrap();
        let v = &built.headers()[header::AUTHORIZATION];
        assert_eq!(v, "Reference k-123");
        assert!(v.is_sensitive(), "a sensitive header is not printed by Debug");
        assert!(!format!("{built:?}").contains("k-123"), "the request's Debug hides the key");
    }

    #[tokio::test]
    async fn a_key_that_cannot_be_a_header_is_refused_without_echoing_it() {
        let auth = ReferenceKey::new(Secret::new("bad\nkey"));
        let err = auth.authenticate(req()).await.unwrap_err();
        assert!(!err.0.contains("bad") && err.0.contains("header"), "{err}");
    }

    #[test]
    fn the_configuration_picks_the_authenticator_and_the_key_never_prints() {
        let with = Config::resolve(None, Some("k-123".into()), None, None, |_| None);
        assert!(!format!("{:?}", ReferenceKey::new(with.api_key.clone().unwrap())).contains("k-123"));
        // `for_config` returns a trait object; the difference is observable only by calling it.
        let _ = for_config(&with);
        let _ = for_config(&Config::default());
    }
}
