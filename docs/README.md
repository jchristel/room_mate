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

**One.** Everything else has landed and moved to [Superseded](Superseded/).

| Document | Status |
|---|---|
| [Room adjacency graph](HANDOVER-adjacency.md) | Built and tested; **one validation item left**, and its stated blocker is stale. It says "every fixture in the repo is generated centreline" — but House A is real finish-face data, so the run it asks for is doable today: confirm no corridor-bridging and no bridging through a thin service room, and record the tolerance that worked so the default can be baked in |

Handoff documents whose work has fully landed live in
[Superseded](Superseded/) — most recently `HANDOVER-ui-layout.md` (every
decision built, the inspector last), `HANDOVER-room-inspector.md` and
`HANDOVER-culling-disable-switch.md`.

Two of those are worth knowing about even though they are superseded, because
each leaves something recorded rather than pending:

- **[Room inspector](Superseded/HANDOVER-room-inspector.md)** — step 6, a
  checkbox property picker, was deliberately not built. Hide-empty plus the name
  filter covered the cases it was for. Act on it only if the need shows up.
- **[Viewport culling kill switch](Superseded/HANDOVER-culling-disable-switch.md)**
  — the switch is permanent (`CULL_ENABLED` in `index.html`) and that document is
  the method for re-measuring whenever the renderer changes. Last run:
  **16.5 ms/frame with culling, 912 ms without.**

Area calculation no longer has a handover: its design moved into
[Area calculation](STRATEGY-AREA-CALCULATION.md), which is the live document and
carries the remaining open items in its "Open" section. The handover stays in
`Superseded/` as the record of how the decisions were reached.
