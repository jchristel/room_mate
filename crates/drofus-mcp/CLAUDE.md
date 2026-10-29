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

## Not decided yet (ask, do not guess)

The dRofus API's address form, authentication, versions and rate limits; which operations the first version exposes;
how credentials are supplied in production. `config.rs` holds only a base address and a key as placeholders.
