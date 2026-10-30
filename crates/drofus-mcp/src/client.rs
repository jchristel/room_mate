//! The HTTP side: one generic path from a prepared request to an answer, whatever the endpoint.
//!
//! **Order of events, and why it matters.** Arguments are checked, the address is built, and only then
//! is the request handed to the [`Authenticator`] -- the single place authentication happens -- and
//! sent. With no API key the call therefore ends at the authentication error with no network
//! traffic, which is how the tools can all be driven end to end before the sign-in is decided.
//!
//! **Nothing here retries, follows a redirect or logs a header.** A failed call is reported once. A
//! redirect is refused rather than followed, so a credential can never be carried to a host the
//! operator did not configure. The `Authorization` header (or anything the authenticator adds) is
//! never read back: errors are built from the status and the response body only.
//!
//! **Answers are bounded twice**: a list by `top` (see `endpoints`), and every answer by
//! [`ANSWER_MAX_BYTES`], because one oversized record is as costly to a model's context as a table.

use std::{sync::Arc, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{header, Response, StatusCode};
use serde_json::{json, Value};

use crate::{
    auth::Authenticator,
    config::Config,
    endpoints::{Endpoint, Kind, Paging, ToolError},
};

/// The largest image returned. Base64 grows it by a third, and the whole thing lands in a model's
/// context, so this is deliberately small; an image above it is refused with its size.
pub const IMAGE_MAX_BYTES: usize = 1_048_576;
/// The largest text answer returned. Larger is refused with advice to narrow the request.
pub const ANSWER_MAX_BYTES: usize = 256 * 1024;
/// How much of a response is read before giving up, whatever it turns out to be.
const READ_MAX_BYTES: usize = 8 * 1024 * 1024;
/// How much of an error body is shown.
const ERROR_BODY_MAX_CHARS: usize = 500;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub enum Answer {
    Json(Value),
    Image { mime: String, bytes: usize, base64: String },
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    auth: Arc<dyn Authenticator>,
    base_url: Option<String>,
    database: Option<String>,
    project: Option<String>,
}

impl Client {
    pub fn new(config: &Config, auth: Arc<dyn Authenticator>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("a client with only a timeout and a redirect policy always builds");
        Self {
            http,
            auth,
            base_url: config.base_url.clone(),
            database: config.database.clone(),
            project: config.project.clone(),
        }
    }

    /// Run one tool call.
    pub async fn call(&self, endpoint: &Endpoint, args: &serde_json::Map<String, Value>) -> Result<Answer, ToolError> {
        let prepared = endpoint.prepare(args, self.database.as_deref(), self.project.as_deref())?;
        let base = self
            .base_url
            .as_deref()
            .ok_or_else(|| ToolError("no dRofus address: start the server with --base-url / DROFUS_BASE_URL".into()))?;
        let url = prepared.url(base)?;
        if url.scheme() != "https" && !is_local(&url) {
            return Err(ToolError(
                "refusing to send a credential over plain http to a non-local address; use an https base address"
                    .into(),
            ));
        }
        let accept = if endpoint.kind == Kind::Binary { "image/*" } else { "application/json" };
        let request = self.http.request(endpoint.kind.method(), url).header(header::ACCEPT, accept);
        let request = self.auth.authenticate(request).await?;

        let response = request.send().await.map_err(|e| {
            let what = if e.is_timeout() {
                "timed out"
            } else if e.is_connect() {
                "could not connect"
            } else {
                "failed"
            };
            ToolError(format!("the request to dRofus {what}; check the configured address ({base})"))
        })?;
        let status = response.status();
        if !status.is_success() {
            let body = read_capped(response, ERROR_BODY_MAX_CHARS * 4).await.map(|b| b.0).unwrap_or_default();
            return Err(status_error(status, &String::from_utf8_lossy(&body)));
        }

        match endpoint.kind {
            Kind::Binary => image_answer(response).await,
            Kind::List => list_answer(response, prepared.paging).await,
            Kind::Get | Kind::Meta => json_answer(response).await.and_then(bounded).map(Answer::Json),
        }
    }
}

/// Loopback only: a mock or a tunnel. Anything else must be https before a key is attached.
fn is_local(url: &reqwest::Url) -> bool {
    url.host_str().is_some_and(|h| {
        h.eq_ignore_ascii_case("localhost")
            || h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
    })
}

/// `401` and `403` are the caller's sign-in, `404` is the id, the rest is shown as it came (truncated).
fn status_error(status: StatusCode, body: &str) -> ToolError {
    ToolError(match status {
        StatusCode::UNAUTHORIZED => "dRofus answered 401: authentication is required".to_string(),
        StatusCode::FORBIDDEN => {
            "dRofus answered 403: authentication was refused, or this user may not read this".to_string()
        }
        StatusCode::NOT_FOUND => "not found: dRofus has no such record (check the id, `db` and `pr`)".to_string(),
        s => {
            let shown: String = body.trim().chars().take(ERROR_BODY_MAX_CHARS).collect();
            if shown.is_empty() {
                format!("dRofus answered {s}")
            } else {
                format!("dRofus answered {s}: {shown}")
            }
        }
    })
}

/// The body, up to `limit` bytes. `Err(n)` is the size it was seen to reach (or announced), which is
/// more than `limit`. Reading stops at the limit, so a huge body is never held whole.
async fn read_capped(mut response: Response, limit: usize) -> Result<(Vec<u8>, ()), usize> {
    if let Some(announced) = response.content_length().and_then(|n| usize::try_from(n).ok())
        && announced > limit
    {
        return Err(announced);
    }
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            return Err(body.len());
        }
    }
    Ok((body, ()))
}

