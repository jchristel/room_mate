# drofus-mcp

An [MCP](https://modelcontextprotocol.io) server for dRofus, over stdio. A client (Claude Desktop, Claude
Code, any MCP host) starts the binary and calls its tools.

**Status: scaffold.** One tool, `server_info`, which says what the server is and what it has not been told yet
(it makes no network call). The dRofus tools are still to be written.

## Standalone by design

This folder depends on nothing else in the repository it lives in: no path dependency, no shared crate, its own
lint table. To fork it, copy `crates/drofus-mcp` out and run `cargo build`; nothing needs editing. Anything it
wants from the roommate server is copied, never imported.

## Configure

| What | Flag | Environment |
|---|---|---|
| dRofus base address | `--base-url` | `DROFUS_BASE_URL` |
| Credential | `--api-key` (shows in the process list: prefer the environment) | `DROFUS_API_KEY` |

A flag wins over the environment. Nothing is read from a file in the repository. A credential is wrapped in
`config::Secret`, which never prints.

## Use

```
cargo run -p drofus-mcp -- --base-url https://drofus.example.com
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
- `src/server.rs`: the tools, one method each, over plain functions that know nothing about MCP (so tests call them
  without a transport).
- `src/main.rs`: parse flags, serve over stdio.

## Checks

From the repository root: `cargo test -p drofus-mcp`, `cargo clippy -p drofus-mcp --all-targets -- -D warnings`,
`cargo fmt --check`.
