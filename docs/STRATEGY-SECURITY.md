# Roommate — Security & Threat Model

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Sources](STRATEGY-SOURCES.md) · [Server](STRATEGY-SERVER.md) ·
[Browser](STRATEGY-BROWSER.md) · [MCP](STRATEGY-MCP.md) ·
[Authored](STRATEGY-AUTHORED.md) · [Entities](STRATEGY-ENTITIES.md)

What this server will have to defend against once it moves off loopback, what it
deliberately will **not**, and the invariants that must hold either way.

Unlike the other strategy docs this one is *mostly* unbuilt by design: it
describes a posture for a deployment that has not happened yet. The controls
that already exist — the CORS policy, the `Host` guard, per-route body limits,
path-safe id components — carry their own rationale in `src/main.rs` and
`src/state.rs` and are not restated here. Deployment mechanics (firewall,
container, volume snapshots) are ops, not code strategy.

## The deployment shift that motivates this doc

Today the server binds **loopback only** (`DEFAULT_HTTP_ADDR` in `lib.rs`).
Nothing off the host can reach it; "access control is the `127.0.0.1` bind" is
the entire story, and no LAN attacker exists to model.

The near-future change is to operate the server on a **dedicated machine or
container, reachable across a trusted local network** — bind widened to a LAN
interface, or `0.0.0.0` inside a container published to one. **That bind change
is what creates the threat surface this doc addresses.** The two states are
mutually exclusive: a loopback server has no LAN attacker; a LAN server no
longer has the loopback bind as its access control. Everything below assumes the
LAN posture, and should ship *with or before* that bind change, not after.

## Trust boundary

- **External network: out of scope.** The machine or container is not reachable
  from outside the local network — enforced at the host/network layer, not in
  this process. This process assumes it never sees a packet from the public
  internet.
- **Local network: semi-trusted.** Anyone on the LAN can reach the port, and we
  do **not** assume every LAN user is friendly. The modelled adversary is a
  *hostile user already on the local network*: they can send any HTTP request to
  any route, as many times as they like.
- **No authentication — deliberate, for now.** No logins, no tokens, no
  per-user identity; every request is anonymous and equally authorized. A
  conscious *current* choice (small trusted LAN, no auth infrastructure to run
  yet), **not** a claim that auth is unnecessary forever. Recorded so its
  absence reads as a decision rather than an oversight.
- **Filesystem: not a client surface.** A LAN client reaches only HTTP, with no
  path to the projects dir or the store except through a route. So the real
  exposure is precisely the mutating routes, which is where the defences below
  sit. Owner-only permissions and a non-root process are ops concerns.

## The invariants an attacker must not be able to break

The assumption is: **a hostile user can, at worst, damage settings; they cannot
destroy a project or its snapshots.** That is true today largely by accident of
what the HTTP surface happens to contain. Stating it as an invariant is what
stops a future route quietly voiding it.

- **No route deletes a project or a snapshot.** There is deliberately no
  `DELETE` handler for projects, models, or snapshots — and the "snapshot delete
  UI" in [Server](STRATEGY-SERVER.md)'s deferred list, when built, must respect
  this section. The store accumulates; nothing over HTTP removes room or
  snapshot data. **This is the single most important invariant here**: the whole
  "can dent settings, can't destroy the record" stance rests on it, and every
  new mutating route is measured against it before it ships.
- **Settings *can* be changed or damaged over HTTP** — the `PUT`/`POST` on
  `/api/settings/*` and the reference upload, by design. So settings are the one
  thing a hostile user can harm, and therefore the one thing that must be
  **recoverable**.
- **A rejected settings save leaves the prior file untouched**, and **ingest can
  add snapshots but never overwrite one**. Both hold today. A hostile pusher can
  append history and can flood, but cannot rewrite or erase what is stored.

The net: maximum achievable damage is (a) mangling settings, undone by backups,
and (b) volume — flooding with writes, bounded by rate limiting. Neither reaches
the irreplaceable data. Both of those bounds are unbuilt; they are the two
sections that follow.

## Settings backups — not built

