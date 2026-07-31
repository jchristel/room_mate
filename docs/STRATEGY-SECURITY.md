# Roommate — Security & Threat Model

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Sources](STRATEGY-SOURCES.md) · [Server](STRATEGY-SERVER.md) ·
[Browser](STRATEGY-BROWSER.md) · [MCP](STRATEGY-MCP.md) ·
[Authored](STRATEGY-AUTHORED.md) · [Entities](STRATEGY-ENTITIES.md)

What this server defends against, what it deliberately does **not**, and the
code-level invariants that follow. Separate from [Server](STRATEGY-SERVER.md)
because it's a different axis: that doc says "here's a capability and why it's
shaped this way"; this one says "here's what we assume an attacker can reach and
what must stay true regardless." Deployment mechanics (firewall, container,
volume snapshots) are ops, not code strategy — they live in
[mcp-host-setup.md](mcp-host-setup.md), not here.

## The deployment shift that motivates this doc

Today the server binds **loopback only** (`DEFAULT_HTTP_ADDR =
"127.0.0.1:5151"` in `lib.rs`). Nothing off the host can reach it; "access
control is the `127.0.0.1` bind" (as [Server](STRATEGY-SERVER.md)'s settings-API
and ingest notes put it) is the entire story, and no attacker exists to model.

The near-future change is to operate the server on a **dedicated machine or
container, reachable across a trusted local network** — bind widened to a LAN
interface (or `0.0.0.0` inside a container published to one). **That bind change
is what creates the threat surface this doc addresses.** The two states are
mutually exclusive: a loopback server has no LAN attacker, a LAN server no
longer has the loopback bind as its access control. Everything below assumes the
LAN posture; none of it is load-bearing while the bind is still loopback, and it
should ship *with or before* that bind change, not after.

## Trust boundary

- **External network: out of scope.** The machine/container is not reachable
  from outside the local network — enforced at the host/network layer (firewall,
  Docker publish scope), not in this process. This process assumes it never sees
  a packet from the public internet.
- **Local network: semi-trusted.** Anyone on the LAN can reach the port. We do
  **not** assume every LAN user is friendly. The modelled adversary is a
  *hostile user already on the local network*: they can send any HTTP request to
  any route, as many times as they like.
- **No authentication — deliberate, for now.** There are no logins, no tokens,
  no per-user identity. Every request is anonymous and equally authorized. This
  is a conscious *current* choice (small trusted LAN, no auth infrastructure to
  run yet), **not** a claim that auth is unnecessary forever — it's the first
  thing to add when the LAN stops being trustworthy or the data stops being
  recreatable. Recorded here so its absence reads as a decision, not an
  oversight.
- **Filesystem: not a client surface.** A LAN *client* reaches only HTTP; it has
  no path to the projects dir or the store except through a route. So the real
  exposure is precisely the mutating routes, and that's where the defences below
  sit. Direct filesystem protection (owner-only perms on the projects and store
  dirs, non-root process) is an ops concern — see mcp-host-setup.md.

## What the attacker can and cannot do — the invariants

The stated assumption is: **a hostile user can, at worst, damage settings; they
cannot destroy a project or its snapshots.** That is currently true, but largely
by accident of the HTTP surface rather than by design. This section makes it an
invariant so a future route can't quietly void it.

- **No route deletes a project or a snapshot.** There is deliberately no
  `DELETE` handler for projects, models, or snapshots (the "snapshot delete UI"
  in [Server](STRATEGY-SERVER.md)'s Deferred list is unbuilt, and when built must
  respect this section). The store accumulates; nothing over HTTP removes room or
  snapshot data. **This is the single most important invariant here** — the whole
  "can dent settings, can't destroy the record" stance rests on it. Any new
  mutating route is measured against it before it ships.
- **Settings *can* be changed or damaged over HTTP** — that's the `PUT`/`POST`
  on `/api/settings/*` and the dRofus upload, by design (the settings UI is the
  supported way to edit them). So settings are the one thing a hostile user can
  harm, and therefore the one thing that must be **recoverable** — see Backups
  below.
