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
| [Comparison sources](HANDOVER-comparison-sources.md) | Steps 1–3, 5–6 landed; step 4 (source-aware comparator) open |
| [UI layout restructure](HANDOVER-ui-layout.md) | Step 4 (server label set) and step 6 (labels toggle) landed; the restructure itself unbuilt |

Handoff documents whose work has fully landed live in
[Superseded](Superseded/), including the former
`settings-infrastructure-handoff.md` and `HANDOVER-area-label-sizing.md`.