Because settings are the one hostile-reachable mutable surface, **every accepted
settings write should first snapshot the file it is about to replace**, so any
change (hostile or merely mistaken) can be rolled back.

- **Where it hooks in:** `settings_api::save_project`, immediately before the
  atomic rename that installs the new file — the point where the prior file
  still exists and the new one has already passed full validation. Reference
  uploads take the same hook if CSV rollback is wanted; both already run under
  `SAVE_LOCK` and share `reload_and_swap`, so the backup step rides the same
  serialized path and cannot race.
- **Backups are copies, not moves, and never overwritten.** The live file must
  stay in place until the rename installs its replacement, so the prior version
  is *copied* to a timestamped name under a `.backups/` subdirectory of the
  projects dir (`.backups/<id>.<rfc3339-utc>.toml`). Keying the timestamp the
  same way as snapshot ids (UTC, lexically sortable) means newest-is-lexical-max
  holds here too and no two backups of one project collide.
- **Retention is a prune, not a cap-at-write.** Keep the last N per project,
  pruning after a successful install. A flood of hostile saves is bounded by the
  rate limiter, so the backup dir cannot grow without limit even under attack.
- **Restore is out-of-band for v1.** Rolling back is "copy a `.backups/` file
  over the live one and let the next save or boot pick it up", done by an
  operator on the box — deliberately *not* an HTTP route, since a restore
  endpoint would itself be a hostile-reachable mutation needing the same
  scrutiny as delete. A restore UI can come later, behind auth.
- **Scope, stated honestly:** this defends against settings being edited or
  mangled. It does **not** defend the projects dir being deleted wholesale —
  a filesystem-permissions concern this process cannot undo, and out of the HTTP
  threat model anyway since a LAN client cannot reach the filesystem.

## Rate limiting — not built

A hostile user can send unlimited requests. The bound: **on the mutating routes,
refuse a client after X writes in a rolling window, then lock that client out
for 24 hours.**

- **Scope: mutating routes only** — the ingest routes, the settings `POST`/`PUT`
  and the reference upload, the ones that cost storage or CPU. Reads stay
  unlimited or on a much looser limit, so one client's flood can never lock
  legitimate viewers out of *seeing* data. The asymmetry is the point: writes
  are the scarce, damageable resource.
- **Client identity is the socket IP.** With no auth and no trusted proxy in
  front, the only honest identity is the peer address from
  `ConnectInfo<SocketAddr>` — *not* a forwarded header, which a hostile client
  sets freely. Coarse (NAT lumps clients, a container network may collapse
  peers) but truthful on a bare LAN, and it degrades safely: worst case a shared
  address is throttled as one.
- **Window then ban, two thresholds.** X requests per rolling window trips the
  limit; tripping it starts a 24h ban for that IP. A `tower`-style sliding window
  handles the per-window count and a small `Mutex<HashMap<IpAddr, ..>>` in shared
  state holds the ban expiry. Middleware on the mutating sub-router checks the
  ban first, then the window, and returns **429 with a `Retry-After`**.
- **Two honest limitations, both acceptable.** The ban map grows with distinct
  offending IPs — prune expired entries on access so it cannot leak. And it is
  in-memory, so a restart clears bans: fine for a flood defence (the flood has to
  restart too), and persisting them would mean writing attacker-controlled data
  to disk. If bans ever need to survive a restart, persist them deliberately.
- **Every trip logged at `warn`** with the offending IP, so abuse is visible
  without logins — the observability substitute for an audit trail we do not have.

Sizing X is a tuning question against real push cadence (a legitimate multi-model
push is a burst of writes), not a security constant. Set it well above the
busiest honest client and revisit from the `warn` logs.

## What this doc will grow into

The moment the LAN stops being trustworthy, or the data stops being cheap to
recreate, the missing piece is **authentication** — per-user identity, which
turns the coarse per-IP rate limit into a per-principal one, turns "settings are
anonymously editable" into "editable by authorized users", and makes a restore
(and eventually a delete) endpoint safe to expose. That is the next section to
write here, not a change to any invariant above: auth *tightens* the trust
boundary, it does not move the can't-destroy-a-project line, which holds with or
without logins.
