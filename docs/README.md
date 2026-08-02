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
| [Entities](STRATEGY-ENTITIES.md) | What makes something a primary entity, what generalizes from rooms to doors and FFE, and how a Revit phase scopes a push _(**built** — each decision now records what shipped and where it departed from the sketch; read its "Deferred" list for what is still open, chiefly R4 and which room owns a door)_ |
| [Security](STRATEGY-SECURITY.md) | Threat model for the LAN-reachable deployment: trust boundary, invariants, settings backups, rate limiting |

## Implementation notes

| Document | Description |
|---|---|
| [Coding Conventions](CODING-CONVENTIONS.md) | The engineering rules this codebase follows (module structure, testing, dependency direction, error stance) |
| [Plan: phasing](PLAN-phasing.md) | **Built (P1–P7), extractor half unverified against Revit.** The ten decisions behind Revit phase support and the phases that implemented them, plus an "As built" section recording where the result departs from the plan |
| [Plan: generalisation](PLAN-generalisation.md) | **R1, R2 and R3 done; only R4 remains.** Four seams that grew room-shaped because rooms were the only primary entity. The two doors prerequisites landed ahead of any door code — read R2's outcome note for the tier-precedence rule the `Door` contract is written against (a tier wins only when it is `Present`), and R1's for why `list_models()` still means "models with rooms". R4 stays open until doors grow a reference source |

> **Reading either plan above: do not trust their `file.rs:NNN` deep
> links.** Both pin line numbers, and the files have moved underneath them —
> spot-checked 2026-07-26, most now land on unrelated code (e.g.
> `areas.rs:270` was cited as "`/areas` has no `revision` field" and is now
> `wall_zone`). The *file* and the *symbol name* in the surrounding prose are
> still right; search for the symbol rather than jumping to the line. New
> cross-references should name the symbol, not the line.

## Open handovers

- **[WebGL renderer](HANDOVER-webgl-renderer.md)** — replace the SVG plan
  renderer with WebGL (PixiJS), keeping the SVG export, the areas overlay and
  today's interaction behaviour. The case for it is one number: the *fitted*
  view, which is what the viewer shows on load, costs 4.3 µs/room on Canvas2D
  against ~40 ns on WebGL, so Canvas2D would miss the frame budget on the
  `big-plate` fixture today. Pan is not the reason — viewport culling already
  fixed that.
  **Built and shipped as the default (P0–P5), to
  [PLAN-webgl-renderer.md](PLAN-webgl-renderer.md)** — read that one for how,
  and for the two places it deliberately departs from the handover. The measured
  result is in [STRATEGY-BROWSER.md](STRATEGY-BROWSER.md) "Renderer": a fitted
  pan of `big-plate` went from **733 ms p95 to 1 ms**, against a ≤16 ms budget.
  `RENDERER = "svg"` still puts the old renderer back for re-measurement. Only
  P6 (delete the live SVG path) remains.

  It also records the decision that reaches furthest past this feature: the
  frontend's zero-build constraint is retired, and `src-js/` (TypeScript, Vite)
  is where new frontend code lands.

Everything else has landed and moved to [Superseded](Superseded/).

That is a statement about *handovers*, not about outstanding work — open items
otherwise live in the strategy doc that owns them rather than in a brief that
outlived its build. `HANDOVER-adjacency.md` is the example:
its two remaining false-positive checks went into
[Area calculation](STRATEGY-AREA-CALCULATION.md)'s "Open" section, beside the
`max_wall_thickness` value whose choice is what creates the risk they test.
Keeping them in a superseded brief would have buried a live item in an archive.

Handoff documents whose work has fully landed live in
[Superseded](Superseded/), and so does one **plan**:
`PLAN-handover-actioning.md`, closed with P1 through P10 all landed and every
handover it reviewed superseded alongside it. It is kept as the record of how
the UI restructure was sequenced and what was deliberately *not* built. The
other two plans stay live because each still carries an open item — R4 for
generalisation, the unverified extractor half for phasing.

Three of those are worth knowing about even though they are superseded,
because each leaves something recorded rather than pending:

- **[Room inspector](Superseded/HANDOVER-room-inspector.md)** — step 6, a
  checkbox property picker, was deliberately not built. Hide-empty plus the name
  filter covered the cases it was for. Act on it only if the need shows up.
- **[Viewport culling kill switch](Superseded/HANDOVER-culling-disable-switch.md)**
  — the switch is permanent (`CULL_ENABLED` in `index.html`) and that document is
  the method for re-measuring whenever the renderer changes. Last run:
  **16.5 ms/frame with culling, 912 ms without.**
- **[QA coverage of the secondary source](Superseded/HANDOVER-qa-cardinality-and-coverage.md)**
  — closed, but it records a mis-diagnosis worth not repeating: two drafts read
  `duplicate_link_values` as "the reference source has duplicates" when it means
  "the rooms do", and so proposed a cardinality *setting* while the loader was
  silently discarding duplicate and blank-id rows. The two sides of a join are
  different questions; a check named after the join does not say which side it
  inspects.

Area calculation no longer has a handover: its design moved into
[Area calculation](STRATEGY-AREA-CALCULATION.md), which is the live document and
carries the remaining open items in its "Open" section. The handover stays in
`Superseded/` as the record of how the decisions were reached.
