//! The seam where authentication will go, and nothing behind it yet.
//!
//! **Not implemented, on purpose.** How a caller signs in to dRofus (the Swagger page's Login goes to an
//! interactive user name and password, or Microsoft single sign-on) is a decision for the owner, and the
//! API definition declares no security scheme, so there is nothing to implement from. `AUTH.md` lists
//! what is known. This file therefore holds the shape of the answer and a stand-in that refuses.
//!
//! **To implement it:** write a type that implements [`Authenticator`] (adding whatever header, token
//! or cookie the scheme asks for to the request it is handed), keep its credentials as
//! `config::Secret`s, and pass it to `Client::new` in `server::DrofusMcp::new`. Nothing else knows how
//! a request is authenticated: the client calls this in exactly one place, just before sending.
//!
//! **Never put a credential in the error you return.** The client forwards [`AuthError`] to the model
//! verbatim.

use std::{fmt, future::Future, pin::Pin};

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

/// The only implementation: every call fails, so a tool works end to end up to this error and no
/// further. Replace it when the scheme is decided (see the module docs).
#[derive(Clone, Copy, Debug, Default)]
pub struct Unauthenticated;

impl Authenticator for Unauthenticated {
    fn authenticate(&self, _request: reqwest::RequestBuilder) -> AuthFuture<'_> {
        Box::pin(async {
            Err(AuthError(
                "authentication is not implemented yet, so no dRofus request was sent. The seam is \
                 `Authenticator` in src/auth.rs; AUTH.md lists what is known about the sign-in and what is \
                 undecided."
                    .to_string(),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unauthenticated_refuses_and_says_where_to_look() {
        let req = reqwest::Client::new().get("http://127.0.0.1:1/");
        let err = Unauthenticated.authenticate(req).await.unwrap_err();
        assert!(err.0.contains("not implemented") && err.0.contains("src/auth.rs"), "{err}");
    }
}
