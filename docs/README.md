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
| [Browser](STRATEGY-BROWSER.md) | Open: serving and consuming the placement transform, level-of-detail, the framework fork — plus the hybrid renderer's coordinate/paint-order invariant |
| [MCP](STRATEGY-MCP.md) | Open: resources and prompts, document tools |
| [Authored](STRATEGY-AUTHORED.md) | User-authored data — connections, PDFs, hierarchy scopes. **Nothing here is built**; read it before building any of it |
| [Entities](STRATEGY-ENTITIES.md) | What makes something a primary entity, and what four of them proved comes for free. Open: door connectivity, design options, the two duHast changes FF&E waits on, FF&E at scale |
| [Security](STRATEGY-SECURITY.md) | Threat model for a LAN-reachable deployment: trust boundary, invariants, and the two unbuilt bounds (settings backups, rate limiting). **Read before widening the bind past `127.0.0.1`** |

## Implementation notes

| Document | Description |
|---|---|
| [Coding Conventions](CODING-CONVENTIONS.md) | The engineering rules this codebase follows (module structure, testing, dependency direction, error stance) |
| [MCP host setup](mcp-host-setup.md) | Client configs for Claude Code and Claude Desktop, plus build and verify steps |
| [Module plan](module-plan.html) | Interactive map of every module across the extractor, server and browser — sized by line count, with its import edges. Open the file in a browser; it is generated from the module headers, so treat a header as the source and this as the view |

## Archive

[Superseded/](Superseded/) holds the handover briefs and plans whose work has
fully landed — kept as the record of how each was sequenced and what was
deliberately *not* built. **Nothing there is live**, and the strategy docs
deliberately no longer link into it: a live item buried in an archive is a live
item nobody finds.

Two things follow from that:

- **When a brief is superseded, its surviving open items move out of it** into
  the strategy doc that owns them. The adjacency handover is the worked example:
  its two remaining false-positive checks now sit in
  [Area calculation](STRATEGY-AREA-CALCULATION.md)'s "Open" section, beside the
  `max_wall_thickness` value whose choice is what creates the risk they test.
- **Do not trust the archive's `file.rs:NNN` deep links.** They pin line numbers
  and the files have moved underneath them — most now land on unrelated code. The
  *file* and the *symbol name* in the surrounding prose are still right; search
  for the symbol. New cross-references should name the symbol, not the line.
