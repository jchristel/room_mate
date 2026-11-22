//! What the server puts in front of its routes and its static files: the read-only CORS policy, the `Host` check that is
//! the other half of that control, and where the viewer's files are served from. Split out of `main.rs` so the security
//! controls read apart from the route table; `build_router` still wires them in one visible chain.

use super::*;

/// Cross-origin policy: **read-only, and that is the security boundary.**
///
/// This used to be `CorsLayer::permissive()`, which answers a preflight for any
/// method from any origin. That is a bigger grant than it looks, because
/// "binds loopback" does not mean "only things you trust can call it" — your
/// browser is local, so any page you visit can issue requests to
/// `localhost:5151` and, under a permissive policy, **read the responses**.
///
/// The server has no authentication of any kind, and `/api/settings` writes
/// project files. With permissive CORS, `POST /api/settings/drofus-check
/// {"path": "C:/Windows/win.ini"}` from a hostile page returned that file's
/// second line. Loopback binding did nothing to stop it; only the CORS policy
/// could. That endpoint has since been deleted along with the `file` reference
/// origin, so the specific read is gone — but the policy it forced still
/// guards every remaining write route, which is why it is asserted over all of
/// them below rather than over the one that leaked.
///
/// So: allow `GET`/`HEAD` cross-origin — the documented reason the layer exists
/// at all, so a viewer served from somewhere else can still read `/rooms` — and
/// grant nothing else. Every mutating route, and every request carrying
/// `Content-Type: application/json` (which forces a preflight), is now refused
/// at the preflight for a cross-origin caller.
///
/// **This costs non-browser clients nothing.** CORS is enforced by browsers and
/// ignored by everything else, so the pyRevit pusher, `curl` and the MCP
/// binary's HTTP client are unaffected. The settings UI is served same-origin
/// from `ServeDir`, so it never involved CORS to begin with.
///
/// Cross-origin `GET` still exposes room data to any page you visit. That is a
/// deliberate, narrower tradeoff kept for the dev convenience above — revisit
/// it with an origin allowlist if this ever runs anywhere but a workstation.
/// Leaving loopback is `--bind`, an explicit opt-in for a CLOSED network with no
/// authentication (see `network`): the pyRevit pusher is not a browser, so CORS
/// never applies to it, and the `Host` check below is what stays on.
///
/// **This layer is half the control, not all of it.** It decides from the
/// `Origin`, and a DNS-rebinding attack is built precisely to make that field
/// harmless — a rebound request is same-origin, so it arrives with no `Origin`
/// at all and this policy never runs. `guard_host` below is the other half, and
/// neither is sufficient alone.
pub(super) fn read_only_cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::HEAD])
        .allow_headers(Any)
        // `ETag` has to be exposed or the conditional-poll saving is same-origin
        // only: a cross-origin reader may *send* `If-None-Match` (allowed above,
        // at the cost of a preflight) but cannot read the tag back off a 200 to
        // send next time, so it would never have one to send. Response headers
        // are opaque to cross-origin JS unless named here, and the safelist does
        // not include `ETag`.
        .expose_headers([header::ETAG])
}

// The host names this server answers to are `network::HostPolicy`'s: loopback, and, when serving
// a network, IP literals, this machine's own names and the operator's. Anything else is a request
// that reached our socket under a name we do not own — see `guard_host`.

