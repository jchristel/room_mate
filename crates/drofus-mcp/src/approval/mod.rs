//! The human gate for writes: a change is proposed (a dry run), a person approves it somewhere the
//! model cannot reach, and only then can it be applied, once.
//!
//! **Nothing here writes to dRofus, and no tool uses it yet.** The crate's rule stands: no write tool
//! exists until the owner decides one should. This is the gate such a tool would have to go through,
//! built and tested first so that the first write tool cannot be written without it. The design is the
//! owner's (see `AUTH.md`'s sibling decision in the project notes): **out-of-band approval, dry run
//! by default.**
//!
//! **The flow.** A write tool calls [`Gate::propose`] with a one-line summary and the payload it would
//! send. That returns a [`Proposal`] (an id, a hash, the expiry) and nothing else happens: this *is*
//! the dry run. A person opens the approval page (`page`, a local web page on `127.0.0.1` that opens in
//! the browser by itself), reads the exact change and clicks Approve or Reject. Later the tool calls
//! [`Gate::apply`] with the id; if the change is approved, unexpired and unused, the gate hands the
//! *stored* payload to the closure and marks it used before the closure runs.
//!
//! **Why out of band.** An approval inside the tool protocol is something the model can also produce.
//! A click on a page whose address and key never appear in a tool answer is not. The address goes to the
//! browser and, only if the browser could not be opened, to stderr (which is not the transport).
//!
//! **What this does not defend against**, said plainly: a process on the same machine, as the same
//! user, that can read another process's command line or open ports. If the host gives the model a
//! shell, the key in the browser's command line is reachable. The threat modelled is a model acting
//! through MCP tools. An OS dialog would be stronger and is a different build.
//!
//! **Properties kept on purpose.**
//! - The payload is stored server-side and what runs is the stored copy, never what the caller passes to
//!   `apply`; a decision names the hash of what the page showed, so approval is of those bytes.
//! - Single use: a change is marked applied *before* it runs, so a failure or crash cannot replay it.
//! - It expires ([`DEFAULT_TTL`]) and the number waiting is bounded ([`MAX_PENDING`]).
//! - Off unless `--allow-writes`: a disabled gate refuses every proposal.
//! - Every step is appended to an audit log (summary and hash, never the payload, never a credential);
//!   if the "applied" line cannot be written, the change is not run.

mod page;

use std::{
    collections::HashMap,
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;

/// How long a proposal can be approved and applied before it lapses.
pub const DEFAULT_TTL: Duration = Duration::from_secs(10 * 60);
/// How many unfinished proposals may exist at once.
pub const MAX_PENDING: usize = 50;
/// The browser is opened again only if the page has not been fetched for this long.
const REOPEN_AFTER: Duration = Duration::from_secs(30);

/// A refusal, in words for the caller. Never holds a credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateError(pub String);

impl std::fmt::Display for GateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug)]
pub struct GateConfig {
    /// `--allow-writes`. Off, every proposal is refused.
    pub enabled: bool,
    /// Where the audit log is appended, one JSON object per line. `None` keeps no log, which is
    /// refused for an enabled gate by [`Gate::new`]'s caller (`main`), not here, so tests can run without one.
    pub audit_log: Option<PathBuf>,
    pub ttl: Duration,
    /// Open the approval page in the browser. Tests turn it off.
    pub open_browser: bool,
}