async fn json_answer(response: Response) -> Result<Value, ToolError> {
    let (body, ()) = read_capped(response, READ_MAX_BYTES).await.map_err(|n| {
        ToolError(format!(
            "dRofus sent an answer of at least {n} bytes, which is too large to read; narrow the request"
        ))
    })?;
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Null);
    }
    Ok(serde_json::from_slice(&body).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned())))
}

fn bounded(value: Value) -> Result<Value, ToolError> {
    let size = value.to_string().len();
    if size > ANSWER_MAX_BYTES {
        return Err(ToolError(format!(
            "the answer is {size} bytes, over the {ANSWER_MAX_BYTES}-byte limit; use `select` to name fewer fields, \
             `filter` to narrow, or a smaller `top`"
        )));
    }
    Ok(value)
}

/// A list is wrapped so the answer itself says whether it was cut and how to get the next page.
async fn list_answer(response: Response, paging: Option<Paging>) -> Result<Answer, ToolError> {
    let body = json_answer(response).await?;
    let (Some(paging), Value::Array(items)) = (paging, &body) else {
        // An empty body is an empty list; anything that is not an array is passed on as it came.
        let other = if body.is_null() {
            json!({"items": [], "returned": 0, "more_may_exist": false})
        } else {
            body
        };
        return bounded(other).map(Answer::Json);
    };
    let returned = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let more = returned >= paging.top;
    let next_skip = paging.skip.saturating_add(returned);
    let mut notes = Vec::new();
    if paging.capped {
        notes.push(format!("`top` was lowered to the ceiling of {}.", crate::endpoints::MAX_TOP));
    }
    if paging.defaulted {
        notes.push(format!("No `top` was given, so at most {} rows were requested.", paging.top));
    }
    if more {
        notes.push(format!("The page is full, so more rows may exist: call again with skip={next_skip}."));
    }
    let answer = json!({
        "items": items,
        "returned": returned,
        "top": paging.top,
        "skip": paging.skip,
        "more_may_exist": more,
        "next_skip": more.then_some(next_skip),
        "note": notes.join(" "),
    });
    bounded(answer).map(Answer::Json)
}

