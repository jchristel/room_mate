# RoomMate — Documentation

Design and strategy documentation for the RoomMate application, capturing the
design decisions behind the Revit → Rust → browser room data pipeline.

## Strategy

| Document | Description |
|---|---|
| [Architecture & Strategy](STRATEGY.md) | Index and overview of the design decisions |
| [Browser](STRATEGY-BROWSER.md) | Browser front-end strategy |
| [Server](STRATEGY-SERVER.md) | Rust server strategy |
| [Area calculation](STRATEGY-AREA-CALCULATION.md) | How rooms become per-tier areas: boundary regimes, the wall zone, what a wall belongs to, and the relationship to IPMS 3 / DIN 277 |
| [Sources](STRATEGY-SOURCES.md) | Data sources strategy |
| [MCP](STRATEGY-MCP.md) | Model Context Protocol integration strategy |
| [Authored](STRATEGY-AUTHORED.md) | User-authored data & documents — connections, PDFs, hierarchy scopes _(design settled, not built)_ |

## Implementation notes

| Document | Description |
|---|---|
| [Coding Conventions](CODING-CONVENTIONS.md) | The engineering rules this codebase follows (module structure, testing, dependency direction, error stance) |
| [Plan: handover actioning](PLAN-handover-actioning.md) | Review of the open handovers against strategy, priorities, and the ordered plan (with per-item landed status) |

## Open handovers

| Document | Status |
|---|---|
| [UI layout restructure](HANDOVER-ui-layout.md) | Every sequencing step built; only Decision 3 (the room inspector) remains — **no longer blocked**: room click-selection landed with the adjacency graph |
| [Room adjacency graph](HANDOVER-adjacency.md) | Built and tested; one validation item left. **Its stated blocker is now stale** — it says "every fixture in the repo is generated centreline", but House A is real finish-face data, so the run it asks for is doable today: confirm no corridor-bridging and no bridging through a thin service room, and record the tolerance that worked |

Handoff documents whose work has fully landed live in
[Superseded](Superseded/) — most recently
`HANDOVER-areas-boundary-location.md` and
`handover-hierarchical-void-closure.md`.

Area calculation no longer has a handover: its design moved into
[Area calculation](STRATEGY-AREA-CALCULATION.md), which is the live document and
carries the remaining open items in its "Open" section. The handover stays in
`Superseded/` as the record of how the decisions were reached.
