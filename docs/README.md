# RoomMate — Documentation

Design and strategy documentation for the RoomMate application, capturing the
design decisions behind the Revit → Rust → browser room data pipeline.

## Strategy

| Document | Description |
|---|---|
| [Architecture & Strategy](STRATEGY.md) | Index and overview of the design decisions |
| [Browser](STRATEGY-BROWSER.md) | Browser front-end strategy |
| [Server](STRATEGY-SERVER.md) | Rust server strategy |
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
| [Room adjacency graph](HANDOVER-adjacency.md) | Built and tested; awaiting validation against a real Revit model (which boundary regime, and the wall tolerance that works) |
| [Areas: boundary location + wall-zone partition](HANDOVER-areas-boundary-location.md) | Decisions settled; artifact fixes landed on `areas-spike-guards`, the declared boundary regime and partition reformulation not started. Read before touching `service::areas` |

Handoff documents whose work has fully landed live in
[Superseded](Superseded/) — most recently `HANDOVER-comparison-sources.md`
and `HANDOVER-area-label-sizing.md`.
