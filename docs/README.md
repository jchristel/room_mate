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
| [Security](STRATEGY-SECURITY.md) | Threat model for the LAN-reachable deployment: trust boundary, invariants, settings backups, rate limiting |

## Implementation notes

| Document | Description |
|---|---|
| [Coding Conventions](CODING-CONVENTIONS.md) | The engineering rules this codebase follows (module structure, testing, dependency direction, error stance) |
| [Plan: handover actioning](PLAN-handover-actioning.md) | **Closed — P1 through P10 all landed**, and all three handovers it reviews are in `Superseded/`. Kept as the record of how the UI restructure was sequenced and what was deliberately not built; it is history, not a work list |

> **Reading either document above: do not trust their `file.rs:NNN` deep
> links.** Both pin line numbers, and the files have moved underneath them —
> spot-checked 2026-07-26, most now land on unrelated code (e.g.
> `areas.rs:270` was cited as "`/areas` has no `revision` field" and is now
> `wall_zone`). The *file* and the *symbol name* in the surrounding prose are
> still right; search for the symbol rather than jumping to the line. New
> cross-references should name the symbol, not the line.

## Open handovers

**Two.** Everything else has landed and moved to [Superseded](Superseded/).

| Document | Status |
|---|---|
| [Room adjacency graph](HANDOVER-adjacency.md) | Built and tested. **Two false-positive checks left** — that the graph does not link rooms across a corridor, or through a thin service room — and they need a hospital-scale finish-face export this repo does not have. House A cannot settle them: a `wall_max` sweep on it saturates at 1.5 ft, so there is nothing at corridor distance to wrongly bridge. The item's other three asks are done or moved to [Area calculation](STRATEGY-AREA-CALCULATION.md) |
| [QA cardinality & unmatched coverage](HANDOVER-qa-cardinality-and-coverage.md) | Not started, two items. **Duplicate link values are unconditionally treated as ambiguous**, which hardcodes a 1:1 assumption a bucket-shaped source (a hardware schedule) breaks; needs a per-source `link_cardinality`. And **unmatched is only checked room→source**: nothing reports source records with no room, so a 200-row CSV against 50 rooms reports zero unmatched and reads as clean. The second is smaller and independent — do it first |

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