/// **The defence `read_only_cors` cannot provide: DNS rebinding.**
///
/// CORS is decided from the *`Origin`*, and a rebinding attack is designed to
/// make that field harmless. A hostile page on `evil.example` is served with a
/// one-second-TTL DNS record; once loaded, the record is re-answered as
/// `127.0.0.1`. The page then fetches `http://evil.example:5151/...`, which the
/// browser now routes to this server — and because the page's own origin *is*
/// `evil.example`, the request is **same-origin**. No preflight is sent, no
/// `Origin` header is attached, and `read_only_cors` never gets a say. The
/// arbitrary-file read it was written to stop is reachable again by a different
/// road.
///
/// The `Host` header is what survives that trick: the browser fills it with the
/// name the page asked for (`evil.example`), not the address it resolved to. So
/// the check is simply "is this a name we own" — loopback, in the three spellings
/// a browser can produce. A rebound request announces someone else's hostname and
/// is refused before it reaches a handler.
///
/// **A missing `Host` is allowed.** HTTP/1.1 requires the header and every
/// browser sends it, so absence means a non-browser client (the pyRevit pusher,
/// `curl`, the MCP binary's HTTP client, and the `oneshot` router tests below),
/// which is not the threat this guards — nothing a rebinding attacker controls
/// can *omit* it.
///
/// This is a name check, not authentication: it stops a remote page from driving
/// this server, and does nothing about a hostile process already on the machine.
/// Real auth in front of `/api/settings` remains the prerequisite for binding
/// anything but a closed network (see `network`).
pub(super) async fn guard_host(
    axum::extract::State(policy): axum::extract::State<Arc<HostPolicy>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let claimed = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .or_else(|| req.uri().host());

    if let Some(claimed) = claimed
        && !policy.allows(claimed)
    {
        tracing::warn!("refused request claiming Host {claimed:?} — not a name this server answers to");
        return (
            StatusCode::FORBIDDEN,
            "this server does not answer to that Host name; use an IP address, this machine's name, or list it with --allow-host\n",
        )
            .into_response();
    }
    next.run(req).await
}

/// Where the viewer's HTML/CSS/JS live: **beside the executable**, falling back
/// to `./static` relative to the working directory.
///
/// This was a bare `ServeDir::new("static")`, which is right exactly when the
/// process was launched from the crate root -- true under `cargo run` and false
/// for every installed build, because a Start Menu shortcut sets whatever
/// working directory it likes. The failure mode is the expensive kind: the API
/// keeps answering normally and only the *pages* 404, so the server looks
/// healthy while the website is simply missing.
///
/// Exe-relative is tried first because that is the shape an installer ships --
/// `roommate.exe` and `static/` in one directory, launched from anywhere. The
/// working-directory fallback is what keeps the repo working, where the binary
/// sits in `target/debug` with no `static/` beside it. Note the fallback is a
/// *relative* path on purpose: it must stay wrong in the same way it was
/// before, so nothing that already relies on being run from the crate root
/// changes behaviour.
///
/// Existence, not contents, decides the branch. A half-copied install is not
/// worth distinguishing from a missing one -- both want the same answer from
/// whoever reads the startup log, which is why `main` prints the path it chose.
pub(super) fn viewer_root() -> PathBuf {
    let exe = std::env::current_exe().ok();
    resolve_viewer_root(exe.as_deref().and_then(Path::parent))
}

/// The decision in `viewer_root`, with the one environment lookup lifted out so
/// both branches are reachable from a test. `std::env::current_exe` cannot be
/// pointed somewhere else, and under `cargo test` it resolves to
/// `target/debug/deps` -- which has no `static/` beside it, so an un-split
/// `viewer_root` could only ever exercise the fallback.
pub(super) fn resolve_viewer_root(exe_dir: Option<&Path>) -> PathBuf {
    exe_dir
        .map(|dir| dir.join("static"))
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(|| PathBuf::from("static"))
}

/// What the viewer's static files are served with, and **the bug it fixes.**
///
/// `ServeDir` sends `last-modified` and nothing else. With no `cache-control`,
/// a browser is free to apply *heuristic* freshness -- roughly a tenth of the
/// file's age -- and reuse a cached copy **without revalidating**, so a plain
/// reload does not re-fetch. That is not a theoretical staleness: the viewer is
/// two independently cached documents, `index.html` and the renderer bundle,
/// and the failure mode when they disagree is silent rather than broken.
///
/// On 2026-09-04 a browser held a NEW `index.html` against a CACHED
/// pre-windows `renderer.bundle.js`. The drawing code lives in the bundle and
/// the controls live in the HTML, so the window toggle appeared, the payload
/// loaded, and the plan drew nothing -- with no console error and nothing wrong
/// on the server. Only a hard reload cleared it, which is not something a
/// reader can be expected to guess.
///
/// `no-cache` is "store it, but revalidate before every use", NOT "do not
/// store": the conditional request still answers 304 from `last-modified`, so
/// the bytes are re-sent only when they actually changed. The cost is one
/// round trip per file on loopback; the alternative is a viewer that can be
/// silently half a release old.
pub(super) fn viewer_cache_control() -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))
}
