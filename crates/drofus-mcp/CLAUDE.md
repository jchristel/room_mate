# drofus-mcp: working notes for a session on this crate

This crate is an MCP server for dRofus and **must stay forkable**: `crates/drofus-mcp` can be copied out into its
own repository with no edit.

## Rules that are expensive to get wrong

- **Touch nothing outside `crates/drofus-mcp`**, except adding the crate to the root workspace (already done). Do
  not import `roommate` or `roommate-shared`, add a path dependency, or use `workspace = true` inheritance. If you
  want something the roommate server has, copy it.
- **Every dependency is spelled out in this crate's `Cargo.toml`** with its own version.
- **No secret in the repository, in a log line or in a tool's answer.** Credentials are `config::Secret`; the only
  way to read one is `expose()`, at the one place that sends it. Tests assert a secret never appears in `Debug` or in
  a tool answer.
- **stdout is the JSON-RPC transport.** Log to stderr only. A stray `println!` corrupts the protocol.
- **Tools are read-only until there is a reason, said in writing, for one that writes.** A tool that changes dRofus
  data is a decision for the person who owns the project.
- **One tool per method over a plain function that knows nothing about MCP**, so a tool is tested without a transport
  (see `server.rs`). A tool's `description` is what a model chooses it by: say what it returns, what it needs and
  what it leaves out.
- **A missing setting is a finding a tool reports, not a reason for the process to exit**, so a client can connect
  and ask what is missing (`server_info`).

## House style (the parent project's, kept here so a fork does not lose it)

- Tests are inline (`#[cfg(test)]`), never a `tests/` tree.
- Doc comments carry the RATIONALE, not a restatement of the what.
- Line endings are LF. A module past about 500 real lines is a split candidate.

## Before saying it is done

From the repository root, all three, with clippy at `-D warnings`:

```
cargo test -p drofus-mcp
cargo clippy -p drofus-mcp --all-targets -- -D warnings
cargo fmt --check
```

Then drive it over stdio (an `initialize`, `tools/list` and `tools/call` on stdin; see the README) rather than
trusting the unit tests alone.

## Current task: the read endpoints, with authentication scaffolded and NOT implemented

**Do this and only this.** Implement MCP tools for the dRofus Project API's **read** endpoints. The inventory is
[`ENDPOINTS.md`](ENDPOINTS.md): 73 `GET` endpoints in 17 groups, plus 16 read-only `OPTIONS` metadata endpoints (which properties can
be selected). **No write endpoint** (`POST`, `PATCH`, `DELETE`) gets a tool, now or as a convenience.

Source of truth: the OpenAPI definition at `https://api-no.drofus.com/swagger/v1/swagger.json` (public: read it with `curl`, no sign-in).
The Swagger page `https://api-no.drofus.com/swagger/index.html` renders with JavaScript, so a plain fetch of the page shows nothing: use
Playwright for the page itself (for example to see what its **Login** button does) and `curl` for the JSON. The definition declares **no
security scheme**, so it does not say how a request is authenticated.

### What to build

- **Every path is `/api/{db}/{pr}/...`** (database, project). Take `db` and `pr` from configuration (`--database` / `DROFUS_DATABASE`,
  `--project` / `DROFUS_PROJECT`) and let each tool override them with optional `db` and `pr` parameters. Say so in `server_info`.
- **List endpoints take `$select`, `$orderby`, `$filter`, `$top`, `$skip`; `/{id}` endpoints take `$select`.** Expose them as optional
  tool parameters and **pass them through unchanged**: never parse or rewrite an OData filter. Describe `$filter` and `$orderby` the way
  the definition does, with its own example.
- **Bound every list answer.** Apply a default `$top` (and a ceiling) so a tool cannot return a whole table by accident, and say in the
  answer when it was capped and how to ask for the next page. A model's context is the scarce resource.
- **Binary endpoints (images) never put raw bytes in a text answer.** Return the `.../meta` JSON, and for the image itself return an MCP
  image content block under a size cap, refusing above it with a message that says the size. Decide the cap in one named constant.
- **One tool per endpoint, named `drofus_<group>_<what>`** (`drofus_rooms_list`, `drofus_rooms_get`, `drofus_rooms_files`,
  `drofus_rooms_properties` for an `OPTIONS` metadata read). Each description says what it returns, what it needs and what it leaves
  out. Do not write 90 copies of the same method by hand: put the request building in a few generic functions
  (list, get by id, sub-resource, binary, metadata) and generate the thin tool methods from a table with a macro, keeping each tool's own
  description.
- **HTTP**: `reqwest` with `default-features = false` and rustls, a timeout, and no automatic retry. Map `401` and `403` to a tool error
  that says authentication is required or refused, `404` to "not found", and anything else to the status plus a truncated body. **Never put
  the `Authorization` header, or anything from it, in an error, a log line or an answer.**
- **Test against a local mock, not the live service**: a small in-process HTTP server in the tests (a dev-dependency is fine) that checks the
  URL built, the query passed through, the paging cap, the error mapping and that a secret never appears. There is no test database.

### Authentication: scaffold the seam, implement nothing

Leave a clear place for it and no behaviour:

- A trait (for example `Authenticator`) that, given a request being built, may add what authenticates it, owned by the HTTP client in
  one place.
- One implementation, `Unauthenticated`, which makes every call fail **before any network call** with an error that says authentication is
  not implemented yet and points at the module to fill in. The tools therefore all work end to end up to that error.
- A `config` slot for whatever credentials turn out to be needed, still as `Secret`s. **Do not guess the scheme.** Do not add a login flow,
  a token cache, a refresh, a bearer token from an environment variable, or user name and password handling. The Swagger page's Login goes to
  an interactive sign-in (user name and password, or Microsoft single sign-on); that is a design decision for the owner, not for you.
- A short `AUTH.md` in the crate listing what you learned about the sign-in (what Login does, any token endpoint you can see from the
  page, what is unknown) so the next session can decide.

### Done means

All three gates from the repository root, `check_drofus_standalone.py` still passing, every tool in `ENDPOINTS.md` present (a test that
compares the tool list with a checked-in list of operations, so a missing one fails), and a drive over stdio (`initialize`, `tools/list`,
a `tools/call` that stops at the authentication error). Report what you did not do.

## Still undecided (ask, do not guess)

How credentials are obtained and supplied in production (see AUTH.md once written), whether a regional host other than `api-no.drofus.com` is needed (the base address is configuration), and which operations beyond reads are ever wanted.
