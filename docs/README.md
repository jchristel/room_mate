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

## Implementation notes

| Document | Description |
|---|---|
| [Coding Conventions](CODING-CONVENTIONS.md) | The engineering rules this codebase follows (module structure, testing, dependency direction, error stance) |
| [Plan: handover actioning](PLAN-handover-actioning.md) | Review of the open handovers against strategy, priorities, and the ordered plan (with per-item landed status) |

## Open handovers

| Document | Status |
|---|---|
| [UI layout restructure](HANDOVER-ui-layout.md) | Every sequencing step built; only Decision 3 (the room inspector) remains, blocked on room click-selection |

Handoff documents whose work has fully landed live in
[Superseded](Superseded/) — most recently `HANDOVER-comparison-sources.md`
and `HANDOVER-area-label-sizing.md`.
