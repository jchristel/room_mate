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

**Windows proved something sharper: a description can be false without a single
word of it changing.** The window record is identical to the door record, so
`get_windows` could have reused `get_doors`' text verbatim and every sentence in
it would still have parsed as true. It would still have been wrong, because the
*data* differs exactly where it matters. A one-sided opening is the exception
for doors and the rule for windows; an opening with no room on either side is a
finding for doors and, in a facade model that links its interiors, the state of
every window in the file — 0 of 158 carried a reference when it was measured,
and so did 0 of its 191 doors. An agent applying the doors reading to that would
report a correct model as broken.

So the test for a description is not "is it accurate about the shape" but **"does
it lead a reader to the right conclusion about the data"**. Where two entities
share a shape and differ in distribution, the second description has to say so
in as many words, and point at the setting that changes the answer —
`room_resolution`, whose `off` is not the same as clean.

**FF&E tested the opposite case and it was easier, which is worth recording so
the windows lesson is not over-generalised.** An item is a *different record*
from an opening — one room rather than two sides, a category, a component id,
and no footprint at all until the exporter grows one — so `get_ffe` could not
have been written by copying anything, and the ways it misleads are visible in
the shape rather than hidden in the distribution. The hard half was elsewhere:
the description has to say that `excluded_components` being large is **normal**
(179 of 647 on the measured model), and that the two tests an agent would reach
for to identify a component both fail, because a component names a room 97.8% of
the time. A number that looks alarming and is not needs the same defending as a
distribution that looks clean and is not.

**Spaces are the case where the misleading reading is a *count*, and neither of
the two before it prepares a reader for that.** A space is not an opening and
not quite a room, but the shape is not what an agent will get wrong. Two things
about the data are:

- **The same room number legitimately names a space in every services model.**
  One file per service means four spaces numbered `ENG137` is four disciplines,
  not a duplicate — and an agent that reports it as one is reporting the
  project's structure as a defect. Measured on RHH: pooled across models, 3,046
  keys looked duplicated; per model, 31 actually were. So `get_spaces`' text has
  to say *read `model_id` before concluding anything about duplication*, in as
  many words.
- **An empty answer is a finding, not an absence.** Zero spaces for a model
  means it was audited and holds none — which is the entity's whole first
  requirement — and it is a *different fact* from the model never having been
  pushed, which is what the no-data reply means. Two emptinesses that mean
  opposite things is exactly the distinction a description has to carry, because
  nothing in the response shape does.

And one about the phase: **the disciplines do not agree on what the phase is
called.** A services model reporting `Future` beside siblings on
`New Construction` is ordinary, so the description points at `phase_by_model`
before an agent reads a low count as missing data. That is the windows lesson
again — a correct model that reads as broken — arriving through a different
door.