impl GateConfig {
    pub fn disabled() -> Self {
        Self { enabled: false, audit_log: None, ttl: DEFAULT_TTL, open_browser: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Pending,
    Approved,
    Rejected,
    Applied,
    Expired,
}

/// What a proposal answers: the dry run. It never says where the approval page is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Proposal {
    pub id: String,
    pub hash: String,
    pub phase: Phase,
    pub expires_in_secs: u64,
    pub note: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    pub id: String,
    pub phase: Phase,
    pub expires_in_secs: u64,
}

struct Entry {
    summary: String,
    payload: Value,
    hash: String,
    phase: Phase,
    created: Instant,
}

#[derive(Default)]
struct Book {
    entries: HashMap<String, Entry>,
    /// When the approval page was last fetched, to decide whether to open the browser again.
    page_seen: Option<Instant>,
}

struct Ui {
    port: u16,
    key: String,
}

struct Shared {
    cfg: GateConfig,
    book: Mutex<Book>,
    ui: OnceCell<Ui>,
}

#[derive(Clone)]
pub struct Gate {
    shared: Arc<Shared>,
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("the operating system's random source is available");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_of(summary: &str, payload: &Value) -> String {
    let mut h = Sha256::new();
    h.update(summary.as_bytes());
    h.update([0]);
    h.update(payload.to_string().as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

impl Entry {
    fn left(&self, ttl: Duration) -> Duration {
        ttl.saturating_sub(self.created.elapsed())
    }

    /// The phase as of now: a change not yet applied lapses when its time is up.
    fn phase_now(&self, ttl: Duration) -> Phase {
        match self.phase {
            Phase::Pending | Phase::Approved if self.left(ttl).is_zero() => Phase::Expired,
            p => p,
        }
    }
}

impl Gate {
    pub fn new(cfg: GateConfig) -> Self {
        Self {
            shared: Arc::new(Shared { cfg, book: Mutex::new(Book::default()), ui: OnceCell::new() }),
        }
    }

    pub fn enabled(&self) -> bool {
        self.shared.cfg.enabled
    }

    fn book(&self) -> std::sync::MutexGuard<'_, Book> {
        self.shared.book.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn audit(&self, event: &str, id: &str, summary: &str, hash: &str) -> std::io::Result<()> {
        let Some(path) = &self.shared.cfg.audit_log else {
            return Ok(());
        };
        let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let line = json!({"at": at, "event": event, "id": id, "summary": summary, "hash": hash});
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(f, "{line}")
    }

    fn audit_soft(&self, event: &str, id: &str, summary: &str, hash: &str) {
        if let Err(e) = self.audit(event, id, summary, hash) {
            tracing::warn!("audit log write failed: {e}");
        }
    }

    /// The dry run: record the change and wait for a person. Nothing is sent anywhere.
    pub async fn propose(&self, summary: &str, payload: Value) -> Result<Proposal, GateError> {
        if !self.enabled() {
            return Err(GateError(
                "writes are disabled: start the server with --allow-writes to enable them".into(),
            ));
        }
        let ttl = self.shared.cfg.ttl;
        let (id, hash) = {
            let mut book = self.book();
            book.entries.retain(|_, e| {
                !matches!(e.phase_now(ttl), Phase::Expired | Phase::Rejected | Phase::Applied)
                    || e.created.elapsed() < ttl
            });
            let waiting = book
                .entries
                .values()
                .filter(|e| matches!(e.phase_now(ttl), Phase::Pending | Phase::Approved))
                .count();
            if waiting >= MAX_PENDING {
                return Err(GateError(format!(
                    "{waiting} changes are already waiting for a person; decide those first"
                )));
            }
            let id = random_hex(8);
            let hash = hash_of(summary, &payload);
            book.entries.insert(
                id.clone(),
                Entry {
                    summary: summary.to_string(),
                    payload,
                    hash: hash.clone(),
                    phase: Phase::Pending,
                    created: Instant::now(),
                },
            );
            (id, hash)
        };
        self.audit_soft("proposed", &id, summary, &hash);
        self.show_page().await;
        Ok(Proposal {
            id,
            hash,
            phase: Phase::Pending,
            expires_in_secs: ttl.as_secs(),
            note: "Dry run: nothing was changed. A person must approve this on the approval page, which was opened on \
                   this machine; then apply it by id.",
        })
    }

    pub fn status(&self, id: &str) -> Result<Status, GateError> {
        let ttl = self.shared.cfg.ttl;
        let book = self.book();
        let e = book.entries.get(id).ok_or_else(|| GateError(format!("no change with id {id}")))?;
        Ok(Status {
            id: id.to_string(),
            phase: e.phase_now(ttl),
            expires_in_secs: e.left(ttl).as_secs(),
        })
    }

    /// Record a person's decision. `hash` is what the page showed; a mismatch refuses, so the approval
    /// is of exactly that change.
    pub(crate) fn decide(&self, id: &str, hash: &str, approve: bool) -> Result<(), GateError> {
        let ttl = self.shared.cfg.ttl;
        let summary = {
            let mut book = self.book();
            let e = book.entries.get_mut(id).ok_or_else(|| GateError("no such change".into()))?;
            if e.hash != hash {
                return Err(GateError("the change no longer matches what was shown".into()));
            }
            match e.phase_now(ttl) {
                Phase::Pending => {}
                Phase::Expired => return Err(GateError("that change has expired".into())),
                p => return Err(GateError(format!("that change is already {p:?}").to_lowercase())),
            }
            e.phase = if approve { Phase::Approved } else { Phase::Rejected };
            e.summary.clone()
        };
        self.audit_soft(if approve { "approved" } else { "rejected" }, id, &summary, hash);
        Ok(())
    }

    /// Run an approved change, once. The closure gets the stored payload; the change is marked applied
    /// first, so neither a failure nor a repeat call can run it twice.
    pub async fn apply<T, F, Fut>(&self, id: &str, run: F) -> Result<T, GateError>
    where
        F: FnOnce(Value) -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        let ttl = self.shared.cfg.ttl;
        let (payload, summary, hash) = {
            let mut book = self.book();
            let e = book.entries.get_mut(id).ok_or_else(|| GateError(format!("no change with id {id}")))?;
            match e.phase_now(ttl) {
                Phase::Approved => {}
                Phase::Pending => {
                    return Err(GateError("not approved yet: a person has to approve it on the approval page".into()))
                }
                Phase::Rejected => return Err(GateError("a person rejected this change".into())),
                Phase::Applied => return Err(GateError("this change was already applied; it cannot run twice".into())),
                Phase::Expired => {
                    return Err(GateError("this change expired before it was applied; propose it again".into()))
                }
            }
            e.phase = Phase::Applied;
            (e.payload.clone(), e.summary.clone(), e.hash.clone())
        };
        if let Err(e) = self.audit("applied", id, &summary, &hash) {
            // The change is spent (applied above) but never ran: safer than running unlogged.
            return Err(GateError(format!(
                "the audit log cannot be written ({e}); the change was not run and is spent"
            )));
        }
        Ok(run(payload).await)
    }

    /// Everything waiting for a decision, oldest first, for the page.
    pub(crate) fn waiting(&self) -> Vec<(String, String, Value, String, u64)> {
        let ttl = self.shared.cfg.ttl;
        let book = self.book();
        let mut rows: Vec<_> = book
            .entries
            .iter()
            .filter(|(_, e)| e.phase_now(ttl) == Phase::Pending)
            .map(|(id, e)| {
                (
                    e.created,
                    id.clone(),
                    e.summary.clone(),
                    e.payload.clone(),
                    e.hash.clone(),
                    e.left(ttl).as_secs(),
                )
            })
            .collect();
        rows.sort_by_key(|r| r.0);
        rows.into_iter().map(|(_, id, s, p, h, left)| (id, s, p, h, left)).collect()
    }

    pub(crate) fn mark_page_seen(&self) {
        self.book().page_seen = Some(Instant::now());
    }

    /// Start the page on first use and open it unless it was fetched recently.
    async fn show_page(&self) {
        let ui = self.shared.ui.get_or_try_init(|| page::serve(self.clone())).await;
        let ui = match ui {
            Ok(ui) => ui,
            Err(e) => {
                tracing::error!("the approval page could not start: {e}");
                return;
            }
        };
        let stale = self.book().page_seen.is_none_or(|t| t.elapsed() > REOPEN_AFTER);
        if stale && self.shared.cfg.open_browser {
            let url = format!("http://127.0.0.1:{}/?k={}", ui.port, ui.key);
            if !page::open_in_browser(&url) {
                tracing::warn!("could not open a browser; open this address yourself to review the change: {url}");
            }
        }
    }

    /// The page's address and key, for tests.
    #[cfg(test)]
    async fn page_address(&self) -> (u16, String) {
        self.show_page().await;
        let ui = self.shared.ui.get().expect("started");
        (ui.port, ui.key.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(ttl: Duration) -> Gate {
        Gate::new(GateConfig { enabled: true, audit_log: None, ttl, open_browser: false })
    }

    #[tokio::test]
    async fn a_disabled_gate_refuses_every_proposal() {
        let g = Gate::new(GateConfig::disabled());
        let err = g.propose("x", json!({})).await.unwrap_err();
        assert!(err.0.contains("--allow-writes"), "{err}");
    }

    #[tokio::test]
    async fn a_proposal_is_a_dry_run_that_cannot_be_applied_until_a_person_approves() {
        let g = gate(DEFAULT_TTL);
        let p = g.propose("rename room 4", json!({"id": 4, "name": "B"})).await.unwrap();
        assert_eq!(p.phase, Phase::Pending);
        let err = g.apply(&p.id, |_| async { 1 }).await.unwrap_err();
        assert!(err.0.contains("not approved"), "{err}");
        assert_eq!(g.status(&p.id).unwrap().phase, Phase::Pending);
    }

    #[tokio::test]
    async fn an_approved_change_runs_the_stored_payload_exactly_once() {
        let g = gate(DEFAULT_TTL);
        let p = g.propose("rename room 4", json!({"id": 4})).await.unwrap();
        g.decide(&p.id, &p.hash, true).unwrap();
        let seen = g.apply(&p.id, |payload| async move { payload }).await.unwrap();
        assert_eq!(seen, json!({"id": 4}));
        let again = g.apply(&p.id, |_| async {}).await.unwrap_err();
        assert!(again.0.contains("already applied"), "{again}");
        assert_eq!(g.status(&p.id).unwrap().phase, Phase::Applied);
    }

    #[tokio::test]
    async fn a_rejected_change_and_a_wrong_hash_never_run() {
        let g = gate(DEFAULT_TTL);
        let p = g.propose("a", json!(1)).await.unwrap();
        assert!(g.decide(&p.id, "deadbeef", true).unwrap_err().0.contains("no longer matches"));
        g.decide(&p.id, &p.hash, false).unwrap();
        assert!(g.apply(&p.id, |_| async {}).await.unwrap_err().0.contains("rejected"));
        assert!(g.decide(&p.id, &p.hash, true).is_err(), "a decision is final");
    }

    #[tokio::test]
    async fn an_unapplied_change_expires() {
        let g = gate(Duration::from_millis(30));
        let p = g.propose("a", json!(1)).await.unwrap();
        g.decide(&p.id, &p.hash, true).unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(g.status(&p.id).unwrap().phase, Phase::Expired);
        assert!(g.apply(&p.id, |_| async {}).await.unwrap_err().0.contains("expired"));
    }

    #[tokio::test]
    async fn the_number_waiting_is_bounded() {
        let g = gate(DEFAULT_TTL);
        for i in 0..MAX_PENDING {
            g.propose("x", json!(i)).await.unwrap();
        }
        assert!(g.propose("x", json!("one more")).await.unwrap_err().0.contains("already waiting"));
    }

    #[tokio::test]
    async fn the_audit_log_records_each_step_without_the_payload() {
        let path = std::env::temp_dir().join(format!("drofus-mcp-audit-{}.jsonl", random_hex(6)));
        let g = Gate::new(GateConfig {
            enabled: true,
            audit_log: Some(path.clone()),
            ttl: DEFAULT_TTL,
            open_browser: false,
        });
        let p = g.propose("rename room 4", json!({"secret_field": "payload-value"})).await.unwrap();
        g.decide(&p.id, &p.hash, true).unwrap();
        g.apply(&p.id, |_| async {}).await.unwrap();
        let log = std::fs::read_to_string(&path).unwrap();
        let events: Vec<String> = log
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap()["event"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(events, ["proposed", "approved", "applied"]);
        assert!(!log.contains("payload-value") && log.contains("rename room 4") && log.contains(&p.hash));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn if_the_audit_line_cannot_be_written_the_change_does_not_run() {
        let dir = std::env::temp_dir()
            .join(format!("drofus-mcp-nodir-{}", random_hex(6)))
            .join("no/such/dir/log.jsonl");
        let g = Gate::new(GateConfig { enabled: true, audit_log: Some(dir), ttl: DEFAULT_TTL, open_browser: false });
        let p = g.propose("a", json!(1)).await.unwrap();
        g.decide(&p.id, &p.hash, true).unwrap();
        let ran = Arc::new(Mutex::new(false));
        let flag = ran.clone();
        let err = g.apply(&p.id, |_| async move { *flag.lock().unwrap() = true }).await.unwrap_err();
        assert!(err.0.contains("not run") && !*ran.lock().unwrap(), "{err}");
    }

    #[tokio::test]
    async fn the_proposal_answer_never_carries_the_page_address() {
        let g = gate(DEFAULT_TTL);
        let p = g.propose("a", json!(1)).await.unwrap();
        let (port, key) = g.page_address().await;
        let shown = serde_json::to_string(&p).unwrap();
        assert!(!shown.contains(&key) && !shown.contains(&port.to_string()), "{shown}");
    }

    mod over_http {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpStream,
        };

        use super::*;

        async fn raw(port: u16, request: String) -> String {
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(request.as_bytes()).await.unwrap();
            let mut out = Vec::new();
            let _ = s.read_to_end(&mut out).await;
            String::from_utf8_lossy(&out).into_owned()
        }

        fn get(port: u16, path: &str, host: &str) -> String {
            let _ = port;
            format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
        }

        fn post(path: &str, host: &str, origin: Option<&str>, body: &str) -> String {
            let origin = origin.map(|o| format!("Origin: {o}\r\n")).unwrap_or_default();
            format!(
                "POST {path} HTTP/1.1\r\nHost: {host}\r\n{origin}Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        }

        #[tokio::test]
        async fn the_page_needs_its_key_and_its_own_host_name() {
            let g = gate(DEFAULT_TTL);
            g.propose("a", json!(1)).await.unwrap();
            let (port, key) = g.page_address().await;
            let host = format!("127.0.0.1:{port}");
            assert!(raw(port, get(port, "/", &host)).await.starts_with("HTTP/1.1 403"));
            assert!(raw(port, get(port, "/?k=wrong", &host)).await.starts_with("HTTP/1.1 403"));
            let rebound = raw(port, get(port, &format!("/?k={key}"), &format!("evil.example:{port}"))).await;
            assert!(
                rebound.starts_with("HTTP/1.1 403"),
                "a rebinding page presents another host name: {rebound}"
            );
            assert!(raw(port, get(port, &format!("/?k={key}"), &host)).await.starts_with("HTTP/1.1 200"));
        }

        #[tokio::test]
        async fn what_the_model_wrote_is_escaped_on_the_page() {
            let g = gate(DEFAULT_TTL);
            g.propose("<script>alert(1)</script>", json!({"name": "<img src=x onerror=y>"}))
                .await
                .unwrap();
            let (port, key) = g.page_address().await;
            let page = raw(port, get(port, &format!("/?k={key}"), &format!("127.0.0.1:{port}"))).await;
            assert!(!page.contains("<script>alert") && !page.contains("<img src=x"), "{page}");
            assert!(page.contains("&lt;script&gt;") && page.contains("&lt;img"));
        }

        #[tokio::test]
        async fn a_click_approves_and_a_foreign_origin_or_missing_key_does_not() {
            let g = gate(DEFAULT_TTL);
            let p = g.propose("a", json!(1)).await.unwrap();
            let (port, key) = g.page_address().await;
            let host = format!("127.0.0.1:{port}");
            let body = format!("k={key}&id={}&hash={}&decision=approve", p.id, p.hash);
            let foreign = raw(port, post("/decide", &host, Some("https://evil.example"), &body)).await;
            assert!(foreign.starts_with("HTTP/1.1 403"), "{foreign}");
            let nokey = raw(
                port,
                post("/decide", &host, None, &format!("id={}&hash={}&decision=approve", p.id, p.hash)),
            )
            .await;
            assert!(nokey.starts_with("HTTP/1.1 403"));
            assert_eq!(g.status(&p.id).unwrap().phase, Phase::Pending);
            let ok = raw(port, post("/decide", &host, Some(&format!("http://{host}")), &body)).await;
            assert!(ok.starts_with("HTTP/1.1 303"), "{ok}");
            assert_eq!(g.status(&p.id).unwrap().phase, Phase::Approved);
        }
    }
}
