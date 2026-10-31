//! The HTTP face of the MCP server: what stands in front of `/mcp` when `mcp.exe --http` is used.
//!
//! **Why it lives here and not in `bin/mcp.rs`.** That binary's rule is that no server framework
//! leaks into it, and `axum` is one. So the binary hands this module a service factory and gets a
//! router back; the tools themselves are unchanged and know nothing of the transport.
//!
//! **The same stance as the HTTP server, deliberately.** Loopback unless the operator opts in, the
//! `Host` allowance is `network::HostPolicy` (so a DNS-rebinding page cannot reach a tool), and
//! there is no per-user identity. The one addition is an optional shared token, because anyone who
//! can reach this port can read every project's data, and a token is the cheapest thing that makes
//! "reachable" and "allowed" different. It is a shared secret, not an account: it keeps a stray
//! browser tab or a neighbour's script out, and it does not tell two colleagues apart.
//!
//! **Stateless, JSON replies.** Every tool is a request and an answer; nothing is pushed to the
//! client. A session would only add state to lose on a restart, so each POST stands alone.
//!
//! rmcp's own `Host` check is turned off in favour of `HostPolicy`: its list is explicit names,
//! while ours also admits any IP literal on a network bind, which is what a client typing
//! `http://192.168.1.20:5152/mcp` presents.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, Service};

use crate::network::HostPolicy;

/// Where the endpoint is mounted. One path, so the address to give a client is one line.
pub const MCP_PATH: &str = "/mcp";

/// Re-exported so the binary can name the type without importing the framework itself.
pub use axum::Router;

/// Listen on `addr` and serve `app` until the process ends.
pub async fn serve(addr: std::net::SocketAddr, app: Router) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("could not bind {addr}: {e}"))?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// POST `body` to `/mcp` in process, as an HTTP client would, and return the status and body.
/// Public (not `cfg(test)`) only so the binary's tests can use it without importing the framework.
pub async fn call(app: Router, token: Option<&str>, body: &str) -> (u16, String) {
    use tower::ServiceExt;
    let mut req = Request::builder()
        .method("POST")
        .uri(MCP_PATH)
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream");
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let res = app.oneshot(req.body(axum::body::Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap_or_default();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// `service_factory` makes one handler per request; it must be cheap (a clone of shared state).
pub fn router<S>(
    service_factory: impl Fn() -> Result<S, std::io::Error> + Send + Sync + 'static,
    hosts: HostPolicy,
    token: Option<String>,
) -> Router
where
    S: Service<RoleServer> + Send + 'static,
{
    let mut config = StreamableHttpServerConfig::default();
    config.stateful_mode = false;
    config.json_response = true;
    // Replaced by `protect` below; see the module doc.
    config = config.disable_allowed_hosts();
    let service = StreamableHttpService::new(service_factory, Arc::new(LocalSessionManager::default()), config);
    protect(Router::new().nest_service(MCP_PATH, service), hosts, token)
}

/// Put the `Host` check and the token in front of `inner`. Split out so they are tested without
/// a protocol exchange.
pub fn protect(inner: Router, hosts: HostPolicy, token: Option<String>) -> Router {
    let guard = Arc::new(Guard { hosts, token });
    inner.layer(middleware::from_fn_with_state(guard, check))
}

struct Guard {
    hosts: HostPolicy,
    token: Option<String>,
}

async fn check(State(guard): State<Arc<Guard>>, req: Request, next: Next) -> Response {
    let claimed = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()).or_else(|| req.uri().host());
    if let Some(claimed) = claimed
        && !guard.hosts.allows(claimed)
    {
        tracing::warn!("refused MCP request claiming Host {claimed:?}");
        return (
            StatusCode::FORBIDDEN,
            "this server does not answer to that Host name; use an IP address, this machine's name, or list it with --allow-host\n",
        )
            .into_response();
    }
    if let Some(expected) = &guard.token {
        let given = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        if !given.is_some_and(|g| same(g.as_bytes(), expected.as_bytes())) {
            // The token is never echoed, and the refusal does not say whether one was sent.
            tracing::warn!("refused MCP request without the right token");
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                "a bearer token is required\n",
            )
                .into_response();
        }
    }
    next.run(req).await
}

/// Equal without stopping at the first differing byte, so the time taken says nothing about how
/// much of a guess was right. A length difference is not hidden: it is not the secret.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::get;
    use tower::ServiceExt;

    fn app(hosts: HostPolicy, token: Option<&str>) -> Router {
        protect(Router::new().route("/mcp", get(|| async { "ok" })), hosts, token.map(String::from))
    }

    async fn status(app: Router, host: &str, auth: Option<&str>) -> StatusCode {
        let mut req = Request::builder().uri("/mcp").header(header::HOST, host);
        if let Some(a) = auth {
            req = req.header(header::AUTHORIZATION, a);
        }
        app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap().status()
    }

    /// With no token configured the guard is the `Host` check alone, as on the HTTP server.
    #[tokio::test]
    async fn test_without_a_token_only_the_host_is_checked() {
        let net = || HostPolicy::for_bind("0.0.0.0".parse().unwrap(), None, &[]);
        assert_eq!(status(app(net(), None), "192.168.1.20:5152", None).await, StatusCode::OK);
        assert_eq!(status(app(net(), None), "evil.example", None).await, StatusCode::FORBIDDEN);
        assert_eq!(
            status(app(HostPolicy::loopback_only(), None), "192.168.1.20", None).await,
            StatusCode::FORBIDDEN,
            "a loopback bind keeps refusing a LAN address"
        );
    }

    #[tokio::test]
    async fn test_a_token_is_required_and_must_match() {
        let hosts = || HostPolicy::loopback_only();
        let t = Some("s3cret");
        assert_eq!(status(app(hosts(), t), "localhost", None).await, StatusCode::UNAUTHORIZED);
        assert_eq!(status(app(hosts(), t), "localhost", Some("Bearer nope")).await, StatusCode::UNAUTHORIZED);
        assert_eq!(status(app(hosts(), t), "localhost", Some("s3cret")).await, StatusCode::UNAUTHORIZED);
        assert_eq!(status(app(hosts(), t), "localhost", Some("Bearer s3cret")).await, StatusCode::OK);
    }

    /// The Host check runs first: a rebinding page learns nothing about the token.
    #[tokio::test]
    async fn test_a_bad_host_is_refused_before_the_token_is_looked_at() {
        let s = status(app(HostPolicy::loopback_only(), Some("s3cret")), "evil.example", Some("Bearer s3cret")).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
    }

    #[test]
    fn test_same_compares_whole_values() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"abc", b"abcd"));
        assert!(same(b"", b""));
    }
}
