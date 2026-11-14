# RoomMate — Documentation

Design and strategy documentation for the Revit → Rust → browser room data
pipeline.

**The strategy docs record outstanding work and the rules that constrain it, not
what already ships** — that lives in the module headers, which already carry the
rationale. The rule, and the ways it gets broken, are one section of
[Coding Conventions](CODING-CONVENTIONS.md): "Code documents what is built; a
strategy doc documents what is not".

## Strategy

| Document | Description |
|---|---|
| [Architecture & Strategy](STRATEGY.md) | Index, the extract-vs-process split principle, and the disciplines that keep it clean |
| [Sources](STRATEGY-SOURCES.md) | Open: an API-polled reference origin, a second producer, incremental extraction — plus why extraction cost decides the optimization axis |
| [Server](STRATEGY-SERVER.md) | Open: deferred endpoints and storage backends, an owning level above project, a coordinate datum shared across projects |
| [Area calculation](STRATEGY-AREA-CALCULATION.md) | What the area number *means* and how it relates to IPMS 3 / DIN 277, plus the open items. **Read before quoting an area to anyone external** |
| [Browser](STRATEGY-BROWSER.md) | Open: serving and consuming the placement transform, level-of-detail, generated TypeScript wire types — plus the framework decision (React, measured against Rust+WASM) and the hybrid renderer's coordinate/paint-order invariant |
| [MCP](STRATEGY-MCP.md) | Open: resources and prompts, document tools |
| [Authored](STRATEGY-AUTHORED.md) | User-authored data — connections, PDFs, hierarchy scopes. **Nothing here is built**; read it before building any of it |
| [Entities](STRATEGY-ENTITIES.md) | What makes something a primary entity, and what seven of them proved comes for free. Open: design options, the geometric space↔room check, FF&E at scale, the ceilings and floors QA reports |
| [Reports](STRATEGY-REPORTS.md) | The `/reports/` page — built. Open: server-side debts (streamed CSV, a plain GET for saved rows), sharing and linking saved reports, and the design rationale for association reports (the join is chosen, never defined), the filter, why a node editor was rejected, and cardinality |
| [Security](STRATEGY-SECURITY.md) | Threat model for a LAN-reachable deployment: trust boundary, invariants, and the one unbuilt bound (rate limiting; settings backups are built). **Read before widening the bind past `127.0.0.1`** |

## Implementation notes

| Document | Description |
|---|---|
| [Coding Conventions](CODING-CONVENTIONS.md) | The engineering rules this codebase follows (module structure, testing, dependency direction, error stance) |
| [Plan: shortest path between rooms](PLAN-connectivity.md) | **Start here for a new session on routing.** Everything planned there is built (doors, open areas with cuts and a hub, vertical links and the stack view, routing methods, saved routes, the width check, the SVG export); it records where things stand, the open decisions (storage type, wall clearance and corridor width inside rooms), the measurements, the routing-method research and its sources, and practical notes. Retired when the storage type is decided |
| [Plan: blueprints](PLAN-blueprints.md) | Room layouts stored as recipes against the room boundary and applied to other rooms (rotate, room-size adaptation), written into Revit by tasks. **Largely built** (stages 0 to 6, the server half of 7 and 8, the sheet library, tag rules, and the 2026-10-31 rework: a blueprint holds only its room, its sheets and tags are bound to it), the Revit side: room layout sheet creation has run in Revit end to end (2026-11-01), placing items has not; sets of adjacent rooms on shared sheets are built (2026-11-14) and not yet run in Revit; the plan keeps the decisions, the critique and what is not built. Room layout sheets are a separate track inside it |
| [Repository layout](REPO-LAYOUT.md) | What each top-level folder is, mapped to the pipeline stage it belongs to |
| [The Revit plugin](REVIT-BUTTONS.md) | The RoomMate panel in Revit: a section per button with its icon and a four-sentence summary, what every window does the same way, and where the log and settings are kept |
| [The extractor](EXTRACTOR.md) | Why extractor and server change together, and the Revit-side status: what has and has not been run in Revit |
| [The API](API.md) | The HTTP routes in one screen |
| [Routes and connections](ROUTES-AND-CONNECTIONS.md) | The viewer's route, reach, open-area and vertical-link tools |
| [Settings reference](SETTINGS.md) | Every setting the server and project files accept, with a sample of each |
| [MCP host setup](mcp-host-setup.md) | Client configs for Claude Code and Claude Desktop, plus build and verify steps |
| [Module plan](module-plan.html) | Interactive map of every module across the extractor, server and browser — sized by lines of code, with its import edges. Open the file in a browser. **Half generated, half written**: the measured `STATS` come from `scripts/module_stats.py` (`--check` reports drift, and catches a module added or renamed on one side only), while the blurbs and import edges are hand-written from the module headers — so treat a header as the source and this as the view |

A brief whose work has fully landed is deleted (git history keeps it), after its surviving
open items move into the strategy doc that owns them. Cross-references name the symbol, never a
`file.rs:NNN` line: the files move and the line numbers drift.