/// The type of an image from its first bytes, for a server that sends `application/octet-stream`.
fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'B', b'M', ..] => Some("image/bmp"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => None,
    }
}

async fn image_answer(response: Response) -> Result<Answer, ToolError> {
    let declared = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase());
    let (bytes, ()) = read_capped(response, IMAGE_MAX_BYTES).await.map_err(|n| {
        ToolError(format!(
            "the image is {n} bytes or more, over the {IMAGE_MAX_BYTES}-byte limit for returning one; use the \
             matching `_meta` tool for its description"
        ))
    })?;
    let mime = match declared {
        Some(m) if m.starts_with("image/") && m != "image/*" => m,
        _ => sniff_image(&bytes)
            .map(str::to_string)
            .ok_or_else(|| ToolError("dRofus sent something that is not a recognisable image".into()))?,
    };
    Ok(Answer::Image { mime, bytes: bytes.len(), base64: STANDARD.encode(&bytes) })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use reqwest::Method;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;
    use crate::{
        auth::{AuthFuture, ReferenceKey, Unauthenticated},
        endpoints::ENDPOINTS,
    };

    const SECRET: &str = "sekrit-token-123";

    /// Adds a header carrying a secret, to prove the authenticator is called once, in the one place, and
    /// that the secret reaches the server and nothing else.
    struct Stamp;
    impl Authenticator for Stamp {
        fn authenticate(&self, request: reqwest::RequestBuilder) -> AuthFuture<'_> {
            Box::pin(async move { Ok(request.header(header::AUTHORIZATION, format!("Bearer {SECRET}"))) })
        }
    }

    struct Mock {
        base: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    /// One canned response for every request; records each request head.
    async fn mock(status: u16, content_type: &'static str, body: Vec<u8>) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let (seen, body) = (seen.clone(), body.clone());
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    seen.lock().unwrap().push(String::from_utf8_lossy(&buf[..n]).into_owned());
                    let head = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(&body).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Mock { base, requests }
    }

    fn client(base: Option<&str>, auth: Arc<dyn Authenticator>) -> Client {
        let config = Config::resolve(base.map(str::to_string), None, Some("dbx".into()), Some("prx".into()), |_| None);
        Client::new(&config, auth)
    }

    fn ep(name: &str) -> &'static Endpoint {
        ENDPOINTS.iter().find(|e| e.name == name).unwrap()
    }

    fn args(v: Value) -> serde_json::Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn first_line(m: &Mock) -> String {
        m.requests.lock().unwrap()[0].lines().next().unwrap().to_string()
    }

    #[tokio::test]
    async fn the_request_line_carries_the_path_and_the_query_unchanged() {
        let m = mock(200, "application/json", b"[{\"id\":1}]".to_vec()).await;
        let c = client(Some(&m.base), Arc::new(Stamp));
        let a = c
            .call(
                ep("drofus_rooms_list"),
                &args(json!({"filter": "name eq 'a b'", "select": "id,name", "top": 5})),
            )
            .await
            .unwrap();
        let line = first_line(&m);
        assert!(line.starts_with("GET /api/dbx/prx/rooms?"), "{line}");
        assert!(
            line.contains("%24filter=name+eq+%27a+b%27")
                && line.contains("%24select=id%2Cname")
                && line.contains("%24top=5"),
            "{line}"
        );
        let head = m.requests.lock().unwrap()[0].to_ascii_lowercase();
        assert!(
            head.contains(&format!("authorization: bearer {SECRET}")),
            "the authenticator ran once, here"
        );
        let Answer::Json(v) = a else { panic!() };
        assert_eq!(v["returned"], 1);
        assert_eq!(v["more_may_exist"], false);
    }

    #[tokio::test]
    async fn a_full_page_says_how_to_ask_for_the_next_one() {
        let rows: Vec<Value> = (0..3).map(|i| json!({"id": i})).collect();
        let m = mock(200, "application/json", serde_json::to_vec(&rows).unwrap()).await;
        let c = client(Some(&m.base), Arc::new(Stamp));
        let Answer::Json(v) = c.call(ep("drofus_items_list"), &args(json!({"top": 3, "skip": 6}))).await.unwrap()
        else {
            panic!()
        };
        assert_eq!(v["more_may_exist"], true);
        assert_eq!(v["next_skip"], 9);
        assert!(v["note"].as_str().unwrap().contains("skip=9"));
        assert!(first_line(&m).contains("%24skip=6"));
    }

    #[tokio::test]
    async fn a_request_for_too_many_rows_is_lowered_and_the_answer_says_so() {
        let m = mock(200, "application/json", b"[]".to_vec()).await;
        let c = client(Some(&m.base), Arc::new(Stamp));
        let Answer::Json(v) = c.call(ep("drofus_items_list"), &args(json!({"top": 99_999}))).await.unwrap() else {
            panic!()
        };
        assert!(first_line(&m).contains("%24top=200"), "{}", first_line(&m));
        assert!(v["note"].as_str().unwrap().contains("lowered"));
    }

    #[tokio::test]
    async fn statuses_map_to_words_and_the_secret_never_appears() {
        for (status, expect) in [
            (401, "authentication is required"),
            (403, "refused"),
            (404, "not found"),
            (500, "500"),
        ] {
            let m = mock(status, "text/plain", b"boom-detail".to_vec()).await;
            let c = client(Some(&m.base), Arc::new(Stamp));
            let err = c.call(ep("drofus_rooms_get"), &args(json!({"id": 4}))).await.unwrap_err();
            assert!(err.0.contains(expect), "{status}: {err}");
            assert!(!err.0.contains(SECRET) && !format!("{err:?}").contains(SECRET));
            assert_eq!(
                err.0.contains("boom-detail"),
                status == 500,
                "only an unmapped status shows the body: {err}"
            );
        }
    }

    #[tokio::test]
    async fn an_error_body_is_truncated() {
        let m = mock(500, "text/plain", "x".repeat(5000).into_bytes()).await;
        let c = client(Some(&m.base), Arc::new(Stamp));
        let err = c.call(ep("drofus_rooms_get"), &args(json!({"id": 1}))).await.unwrap_err();
        assert!(err.0.len() < 600, "{}", err.0.len());
    }

    #[tokio::test]
    async fn unauthenticated_stops_before_any_network_call() {
        let m = mock(200, "application/json", b"{}".to_vec()).await;
        let c = client(Some(&m.base), Arc::new(Unauthenticated));
        let err = c.call(ep("drofus_rooms_get"), &args(json!({"id": 1}))).await.unwrap_err();
        assert!(err.0.contains("no API key is configured"), "{err}");
        assert!(m.requests.lock().unwrap().is_empty(), "no request was sent");
    }

    #[tokio::test]
    async fn the_reference_key_reaches_the_server_and_never_an_answer() {
        let m = mock(200, "application/json", b"{\"id\":1}".to_vec()).await;
        let c = client(Some(&m.base), Arc::new(ReferenceKey::new(crate::config::Secret::new(SECRET))));
        let Answer::Json(v) = c.call(ep("drofus_rooms_get"), &args(json!({"id": 1}))).await.unwrap() else {
            panic!()
        };
        let head = m.requests.lock().unwrap()[0].to_ascii_lowercase();
        assert!(head.contains(&format!("authorization: reference {SECRET}")), "{head}");
        assert!(!v.to_string().contains(SECRET));
    }

    #[tokio::test]
    async fn a_credential_is_not_attached_to_plain_http_off_this_machine() {
        let c = client(Some("http://drofus.example.invalid"), Arc::new(Stamp));
        let err = c.call(ep("drofus_rooms_get"), &args(json!({"id": 1}))).await.unwrap_err();
        assert!(err.0.contains("plain http"), "{err}");
        assert!(
            is_local(&reqwest::Url::parse("http://[::1]:5/").unwrap())
                && is_local(&reqwest::Url::parse("http://localhost/").unwrap())
        );
        assert!(!is_local(&reqwest::Url::parse("http://10.0.0.5/").unwrap()));
    }

    #[tokio::test]
    async fn bad_arguments_and_a_missing_address_are_reported_before_authentication() {
        let c = client(None, Arc::new(Unauthenticated));
        assert!(c
            .call(ep("drofus_rooms_get"), &args(json!({})))
            .await
            .unwrap_err()
            .0
            .contains("`id` is required"));
        assert!(c
            .call(ep("drofus_rooms_get"), &args(json!({"id": 1})))
            .await
            .unwrap_err()
            .0
            .contains("--base-url"));
    }

    #[tokio::test]
    async fn an_image_is_returned_as_base64_with_its_type() {
        let png = [0x89, b'P', b'N', b'G', 1, 2, 3].to_vec();
        let m = mock(200, "image/png", png.clone()).await;
        let c = client(Some(&m.base), Arc::new(Stamp));
        let Answer::Image { mime, bytes, base64 } =
            c.call(ep("drofus_rooms_image"), &args(json!({"roomimageid": 2}))).await.unwrap()
        else {
            panic!()
        };
        assert_eq!((mime.as_str(), bytes), ("image/png", 7));
        assert_eq!(STANDARD.decode(base64).unwrap(), png);
        assert!(first_line(&m).starts_with("GET /api/dbx/prx/rooms/images/2 "));
    }

    #[tokio::test]
    async fn an_octet_stream_image_is_recognised_by_its_first_bytes() {
        let m = mock(200, "application/octet-stream", vec![0xFF, 0xD8, 0xFF, 0xE0, 0]).await;
        let c = client(Some(&m.base), Arc::new(Stamp));
        let Answer::Image { mime, .. } = c.call(ep("drofus_projects_smallimage"), &args(json!({}))).await.unwrap()
        else {
            panic!()
        };
        assert_eq!(mime, "image/jpeg");
    }

    #[tokio::test]
    async fn an_image_over_the_cap_is_refused_with_its_size() {
        let m = mock(200, "image/png", vec![0u8; IMAGE_MAX_BYTES + 1000]).await;
        let c = client(Some(&m.base), Arc::new(Stamp));
        let err = c.call(ep("drofus_rooms_image"), &args(json!({"roomimageid": 2}))).await.unwrap_err();
        assert!(err.0.contains(&(IMAGE_MAX_BYTES + 1000).to_string()) && err.0.contains("_meta"), "{err}");
    }

    #[tokio::test]
    async fn an_oversized_record_is_refused_with_advice() {
        let big = json!({"blob": "y".repeat(ANSWER_MAX_BYTES + 10)});
        let m = mock(200, "application/json", serde_json::to_vec(&big).unwrap()).await;
        let c = client(Some(&m.base), Arc::new(Stamp));
        let err = c.call(ep("drofus_rooms_get"), &args(json!({"id": 1}))).await.unwrap_err();
        assert!(err.0.contains("select"), "{err}");
    }

    #[tokio::test]
    async fn metadata_uses_options_and_a_redirect_is_not_followed() {
        let m = mock(200, "application/json", b"[{\"name\":\"id\"}]".to_vec()).await;
        let c = client(Some(&m.base), Arc::new(Stamp));
        c.call(ep("drofus_rooms_properties"), &args(json!({"depth": 1}))).await.unwrap();
        assert!(first_line(&m).starts_with("OPTIONS /api/dbx/prx/rooms?depth=1"), "{}", first_line(&m));
        assert_eq!(Kind::Meta.method(), Method::OPTIONS);
        let r = mock(302, "text/plain", Vec::new()).await;
        let err = client(Some(&r.base), Arc::new(Stamp))
            .call(ep("drofus_rooms_get"), &args(json!({"id": 1})))
            .await
            .unwrap_err();
        assert!(err.0.contains("302"), "{err}");
    }
}