- **A rejected settings save leaves the prior file untouched** (existing
  behaviour, [Server](STRATEGY-SERVER.md)'s settings-API note): the candidate is
  validated through the full startup pipeline as a temp file and only
  atomically renamed into place on success. A hostile *malformed* save can't
  corrupt the live file; it's rejected before install.
- **Ingest can add snapshots but not overwrite one.** A duplicate `taken_at` is
  skipped-with-warning, never overwritten (existing store behaviour). So a
  hostile pusher can *append* history and can flood (see Rate limiting) but
  cannot rewrite or erase an existing snapshot.

The net: the attacker's maximum achievable damage is (a) mangling settings —
undone by backups — and (b) volume: flooding the server with writes — bounded by
rate limiting. Neither reaches the irreplaceable data.

## Settings backups — every change is rollback-able

Because settings are the one hostile-reachable mutable surface, **every accepted
settings write first snapshots the file it's about to replace**, so any change
(hostile or merely mistaken) can be rolled back.

- **Where it hooks in:** `settings_api::save_project`, immediately before the
  atomic `std::fs::rename` that installs the new file — the point where the
  prior file still exists on disk and the new one has already passed full
  validation. The same applies to `upload_drofus` if CSV rollback is wanted;
  both already run under `SAVE_LOCK` and share `reload_and_swap`, so the backup
  step rides the same serialized path and can't race.
- **Backups are copies, not moves, and never overwritten.** The live file must
  stay in place until the atomic rename actually installs its replacement, so
  the prior version is *copied* to a timestamped name under a `.backups/`
  subdirectory of the projects dir (`.backups/<id>.<rfc3339-utc>.toml`). A
  timestamp keyed the same way as snapshot ids (UTC, lexically sortable) means
  newest-is-lexical-max here too, and no two backups of one project collide.
- **Retention is a prune, not a cap-at-write.** Keep the last N per project
  (prune older ones after a successful install); a flood of hostile saves is
  bounded by the rate limiter below, so the backup dir can't grow without
  limit even under attack.
- **Restore is out-of-band for v1.** Rolling back is "copy a `.backups/` file
  over the live one and let the next save/boot pick it up," done by an operator
  on the box — deliberately *not* an HTTP route, since a restore endpoint would
  itself be a hostile-reachable mutation and would need the same scrutiny as
  delete. A restore UI can come later behind auth.
- **Scope, stated honestly:** this defends against settings being *edited or
  mangled*. It does **not** defend the projects dir being deleted wholesale —
  that's a filesystem-permissions/backup concern (owner-only dir, host-level
  volume snapshots), not something this process can undo. Consistent with the
  trust boundary: a LAN client can't reach the filesystem, so "dir deleted" is
  not in the HTTP threat model anyway.

## Rate limiting — bound the write flood

A hostile user can send unlimited requests. The bound: **on the mutating routes,
refuse a client after X writes in a rolling window, then lock that client out
for 24 hours.**

- **Scope: mutating routes only.** The limiter guards `POST /rooms`,
  `POST /rooms/stream`, the settings `POST`/`PUT`, and `POST
  /projects/{id}/drofus` — the routes that cost storage or CPU. Reads
  (`GET /rooms`, the viewer's 2s poll, etc.) stay unlimited, or on a much looser
  limit, so one client's flood can never lock legitimate viewers out of *seeing*
  data. The asymmetry is the point: writes are the scarce, damageable resource.
- **Client identity is the socket IP.** With no auth and no trusted proxy in
  front, the only honest identity is the peer address from `ConnectInfo<
  SocketAddr>` — *not* a forwarded header, which a hostile client sets freely.
  This is coarse (NAT lumps clients; a container network may collapse peers) but
  it's the truthful unit on a bare LAN, and it degrades safely: worst case a
  shared address is throttled as one.
- **Window then ban, two thresholds.** X requests per rolling window trips the
  limit; tripping it starts a 24h ban for that IP. A `tower`/`tower_governor`
  sliding window handles the per-window count; a small `Mutex<HashMap<IpAddr,
  ..>>` in shared state holds the ban expiry. Middleware
  (`from_fn_with_state`) on the mutating sub-router checks the ban first, then
  the window, and returns **429 with a `Retry-After`** on refusal.
- **Two honest limitations, both acceptable here.** The ban map grows with
  distinct offending IPs — prune expired entries on access so it can't leak. And
  it's in-memory, so a restart clears bans — fine for a flood defence (the flood
  has to restart too), and persisting bans would mean writing attacker-controlled
  data to disk, which we'd rather not. If bans ever need to survive restart,
  persist them deliberately, not by default.
