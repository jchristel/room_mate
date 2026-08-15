# Roommate — MCP

Part of the Roommate strategy docs: [Index](STRATEGY.md) ·
[Sources](STRATEGY-SOURCES.md) · [Server](STRATEGY-SERVER.md) ·
[Browser](STRATEGY-BROWSER.md) · [Authored](STRATEGY-AUTHORED.md) ·
[Entities](STRATEGY-ENTITIES.md) · [Security](STRATEGY-SECURITY.md)

**Open work only.** The stdio MCP server is built and its rationale lives in
`src/bin/mcp.rs`'s module header — the tool set, the stderr-only logging rule,
the `ServiceError` → `McpError` mapping, and why settings tools are read-only.
Host wiring is in [mcp-host-setup.md](mcp-host-setup.md).

## Deferred

- **No resources or prompts exposed, only tools.** That matches the original
  "the read side as tools" scope rather than falling short of it. Worth
  revisiting if a client wants to browse stored snapshots as *resources* instead
  of calling a tool per query — nothing today motivates it.

- **Document tools**, when [Authored](STRATEGY-AUTHORED.md) is built:
  `search_documents`, `get_document_page`, `list_documents`. All three read-only,
  so they stay inside the existing boundary. Designed there, not here.

## The rule a new read route inherits

`bin/mcp.rs` keeps **one tool per HTTP read route**, and its header states the
count. A route added without its tool is how the two front doors drift, and the
drift is invisible from either side alone — which is why
`scripts/weekly_review.py` checks the claim on every run.

The corollary is the part that does not come for free: **a tool description is
the only documentation an agent reads, so a stale one is a wrong answer, not a
doc debt.** When a change makes a description false, correcting it belongs in
that same change. Doors proved both halves — the entity cost one new tool and
three corrected descriptions, and the descriptions were the work.
