# drofus-mcp

An [MCP](https://modelcontextprotocol.io) server for dRofus, over stdio. A client (Claude Desktop, Claude
Code, any MCP host) starts the binary and calls its tools.

**Status: the read endpoints are in, authenticated by an API key.** 89 tools cover every `GET` and read-only `OPTIONS`
operation of the dRofus Project API (`ENDPOINTS.md`), plus `server_info`. No write endpoint has a tool. Every dRofus
tool sends `Authorization: Reference <key>` (the API key from dRofus's Power Query credentials dialog, as
`DROFUS_API_KEY`); with no key it stops with an error saying so, before any request is sent. See
[`AUTH.md`](AUTH.md): the scheme is the one the duHast PushIt client uses, and has not been tried against the live
service from here.

## The tools

Named `drofus_<group>_<what>`: `drofus_rooms_list`, `drofus_rooms_get`, `drofus_rooms_files`,
`drofus_rooms_image` (an image content block, refused above 1 MiB with its size), `drofus_rooms_image_meta`,
`drofus_rooms_properties` (the `OPTIONS` metadata read). Every one takes optional `db` and `pr`.

- **List tools** take `select`, `orderby`, `filter`, `top` and `skip` (the dRofus `$` options without the `$`, which
  some MCP clients reject in a property name) and pass them through unchanged. `top` defaults to 50 and is lowered to
  200; the answer says when it was and, when a page is full, which `skip` asks for the next.
- **Get tools** take `select`. Any text answer over 256 KiB is refused with advice to narrow it.

The table behind them is `src/endpoints/table.rs`, generated from the OpenAPI definition by `gen_endpoints.py`
(which also writes `operations.txt`, the list a test compares the tools against).

## Writes: none, and a gate waiting for the first one

No tool writes. `src/approval` is the gate any future write tool must pass, built first: a proposal is a **dry run**
(it returns an id and a hash and changes nothing), a person approves it on a **local page on 127.0.0.1** that opens
in the browser (the address and key never appear in a tool answer, so the model cannot click it), and only then can it
be applied, **once**, from the stored payload. What shows the change to a person is an `Approver` (a trait: the
web page today, a native window later) and the gate's rules do not depend on which. It is off unless `--allow-writes` is given, which also needs
`--audit-log <file>`. Its limits are written in the module header.

## Standalone by design

This folder depends on nothing else in the repository it lives in: no path dependency, no shared crate, its own
lint table. To fork it, copy `crates/drofus-mcp` out and run `cargo build`; nothing needs editing. Anything it
wants from the roommate server is copied, never imported.

## Configure

| What | Flag | Environment |
|---|---|---|
| dRofus API base address | `--base-url` | `DROFUS_BASE_URL` |
| Database (`/api/{db}/...`) | `--database` | `DROFUS_DATABASE` |
| Project (`/api/{db}/{pr}/...`) | `--project` | `DROFUS_PROJECT` |
| API key (`Authorization: Reference`) | `--api-key` (shows in the process list: prefer the environment) | `DROFUS_API_KEY` |

A flag wins over the environment. Nothing is read from a file in the repository. A credential is wrapped in
`config::Secret`, which never prints.

## Use

```
cargo run -p drofus-mcp -- --base-url https://api-no.drofus.com --database <db> --project <pr>
```

An MCP client entry (for example `.mcp.json`) looks like:

```json
{
  "mcpServers": {
    "drofus": {
      "command": "target/debug/drofus-mcp",
      "env": { "DROFUS_BASE_URL": "https://drofus.example.com", "DROFUS_API_KEY": "..." }
    }
  }
}
```

Logs go to stderr; stdout is the transport.

## Layout

- `src/config.rs`: flags and environment to a `Config`, secrets kept out of every `Debug`.
- `src/endpoints.rs` (+ generated `endpoints/table.rs`): the operations, tool schemas, and arguments to request.
- `src/client.rs`: the one HTTP path, error mapping, size caps.
- `src/auth.rs`: the `Authenticator` seam, `ReferenceKey` (the API key) and `Unauthenticated` (no key).
- `src/approval/`: the write gate; `Approver` is the trait a surface implements, `WebApprover` is the local page.
- `src/server.rs`: registers the tools and adapts answers to MCP.
- `src/main.rs`: parse flags, serve over stdio.

## Checks

From the repository root: `cargo test -p drofus-mcp`, `cargo clippy -p drofus-mcp --all-targets -- -D warnings`,
`cargo fmt --check`.