- **Every trip is logged at `warn`** with the offending IP, so abuse is visible
  even without logins — the observability substitute for an audit trail we don't
  have yet.

Sizing X is a tuning question against real push cadence (a legitimate multi-model
project push is a burst of writes), not a security constant — set it well above
the busiest honest client and revisit from the `warn` logs.

## The browser is the local attacker — two controls, both required

Binding `127.0.0.1` does not mean "only things you trust can call this." Your
browser runs locally, so a page you merely *visit* can address the server. Two
separate controls answer two separate roads in, and each is blind to the other's:

- **`read_only_cors`** grants a cross-origin caller `GET`/`HEAD` and nothing
  else. It stopped a real hole: under the previous `CorsLayer::permissive()`, a
  hostile page could `POST /api/settings/drofus-check {"path":
  "C:/Windows/win.ini"}` and read that file back. (That endpoint no longer
  exists — see below — but the policy it motivated still guards every other
  write route.)
- **`guard_host`** rejects any request whose `Host` header is not a loopback
  name. This covers **DNS rebinding**, which CORS structurally cannot: the
  attacker serves a page from `evil.example` on a one-second DNS TTL, then
  re-answers that name as `127.0.0.1`. The page's fetches are now *same-origin*
  — no preflight, no `Origin` header, the CORS layer never consults anything.
  The `Host` is what the attacker cannot forge, because the browser fills it
  with the name the page asked for. It is layered outside CORS so it runs first.

A request with no `Host` at all is allowed: HTTP/1.1 requires it and every
browser sends it, so absence means a non-browser client (`curl`, the pyRevit
pusher, the MCP binary's HTTP client), which is not the threat. Nothing a
rebinding attacker controls can omit it.

### The server no longer reads a file path on the caller's behalf

The settings API used to accept a CSV path two ways: `type = "file"` on a
reference source (read at every boot) and `POST /api/settings/reference-check`
(a dry-run of that path, for the UI's "check" button). Together they made the
server an **unauthenticated arbitrary-file-read oracle** for anything that
could reach loopback.

Path validation was considered and rejected as theatre — a settings file could
legitimately name an absolute path (a CSV on a network share), and confining
the *preview* endpoint while the *save* endpoint accepted the same paths would
only have moved the same read one call to the left. Anyone who could reach
`POST /api/settings/projects` could write a settings file naming any path and
read it back through `/rooms`.

**Both are now gone.** `ReferenceOrigin::File` is removed and reference data
arrives only by upload, so no path from a settings file is ever opened, and
`reference-check` — whose only job was to dry-run such a path — was deleted
with it. The class is closed by deletion rather than by validation, which is
the stronger form: there is no longer a path for a check to get wrong.

What remains is the store's own tree, whose components are ids the server
generated or validated (`is_path_safe_component`), never free text a caller
supplies as a path.

## Belt-and-braces already in place

Not new, but part of the same posture, noted so they're not re-litigated:

- **Body limits on every ingest route** (`ROOMS_BODY_LIMIT_BYTES`,
  `REFERENCE_BODY_LIMIT_BYTES` in `main.rs`; the streaming route bounds memory by
  reading line-by-line instead). One giant body can't OOM the process — a flood
  variant the request-count limiter alone wouldn't catch.
- **Snapshot ids are structurally path-safe** (RFC3339 UTC, can't contain `/`,
  `\`, or `..` — [Index](STRATEGY.md) "The upload envelope"), and
  `is_path_safe_component` guards project ids before they touch a filename. A
  hostile id can't escape the store dir.
- **MCP stays read-only** ([MCP](STRATEGY-MCP.md)): the second front door
  mutates nothing, so widening deployment doesn't widen the write surface. Its
  port must not be exposed off-host either — again ops, not code.

## What this doc will grow into

The moment the LAN stops being trustworthy, or the data stops being cheap to
recreate, the missing piece is **authentication** — per-user identity, which
turns the coarse per-IP rate limit into a per-principal one, turns "settings are
anonymously editable" into "editable by authorized users," and makes a restore
(and eventually a delete) endpoint safe to expose. That's the next section to
write here, not a change to any invariant above: auth *tightens* the trust
boundary, it doesn't move the can't-destroy-a-project line, which holds with or
without logins.
